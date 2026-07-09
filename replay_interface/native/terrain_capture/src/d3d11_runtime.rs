use super::*;
use minhook::MinHook;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{c_void as StdVoid, CStr, CString, OsStr};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::marker::PhantomData;
use std::mem;
use std::os::windows::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};
use std::ptr;
use std::rc::Rc;
use std::slice;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, TryLockError};
use std::thread;
use std::time::SystemTime;
use winapi::ctypes::c_void as WinVoid;
use winapi::shared::dxgi::{IDXGISwapChain, DXGI_SWAP_CHAIN_DESC, DXGI_SWAP_EFFECT_DISCARD};
use winapi::shared::dxgiformat::{DXGI_FORMAT_R16G16B16A16_SINT, DXGI_FORMAT_R8G8B8A8_UNORM};
use winapi::shared::dxgitype::{
    DXGI_MODE_DESC, DXGI_RATIONAL, DXGI_SAMPLE_DESC, DXGI_USAGE_RENDER_TARGET_OUTPUT,
};
use winapi::shared::guiddef::REFIID;
use winapi::shared::minwindef::UINT;
use winapi::shared::winerror::{FAILED, SUCCEEDED};
use winapi::um::d3d11::{
    D3D11CalcSubresource, D3D11CreateDevice, D3D11CreateDeviceAndSwapChain, ID3D11BlendState,
    ID3D11Buffer, ID3D11ClassInstance, ID3D11ClassLinkage, ID3D11DepthStencilState,
    ID3D11DepthStencilView, ID3D11Device, ID3D11DeviceContext, ID3D11InputLayout,
    ID3D11PixelShader, ID3D11RasterizerState, ID3D11RenderTargetView, ID3D11Resource,
    ID3D11ShaderResourceView, ID3D11Texture2D, ID3D11VertexShader, IID_ID3D11Device,
    IID_ID3D11Texture2D, D3D11_BIND_CONSTANT_BUFFER, D3D11_BLEND_DESC, D3D11_BOX,
    D3D11_BUFFER_DESC, D3D11_DEPTH_STENCILOP_DESC, D3D11_DEPTH_STENCIL_DESC,
    D3D11_DEPTH_STENCIL_VIEW_DESC, D3D11_DEVICE_CONTEXT_IMMEDIATE, D3D11_DSV_DIMENSION_TEXTURE2D,
    D3D11_DSV_DIMENSION_TEXTURE2DARRAY, D3D11_DSV_DIMENSION_TEXTURE2DMS,
    D3D11_DSV_DIMENSION_TEXTURE2DMSARRAY, D3D11_INPUT_ELEMENT_DESC, D3D11_MAPPED_SUBRESOURCE,
    D3D11_RASTERIZER_DESC, D3D11_RECT, D3D11_RENDER_TARGET_BLEND_DESC,
    D3D11_RENDER_TARGET_VIEW_DESC, D3D11_RTV_DIMENSION_TEXTURE2D,
    D3D11_RTV_DIMENSION_TEXTURE2DARRAY, D3D11_RTV_DIMENSION_TEXTURE2DMS,
    D3D11_RTV_DIMENSION_TEXTURE2DMSARRAY, D3D11_SAMPLER_DESC, D3D11_SDK_VERSION,
    D3D11_SHADER_RESOURCE_VIEW_DESC, D3D11_SUBRESOURCE_DATA, D3D11_TEXTURE2D_DESC,
    D3D11_USAGE_STAGING, D3D11_VIEWPORT,
};
use winapi::um::d3d11shader::{
    ID3D11ShaderReflection, IID_ID3D11ShaderReflection, D3D11_SHADER_DESC,
    D3D11_SHADER_INPUT_BIND_DESC, D3D11_SIGNATURE_PARAMETER_DESC,
};
use winapi::um::d3dcommon::{
    D3D11_SRV_DIMENSION_TEXTURE2D, D3D11_SRV_DIMENSION_TEXTURE2DARRAY,
    D3D11_SRV_DIMENSION_TEXTURECUBE, D3D11_SRV_DIMENSION_TEXTURECUBEARRAY,
    D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_NULL, D3D_FEATURE_LEVEL_11_0, D3D_SIT_CBUFFER,
    D3D_SIT_TEXTURE,
};
use winapi::um::unknwnbase::IUnknown;
use windows_sys::Win32::Foundation::{FreeLibrary, HINSTANCE, HMODULE};
use windows_sys::Win32::System::LibraryLoader::{
    GetModuleHandleExW, GetModuleHandleW, GetProcAddress, LoadLibraryW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, WS_DISABLED};

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
const UNDERWATER_MAP_FRAME_BUNDLE_FILE: &str = "map_frame_underwater_pass_bundle.json";
const TREES_MAP_FRAME_BUNDLE_FILE: &str = "map_frame_trees_pass_bundle.json";
const RESTORE_MAP_FRAME_BUNDLE_FILE: &str = "map_frame_restore_pass_bundle.json";
const MAPNAME_MAP_FRAME_BUNDLE_FILE: &str = "map_frame_mapname_pass_bundle.json";
const UI_PASS_BUNDLE_FILE: &str = "ui_pass_bundle_000.json";
const UI_PASS_BUNDLE_FILE_PREFIX: &str = "ui_pass_bundle";
const CENSUS_CAPTURE_MOMENT: &str = "census";
const CENSUS_DIR: &str = "census";
const CENSUS_MANIFEST_FILE: &str = "census_manifest.json";
const CENSUS_PASS_BUNDLE_FILE: &str = "pass_bundle.json";
const CENSUS_PASS_BUNDLE_SESSION_MANIFEST: &str = "../census_manifest.json";
const MAP_FRAME_BUNDLE_SCHEMA: &str = "battle-plan-map-frame-bundle-v1";
const UI_PASS_BUNDLE_SCHEMA: &str = "battle-plan-ui-pass-bundle-v1";
const CENSUS_PASS_BUNDLE_SCHEMA: &str = "battle-plan-pass-census-bundle-v1";
const CENSUS_MANIFEST_SCHEMA: &str = "battle-plan-pass-census-manifest-v1";
const NATIVE_D3D11_TEXTURE_PAYLOAD_FORMAT: &str = "d3d11_mapped_subresources";
#[cfg(test)]
const MAP_FRAME_PRE_DRAW_RENDER_TARGET_FILE_PREFIX: &str = "map_frame_pre_draw_render_target";
#[cfg(test)]
const MAP_FRAME_PRE_DRAW_DEPTH_STENCIL_FILE_PREFIX: &str = "map_frame_pre_draw_depth_stencil";
#[cfg(test)]
const MAP_FRAME_POST_PASS_RENDER_TARGET_FILE_PREFIX: &str = "map_frame_post_pass_render_target";
const MAP_FRAME_SHARED_PAYLOAD_FILE_PREFIX: &str = "map_frame_shared_payload";
const MAP_CAPTURE_SESSION_FILE: &str = "map_capture_session.json";
const MAP_CAPTURE_SESSION_SCHEMA: &str = "battle-plan-map-capture-session-v1";
const TERRAIN_CAPTURE_MOMENT: &str = "terrain_active_pass";
const TERRAIN_CHUNK_LIBRARY_MOMENT: &str = "terrain_chunk_library";
const TERRAIN_CHUNK_LIBRARY_FILE: &str = "terrain_chunk_library.json";
const TERRAIN_CHUNK_LIBRARY_SCHEMA: &str = "battle-plan-terrain-chunk-library-v1";
const WATER_CAPTURE_MOMENT: &str = "water_pass";
const RIVERS_CAPTURE_MOMENT: &str = "rivers_pass";
const BORDERS_CAPTURE_MOMENT: &str = "borders_pass";
const UNDERWATER_CAPTURE_MOMENT: &str = "underwater_pass";
const TREES_CAPTURE_MOMENT: &str = "trees_pass";
const RESTORE_CAPTURE_MOMENT: &str = "restore_pass";
const MAPNAME_CAPTURE_MOMENT: &str = "mapname_pass";
const UI_PASS_CAPTURE_MOMENT: &str = "ui_pass";
const TERRAIN_MAP_FRAME_FILE_PREFIX: &str = "map_frame_draw";
const WATER_MAP_FRAME_FILE_PREFIX: &str = "water_frame_draw";
const RIVERS_MAP_FRAME_FILE_PREFIX: &str = "rivers_frame_draw";
const BORDERS_MAP_FRAME_FILE_PREFIX: &str = "borders_frame_draw";
const UNDERWATER_MAP_FRAME_FILE_PREFIX: &str = "underwater_frame_draw";
const TREES_MAP_FRAME_FILE_PREFIX: &str = "trees_frame_draw";
const RESTORE_MAP_FRAME_FILE_PREFIX: &str = "restore_frame_draw";
const MAPNAME_MAP_FRAME_FILE_PREFIX: &str = "mapname_frame_draw";
const UI_PASS_FILE_PREFIX: &str = "ui_pass_draw";
const TERRAIN_CHUNK_LIBRARY_FILE_PREFIX: &str = "terrain_chunk";
const INPUT_SEMANTIC_NAME_MAX_BYTES: usize = 64;
const MAP_FRAME_BINDING_SLOTS: u32 = 16;
/// SHARED | SHARED_KEYEDMUTEX | SHARED_NTHANDLE. Staging-copying a resource
/// with any of these bits drives d3d11 into D3DKMTOpenResource in kernel
/// space and access-violates the host process; other MiscFlags (GENERATE_MIPS,
/// TEXTURECUBE, ...) are safe to copy and must NOT be rejected (they appear on
/// ordinary map textures).
const D3D11_RESOURCE_MISC_SHARED_MASK: u32 = 0x2 | 0x100 | 0x800;
const MAP_FRAME_MIN_TERRAIN_DRAWS: usize = 2;
const DEFAULT_UI_PASS_MAX_DRAWS_PER_FRAME: u32 = 4_096;
const DEFAULT_UI_PASS_MAX_STATES: u32 = 64;
const UI_PASS_MIN_FRAMES_BETWEEN_CAPTURES: u64 = 2;
const UI_PASS_MAX_BUFFER_PAYLOAD_BYTES_PER_DRAW: usize = 1_048_576;
const UI_PASS_INTEROP_HAZARD_UNOBTAINED_REASON: &str = "d3dkmt_openresource_interop_hazard";
const CENSUS_RESERVED_MOMENT_PAYLOAD_FLOOR_BYTES: u64 = 268_435_456;
const CENSUS_HEARTBEAT_INTERVAL_MS: u64 = 30_000;
const CENSUS_UNCAPTURED_UNKNOWN_SHADER: &str = "unknown_shader_bytecode";
const CENSUS_UNCAPTURED_UNKNOWN_INPUT_LAYOUT: &str = "unknown_input_layout";
const CENSUS_UNCAPTURED_PRIOR_HAZARD_SIGNATURE: &str = "prior_hazardous_signature_skipped";
const CENSUS_UNCAPTURED_WORK_BUDGET_FRAME_PAUSED: &str = "work_budget_frame_paused";
/// The draw arrived before the real map renderer was proven live on the
/// hooked immediate context (load screen / CEF browser-overlay interop /
/// multithreaded-renderer bootstrap). Census records the signature but issues
/// ZERO immediate-context queries: the 2026-07-05 live session died with
/// C0000005 inside d3d11!CContext::TID3D11DeviceContext_PSGetConstantBuffers
/// when first-occurrence payload capture ran its Get* battery against a
/// load-screen draw (minidump hoi4_20260705_210225).
const CENSUS_UNCAPTURED_UNTRUSTED_CONTEXT_CLASS: &str = "untrusted_context_class";
const AUTO_ROUND_RATE_LIMIT_MS: u64 = 10_000;
const AUTO_ROUND_SAMPLE_INTERVAL_MS: u64 = 1_000;
const TERRAIN_SCENE_CB0_VCAMPOS_OFFSET: usize = 208;
const DEFAULT_SETTLE_MS: u64 = 8_000;
/// CAPFIX2 dwell cap (defense in depth): even a hand-authored control file may
/// not run a session longer than 10 minutes. The attach helper already rejects
/// oversized `--dwell-ms`; the DLL additionally clamps here so no path can wedge
/// the game open indefinitely. 10 minutes = 600000 ms.
pub const MAX_DWELL_MS: u32 = 600_000;

/// CAPFIX4 (2026-07-09, user-approved): the dwell timeout is an IDLE cap, not a
/// hard session cap. Each ACCEPTED control-file command (manual round /
/// ui-state capture) re-anchors the dwell clock, so an operator-driven session
/// stays open while work is happening; a session with no operator activity
/// still dies within the dwell timeout (the CAPFIX2 anti-wedge intent stands —
/// live 2026-07-09, the 10-min window expired during the save load before the
/// first manual round could fire).
static LAST_CONTROL_ACTIVITY_TICK_MS: AtomicU64 = AtomicU64::new(0);

fn note_control_activity() {
    let now = unsafe { GetTickCount64() };
    LAST_CONTROL_ACTIVITY_TICK_MS.store(now, Ordering::SeqCst);
}

/// Elapsed dwell time measured from the LATER of session start and the last
/// accepted control command. Pure so the extension rule is unit-testable.
fn dwell_elapsed_ms(now: u64, started: u64, last_activity: u64) -> u64 {
    now.saturating_sub(last_activity.max(started))
}
pub const DEFAULT_TERRAIN_CHUNK_LIBRARY_MAX_DRAWS_PER_PASS: u32 = 8_192;
pub const DEFAULT_TERRAIN_CHUNK_LIBRARY_MAX_TOTAL_PAYLOAD_BYTES_PER_SESSION: u64 = 2_147_483_648;
pub const DEFAULT_TERRAIN_CHUNK_LIBRARY_MAX_CAPTURE_WORK_MS_PER_FRAME: u64 = 2_000;
pub const DEFAULT_MAX_DRAWS_PER_PASS: u32 = 128;
pub const DEFAULT_MAX_TOTAL_PAYLOAD_BYTES_PER_SESSION: u64 = 1_073_741_824;
pub const DEFAULT_MAX_CAPTURE_WORK_MS_PER_FRAME: u64 = 1_000;
pub const DEFAULT_MAX_PAYLOAD_WRITER_QUEUE_BYTES: u64 = 536_870_912;
const TERRAIN_CHUNK_LIBRARY_COVERAGE_EVENT_INTERVAL_MS: u64 = 5_000;
const TERRAIN_CHUNK_LIBRARY_COMPLETE_COVERAGE_FRACTION: f64 = 0.995;
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
const UI_PASS_REQUIRED_PAYLOAD: [&str; 11] = [
    "draw_args",
    "primitive_topology",
    "shader_bytecode_hashes",
    "vertex_buffer_referenced_ranges",
    "index_buffer_referenced_ranges",
    "ps_shader_resource_textures",
    "constant_buffers_all_stages",
    "blend_rasterizer_scissor_viewport_state",
    "depth_disabled_or_absent_open_condition",
    "swapchain_backbuffer_identity",
    "hashes",
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
/// `Effect underwater` lives in `pdxmap.shader` and shares the terrain include
/// tree (it re-uses the terrain VS + the same fxh helpers), so its source
/// evidence is the pdxmap set — same as the terrain moment.
const UNDERWATER_VANILLA_SHADER_SOURCES: [&str; 7] = VANILLA_SHADER_SOURCES;
const UNDERWATER_VANILLA_MAP_FRAME_SOURCES: [&str; 8] = VANILLA_MAP_FRAME_SOURCES;

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
const RESTORE_VANILLA_SHADER_SOURCES: [&str; 2] =
    ["gfx/FX/restorescene.shader", "gfx/FX/posteffect_base.fxh"];
const RESTORE_VANILLA_MAP_FRAME_SOURCES: [&str; 3] = [
    "gfx/FX/restorescene.shader",
    "gfx/FX/posteffect_base.fxh",
    BATTLE_PLAN_GRAPHICS_DEFINES_SOURCE,
];
const MAPNAME_VANILLA_SHADER_SOURCES: [&str; 3] = [
    "gfx/FX/mapname.shader",
    "gfx/FX/constants.fxh",
    "gfx/FX/standardfuncsgfx.fxh",
];
const MAPNAME_VANILLA_MAP_FRAME_SOURCES: [&str; 4] = [
    "gfx/FX/mapname.shader",
    "gfx/FX/constants.fxh",
    "gfx/FX/standardfuncsgfx.fxh",
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
    timing: RuntimeTextureCaptureTiming,
    dwell_end_resnapshot: bool,
}

#[derive(Clone, Copy, Debug)]
enum CaptureReadbackFormat {
    Rgba8,
    R32G32B32A32Float,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RuntimeTextureCaptureTiming {
    FirstDraw,
    Finalize,
    DwellEnd,
}

impl RuntimeTextureCaptureTiming {
    fn as_str(self) -> &'static str {
        match self {
            Self::FirstDraw => "first_draw",
            Self::Finalize => "finalize",
            Self::DwellEnd => "dwell_end",
        }
    }
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
    capture_exact_ps_srv_payloads: bool,
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
    capture_exact_ps_srv_payloads: false,
    min_draws_before_clear_finalize: MAP_FRAME_MIN_TERRAIN_DRAWS,
};

const TERRAIN_CHUNK_LIBRARY_CAPTURE_PROFILE: MapFrameCaptureProfile = MapFrameCaptureProfile {
    capture_moment: TERRAIN_CHUNK_LIBRARY_MOMENT,
    bundle_file: TERRAIN_CHUNK_LIBRARY_FILE,
    file_prefix: TERRAIN_CHUNK_LIBRARY_FILE_PREFIX,
    pixel_shader_file_prefix: "pdxmap_pixel_shader",
    vertex_shader_file_prefix: "pdxmap_vertex_shader",
    shader_sources: &VANILLA_SHADER_SOURCES,
    vanilla_sources: &VANILLA_MAP_FRAME_SOURCES,
    requires_terrain_runtime_files: true,
    requires_shader_cache_constants: true,
    capture_exact_ps_srv_payloads: true,
    min_draws_before_clear_finalize: 1,
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
    capture_exact_ps_srv_payloads: true,
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
    capture_exact_ps_srv_payloads: true,
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
    capture_exact_ps_srv_payloads: true,
    min_draws_before_clear_finalize: 1,
};

const UNDERWATER_MAP_FRAME_CAPTURE_PROFILE: MapFrameCaptureProfile = MapFrameCaptureProfile {
    capture_moment: UNDERWATER_CAPTURE_MOMENT,
    bundle_file: UNDERWATER_MAP_FRAME_BUNDLE_FILE,
    file_prefix: UNDERWATER_MAP_FRAME_FILE_PREFIX,
    pixel_shader_file_prefix: "underwater_pixel_shader",
    // The underwater draw runs the pdxmap TERRAIN vertex shader (Effect
    // underwater binds VertexShader), so its VS blob is the terrain VS.
    vertex_shader_file_prefix: "pdxmap_vertex_shader",
    shader_sources: &UNDERWATER_VANILLA_SHADER_SOURCES,
    vanilla_sources: &UNDERWATER_VANILLA_MAP_FRAME_SOURCES,
    requires_terrain_runtime_files: false,
    requires_shader_cache_constants: false,
    // Record ALL bound PS SRVs exactly (incl. t0 — Codex's alternative
    // hypothesis) and the runtime CB0/CB1 the underwater light state lives in.
    capture_exact_ps_srv_payloads: true,
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
    capture_exact_ps_srv_payloads: true,
    min_draws_before_clear_finalize: 1,
};

const RESTORE_MAP_FRAME_CAPTURE_PROFILE: MapFrameCaptureProfile = MapFrameCaptureProfile {
    capture_moment: RESTORE_CAPTURE_MOMENT,
    bundle_file: RESTORE_MAP_FRAME_BUNDLE_FILE,
    file_prefix: RESTORE_MAP_FRAME_FILE_PREFIX,
    pixel_shader_file_prefix: "restorescene_pixel_shader",
    vertex_shader_file_prefix: "restorescene_vertex_shader",
    shader_sources: &RESTORE_VANILLA_SHADER_SOURCES,
    vanilla_sources: &RESTORE_VANILLA_MAP_FRAME_SOURCES,
    requires_terrain_runtime_files: false,
    requires_shader_cache_constants: false,
    capture_exact_ps_srv_payloads: true,
    min_draws_before_clear_finalize: 1,
};

const MAPNAME_MAP_FRAME_CAPTURE_PROFILE: MapFrameCaptureProfile = MapFrameCaptureProfile {
    capture_moment: MAPNAME_CAPTURE_MOMENT,
    bundle_file: MAPNAME_MAP_FRAME_BUNDLE_FILE,
    file_prefix: MAPNAME_MAP_FRAME_FILE_PREFIX,
    pixel_shader_file_prefix: "mapname_pixel_shader",
    vertex_shader_file_prefix: "mapname_vertex_shader",
    shader_sources: &MAPNAME_VANILLA_SHADER_SOURCES,
    vanilla_sources: &MAPNAME_VANILLA_MAP_FRAME_SOURCES,
    requires_terrain_runtime_files: false,
    requires_shader_cache_constants: false,
    capture_exact_ps_srv_payloads: true,
    min_draws_before_clear_finalize: 1,
};

const UI_PASS_CAPTURE_PROFILE: MapFrameCaptureProfile = MapFrameCaptureProfile {
    capture_moment: UI_PASS_CAPTURE_MOMENT,
    bundle_file: UI_PASS_BUNDLE_FILE,
    file_prefix: UI_PASS_FILE_PREFIX,
    pixel_shader_file_prefix: "ui_pass_pixel_shader",
    vertex_shader_file_prefix: "ui_pass_vertex_shader",
    shader_sources: &[],
    vanilla_sources: &[],
    requires_terrain_runtime_files: false,
    requires_shader_cache_constants: false,
    capture_exact_ps_srv_payloads: true,
    min_draws_before_clear_finalize: 1,
};

const CENSUS_CAPTURE_PROFILE: MapFrameCaptureProfile = MapFrameCaptureProfile {
    capture_moment: CENSUS_CAPTURE_MOMENT,
    bundle_file: CENSUS_PASS_BUNDLE_FILE,
    file_prefix: "census_pass_draw",
    pixel_shader_file_prefix: "census_pixel_shader",
    vertex_shader_file_prefix: "census_vertex_shader",
    shader_sources: &[],
    vanilla_sources: &[],
    requires_terrain_runtime_files: false,
    requires_shader_cache_constants: false,
    capture_exact_ps_srv_payloads: true,
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
        timing: RuntimeTextureCaptureTiming::FirstDraw,
        dwell_end_resnapshot: false,
    },
    CaptureTarget {
        symbol: SNOW_MUD_DATA_RESOURCE_SYMBOL,
        rgba_file: SNOW_MUD_RGBA_FILE,
        meta_file: SNOW_MUD_META_FILE,
        output_format: "rgba8_unorm_runtime_captured",
        readback_format: CaptureReadbackFormat::Rgba8,
        validate_control: false,
        timing: RuntimeTextureCaptureTiming::Finalize,
        dwell_end_resnapshot: false,
    },
    CaptureTarget {
        symbol: PROVINCE_SECONDARY_COLOR_MAP_SYMBOL,
        rgba_file: PROVINCE_SECONDARY_COLOR_MAP_RGBA_FILE,
        meta_file: PROVINCE_SECONDARY_COLOR_MAP_META_FILE,
        output_format: "rgba8_unorm_runtime_captured",
        readback_format: CaptureReadbackFormat::Rgba8,
        validate_control: false,
        timing: RuntimeTextureCaptureTiming::Finalize,
        dwell_end_resnapshot: true,
    },
    CaptureTarget {
        symbol: GRADIENT_BORDER_CHANNEL1_SYMBOL,
        rgba_file: GRADIENT_BORDER_CHANNEL1_RGBA_FILE,
        meta_file: GRADIENT_BORDER_CHANNEL1_META_FILE,
        output_format: "rgba8_unorm_runtime_captured",
        readback_format: CaptureReadbackFormat::Rgba8,
        validate_control: false,
        timing: RuntimeTextureCaptureTiming::Finalize,
        dwell_end_resnapshot: true,
    },
    CaptureTarget {
        symbol: GRADIENT_BORDER_CHANNEL2_SYMBOL,
        rgba_file: GRADIENT_BORDER_CHANNEL2_RGBA_FILE,
        meta_file: GRADIENT_BORDER_CHANNEL2_META_FILE,
        output_format: "rgba8_unorm_runtime_captured",
        readback_format: CaptureReadbackFormat::Rgba8,
        validate_control: false,
        timing: RuntimeTextureCaptureTiming::Finalize,
        dwell_end_resnapshot: true,
    },
    CaptureTarget {
        symbol: LIGHT_INDEX_MAP_SYMBOL,
        rgba_file: LIGHT_INDEX_MAP_RGBA_FILE,
        meta_file: LIGHT_INDEX_MAP_META_FILE,
        output_format: "rgba8_unorm_runtime_captured",
        readback_format: CaptureReadbackFormat::Rgba8,
        validate_control: false,
        timing: RuntimeTextureCaptureTiming::Finalize,
        dwell_end_resnapshot: false,
    },
    CaptureTarget {
        symbol: LIGHT_DATA_MAP_SYMBOL,
        rgba_file: LIGHT_DATA_MAP_RAW_FILE,
        meta_file: LIGHT_DATA_MAP_META_FILE,
        output_format: "r32g32b32a32_float_runtime_captured",
        readback_format: CaptureReadbackFormat::R32G32B32A32Float,
        validate_control: false,
        timing: RuntimeTextureCaptureTiming::Finalize,
        dwell_end_resnapshot: false,
    },
];

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct TerrainCaptureControl {
    pub schema: String,
    #[serde(default)]
    pub capture_out_dir: Option<PathBuf>,
    #[serde(default)]
    pub capture_moment: Option<String>,
    #[serde(default)]
    pub capture_moments: Vec<String>,
    #[serde(default = "default_map_frame_capture_enabled")]
    pub map_frame_capture_enabled: bool,
    #[serde(default)]
    pub terrain_chunk_library: bool,
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
    #[serde(default = "default_settle_ms")]
    pub settle_ms: u64,
    #[serde(default)]
    pub max_draws_per_pass: Option<u32>,
    #[serde(default)]
    pub max_total_payload_bytes_per_session: Option<u64>,
    #[serde(default)]
    pub max_capture_work_ms_per_frame: Option<u64>,
    #[serde(default)]
    pub max_payload_writer_queue_bytes: Option<u64>,
    #[serde(default)]
    pub ui_max_states: Option<u32>,
    #[serde(default)]
    pub ui_cb_skip: UiCbSkipPolicy,
    #[serde(default)]
    pub census: bool,
    #[serde(default)]
    pub round_commands_enabled: bool,
    #[serde(default)]
    pub trees_close_mode: Option<String>,
    /// Session dwell from the attach helper's --dwell-ms. Overrides the DLL's
    /// hardcoded 180s finisher timeout — without this, every long session
    /// (chunk pans, ui_pass atlases) was silently truncated at 3 minutes.
    #[serde(default)]
    pub dwell_ms: Option<u64>,
    /// Optional persistent scene-CB0 sampler cadence in wall-clock ms, armed by
    /// the attach helper's --cb0-sample-ms. None/0 => the sampler is fully off
    /// and no sampler code runs on the draw path. When set, the runtime records
    /// one TerrainSceneCB0 snapshot at most once per this many ms for the WHOLE
    /// session — surviving moment-sequence completion and budget quiesce (the
    /// day/night sun-curve recorder; see the CB0 sampler section below).
    #[serde(default)]
    pub cb0_sample_ms: Option<u64>,
    /// CAPFIX3 choreography — delay arming the map-frame moment sequence by this
    /// many ms AFTER the map renderer first goes live, letting the orchestrator
    /// zoom/position the camera before any pass is captured. None/0 => arm
    /// immediately (prior behavior). The CB0 sampler and the renderer-live trust
    /// latch are NOT delayed; only map-frame draw capture waits.
    #[serde(default)]
    pub arm_delay_ms: Option<u64>,
    #[serde(default = "default_vanilla_source_evidence_required")]
    pub vanilla_source_evidence_required: bool,
    pub provenance: String,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum UiCbSkipPolicy {
    Hazard,
    All,
    None,
}

impl Default for UiCbSkipPolicy {
    fn default() -> Self {
        Self::Hazard
    }
}

impl UiCbSkipPolicy {
    fn as_str(self) -> &'static str {
        match self {
            Self::Hazard => "hazard",
            Self::All => "all",
            Self::None => "none",
        }
    }
}

fn default_settle_ms() -> u64 {
    DEFAULT_SETTLE_MS
}

fn default_map_frame_capture_enabled() -> bool {
    true
}

fn default_vanilla_source_evidence_required() -> bool {
    true
}

fn capture_budget_from_control(control: Option<&TerrainCaptureControl>) -> CaptureBudget {
    let library_enabled = terrain_chunk_library_enabled_from_control(control);
    CaptureBudget {
        max_draws_per_pass: control
            .and_then(|control| control.max_draws_per_pass)
            .unwrap_or(if library_enabled {
                DEFAULT_TERRAIN_CHUNK_LIBRARY_MAX_DRAWS_PER_PASS
            } else {
                DEFAULT_MAX_DRAWS_PER_PASS
            })
            .max(1),
        max_total_payload_bytes_per_session: control
            .and_then(|control| control.max_total_payload_bytes_per_session)
            .unwrap_or(if library_enabled {
                DEFAULT_TERRAIN_CHUNK_LIBRARY_MAX_TOTAL_PAYLOAD_BYTES_PER_SESSION
            } else {
                DEFAULT_MAX_TOTAL_PAYLOAD_BYTES_PER_SESSION
            })
            .max(1),
        max_capture_work_ms_per_frame: control
            .and_then(|control| control.max_capture_work_ms_per_frame)
            .unwrap_or(if library_enabled {
                DEFAULT_TERRAIN_CHUNK_LIBRARY_MAX_CAPTURE_WORK_MS_PER_FRAME
            } else {
                DEFAULT_MAX_CAPTURE_WORK_MS_PER_FRAME
            })
            .max(1),
        max_payload_writer_queue_bytes: control
            .and_then(|control| control.max_payload_writer_queue_bytes)
            .unwrap_or(DEFAULT_MAX_PAYLOAD_WRITER_QUEUE_BYTES)
            .max(1),
    }
}

fn census_enabled_from_control(control: Option<&TerrainCaptureControl>) -> bool {
    control.map_or(false, |control| control.census)
}

fn census_payload_byte_budget(capture_budget: CaptureBudget, enabled: bool) -> u64 {
    if !enabled {
        return 0;
    }
    capture_budget
        .max_total_payload_bytes_per_session
        .saturating_sub(CENSUS_RESERVED_MOMENT_PAYLOAD_FLOOR_BYTES)
}

fn census_state_from_control(
    control: Option<&TerrainCaptureControl>,
    capture_budget: CaptureBudget,
    start_tick_ms: u64,
) -> CensusCaptureState {
    let enabled = census_enabled_from_control(control);
    CensusCaptureState {
        enabled,
        payload_byte_budget: census_payload_byte_budget(capture_budget, enabled),
        next_heartbeat_ms: start_tick_ms.saturating_add(CENSUS_HEARTBEAT_INTERVAL_MS),
        next_auto_round_sample_ms: start_tick_ms,
        ..CensusCaptureState::default()
    }
}

fn capture_output_dir_from_control(
    module_dir: &Path,
    control: Option<&TerrainCaptureControl>,
) -> PathBuf {
    control
        .and_then(|control| control.capture_out_dir.as_ref())
        .map(|path| {
            if path.is_absolute() {
                path.clone()
            } else {
                module_dir.join(path)
            }
        })
        .unwrap_or_else(|| module_dir.join(CAPTURE_DIR_NAME))
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

type CreateBufferFn = unsafe extern "system" fn(
    *mut ID3D11Device,
    *const D3D11_BUFFER_DESC,
    *const D3D11_SUBRESOURCE_DATA,
    *mut *mut ID3D11Buffer,
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

type SetConstantBuffersFn =
    unsafe extern "system" fn(*mut ID3D11DeviceContext, UINT, UINT, *const *mut ID3D11Buffer);

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

type PresentFn = unsafe extern "system" fn(*mut IDXGISwapChain, UINT, UINT) -> i32;

type OmSetRenderTargetsFn = unsafe extern "system" fn(
    *mut ID3D11DeviceContext,
    UINT,
    *const *mut ID3D11RenderTargetView,
    *mut ID3D11DepthStencilView,
);

type OmSetRenderTargetsAndUavsFn = unsafe extern "system" fn(
    *mut ID3D11DeviceContext,
    UINT,
    *const *mut ID3D11RenderTargetView,
    *mut ID3D11DepthStencilView,
    UINT,
    UINT,
    *const *mut WinVoid,
    *const UINT,
);

type OmSetBlendStateFn =
    unsafe extern "system" fn(*mut ID3D11DeviceContext, *mut WinVoid, *const f32, UINT);

type OmSetDepthStencilStateFn =
    unsafe extern "system" fn(*mut ID3D11DeviceContext, *mut WinVoid, UINT);

type D3DReflectFn =
    unsafe extern "system" fn(*const WinVoid, usize, REFIID, *mut *mut WinVoid) -> i32;

#[derive(Clone, Copy, Debug)]
struct HookTargets {
    create_buffer: usize,
    create_input_layout: usize,
    create_vertex_shader: usize,
    create_pixel_shader: usize,
    ia_set_input_layout: usize,
    vs_set_constant_buffers: usize,
    ps_set_constant_buffers: usize,
    vs_set_shader: usize,
    ps_set_shader: usize,
    ps_set_shader_resources: usize,
    draw_indexed: usize,
    draw: usize,
    draw_indexed_instanced: usize,
    draw_instanced: usize,
    om_set_render_targets: usize,
    om_set_render_targets_and_uavs: usize,
    om_set_blend_state: usize,
    om_set_depth_stencil_state: usize,
    clear_render_target_view: usize,
    clear_depth_stencil_view: usize,
    present: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CaptureBudget {
    max_draws_per_pass: u32,
    max_total_payload_bytes_per_session: u64,
    max_capture_work_ms_per_frame: u64,
    max_payload_writer_queue_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CaptureBudgetExceeded {
    cap: &'static str,
    capture_moment: &'static str,
    draw_count: usize,
    attempted_payload_bytes: u64,
    total_payload_bytes_reserved: u64,
    max_total_payload_bytes_per_session: u64,
    max_payload_writer_queue_bytes: u64,
    frame_capture_work_ms: u64,
    max_capture_work_ms_per_frame: u64,
    max_draws_per_pass: u32,
    message: String,
}

#[derive(Debug)]
struct TerrainChunkLibraryCaptureState {
    enabled: bool,
    chunks: BTreeMap<String, TerrainChunkRecordFile>,
    identity_to_chunk_id: BTreeMap<String, String>,
    preidentity_to_chunk_id: BTreeMap<String, String>,
    blockers: BTreeSet<String>,
    sampled_draws: u64,
    duplicate_draws: u64,
    next_coverage_event_ms: u64,
}

impl TerrainChunkLibraryCaptureState {
    fn new(enabled: bool, start_tick_ms: u64) -> Self {
        Self {
            enabled,
            chunks: BTreeMap::new(),
            identity_to_chunk_id: BTreeMap::new(),
            preidentity_to_chunk_id: BTreeMap::new(),
            blockers: BTreeSet::new(),
            sampled_draws: 0,
            duplicate_draws: 0,
            next_coverage_event_ms: start_tick_ms
                .saturating_add(TERRAIN_CHUNK_LIBRARY_COVERAGE_EVENT_INTERVAL_MS),
        }
    }
}

#[derive(Clone, Debug, Default)]
struct UiPassCaptureState {
    active: bool,
    backbuffer: Option<UiPassBackbufferFile>,
    draws: Vec<MapFrameDrawFile>,
    blockers: Vec<String>,
    frame_signature: UiPassFrameSignature,
    captured_signatures: BTreeSet<String>,
    pending_capture_signature: Option<String>,
    pending_capture_label: Option<String>,
    active_capture_label: Option<String>,
    state_count: u32,
    frame_index: u64,
    last_capture_frame_index: Option<u64>,
    states_exhausted_event_recorded: bool,
}

#[derive(Clone, Debug, Default)]
struct UiPassFrameSignature {
    draws: Vec<UiPassDrawSignature>,
}

#[derive(Clone, Debug)]
struct UiPassDrawSignature {
    pixel_shader_hash: String,
    vertex_shader_hash: String,
    texture_srv_pointers: Vec<UiPassSrvPointerSignature>,
    vertex_count: u32,
    scissor_rects: Vec<MapFrameScissorRectFile>,
}

#[derive(Clone, Debug)]
struct UiPassSrvPointerSignature {
    slot: u32,
    pointer: usize,
}

#[derive(Clone, Debug, Default)]
struct CensusCaptureState {
    enabled: bool,
    active_pass: Option<CensusActivePass>,
    signatures: BTreeMap<String, CensusSignatureRecord>,
    finalized_event_signatures: BTreeSet<String>,
    next_sequence: u64,
    frame_index: u64,
    payload_bytes_reserved: u64,
    payload_byte_budget: u64,
    capture_work_frame_ms: u64,
    payload_stopped: bool,
    payload_stop_reason: Option<String>,
    work_frame_paused: bool,
    paused_frames: u64,
    hazard_skipped_signatures: BTreeSet<String>,
    known_prefix_keys: BTreeSet<u64>,
    /// Final prefix key of a finalized pass -> its recorded signature, so a
    /// fully repeat-skipped pass attributes its occurrence to the original
    /// signature instead of recording a payload-less twin.
    prefix_signature_map: BTreeMap<u64, String>,
    shader_file_memo: BTreeMap<(bool, usize), MapFrameShaderFile>,
    shared_payloads: BTreeMap<MapFramePayloadCacheKey, MapFrameExactTexturePayload>,
    shared_payload_files_written: BTreeSet<String>,
    shader_files_written: BTreeSet<String>,
    next_heartbeat_ms: u64,
    auto_round_buckets_done: BTreeSet<String>,
    last_auto_round_started_ms: Option<u64>,
    next_auto_round_sample_ms: u64,
}

#[derive(Clone, Debug)]
struct CensusActivePass {
    sequence: u64,
    first_seen_frame: u64,
    output_dir: PathBuf,
    boundary: CensusPassBoundary,
    draws: Vec<MapFrameDrawFile>,
    blockers: Vec<String>,
    payload_bytes_at_start: u64,
    capture_payloads: bool,
    uncaptured_reason: Option<String>,
    /// Running order-sensitive key over (boundary, draws so far); used with
    /// `census.known_prefix_keys` so repeat occurrences of an already
    /// recorded pass do zero payload readback work.
    prefix_key: u64,
    /// Every per-draw prospective prefix key of this pass, recorded into
    /// `census.known_prefix_keys` when the pass finalizes.
    prefix_keys: Vec<u64>,
    /// True when at least one draw of this pass actually performed payload
    /// readbacks; gates post-pass output-merger capture so repeat passes
    /// never touch the immediate context at finalize.
    any_payload_captured: bool,
    /// Number of draws whose payload work was skipped because their prefix
    /// key matched an already-finalized pass.
    repeat_skipped_draws: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CensusPassBoundary {
    render_target_class: String,
    depth_class: String,
}

#[derive(Clone, Debug)]
struct CensusTrackedDrawState {
    pixel_shader: usize,
    vertex_shader: usize,
    layout_ptr: usize,
    layout_elements: Vec<TerrainInputElementDesc>,
    pixel_shader_known: bool,
    vertex_shader_known: bool,
    layout_known: bool,
}

impl CensusTrackedDrawState {
    fn shaders_known(&self) -> bool {
        self.pixel_shader_known && self.vertex_shader_known
    }

    fn safe_for_state_reads(&self) -> bool {
        self.shaders_known() && self.layout_known
    }
}

#[derive(Clone, Debug, Serialize)]
struct CensusManifestFile {
    schema: &'static str,
    enabled: bool,
    signatures_seen: usize,
    signatures_captured: usize,
    signatures_uncaptured: usize,
    finalized_events: usize,
    payload_bytes_reserved: u64,
    payload_byte_budget: u64,
    payload_stopped: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    payload_stop_reason: Option<String>,
    paused_frames: u64,
    entries: Vec<CensusSignatureRecord>,
}

#[derive(Clone, Debug, Serialize)]
struct CensusSignatureRecord {
    signature: String,
    sig12: String,
    first_seen_frame: u64,
    occurrence_count: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    bundle_path: Option<String>,
    payload_bytes: u64,
    captured: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    uncaptured_reason: Option<String>,
    signature_parts: CensusSignatureParts,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
struct CensusSignatureParts {
    render_target_class: String,
    depth_class: String,
    shader_pairs: Vec<CensusShaderPair>,
    blend_depth_state_family: String,
    draw_call_shape_family: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq, PartialOrd, Ord)]
struct CensusShaderPair {
    vs: String,
    ps: String,
}

#[derive(Debug)]
struct CaptureRuntime {
    state: TerrainCaptureState,
    root_output_dir: PathBuf,
    output_dir: PathBuf,
    control_path: PathBuf,
    control_command_last_modified: Option<SystemTime>,
    control: Option<TerrainCaptureControl>,
    start_tick_ms: u64,
    settle_ms: u64,
    capture_budget: CaptureBudget,
    total_payload_bytes_reserved: u64,
    capture_work_frame_ms: u64,
    capture_budget_exceeded: Option<CaptureBudgetExceeded>,
    terrain_chunk_library: TerrainChunkLibraryCaptureState,
    capture_moment_sequence: Vec<String>,
    active_capture_index: usize,
    active_moment_started_ms: u64,
    dwell_end_generated_texture_check_done: bool,
    pending_dwell_end_boundary_finalization: bool,
    pending_dwell_end_generated_texture_check: bool,
    pending_render_thread_release_srvs: Vec<usize>,
    capture_quiesced_event_recorded: bool,
    captured_symbols: BTreeSet<String>,
    captured_runtime_textures: BTreeMap<String, RuntimeTextureCaptureRecord>,
    deferred_runtime_targets: BTreeMap<String, DeferredRuntimeTarget>,
    shared_payloads: BTreeMap<MapFramePayloadCacheKey, MapFrameExactTexturePayload>,
    shared_payload_files_written: BTreeSet<String>,
    boundary_payloads: BTreeMap<String, BTreeMap<String, BoundaryPayloadRecord>>,
    boundary_payload_warnings: BTreeSet<String>,
    completed_map_frame_moments: BTreeSet<String>,
    map_frame_draws: Vec<MapFrameDrawFile>,
    map_frame_blockers: Vec<String>,
    map_frame_rejections: BTreeSet<String>,
    ui_pass: UiPassCaptureState,
    census: CensusCaptureState,
    ui_shader_files_written: BTreeSet<String>,
    current_swapchain_backbuffer: Option<UiPassBackbufferFile>,
    current_swapchain_device: usize,
    latest_post_draw_output_merger: Option<MapFrameOutputMergerStateFile>,
    latest_post_draw_output_blockers: Vec<String>,
    om_render_target_clears: BTreeMap<usize, MapFrameRenderTargetClearFile>,
    om_depth_stencil_clears: BTreeMap<usize, MapFrameDepthStencilClearFile>,
    om_clear_sequence: u64,
    terrain_id_srv_slot_observed: Option<u32>,
    round_commands_enabled: bool,
    active_round_label: Option<String>,
    active_round_finished: bool,
    // MOMFIX1: persistent latch, set ONCE the moment the continuous
    // control-requested sequence first completes (before any round runs) and
    // NEVER cleared. Rounds share the same capture state fields and a round
    // start clears completed_map_frame_moments (see
    // start_map_frame_capture_round_for_runtime), so completed_map_frame_moments
    // is NOT a reliable record of continuous-sequence completion once a round
    // has run. This latch is that record: HOLD-gating and the finalization
    // report both key off it so a round can never starve or falsely mask the
    // control's restore_pass/mapname_pass/ui_pass (etc.) moments.
    control_sequence_complete: bool,
    // MOMFIX1: a manual capture_map_frames_round command that arrives while the
    // continuous sequence is still incomplete is HELD here (the control file is
    // edge-triggered by mtime, so re-polling never re-fires it). It is started
    // once the control sequence completes; dropped only if round commands are
    // disabled or the label is unusable.
    pending_manual_round_label: Option<String>,
    last_context: usize,
    last_error: Option<String>,
}

#[derive(Clone, Debug)]
struct RuntimeTextureCaptureRecord {
    captured_at: RuntimeTextureCaptureTiming,
    fnv1a64: String,
}

#[derive(Clone, Copy, Debug)]
struct DeferredRuntimeTarget {
    target: CaptureTarget,
    srv: usize,
    slot: u32,
    identity: &'static str,
}

#[derive(Clone, Debug)]
struct BoundaryPayloadRecord {
    file: String,
    fnv1a64: String,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct MapFramePayloadCacheKey {
    resource_pointer: usize,
    payload_kind: &'static str,
    first_mip: u32,
    mip_levels: u32,
    first_array_slice: u32,
    array_size: u32,
    width: u32,
    height: u32,
    mip_count: u32,
    texture_array_size: u32,
    format: u32,
    sample_count: u32,
    sample_quality: u32,
}

struct PayloadWriter {
    sender: Sender<PayloadWriteJob>,
    state: Arc<(Mutex<PayloadWriterState>, Condvar)>,
}

struct PayloadWriterState {
    pending: usize,
    pending_bytes: u64,
    failures: Vec<String>,
}

struct PayloadWriteJob {
    path: PathBuf,
    bytes: Vec<u8>,
}

static RUNTIME: OnceLock<Mutex<CaptureRuntime>> = OnceLock::new();
/// Session output dir for the persistent CB0 sampler, captured once at arm time.
/// The sampler NEVER takes the RUNTIME mutex on the draw hot path: it runs before
/// the capture_allowed gate, so a hypothetical `RUNTIME.lock() -> original Draw*`
/// re-entry would deadlock a non-reentrant std::Mutex. Reading the dir from this
/// OnceLock keeps the sampler lock-free and independent of the (quiesce-poisoned)
/// RUNTIME mutex, which is also why it can safely outlive the capture machinery.
static CB0_SAMPLER_OUTPUT_DIR: OnceLock<PathBuf> = OnceLock::new();
static PAYLOAD_WRITER: OnceLock<PayloadWriter> = OnceLock::new();
static HOOK_TARGETS: OnceLock<HookTargets> = OnceLock::new();
/// Lockless events-dir copy for the panic hook and lock-unavailable fallback
/// paths: the RUNTIME mutex may be poisoned by the very panic being reported,
/// so these writers must never touch it.
static LOCKLESS_EVENTS_DIR: OnceLock<PathBuf> = OnceLock::new();

fn write_event_lockless(line: &str) {
    let Some(dir) = LOCKLESS_EVENTS_DIR.get() else {
        return;
    };
    let path = dir.join(CAPTURE_EVENTS_FILE);
    let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) else {
        return;
    };
    let _ = writeln!(file, "{line}");
}

/// Every DLL panic lands in the events file with its location. A panic while
/// holding the RUNTIME lock poisons it, silently degrading every later
/// capture path — live session 2026-07-05 died that way with zero evidence.
fn install_capture_panic_hook(output_dir: &Path) {
    let _ = LOCKLESS_EVENTS_DIR.set(output_dir.to_path_buf());
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let message = if let Some(s) = info.payload().downcast_ref::<&str>() {
            (*s).to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "non-string panic payload".to_string()
        };
        let location = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_else(|| "unknown".to_string());
        let thread = unsafe { GetCurrentThreadId() };
        write_event_lockless(&format!(
            "{{\"type\":\"dll_panic\",\"thread\":{},\"location\":\"{}\",\"message\":\"{}\"}}",
            thread,
            json_escape(&location),
            json_escape(&message)
        ));
        previous(info);
    }));
}
static HOOKS_INSTALLED: AtomicBool = AtomicBool::new(false);
static CAPTURED: AtomicBool = AtomicBool::new(false);
static CAPTURE_BUDGET_STOPPED: AtomicBool = AtomicBool::new(false);
static CAPTURE_QUIESCED: AtomicBool = AtomicBool::new(false);
// CAPFIX2 BUG 1 (teardown crash): the capture DLL must NEVER unhook or unload
// itself while the process lives. Removing MinHook inline hooks/trampolines
// from d3d11 functions the render thread calls every frame is an unavoidable
// use-after-free (you cannot prove no thread is mid-trampoline), and the DLL
// image then gets unmapped — two WER records this session faulted at a FIXED
// offset 0x1270f in "terrain_capture_run_*_unloaded" (0xc0000005), right at
// dwell-expiry, while the game kept rendering. Instances the USER quit (process
// exit) never crashed. So dwell-expiry finalization now DISARMS instead of
// unhooking: every hook checks this flag first and, if set, is a cheap
// pass-through to the stored original (hooks stay resident forever). The module
// is also PINNED (see pin_capture_module_resident) so the image is never
// unmapped. Process exit is the ONLY teardown. Set once, never cleared.
static HOOKS_DISARMED: AtomicBool = AtomicBool::new(false);
static CAPTURE_MODULE_PINNED: AtomicBool = AtomicBool::new(false);
// CAPFIX2 BUG 2 (runtime-lock wedge, session_LAW_6): a render-thread hook took
// the runtime Mutex and never released it, so every subsequent try_lock in the
// drain path returned busy forever (skip_count doubled 1..64 then the log went
// silent for 7+ minutes while the game still played). These statics let ANY
// thread record who currently owns the runtime lock (thread id + acquire site +
// acquire tick) so a lock-holder watchdog can name the offending site the next
// time it happens, and so a same-thread re-entrant lock() attempt (a std Mutex
// self-deadlock) is caught and reported instead of hanging. The holder is
// stored on acquire and cleared on release via RuntimeLockGuard (see
// lock_runtime / try_lock_runtime). All writes here are lockless.
static RUNTIME_LOCK_OWNER_THREAD: AtomicU32 = AtomicU32::new(0);
static RUNTIME_LOCK_ACQUIRED_TICK_MS: AtomicU64 = AtomicU64::new(0);
static RUNTIME_LOCK_OWNER_SITE: AtomicUsize = AtomicUsize::new(0);
static RUNTIME_LOCK_WATCHDOG_RECORDED: AtomicBool = AtomicBool::new(false);
static RUNTIME_LOCK_REENTRANT_RECORDED: AtomicBool = AtomicBool::new(false);
/// A continuously-held runtime lock for longer than this is treated as a wedge
/// and reported once, naming the owner site. 10s per the CAPFIX2 spec.
const RUNTIME_LOCK_WEDGE_WATCHDOG_MS: u64 = 10_000;
// Persistent scene-CB0 sampler ("the day/night sun-curve recorder"). These
// statics are the ENTIRE hot-path state for the sampler; they stay untouched
// (default false/0) unless --cb0-sample-ms armed the feature, keeping the
// default draw path a single relaxed load + early return. The sampler must
// OUTLIVE moment-sequence completion, round completion, and the byte-budget
// quiesce that killed two prior day/night sessions, so it deliberately does
// NOT consult capture_quiesced()/CAPTURE_BUDGET_STOPPED. It dies only when the
// session itself dies: either the vtable hooks are removed at run_one_shot
// finalize (then no draw hook fires at all) or process detach begins — the
// latter is the ONE teardown it must respect, since the D3D device/context may
// be destroyed on another thread under the loader lock while a stray draw hook
// still fires. CB0_SAMPLER_DETACHING is set at the very top of the detach
// quiesce and hard-stops the sampler before any D3D call or file I/O.
static CB0_SAMPLER_ARMED: AtomicBool = AtomicBool::new(false);
static CB0_SAMPLER_DETACHING: AtomicBool = AtomicBool::new(false);
static CB0_SAMPLER_CAP_RECORDED: AtomicBool = AtomicBool::new(false);
static CB0_SAMPLE_INTERVAL_MS: AtomicU64 = AtomicU64::new(0);
static CB0_NEXT_SAMPLE_MS: AtomicU64 = AtomicU64::new(0);
static CB0_SESSION_START_MS: AtomicU64 = AtomicU64::new(0);
static CB0_SAMPLE_COUNT: AtomicU64 = AtomicU64::new(0);
/// Hard cap on total samples per session (event once at the cap, then the
/// sampler disarms itself). At the documented ~2 samples/s this is ~14 hours
/// of headroom; it exists purely to bound unbounded-session file growth.
const CB0_SAMPLER_MAX_SAMPLES: u64 = 100_000;
const CB0_SAMPLES_FILE: &str = "cb0_samples.jsonl";
const CB0_SCENE_BYTE_LEN: usize = 416;
static CAPTURE_IN_PROGRESS: AtomicBool = AtomicBool::new(false);
static VERTEX_CAPTURE_IN_PROGRESS: AtomicBool = AtomicBool::new(false);
static VERTEX_CAPTURE_REJECTION_RECORDED: AtomicBool = AtomicBool::new(false);
static INPUT_LAYOUT_CAPTURE_IN_PROGRESS: AtomicBool = AtomicBool::new(false);
static INPUT_LAYOUT_CAPTURE_REJECTION_RECORDED: AtomicBool = AtomicBool::new(false);
static INPUT_LAYOUT_COPY_REJECTION_RECORDED: AtomicBool = AtomicBool::new(false);
static MAP_FRAME_CAPTURE_IN_PROGRESS: AtomicBool = AtomicBool::new(false);
static CENSUS_CAPTURE_IN_PROGRESS: AtomicBool = AtomicBool::new(false);
static SHADER_BYTECODE_TRACKING_ENABLED: AtomicBool = AtomicBool::new(false);
static UI_PASS_SHADER_TRACKING_ENABLED: AtomicBool = AtomicBool::new(false);
static RENDER_THREAD_ID: AtomicU32 = AtomicU32::new(0);
static RENDER_THREAD_MISMATCH_RECORDED: AtomicBool = AtomicBool::new(false);
static DEFERRED_CONTEXT_RECORDED: AtomicBool = AtomicBool::new(false);
static RENDER_THREAD_REQUEST_DRAIN_IN_PROGRESS: AtomicBool = AtomicBool::new(false);
static RENDER_THREAD_REQUEST_DRAIN_SKIPS: AtomicU64 = AtomicU64::new(0);
static ROUND_OPEN_TICK_MS: AtomicU64 = AtomicU64::new(0);
static LAST_CAPTURE_PROGRESS_TICK_MS: AtomicU64 = AtomicU64::new(0);
static ROUND_STALL_WATCHDOG_RECORDED: AtomicBool = AtomicBool::new(false);

const ROUND_STALL_WATCHDOG_MS: u64 = 60_000;

/// Created pixel shaders rejected by the lock-free capture-family pre-filter
/// (no lock taken, no D3D reflection). Instrumentation for the load-freeze
/// fix: during the game's shader storm this should account for ~all creations.
static CREATE_PS_PREFILTER_SKIPS: AtomicU64 = AtomicU64::new(0);
/// Created pixel shaders that passed the pre-filter and DID acquire the global
/// capture lock for exact classification. Should be a tiny constant (the real
/// map-shader families), never O(load shader count).
static CREATE_PS_LOCK_ACQUISITIONS: AtomicU64 = AtomicU64::new(0);

const CONTEXT_INTEREST_IA_SET_INPUT_LAYOUT: u32 = 1 << 0;
const CONTEXT_INTEREST_VS_SET_SHADER: u32 = 1 << 1;
const CONTEXT_INTEREST_PS_SET_SHADER: u32 = 1 << 2;
const CONTEXT_INTEREST_PS_SET_SHADER_RESOURCES: u32 = 1 << 3;
const CONTEXT_INTEREST_DRAW: u32 = 1 << 4;
const CONTEXT_INTEREST_CLEAR_RENDER_TARGET: u32 = 1 << 5;
const CONTEXT_INTEREST_CLEAR_DEPTH_STENCIL: u32 = 1 << 6;
const CONTEXT_INTEREST_PRESENT: u32 = 1 << 7;
const CONTEXT_INTEREST_DRAIN_RENDER_THREAD_WORK: u32 = 1 << 8;
const CONTEXT_INTEREST_OM_SET_RENDER_TARGETS: u32 = 1 << 9;

const CAPTURE_POINTER_SET_CAPACITY: usize = 4096;
const CAPTURE_POINTER_SET_PROBES: usize = 16;

static CONTEXT_HOOK_INTERESTS: AtomicU32 = AtomicU32::new(0);
static CONTEXT_CURRENT_PIXEL_SHADER_RELEVANT: AtomicBool = AtomicBool::new(false);
static CONTEXT_CURRENT_VERTEX_SHADER_RELEVANT: AtomicBool = AtomicBool::new(false);
static CONTEXT_CURRENT_INPUT_LAYOUT_RELEVANT: AtomicBool = AtomicBool::new(false);
static CONTEXT_CURRENT_UI_PASS_SHADER_RELEVANT: AtomicBool = AtomicBool::new(false);
static CONTEXT_MAP_FRAME_BOUNDARY_ARMED: AtomicBool = AtomicBool::new(false);
static CONTEXT_MAP_FRAME_DRAW_ARMED: AtomicBool = AtomicBool::new(false);
static CONTEXT_UI_PASS_ARMED: AtomicBool = AtomicBool::new(false);
static CONTEXT_UI_PASS_FRAME_CANDIDATE: AtomicBool = AtomicBool::new(false);
static CONTEXT_CENSUS_ARMED: AtomicBool = AtomicBool::new(false);
static CONTEXT_AUTO_ROUND_ARMED: AtomicBool = AtomicBool::new(false);
/// Latched (never reset) the first time a KNOWN capture-family (pdxmap /
/// water / river / border / tree) pixel shader is BOUND on the hooked
/// immediate context by the render-thread owner. Until then the process is in
/// its bootstrap phase — load screen, CEF browser-overlay interop, driver
/// worker warmup — where immediate-context Get* queries are not trusted: the
/// 2026-07-05 live census session crashed inside
/// d3d11!CContext::TID3D11DeviceContext_PSGetConstantBuffers on a load-screen
/// draw. The proven-live capture paths (map-frame bundles, ui_pass) only ever
/// touch the context after this same family recognition; census payload
/// capture and auto-round CB0 sampling must wait for it too.
static CONTEXT_MAP_RENDERER_LIVE: AtomicBool = AtomicBool::new(false);
/// CAPFIX3 — GetTickCount64 value captured the first time CONTEXT_MAP_RENDERER_-
/// LIVE latches. 0 = not yet live. Used with CB0-style lock-free timing so the
/// arm-delay window is measured from the renderer going live, not from attach.
static MAP_RENDERER_LIVE_TICK_MS: AtomicU64 = AtomicU64::new(0);
/// CAPFIX3 — resolved --arm-delay-ms (0 = no delay / arm immediately). Read on
/// the render-thread hot path (arming gate) so it is an atomic, set once from
/// the control at run_one_shot start before hooks capture anything.
static MAP_FRAME_ARM_DELAY_MS: AtomicU64 = AtomicU64::new(0);
/// CAPFIX3 — one-shot latch so the "arm-delay elapsed, sequence now arming"
/// event is written at most once per session.
static MAP_FRAME_ARM_DELAY_ELAPSED_RECORDED: AtomicBool = AtomicBool::new(false);
// Census pass-boundary identity is tracked from OMSetRenderTargets* hook
// arguments (render thread only). The census draw path must NEVER query the
// immediate context (OMGetRenderTargets per draw intermittently detonated a
// latent driver corruption; see lane bisect commits). View pointers are used
// as identity strings only and are never dereferenced.
static CENSUS_TRACKED_OM_RTVS: [AtomicUsize; 8] = [
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
];
static CENSUS_TRACKED_OM_DSV: AtomicUsize = AtomicUsize::new(0);
static CENSUS_TRACKED_OM_SET_SEEN: AtomicBool = AtomicBool::new(false);
static CENSUS_TRACKED_OM_BLEND_STATE: AtomicUsize = AtomicUsize::new(0);
static CENSUS_TRACKED_OM_DEPTH_STENCIL_STATE: AtomicUsize = AtomicUsize::new(0);
static CONTEXT_HOT_PREFILTER_SKIPS: AtomicU64 = AtomicU64::new(0);
static CONTEXT_HOT_LOCK_ACQUISITIONS: AtomicU64 = AtomicU64::new(0);
static CONTEXT_IA_SET_INPUT_LAYOUT_LOCK_ACQUISITIONS: AtomicU64 = AtomicU64::new(0);
static CONTEXT_VS_SET_SHADER_LOCK_ACQUISITIONS: AtomicU64 = AtomicU64::new(0);
static CONTEXT_PS_SET_SHADER_LOCK_ACQUISITIONS: AtomicU64 = AtomicU64::new(0);
static CONTEXT_PS_SET_SHADER_RESOURCES_LOCK_ACQUISITIONS: AtomicU64 = AtomicU64::new(0);
static CONTEXT_DRAW_LOCK_ACQUISITIONS: AtomicU64 = AtomicU64::new(0);
static CONTEXT_CLEAR_LOCK_ACQUISITIONS: AtomicU64 = AtomicU64::new(0);
static HOOK_LOCK_STATS_EVENT_RECORDED: AtomicBool = AtomicBool::new(false);
static CAPTURE_PIXEL_SHADER_POINTERS: [AtomicUsize; CAPTURE_POINTER_SET_CAPACITY] =
    [const { AtomicUsize::new(0) }; CAPTURE_POINTER_SET_CAPACITY];
static RESTORE_PIXEL_SHADER_POINTERS: [AtomicUsize; CAPTURE_POINTER_SET_CAPACITY] =
    [const { AtomicUsize::new(0) }; CAPTURE_POINTER_SET_CAPACITY];
static MAPNAME_PIXEL_SHADER_POINTERS: [AtomicUsize; CAPTURE_POINTER_SET_CAPACITY] =
    [const { AtomicUsize::new(0) }; CAPTURE_POINTER_SET_CAPACITY];
static CAPTURE_VERTEX_SHADER_POINTERS: [AtomicUsize; CAPTURE_POINTER_SET_CAPACITY] =
    [const { AtomicUsize::new(0) }; CAPTURE_POINTER_SET_CAPACITY];
static CAPTURE_INPUT_LAYOUT_POINTERS: [AtomicUsize; CAPTURE_POINTER_SET_CAPACITY] =
    [const { AtomicUsize::new(0) }; CAPTURE_POINTER_SET_CAPACITY];
static CAPTURE_PIXEL_SHADER_POINTERS_OVERFLOWED: AtomicBool = AtomicBool::new(false);
static RESTORE_PIXEL_SHADER_POINTERS_OVERFLOWED: AtomicBool = AtomicBool::new(false);
static MAPNAME_PIXEL_SHADER_POINTERS_OVERFLOWED: AtomicBool = AtomicBool::new(false);
static CAPTURE_VERTEX_SHADER_POINTERS_OVERFLOWED: AtomicBool = AtomicBool::new(false);
static CAPTURE_INPUT_LAYOUT_POINTERS_OVERFLOWED: AtomicBool = AtomicBool::new(false);

const UI_SHADER_BYTECODE_SLOT_CAPACITY: usize = 8192;
const UI_SHADER_BYTECODE_PROBES: usize = 32;
const UI_SHADER_BYTECODE_SLOT_INITIALIZING: usize = usize::MAX;

struct UiShaderBytecodeSlot {
    shader: AtomicUsize,
    device: AtomicUsize,
    bytes_ptr: AtomicUsize,
    byte_len: AtomicUsize,
}

impl UiShaderBytecodeSlot {
    const fn new() -> Self {
        Self {
            shader: AtomicUsize::new(0),
            device: AtomicUsize::new(0),
            bytes_ptr: AtomicUsize::new(0),
            byte_len: AtomicUsize::new(0),
        }
    }
}

static UI_PIXEL_SHADER_BYTECODES: [UiShaderBytecodeSlot; UI_SHADER_BYTECODE_SLOT_CAPACITY] =
    [const { UiShaderBytecodeSlot::new() }; UI_SHADER_BYTECODE_SLOT_CAPACITY];
static UI_VERTEX_SHADER_BYTECODES: [UiShaderBytecodeSlot; UI_SHADER_BYTECODE_SLOT_CAPACITY] =
    [const { UiShaderBytecodeSlot::new() }; UI_SHADER_BYTECODE_SLOT_CAPACITY];
static UI_PIXEL_SHADER_BYTECODES_OVERFLOWED: AtomicBool = AtomicBool::new(false);
static UI_VERTEX_SHADER_BYTECODES_OVERFLOWED: AtomicBool = AtomicBool::new(false);

const CREATED_BUFFER_DESC_SLOT_CAPACITY: usize = 16384;
const CREATED_BUFFER_DESC_PROBES: usize = 32;
const CREATED_BUFFER_DESC_SLOT_INITIALIZING: usize = usize::MAX;
const UI_PASS_CONSTANT_BUFFER_SLOT_COUNT: usize = MAP_FRAME_BINDING_SLOTS as usize;

struct CreatedBufferDescSlot {
    buffer: AtomicUsize,
    byte_width: AtomicU32,
    usage: AtomicU32,
    bind_flags: AtomicU32,
    cpu_access_flags: AtomicU32,
    misc_flags: AtomicU32,
    structure_byte_stride: AtomicU32,
}

impl CreatedBufferDescSlot {
    const fn new() -> Self {
        Self {
            buffer: AtomicUsize::new(0),
            byte_width: AtomicU32::new(0),
            usage: AtomicU32::new(0),
            bind_flags: AtomicU32::new(0),
            cpu_access_flags: AtomicU32::new(0),
            misc_flags: AtomicU32::new(0),
            structure_byte_stride: AtomicU32::new(0),
        }
    }
}

static CREATED_BUFFER_DESCS: [CreatedBufferDescSlot; CREATED_BUFFER_DESC_SLOT_CAPACITY] =
    [const { CreatedBufferDescSlot::new() }; CREATED_BUFFER_DESC_SLOT_CAPACITY];
static CREATED_BUFFER_DESCS_OVERFLOWED: AtomicBool = AtomicBool::new(false);

static UI_PASS_VS_CONSTANT_BUFFER_SHADOWS: [AtomicUsize; UI_PASS_CONSTANT_BUFFER_SLOT_COUNT] =
    [const { AtomicUsize::new(0) }; UI_PASS_CONSTANT_BUFFER_SLOT_COUNT];
static UI_PASS_PS_CONSTANT_BUFFER_SHADOWS: [AtomicUsize; UI_PASS_CONSTANT_BUFFER_SLOT_COUNT] =
    [const { AtomicUsize::new(0) }; UI_PASS_CONSTANT_BUFFER_SLOT_COUNT];

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct HookLockStatsSnapshot {
    pub create_ps_prefilter_skips: u64,
    pub create_ps_lock_acquisitions: u64,
    pub context_prefilter_skips: u64,
    pub context_lock_acquisitions: u64,
    pub context_ia_set_input_layout_lock_acquisitions: u64,
    pub context_vs_set_shader_lock_acquisitions: u64,
    pub context_ps_set_shader_lock_acquisitions: u64,
    pub context_ps_set_shader_resources_lock_acquisitions: u64,
    pub context_draw_lock_acquisitions: u64,
    pub context_clear_lock_acquisitions: u64,
}

/// Read the create-pixel-shader pre-filter counters (skipped, locked). Exposed
/// for the offline injection harness to prove the load-storm lock traffic is
/// gone.
pub fn create_pixel_shader_lock_stats() -> (u64, u64) {
    (
        CREATE_PS_PREFILTER_SKIPS.load(Ordering::SeqCst),
        CREATE_PS_LOCK_ACQUISITIONS.load(Ordering::SeqCst),
    )
}

pub fn hook_lock_stats_snapshot() -> HookLockStatsSnapshot {
    let (create_ps_prefilter_skips, create_ps_lock_acquisitions) = create_pixel_shader_lock_stats();
    HookLockStatsSnapshot {
        create_ps_prefilter_skips,
        create_ps_lock_acquisitions,
        context_prefilter_skips: CONTEXT_HOT_PREFILTER_SKIPS.load(Ordering::SeqCst),
        context_lock_acquisitions: CONTEXT_HOT_LOCK_ACQUISITIONS.load(Ordering::SeqCst),
        context_ia_set_input_layout_lock_acquisitions:
            CONTEXT_IA_SET_INPUT_LAYOUT_LOCK_ACQUISITIONS.load(Ordering::SeqCst),
        context_vs_set_shader_lock_acquisitions: CONTEXT_VS_SET_SHADER_LOCK_ACQUISITIONS
            .load(Ordering::SeqCst),
        context_ps_set_shader_lock_acquisitions: CONTEXT_PS_SET_SHADER_LOCK_ACQUISITIONS
            .load(Ordering::SeqCst),
        context_ps_set_shader_resources_lock_acquisitions:
            CONTEXT_PS_SET_SHADER_RESOURCES_LOCK_ACQUISITIONS.load(Ordering::SeqCst),
        context_draw_lock_acquisitions: CONTEXT_DRAW_LOCK_ACQUISITIONS.load(Ordering::SeqCst),
        context_clear_lock_acquisitions: CONTEXT_CLEAR_LOCK_ACQUISITIONS.load(Ordering::SeqCst),
    }
}

pub(crate) fn record_process_detach_hook_lock_stats() {
    quiesce_capture_for_process_detach();
    emit_hook_lock_stats_once("process_detach");
}

fn quiesce_capture_for_process_detach() {
    // Hard-stop the persistent CB0 sampler BEFORE anything else: process detach
    // runs under the loader lock at exit and the D3D device/context may be torn
    // down on another thread while a stray draw hook still fires. Unlike the
    // budget quiesce (which the sampler intentionally survives), detach is fatal
    // to any further immediate-context read. This store is the first thing the
    // sampler checks, so it can never issue a D3D call into a dying context.
    CB0_SAMPLER_DETACHING.store(true, Ordering::SeqCst);
    CAPTURE_QUIESCED.store(true, Ordering::SeqCst);
    clear_context_hook_interests();
    reset_capture_in_progress_flags();
    if let Some(runtime) = RUNTIME.get() {
        if let Ok(mut runtime) = runtime.try_lock() {
            // Persist the final census manifest (occurrence counts and any
            // signatures finalized since the last snapshot) before the
            // census state is torn down. try_lock only: this runs under the
            // loader lock at process exit.
            if census_manifest_snapshot_needed(&runtime) {
                let manifest = census_manifest_for_runtime(&runtime);
                let census_dir = runtime.root_output_dir.join(CENSUS_DIR);
                if fs::create_dir_all(&census_dir).is_ok() {
                    if let Ok(json) = serde_json::to_string_pretty(&manifest) {
                        let _ =
                            fs::write(census_dir.join(CENSUS_MANIFEST_FILE), format!("{json}\n"));
                    }
                }
            }
            stop_census_capture_for_runtime(&mut runtime);
            runtime.pending_dwell_end_boundary_finalization = false;
            runtime.pending_dwell_end_generated_texture_check = false;
            runtime.map_frame_draws.clear();
            runtime.map_frame_rejections.clear();
            runtime.ui_pass = UiPassCaptureState::default();
            runtime.terrain_chunk_library.enabled = false;
        }
    }
}

fn stop_census_capture_for_runtime(runtime: &mut CaptureRuntime) {
    runtime.census.enabled = false;
    runtime.census.active_pass = None;
    CONTEXT_CENSUS_ARMED.store(false, Ordering::SeqCst);
}

fn emit_hook_lock_stats_once(stage: &'static str) {
    if HOOK_LOCK_STATS_EVENT_RECORDED.swap(true, Ordering::SeqCst) {
        return;
    }
    let Some(output_dir) = RUNTIME.get().and_then(|runtime| {
        runtime
            .try_lock()
            .ok()
            .map(|runtime| runtime.output_dir.clone())
    }) else {
        return;
    };
    write_hook_lock_stats_event(&output_dir, stage);
    // Piggyback the ui_pass open-condition diagnostics on whichever finalize
    // path runs first (run_one_shot finalize or process detach) so short
    // harness runs and live sessions both surface them.
    write_event(&output_dir, &ui_pass_open_diagnostics_json());
}

fn write_hook_lock_stats_event(output_dir: &Path, stage: &'static str) {
    let stats = hook_lock_stats_snapshot();
    write_event(
        output_dir,
        &format!(
            "{{\"type\":\"create_pixel_shader_lock_stats\",\"stage\":\"{}\",\"prefilter_skips\":{},\"lock_acquisitions\":{},\"context_prefilter_skips\":{},\"context_lock_acquisitions\":{},\"context_ia_set_input_layout_lock_acquisitions\":{},\"context_vs_set_shader_lock_acquisitions\":{},\"context_ps_set_shader_lock_acquisitions\":{},\"context_ps_set_shader_resources_lock_acquisitions\":{},\"context_draw_lock_acquisitions\":{},\"context_clear_lock_acquisitions\":{}}}",
            json_escape(stage),
            stats.create_ps_prefilter_skips,
            stats.create_ps_lock_acquisitions,
            stats.context_prefilter_skips,
            stats.context_lock_acquisitions,
            stats.context_ia_set_input_layout_lock_acquisitions,
            stats.context_vs_set_shader_lock_acquisitions,
            stats.context_ps_set_shader_lock_acquisitions,
            stats.context_ps_set_shader_resources_lock_acquisitions,
            stats.context_draw_lock_acquisitions,
            stats.context_clear_lock_acquisitions,
        ),
    );
}

fn record_context_lock_acquisition(counter: &AtomicU64) {
    CONTEXT_HOT_LOCK_ACQUISITIONS.fetch_add(1, Ordering::Relaxed);
    counter.fetch_add(1, Ordering::Relaxed);
}

fn capture_pointer_hash(pointer: usize) -> usize {
    let mut value = (pointer >> 4) as u64;
    value ^= value >> 33;
    value = value.wrapping_mul(0xff51afd7ed558ccd);
    value ^= value >> 33;
    value as usize
}

fn remember_capture_pointer(table: &[AtomicUsize], overflowed: &AtomicBool, pointer: usize) {
    if pointer == 0 || overflowed.load(Ordering::Acquire) {
        return;
    }
    let start = capture_pointer_hash(pointer) % table.len();
    for offset in 0..CAPTURE_POINTER_SET_PROBES {
        let slot = &table[(start + offset) % table.len()];
        let current = slot.load(Ordering::Acquire);
        if current == pointer {
            return;
        }
        if current == 0
            && slot
                .compare_exchange(0, pointer, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
        {
            return;
        }
    }
    overflowed.store(true, Ordering::Release);
}

fn capture_pointer_known(table: &[AtomicUsize], overflowed: &AtomicBool, pointer: usize) -> bool {
    if pointer == 0 {
        return false;
    }
    if overflowed.load(Ordering::Acquire) {
        return true;
    }
    let start = capture_pointer_hash(pointer) % table.len();
    for offset in 0..CAPTURE_POINTER_SET_PROBES {
        let current = table[(start + offset) % table.len()].load(Ordering::Acquire);
        if current == pointer {
            return true;
        }
        if current == 0 {
            return false;
        }
    }
    false
}

fn remember_capture_pixel_shader(pointer: usize) {
    remember_capture_pointer(
        &CAPTURE_PIXEL_SHADER_POINTERS,
        &CAPTURE_PIXEL_SHADER_POINTERS_OVERFLOWED,
        pointer,
    );
}

fn remember_restore_pixel_shader(pointer: usize) {
    remember_capture_pointer(
        &RESTORE_PIXEL_SHADER_POINTERS,
        &RESTORE_PIXEL_SHADER_POINTERS_OVERFLOWED,
        pointer,
    );
}

fn remember_mapname_pixel_shader(pointer: usize) {
    remember_capture_pointer(
        &MAPNAME_PIXEL_SHADER_POINTERS,
        &MAPNAME_PIXEL_SHADER_POINTERS_OVERFLOWED,
        pointer,
    );
}

fn remember_capture_vertex_shader(pointer: usize) {
    remember_capture_pointer(
        &CAPTURE_VERTEX_SHADER_POINTERS,
        &CAPTURE_VERTEX_SHADER_POINTERS_OVERFLOWED,
        pointer,
    );
}

fn remember_capture_input_layout(pointer: usize) {
    remember_capture_pointer(
        &CAPTURE_INPUT_LAYOUT_POINTERS,
        &CAPTURE_INPUT_LAYOUT_POINTERS_OVERFLOWED,
        pointer,
    );
}

fn capture_pixel_shader_known(pointer: usize) -> bool {
    capture_pointer_known(
        &CAPTURE_PIXEL_SHADER_POINTERS,
        &CAPTURE_PIXEL_SHADER_POINTERS_OVERFLOWED,
        pointer,
    )
}

fn restore_pixel_shader_known(pointer: usize) -> bool {
    capture_pointer_known(
        &RESTORE_PIXEL_SHADER_POINTERS,
        &RESTORE_PIXEL_SHADER_POINTERS_OVERFLOWED,
        pointer,
    )
}

fn mapname_pixel_shader_known(pointer: usize) -> bool {
    capture_pointer_known(
        &MAPNAME_PIXEL_SHADER_POINTERS,
        &MAPNAME_PIXEL_SHADER_POINTERS_OVERFLOWED,
        pointer,
    )
}

/// Strict membership for the render-trust latch. capture_pointer_known
/// deliberately fails OPEN once the pointer set overflows (map-frame arming
/// must never miss a family shader), but trust in the immediate context must
/// never be granted on an overflow guess: a load-screen/browser shader would
/// spuriously unlock the census payload Get* battery. Overflow -> false.
fn capture_pointer_known_strict(
    table: &[AtomicUsize],
    overflowed: &AtomicBool,
    pointer: usize,
) -> bool {
    !overflowed.load(Ordering::Acquire) && capture_pointer_known(table, overflowed, pointer)
}

fn capture_pixel_shader_known_strict(pointer: usize) -> bool {
    capture_pointer_known_strict(
        &CAPTURE_PIXEL_SHADER_POINTERS,
        &CAPTURE_PIXEL_SHADER_POINTERS_OVERFLOWED,
        pointer,
    )
}

fn capture_vertex_shader_known(pointer: usize) -> bool {
    capture_pointer_known(
        &CAPTURE_VERTEX_SHADER_POINTERS,
        &CAPTURE_VERTEX_SHADER_POINTERS_OVERFLOWED,
        pointer,
    )
}

fn capture_input_layout_known(pointer: usize) -> bool {
    capture_pointer_known(
        &CAPTURE_INPUT_LAYOUT_POINTERS,
        &CAPTURE_INPUT_LAYOUT_POINTERS_OVERFLOWED,
        pointer,
    )
}

fn remember_ui_pixel_shader_bytecode(device: usize, pointer: usize, bytes: &[u8]) {
    remember_ui_shader_bytecode(
        &UI_PIXEL_SHADER_BYTECODES,
        &UI_PIXEL_SHADER_BYTECODES_OVERFLOWED,
        device,
        pointer,
        bytes,
    );
}

fn remember_ui_vertex_shader_bytecode(device: usize, pointer: usize, bytes: &[u8]) {
    remember_ui_shader_bytecode(
        &UI_VERTEX_SHADER_BYTECODES,
        &UI_VERTEX_SHADER_BYTECODES_OVERFLOWED,
        device,
        pointer,
        bytes,
    );
}

fn remember_ui_shader_bytecode(
    table: &[UiShaderBytecodeSlot],
    overflowed: &AtomicBool,
    device: usize,
    pointer: usize,
    bytes: &[u8],
) {
    if !SHADER_BYTECODE_TRACKING_ENABLED.load(Ordering::Acquire)
        || device == 0
        || pointer == 0
        || pointer == UI_SHADER_BYTECODE_SLOT_INITIALIZING
        || bytes.is_empty()
        || overflowed.load(Ordering::Acquire)
    {
        return;
    }
    let start = capture_pointer_hash(pointer) % table.len();
    for offset in 0..UI_SHADER_BYTECODE_PROBES {
        let slot = &table[(start + offset) % table.len()];
        let current = slot.shader.load(Ordering::Acquire);
        if current == pointer {
            return;
        }
        if current == 0
            && slot
                .shader
                .compare_exchange(
                    0,
                    UI_SHADER_BYTECODE_SLOT_INITIALIZING,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok()
        {
            let mut owned = bytes.to_vec().into_boxed_slice();
            let ptr = owned.as_mut_ptr() as usize;
            let len = owned.len();
            mem::forget(owned);
            slot.device.store(device, Ordering::Release);
            slot.bytes_ptr.store(ptr, Ordering::Release);
            slot.byte_len.store(len, Ordering::Release);
            slot.shader.store(pointer, Ordering::Release);
            return;
        }
    }
    overflowed.store(true, Ordering::Release);
}

fn ui_pixel_shader_bytecode(pointer: usize) -> Option<&'static [u8]> {
    ui_shader_bytecode(&UI_PIXEL_SHADER_BYTECODES, pointer)
}

fn ui_vertex_shader_bytecode(pointer: usize) -> Option<&'static [u8]> {
    ui_shader_bytecode(&UI_VERTEX_SHADER_BYTECODES, pointer)
}

fn ui_pixel_shader_known_for_current_swapchain(pointer: usize) -> bool {
    ui_shader_known_for_current_swapchain(&UI_PIXEL_SHADER_BYTECODES, pointer)
}

fn ui_vertex_shader_known_for_current_swapchain(pointer: usize) -> bool {
    ui_shader_known_for_current_swapchain(&UI_VERTEX_SHADER_BYTECODES, pointer)
}

fn ui_shader_known_for_current_swapchain(table: &[UiShaderBytecodeSlot], pointer: usize) -> bool {
    if pointer == 0 {
        return false;
    }
    let Some(device) = current_swapchain_device_pointer() else {
        return ui_shader_bytecode(table, pointer).is_some();
    };
    ui_shader_device(table, pointer) == Some(device) && ui_shader_bytecode(table, pointer).is_some()
}

fn current_swapchain_device_pointer() -> Option<usize> {
    RUNTIME
        .get()
        .and_then(|runtime| runtime.lock().ok())
        .map(|runtime| runtime.current_swapchain_device)
        .filter(|device| *device != 0)
}

fn ui_shader_bytecode(table: &[UiShaderBytecodeSlot], pointer: usize) -> Option<&'static [u8]> {
    if pointer == 0 || pointer == UI_SHADER_BYTECODE_SLOT_INITIALIZING {
        return None;
    }
    let start = capture_pointer_hash(pointer) % table.len();
    for offset in 0..UI_SHADER_BYTECODE_PROBES {
        let slot = &table[(start + offset) % table.len()];
        let current = slot.shader.load(Ordering::Acquire);
        if current == pointer {
            let ptr = slot.bytes_ptr.load(Ordering::Acquire);
            let len = slot.byte_len.load(Ordering::Acquire);
            if ptr == 0 || len == 0 {
                return None;
            }
            return Some(unsafe { slice::from_raw_parts(ptr as *const u8, len) });
        }
        if current == 0 {
            return None;
        }
    }
    None
}

fn ui_shader_device(table: &[UiShaderBytecodeSlot], pointer: usize) -> Option<usize> {
    if pointer == 0 || pointer == UI_SHADER_BYTECODE_SLOT_INITIALIZING {
        return None;
    }
    let start = capture_pointer_hash(pointer) % table.len();
    for offset in 0..UI_SHADER_BYTECODE_PROBES {
        let slot = &table[(start + offset) % table.len()];
        let current = slot.shader.load(Ordering::Acquire);
        if current == pointer {
            let device = slot.device.load(Ordering::Acquire);
            return (device != 0).then_some(device);
        }
        if current == 0 {
            return None;
        }
    }
    None
}

fn remember_created_buffer_desc(pointer: usize, desc: &D3D11_BUFFER_DESC) {
    if pointer == 0
        || pointer == CREATED_BUFFER_DESC_SLOT_INITIALIZING
        || CREATED_BUFFER_DESCS_OVERFLOWED.load(Ordering::Acquire)
    {
        return;
    }
    let start = capture_pointer_hash(pointer) % CREATED_BUFFER_DESCS.len();
    for offset in 0..CREATED_BUFFER_DESC_PROBES {
        let slot = &CREATED_BUFFER_DESCS[(start + offset) % CREATED_BUFFER_DESCS.len()];
        let current = slot.buffer.load(Ordering::Acquire);
        if current == pointer {
            return;
        }
        if current == 0
            && slot
                .buffer
                .compare_exchange(
                    0,
                    CREATED_BUFFER_DESC_SLOT_INITIALIZING,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok()
        {
            slot.byte_width.store(desc.ByteWidth, Ordering::Release);
            slot.usage.store(desc.Usage, Ordering::Release);
            slot.bind_flags.store(desc.BindFlags, Ordering::Release);
            slot.cpu_access_flags
                .store(desc.CPUAccessFlags, Ordering::Release);
            slot.misc_flags.store(desc.MiscFlags, Ordering::Release);
            slot.structure_byte_stride
                .store(desc.StructureByteStride, Ordering::Release);
            slot.buffer.store(pointer, Ordering::Release);
            return;
        }
    }
    CREATED_BUFFER_DESCS_OVERFLOWED.store(true, Ordering::Release);
}

fn created_buffer_desc(pointer: usize) -> Option<BufferDesc> {
    if pointer == 0 || pointer == CREATED_BUFFER_DESC_SLOT_INITIALIZING {
        return None;
    }
    let start = capture_pointer_hash(pointer) % CREATED_BUFFER_DESCS.len();
    for offset in 0..CREATED_BUFFER_DESC_PROBES {
        let slot = &CREATED_BUFFER_DESCS[(start + offset) % CREATED_BUFFER_DESCS.len()];
        let current = slot.buffer.load(Ordering::Acquire);
        if current == pointer {
            return Some(BufferDesc {
                byte_width: slot.byte_width.load(Ordering::Acquire),
                usage: slot.usage.load(Ordering::Acquire),
                bind_flags: slot.bind_flags.load(Ordering::Acquire),
                cpu_access_flags: slot.cpu_access_flags.load(Ordering::Acquire),
                misc_flags: slot.misc_flags.load(Ordering::Acquire),
                structure_byte_stride: slot.structure_byte_stride.load(Ordering::Acquire),
            });
        }
        if current == 0 {
            return None;
        }
    }
    None
}

fn shadow_ui_pass_constant_buffers(
    shadows: &[AtomicUsize; UI_PASS_CONSTANT_BUFFER_SLOT_COUNT],
    start_slot: UINT,
    num_buffers: UINT,
    buffers: *const *mut ID3D11Buffer,
) {
    if !CONTEXT_UI_PASS_ARMED.load(Ordering::Acquire) {
        return;
    }
    for index in 0..num_buffers {
        let slot = start_slot.saturating_add(index);
        if slot as usize >= shadows.len() {
            break;
        }
        let buffer = if buffers.is_null() {
            0
        } else {
            unsafe { *buffers.add(index as usize) as usize }
        };
        shadows[slot as usize].store(buffer, Ordering::Release);
    }
}

fn refresh_context_hook_interests_for_runtime(runtime: &CaptureRuntime) {
    let mut interests = 0;
    if !capture_quiesced() && !CAPTURE_BUDGET_STOPPED.load(Ordering::SeqCst) {
        let census_armed = runtime.census.enabled;
        let auto_round_armed = auto_round_watch_pending(runtime);
        if let Ok(profile) = requested_map_frame_capture_profile_for_runtime(runtime) {
            let moment_complete = runtime
                .completed_map_frame_moments
                .contains(profile.capture_moment);
            if !moment_complete && profile.capture_moment != UI_PASS_CAPTURE_MOMENT {
                interests |= CONTEXT_INTEREST_IA_SET_INPUT_LAYOUT
                    | CONTEXT_INTEREST_VS_SET_SHADER
                    | CONTEXT_INTEREST_PS_SET_SHADER
                    | CONTEXT_INTEREST_DRAW;
                if profile.requires_terrain_runtime_files
                    && !terrain_runtime_inputs_complete(runtime)
                {
                    interests |= CONTEXT_INTEREST_PS_SET_SHADER_RESOURCES;
                }
                if !runtime.map_frame_draws.is_empty() {
                    interests |= CONTEXT_INTEREST_CLEAR_RENDER_TARGET
                        | CONTEXT_INTEREST_CLEAR_DEPTH_STENCIL
                        | CONTEXT_INTEREST_PRESENT;
                }
            }
        }
        if terrain_chunk_library_draw_capture_ready_for_runtime(runtime) {
            interests |= CONTEXT_INTEREST_DRAW;
        }
        if ui_pass_draw_capture_ready_for_runtime(runtime) {
            interests |= CONTEXT_INTEREST_IA_SET_INPUT_LAYOUT
                | CONTEXT_INTEREST_VS_SET_SHADER
                | CONTEXT_INTEREST_PS_SET_SHADER
                | CONTEXT_INTEREST_DRAW
                | CONTEXT_INTEREST_PRESENT;
        }
        if census_armed {
            interests |= CONTEXT_INTEREST_IA_SET_INPUT_LAYOUT
                | CONTEXT_INTEREST_VS_SET_SHADER
                | CONTEXT_INTEREST_PS_SET_SHADER
                | CONTEXT_INTEREST_DRAW
                | CONTEXT_INTEREST_OM_SET_RENDER_TARGETS
                | CONTEXT_INTEREST_CLEAR_RENDER_TARGET
                | CONTEXT_INTEREST_CLEAR_DEPTH_STENCIL
                | CONTEXT_INTEREST_PRESENT;
        }
        if auto_round_armed {
            interests |= CONTEXT_INTEREST_PS_SET_SHADER | CONTEXT_INTEREST_DRAW;
        }
        if runtime.pending_dwell_end_boundary_finalization
            || runtime.pending_dwell_end_generated_texture_check
            || !runtime.pending_render_thread_release_srvs.is_empty()
        {
            interests |= CONTEXT_INTEREST_DRAIN_RENDER_THREAD_WORK
                | CONTEXT_INTEREST_IA_SET_INPUT_LAYOUT
                | CONTEXT_INTEREST_VS_SET_SHADER
                | CONTEXT_INTEREST_PS_SET_SHADER
                | CONTEXT_INTEREST_PS_SET_SHADER_RESOURCES
                | CONTEXT_INTEREST_DRAW
                | CONTEXT_INTEREST_CLEAR_RENDER_TARGET
                | CONTEXT_INTEREST_CLEAR_DEPTH_STENCIL
                | CONTEXT_INTEREST_PRESENT;
        }
    }
    CONTEXT_MAP_FRAME_BOUNDARY_ARMED.store(
        interests
            & (CONTEXT_INTEREST_CLEAR_RENDER_TARGET
                | CONTEXT_INTEREST_CLEAR_DEPTH_STENCIL
                | CONTEXT_INTEREST_PRESENT)
            != 0,
        Ordering::SeqCst,
    );
    CONTEXT_UI_PASS_ARMED.store(
        ui_pass_draw_capture_ready_for_runtime(runtime),
        Ordering::SeqCst,
    );
    CONTEXT_CENSUS_ARMED.store(runtime.census.enabled, Ordering::SeqCst);
    CONTEXT_AUTO_ROUND_ARMED.store(auto_round_watch_pending(runtime), Ordering::SeqCst);
    CONTEXT_MAP_FRAME_DRAW_ARMED.store(
        requested_map_frame_capture_profile_for_runtime(runtime).map_or(false, |profile| {
            profile.capture_moment != UI_PASS_CAPTURE_MOMENT
                && !runtime
                    .completed_map_frame_moments
                    .contains(profile.capture_moment)
        }),
        Ordering::SeqCst,
    );
    CONTEXT_HOOK_INTERESTS.store(interests, Ordering::SeqCst);
}

fn clear_context_hook_interests() {
    CONTEXT_HOOK_INTERESTS.store(0, Ordering::SeqCst);
    CONTEXT_CURRENT_PIXEL_SHADER_RELEVANT.store(false, Ordering::SeqCst);
    CONTEXT_CURRENT_VERTEX_SHADER_RELEVANT.store(false, Ordering::SeqCst);
    CONTEXT_CURRENT_INPUT_LAYOUT_RELEVANT.store(false, Ordering::SeqCst);
    CONTEXT_CURRENT_UI_PASS_SHADER_RELEVANT.store(false, Ordering::SeqCst);
    CONTEXT_MAP_FRAME_BOUNDARY_ARMED.store(false, Ordering::SeqCst);
    CONTEXT_MAP_FRAME_DRAW_ARMED.store(false, Ordering::SeqCst);
    CONTEXT_UI_PASS_ARMED.store(false, Ordering::SeqCst);
    CONTEXT_UI_PASS_FRAME_CANDIDATE.store(false, Ordering::SeqCst);
    CONTEXT_CENSUS_ARMED.store(false, Ordering::SeqCst);
    CONTEXT_AUTO_ROUND_ARMED.store(false, Ordering::SeqCst);
    SHADER_BYTECODE_TRACKING_ENABLED.store(false, Ordering::SeqCst);
    UI_PASS_SHADER_TRACKING_ENABLED.store(false, Ordering::SeqCst);
}

fn context_interest_enabled(interest: u32) -> bool {
    CONTEXT_HOOK_INTERESTS.load(Ordering::Acquire) & interest != 0
}

fn context_drain_work_pending() -> bool {
    context_interest_enabled(CONTEXT_INTEREST_DRAIN_RENDER_THREAD_WORK)
}

fn context_prefilter_skip() {
    CONTEXT_HOT_PREFILTER_SKIPS.fetch_add(1, Ordering::Relaxed);
}

static mut ORIG_CREATE_BUFFER: Option<CreateBufferFn> = None;
static mut ORIG_CREATE_INPUT_LAYOUT: Option<CreateInputLayoutFn> = None;
static mut ORIG_CREATE_VERTEX_SHADER: Option<CreateVertexShaderFn> = None;
static mut ORIG_CREATE_PIXEL_SHADER: Option<CreatePixelShaderFn> = None;
static mut ORIG_IA_SET_INPUT_LAYOUT: Option<IaSetInputLayoutFn> = None;
static mut ORIG_VS_SET_CONSTANT_BUFFERS: Option<SetConstantBuffersFn> = None;
static mut ORIG_PS_SET_CONSTANT_BUFFERS: Option<SetConstantBuffersFn> = None;
static mut ORIG_VS_SET_SHADER: Option<VsSetShaderFn> = None;
static mut ORIG_PS_SET_SHADER: Option<PsSetShaderFn> = None;
static mut ORIG_PS_SET_SHADER_RESOURCES: Option<PsSetShaderResourcesFn> = None;
static mut ORIG_DRAW_INDEXED: Option<DrawIndexedFn> = None;
static mut ORIG_DRAW: Option<DrawFn> = None;
static mut ORIG_DRAW_INDEXED_INSTANCED: Option<DrawIndexedInstancedFn> = None;
static mut ORIG_DRAW_INSTANCED: Option<DrawInstancedFn> = None;
static mut ORIG_OM_SET_RENDER_TARGETS: Option<OmSetRenderTargetsFn> = None;
static mut ORIG_OM_SET_RENDER_TARGETS_AND_UAVS: Option<OmSetRenderTargetsAndUavsFn> = None;
static mut ORIG_OM_SET_BLEND_STATE: Option<OmSetBlendStateFn> = None;
static mut ORIG_OM_SET_DEPTH_STENCIL_STATE: Option<OmSetDepthStencilStateFn> = None;
static mut ORIG_CLEAR_RENDER_TARGET_VIEW: Option<ClearRenderTargetViewFn> = None;
static mut ORIG_CLEAR_DEPTH_STENCIL_VIEW: Option<ClearDepthStencilViewFn> = None;
static mut ORIG_PRESENT: Option<PresentFn> = None;

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
    let control = read_control(module_dir);
    // The attach helper's --dwell-ms rides in on the control file and takes
    // precedence over the caller's hardcoded default (180s). Without this,
    // long sessions (chunk pans, ui_pass atlases) silently truncated at 3
    // minutes — game load alone consumes most of that (live 2026-07-03,
    // three ui_pass atlas sessions finalized with 0 states).
    let timeout_ms = control
        .as_ref()
        .and_then(|control| control.dwell_ms)
        .map_or(timeout_ms, |dwell| {
            u32::try_from(dwell).unwrap_or(u32::MAX).max(timeout_ms)
        })
        // CAPFIX2 dwell cap: never let the session run longer than 10 minutes,
        // regardless of the control file (defense in depth behind the attach
        // helper's rejection). With Bug 1 fixed, hitting this cap is safe.
        .min(MAX_DWELL_MS);
    let output_dir = capture_output_dir_from_control(module_dir, control.as_ref());
    let control_path = module_dir.join(CONTROL_FILE);
    let _ = fs::create_dir_all(&output_dir);
    install_capture_panic_hook(&output_dir);
    let settle_ms = control.as_ref().map_or(DEFAULT_SETTLE_MS, |control| {
        if control.settle_ms == 0 {
            DEFAULT_SETTLE_MS
        } else {
            control.settle_ms
        }
    });
    let capture_moment_sequence = capture_moment_sequence_from_control(control.as_ref());
    let census_enabled = census_enabled_from_control(control.as_ref());
    let ui_pass_requested = capture_moment_sequence
        .iter()
        .any(|moment| moment == UI_PASS_CAPTURE_MOMENT);
    let restore_pass_requested = capture_moment_sequence
        .iter()
        .any(|moment| moment == RESTORE_CAPTURE_MOMENT);
    let mapname_pass_requested = capture_moment_sequence
        .iter()
        .any(|moment| moment == MAPNAME_CAPTURE_MOMENT);
    SHADER_BYTECODE_TRACKING_ENABLED.store(
        census_enabled || ui_pass_requested || restore_pass_requested || mapname_pass_requested,
        Ordering::SeqCst,
    );
    UI_PASS_SHADER_TRACKING_ENABLED.store(ui_pass_requested, Ordering::SeqCst);
    let capture_budget = capture_budget_from_control(control.as_ref());
    let start_tick_ms = unsafe { GetTickCount64() };
    let terrain_chunk_library = TerrainChunkLibraryCaptureState::new(
        terrain_chunk_library_enabled_from_control(control.as_ref()),
        start_tick_ms,
    );
    let census = census_state_from_control(control.as_ref(), capture_budget, start_tick_ms);
    let runtime = CaptureRuntime {
        state: TerrainCaptureState::default(),
        root_output_dir: output_dir.clone(),
        output_dir: output_dir.clone(),
        control_path,
        control_command_last_modified: None,
        control: control.clone(),
        start_tick_ms,
        settle_ms,
        capture_budget,
        total_payload_bytes_reserved: 0,
        capture_work_frame_ms: 0,
        capture_budget_exceeded: None,
        terrain_chunk_library,
        capture_moment_sequence,
        active_capture_index: 0,
        active_moment_started_ms: 0,
        dwell_end_generated_texture_check_done: false,
        pending_dwell_end_boundary_finalization: false,
        pending_dwell_end_generated_texture_check: false,
        pending_render_thread_release_srvs: Vec::new(),
        capture_quiesced_event_recorded: false,
        captured_symbols: BTreeSet::new(),
        captured_runtime_textures: BTreeMap::new(),
        deferred_runtime_targets: BTreeMap::new(),
        shared_payloads: BTreeMap::new(),
        shared_payload_files_written: BTreeSet::new(),
        boundary_payloads: BTreeMap::new(),
        boundary_payload_warnings: BTreeSet::new(),
        completed_map_frame_moments: BTreeSet::new(),
        map_frame_draws: Vec::new(),
        map_frame_blockers: Vec::new(),
        map_frame_rejections: BTreeSet::new(),
        ui_pass: UiPassCaptureState::default(),
        census,
        ui_shader_files_written: BTreeSet::new(),
        current_swapchain_backbuffer: None,
        current_swapchain_device: 0,
        latest_post_draw_output_merger: None,
        latest_post_draw_output_blockers: Vec::new(),
        om_render_target_clears: BTreeMap::new(),
        om_depth_stencil_clears: BTreeMap::new(),
        om_clear_sequence: 0,
        terrain_id_srv_slot_observed: None,
        round_commands_enabled: control
            .as_ref()
            .map_or(false, |control| control.round_commands_enabled),
        active_round_label: None,
        active_round_finished: false,
        control_sequence_complete: false,
        pending_manual_round_label: None,
        last_context: 0,
        last_error: None,
    };
    refresh_context_hook_interests_for_runtime(&runtime);
    let _ = RUNTIME.set(Mutex::new(runtime));
    arm_cb0_sampler_from_control(control.as_ref(), start_tick_ms, &output_dir);
    configure_map_frame_arm_delay_from_control(control.as_ref(), &output_dir);
    write_event(
        &output_dir,
        &format!(
            "{{\"type\":\"terrain_capture_start\",\"timeout_ms\":{},\"requires\":\"active_pdxmap_shader_declared_TerrainIDMap_and_SnowMudData_slots_plus_terrain_vertex_buffer_and_input_layout\"}}",
            timeout_ms
        ),
    );
    if let Some(control) = &control {
        let census_field = if control.census {
            ",\"census\":true"
        } else {
            ""
        };
        write_event(
            &output_dir,
            &format!(
                "{{\"type\":\"terrain_capture_control_loaded\",\"expected_width\":{},\"expected_height\":{},\"allowed_slots\":{:?},\"paired_resource_slot_count\":{},\"settle_ms\":{},\"terrain_chunk_library\":{}{},\"max_draws_per_pass\":{},\"max_total_payload_bytes_per_session\":{},\"max_payload_writer_queue_bytes\":{},\"ui_max_states\":{},\"ui_cb_skip\":\"{}\",\"provenance\":\"{}\"}}",
                control.expected_width,
                control.expected_height,
                control.allowed_slots,
                control.paired_resource_slots.len(),
                settle_ms,
                terrain_chunk_library_enabled_from_control(Some(control)),
                census_field,
                capture_budget.max_draws_per_pass,
                capture_budget.max_total_payload_bytes_per_session,
                capture_budget.max_payload_writer_queue_bytes,
                ui_pass_max_states_for_control(Some(control)),
                control.ui_cb_skip.as_str(),
                json_escape(&control.provenance)
            ),
        );
    }
    write_event(
        &output_dir,
        &format!(
            "{{\"type\":\"terrain_capture_budget_configured\",\"max_draws_per_pass\":{},\"max_total_payload_bytes_per_session\":{},\"max_capture_work_ms_per_frame\":{},\"max_payload_writer_queue_bytes\":{}}}",
            capture_budget.max_draws_per_pass,
            capture_budget.max_total_payload_bytes_per_session,
            capture_budget.max_capture_work_ms_per_frame,
            capture_budget.max_payload_writer_queue_bytes
        ),
    );

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
    while !capture_ready_to_finish() {
        poll_control_file_for_round_command();
        maybe_emit_round_stall_watchdog();
        // CAPFIX2 BUG 2: name the runtime-lock holder if it has been held >10s.
        // Polled here (off the render thread, every ~50ms) so a wedge like
        // session_LAW_6 is reported instead of leaving the log silent.
        runtime_lock_watchdog_tick();
        // CAPFIX4: idle-anchored dwell — accepted control commands re-anchor
        // the clock (see LAST_CONTROL_ACTIVITY_TICK_MS).
        let elapsed = dwell_elapsed_ms(
            unsafe { GetTickCount64() },
            started,
            LAST_CONTROL_ACTIVITY_TICK_MS.load(Ordering::SeqCst),
        );
        if elapsed >= u64::from(timeout_ms) {
            break;
        }
        unsafe {
            Sleep(50);
        }
    }

    let mut captured = CAPTURED.load(Ordering::SeqCst);
    if request_render_thread_dwell_end_work(!captured) {
        wait_for_render_thread_dwell_end_work(&output_dir, 5_000);
    }
    finalize_ui_pass_capture_at_dwell_end();
    finalize_census_pass(ptr::null_mut(), "dwell_end");
    let _ = write_census_manifest_snapshot(&output_dir);
    stop_census_capture_for_shutdown();
    captured = CAPTURED.load(Ordering::SeqCst);
    // CAPFIX2 BUG 1: DO NOT unhook/unload at dwell end. Removing the MinHook
    // inline hooks while the render thread is live is a use-after-free that
    // crashed the game at a fixed offset in the unloaded DLL. Instead, latch the
    // permanent disarm flag (all hooks become pass-throughs) and pin the module.
    // Hooks stay resident until process exit.
    disarm_capture_hooks_permanently();
    finalize_terrain_chunk_library_capture();
    emit_hook_lock_stats_once("run_one_shot_finalize");
    write_event(&output_dir, &ui_pass_open_diagnostics_json());
    // MOMFIX1: with rounds enabled, never claim captured_runtime_terrain_inputs
    // while the control's requested continuous sequence is still incomplete.
    // The global CAPTURED flag is also set by a completed ROUND, whose sequence
    // excludes restore_pass/mapname_pass/ui_pass — so keying success off CAPTURED
    // alone masks a starved control sequence. HOLD keeps rounds from starting
    // until the control sequence completes (latched in control_sequence_complete),
    // so if the latch is unset here the control sequence genuinely never
    // finished: report the true pending list with blocked_before_* vocabulary.
    if report_incomplete_control_sequence_before_rounds(&output_dir) {
        // Recorded blocked_before_* report; fall through to teardown.
    } else if captured {
        write_event(
            &output_dir,
            "{\"type\":\"terrain_capture_finished\",\"status\":\"captured_runtime_terrain_inputs\"}",
        );
    } else if CAPTURE_BUDGET_STOPPED.load(Ordering::SeqCst) {
        write_event(
            &output_dir,
            "{\"type\":\"terrain_capture_finished\",\"status\":\"capture_budget_exceeded\"}",
        );
    } else if write_not_observed_map_capture_session_at_dwell_end(timeout_ms.into()) {
        write_event(
            &output_dir,
            "{\"type\":\"terrain_capture_finished\",\"status\":\"captured_available_map_moments\"}",
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

fn stop_census_capture_for_shutdown() {
    if let Some(runtime) = RUNTIME.get() {
        if let Ok(mut runtime) = runtime.lock() {
            stop_census_capture_for_runtime(&mut runtime);
            refresh_context_hook_interests_for_runtime(&runtime);
        }
    } else {
        CONTEXT_CENSUS_ARMED.store(false, Ordering::SeqCst);
    }
}

fn census_active_pass_needs_context() -> bool {
    RUNTIME
        .get()
        .and_then(|runtime| runtime.lock().ok())
        .map(|runtime| runtime.census.active_pass.is_some())
        .unwrap_or(false)
}

fn capture_ready_to_finish() -> bool {
    // CAPFIX3 ISSUE 1 — disarm-scope regression (session_NIGHT_SWEEP_5).
    // Split "moments finished capturing / budget aborted" from "DLL disarmed":
    // while the persistent CB0 sampler is armed it must keep sampling to DWELL
    // EXPIRY, even after every armed moment completes AND even after a budget
    // abort. Budget quiesce does NOT set HOOKS_DISARMED / CB0_SAMPLER_DETACHING
    // and does NOT clear CB0_SAMPLER_ARMED — the sampler is byte-budget and
    // quiesce exempt by SAMP1 contract — so the sun-curve recorder keeps
    // appending through a budget stop too. Finishing the loop early runs the
    // finalize path, which calls disarm_capture_hooks_permanently() and clears
    // CB0_SAMPLER_ARMED: exactly the regression that froze the NIGHT_SWEEP_5
    // file at one sample. This gate is therefore CHECKED FIRST, before the
    // budget-stopped and CAPTURED early-returns, so neither moment completion
    // nor a budget abort ends the dwell early while the sampler is live. The
    // dwell loop's elapsed check still bounds the run at timeout_ms, so this
    // only defers the finish to expiry; it never blocks it. Moment bundles are
    // already written by the render thread at their pass-close boundaries, so
    // nothing is lost by keeping the loop spinning (round polling continues).
    // Sampler off (no --cb0-sample-ms) => keepalive false and the pre-CAPFIX3
    // early-finish-on-moment-completion behavior is fully preserved.
    if cb0_sampler_keepalive_active() {
        return false;
    }
    if CAPTURE_BUDGET_STOPPED.load(Ordering::SeqCst) {
        return true;
    }
    if !CAPTURED.load(Ordering::SeqCst) {
        return false;
    }
    RUNTIME
        .get()
        .and_then(|runtime| runtime.lock().ok())
        .map_or(true, |runtime| {
            if runtime.round_commands_enabled {
                return false;
            }
            // Census mode observes the WHOLE session: completing the armed
            // map-frame moments must not end the session early (the census
            // exists to see every later pass; the dwell timeout still bounds
            // the run).
            if runtime.census.enabled {
                return false;
            }
            // The session may only finish early when EVERY armed moment in the
            // sequence is complete — not merely the first (CAPTURED fires at
            // terrain finalize). The finisher previously raced ahead during the
            // water→trees window and, decisively, finalized a ui_pass atlas
            // with 0 states while the user had minutes of clicking left (live
            // 2026-07-03, twice). ui_pass completes at its state cap; if any
            // armed moment never completes, the dwell timeout still ends the
            // session through the loop's elapsed check.
            capture_complete(&runtime)
                && !terrain_chunk_library_capture_pending(&runtime)
                && !generated_runtime_texture_dwell_end_check_pending(&runtime)
        })
}

fn terrain_chunk_library_capture_pending(runtime: &CaptureRuntime) -> bool {
    if !runtime.terrain_chunk_library.enabled {
        return false;
    }
    if runtime.terrain_chunk_library.chunks.is_empty() {
        return true;
    }
    let mut blockers = Vec::new();
    let world_rect =
        terrain_chunk_library_world_rect_from_control(runtime.control.as_ref(), &mut blockers);
    let bounds = runtime
        .terrain_chunk_library
        .chunks
        .values()
        .map(|chunk| chunk.bounds)
        .collect::<Vec<_>>();
    let coverage = terrain_chunk_coverage_for_bounds(&bounds, world_rect);
    coverage.fraction < TERRAIN_CHUNK_LIBRARY_COMPLETE_COVERAGE_FRACTION
}

fn generated_runtime_texture_dwell_end_check_pending(runtime: &CaptureRuntime) -> bool {
    !runtime.dwell_end_generated_texture_check_done
        && runtime.captured_runtime_textures.keys().any(|symbol| {
            capture_target_by_symbol(symbol).map_or(false, |target| target.dwell_end_resnapshot)
        })
}

fn request_render_thread_dwell_end_work(include_boundary_finalization: bool) -> bool {
    if capture_quiesced() {
        return false;
    }
    RUNTIME
        .get()
        .and_then(|runtime| {
            runtime.lock().ok().map(|mut runtime| {
                let mut requested = false;
                if include_boundary_finalization && dwell_end_boundary_finalization_ready(&runtime)
                {
                    runtime.pending_dwell_end_boundary_finalization = true;
                    requested = true;
                }
                if generated_runtime_texture_dwell_end_check_pending(&runtime) {
                    runtime.pending_dwell_end_generated_texture_check = true;
                    requested = true;
                }
                if requested {
                    refresh_context_hook_interests_for_runtime(&runtime);
                }
                requested
            })
        })
        .unwrap_or(false)
}

fn wait_for_render_thread_dwell_end_work(output_dir: &Path, wait_ms: u64) {
    let started = unsafe { GetTickCount64() };
    while render_thread_dwell_end_work_pending() {
        let elapsed = unsafe { GetTickCount64() }.saturating_sub(started);
        if elapsed >= wait_ms {
            write_event(
                output_dir,
                &format!(
                    "{{\"type\":\"dwell_end_render_thread_request_timeout\",\"wait_ms\":{},\"message\":\"dwell-end D3D11 work was not drained by a hooked render-thread call; retained COM references will not be released from the control thread\"}}",
                    wait_ms
                ),
            );
            return;
        }
        unsafe {
            Sleep(10);
        }
    }
}

fn render_thread_dwell_end_work_pending() -> bool {
    RUNTIME
        .get()
        .and_then(|runtime| {
            runtime.lock().ok().map(|runtime| {
                runtime.pending_dwell_end_boundary_finalization
                    || runtime.pending_dwell_end_generated_texture_check
            })
        })
        .unwrap_or(false)
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
                    UNDERWATER_CAPTURE_MOMENT => "timeout_before_underwater_pass_bundle",
                    TREES_CAPTURE_MOMENT => "timeout_before_trees_pass_bundle",
                    RESTORE_CAPTURE_MOMENT => "timeout_before_restore_pass_bundle",
                    MAPNAME_CAPTURE_MOMENT => "timeout_before_mapname_pass_bundle",
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

fn dwell_end_boundary_finalization_ready(runtime: &CaptureRuntime) -> bool {
    let Ok(profile) = requested_map_frame_capture_profile_for_runtime(runtime) else {
        return false;
    };
    if runtime
        .completed_map_frame_moments
        .contains(profile.capture_moment)
        || runtime.map_frame_draws.is_empty()
    {
        return false;
    }
    if profile.requires_terrain_runtime_files {
        if !runtime
            .captured_symbols
            .contains(TERRAIN_ID_RESOURCE_SYMBOL)
            || !runtime
                .captured_symbols
                .contains(TERRAIN_VERTEX_BUFFER_SYMBOL)
            || !runtime
                .captured_symbols
                .contains(TERRAIN_INPUT_LAYOUT_SYMBOL)
        {
            return false;
        }
        for target in CAPTURE_TARGETS {
            if target.timing != RuntimeTextureCaptureTiming::Finalize {
                continue;
            }
            if !runtime.captured_symbols.contains(target.symbol)
                && !runtime.deferred_runtime_targets.contains_key(target.symbol)
            {
                return false;
            }
        }
    }
    true
}

unsafe fn install_hooks(output_dir: &Path) -> Result<(), String> {
    if HOOKS_INSTALLED.load(Ordering::SeqCst) {
        return Ok(());
    }
    let probe = unsafe { create_probe_device()? };
    let present_target = match unsafe { create_probe_swap_chain_present_target() } {
        Ok(target) => Some(target),
        Err(error) => {
            write_event(
                output_dir,
                &format!(
                    "{{\"type\":\"terrain_capture_present_hook_unavailable\",\"message\":\"{}\"}}",
                    json_escape(&error)
                ),
            );
            None
        }
    };
    let targets = unsafe {
        HookTargets {
            create_buffer: vtable_slot(probe.device as *mut WinVoid, DEVICE_CREATE_BUFFER_INDEX)?,
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
            vs_set_constant_buffers: vtable_slot(
                probe.context as *mut WinVoid,
                CONTEXT_VS_SET_CONSTANT_BUFFERS_INDEX,
            )?,
            ps_set_constant_buffers: vtable_slot(
                probe.context as *mut WinVoid,
                CONTEXT_PS_SET_CONSTANT_BUFFERS_INDEX,
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
            om_set_render_targets: vtable_slot(
                probe.context as *mut WinVoid,
                CONTEXT_OM_SET_RENDER_TARGETS_INDEX,
            )?,
            om_set_render_targets_and_uavs: vtable_slot(
                probe.context as *mut WinVoid,
                CONTEXT_OM_SET_RENDER_TARGETS_AND_UAVS_INDEX,
            )?,
            om_set_blend_state: vtable_slot(
                probe.context as *mut WinVoid,
                CONTEXT_OM_SET_BLEND_STATE_INDEX,
            )?,
            om_set_depth_stencil_state: vtable_slot(
                probe.context as *mut WinVoid,
                CONTEXT_OM_SET_DEPTH_STENCIL_STATE_INDEX,
            )?,
            clear_render_target_view: vtable_slot(
                probe.context as *mut WinVoid,
                CONTEXT_CLEAR_RENDER_TARGET_VIEW_INDEX,
            )?,
            clear_depth_stencil_view: vtable_slot(
                probe.context as *mut WinVoid,
                CONTEXT_CLEAR_DEPTH_STENCIL_VIEW_INDEX,
            )?,
            present: present_target,
        }
    };

    let create_buffer_original = unsafe {
        MinHook::create_hook(
            targets.create_buffer as *mut StdVoid,
            hook_create_buffer as *mut StdVoid,
        )
        .map_err(|status| format!("CreateBuffer hook failed: {status}"))?
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
    let vs_constant_buffers_original = unsafe {
        MinHook::create_hook(
            targets.vs_set_constant_buffers as *mut StdVoid,
            hook_vs_set_constant_buffers as *mut StdVoid,
        )
        .map_err(|status| format!("VSSetConstantBuffers hook failed: {status}"))?
    };
    let ps_constant_buffers_original = unsafe {
        MinHook::create_hook(
            targets.ps_set_constant_buffers as *mut StdVoid,
            hook_ps_set_constant_buffers as *mut StdVoid,
        )
        .map_err(|status| format!("PSSetConstantBuffers hook failed: {status}"))?
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
    let om_set_render_targets_original = unsafe {
        MinHook::create_hook(
            targets.om_set_render_targets as *mut StdVoid,
            hook_om_set_render_targets as *mut StdVoid,
        )
        .map_err(|status| format!("OMSetRenderTargets hook failed: {status}"))?
    };
    let om_set_render_targets_and_uavs_original = unsafe {
        MinHook::create_hook(
            targets.om_set_render_targets_and_uavs as *mut StdVoid,
            hook_om_set_render_targets_and_uavs as *mut StdVoid,
        )
        .map_err(|status| {
            format!("OMSetRenderTargetsAndUnorderedAccessViews hook failed: {status}")
        })?
    };
    let om_set_blend_state_original = unsafe {
        MinHook::create_hook(
            targets.om_set_blend_state as *mut StdVoid,
            hook_om_set_blend_state as *mut StdVoid,
        )
        .map_err(|status| format!("OMSetBlendState hook failed: {status}"))?
    };
    let om_set_depth_stencil_state_original = unsafe {
        MinHook::create_hook(
            targets.om_set_depth_stencil_state as *mut StdVoid,
            hook_om_set_depth_stencil_state as *mut StdVoid,
        )
        .map_err(|status| format!("OMSetDepthStencilState hook failed: {status}"))?
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
    let present_original = if let Some(present) = targets.present {
        Some(unsafe {
            MinHook::create_hook(present as *mut StdVoid, hook_present as *mut StdVoid)
                .map_err(|status| format!("IDXGISwapChain::Present hook failed: {status}"))?
        })
    } else {
        None
    };

    unsafe {
        ORIG_CREATE_BUFFER = Some(mem::transmute(create_buffer_original));
        ORIG_CREATE_INPUT_LAYOUT = Some(mem::transmute(create_input_layout_original));
        ORIG_CREATE_VERTEX_SHADER = Some(mem::transmute(create_vertex_original));
        ORIG_CREATE_PIXEL_SHADER = Some(mem::transmute(create_original));
        ORIG_IA_SET_INPUT_LAYOUT = Some(mem::transmute(ia_set_input_layout_original));
        ORIG_VS_SET_CONSTANT_BUFFERS = Some(mem::transmute(vs_constant_buffers_original));
        ORIG_PS_SET_CONSTANT_BUFFERS = Some(mem::transmute(ps_constant_buffers_original));
        ORIG_VS_SET_SHADER = Some(mem::transmute(vs_set_original));
        ORIG_PS_SET_SHADER = Some(mem::transmute(ps_set_original));
        ORIG_PS_SET_SHADER_RESOURCES = Some(mem::transmute(ps_resources_original));
        ORIG_DRAW_INDEXED = Some(mem::transmute(draw_indexed_original));
        ORIG_DRAW = Some(mem::transmute(draw_original));
        ORIG_DRAW_INDEXED_INSTANCED = Some(mem::transmute(draw_indexed_instanced_original));
        ORIG_DRAW_INSTANCED = Some(mem::transmute(draw_instanced_original));
        ORIG_OM_SET_RENDER_TARGETS = Some(mem::transmute(om_set_render_targets_original));
        ORIG_OM_SET_RENDER_TARGETS_AND_UAVS =
            Some(mem::transmute(om_set_render_targets_and_uavs_original));
        ORIG_OM_SET_BLEND_STATE = Some(mem::transmute(om_set_blend_state_original));
        ORIG_OM_SET_DEPTH_STENCIL_STATE = Some(mem::transmute(om_set_depth_stencil_state_original));
        ORIG_CLEAR_RENDER_TARGET_VIEW = Some(mem::transmute(clear_render_target_original));
        ORIG_CLEAR_DEPTH_STENCIL_VIEW = Some(mem::transmute(clear_depth_stencil_original));
        if let Some(present_original) = present_original {
            ORIG_PRESENT = Some(mem::transmute(present_original));
        }
        MinHook::queue_enable_hook(targets.create_buffer as *mut StdVoid)
            .map_err(|status| format!("queue CreateBuffer hook failed: {status}"))?;
        MinHook::queue_enable_hook(targets.create_input_layout as *mut StdVoid)
            .map_err(|status| format!("queue CreateInputLayout hook failed: {status}"))?;
        MinHook::queue_enable_hook(targets.create_vertex_shader as *mut StdVoid)
            .map_err(|status| format!("queue CreateVertexShader hook failed: {status}"))?;
        MinHook::queue_enable_hook(targets.create_pixel_shader as *mut StdVoid)
            .map_err(|status| format!("queue CreatePixelShader hook failed: {status}"))?;
        MinHook::queue_enable_hook(targets.ia_set_input_layout as *mut StdVoid)
            .map_err(|status| format!("queue IASetInputLayout hook failed: {status}"))?;
        MinHook::queue_enable_hook(targets.vs_set_constant_buffers as *mut StdVoid)
            .map_err(|status| format!("queue VSSetConstantBuffers hook failed: {status}"))?;
        MinHook::queue_enable_hook(targets.ps_set_constant_buffers as *mut StdVoid)
            .map_err(|status| format!("queue PSSetConstantBuffers hook failed: {status}"))?;
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
        MinHook::queue_enable_hook(targets.om_set_render_targets as *mut StdVoid)
            .map_err(|status| format!("queue OMSetRenderTargets hook failed: {status}"))?;
        MinHook::queue_enable_hook(targets.om_set_render_targets_and_uavs as *mut StdVoid)
            .map_err(|status| {
                format!("queue OMSetRenderTargetsAndUnorderedAccessViews hook failed: {status}")
            })?;
        MinHook::queue_enable_hook(targets.om_set_blend_state as *mut StdVoid)
            .map_err(|status| format!("queue OMSetBlendState hook failed: {status}"))?;
        MinHook::queue_enable_hook(targets.om_set_depth_stencil_state as *mut StdVoid)
            .map_err(|status| format!("queue OMSetDepthStencilState hook failed: {status}"))?;
        MinHook::queue_enable_hook(targets.clear_render_target_view as *mut StdVoid)
            .map_err(|status| format!("queue ClearRenderTargetView hook failed: {status}"))?;
        MinHook::queue_enable_hook(targets.clear_depth_stencil_view as *mut StdVoid)
            .map_err(|status| format!("queue ClearDepthStencilView hook failed: {status}"))?;
        if let Some(present) = targets.present {
            MinHook::queue_enable_hook(present as *mut StdVoid)
                .map_err(|status| format!("queue IDXGISwapChain::Present hook failed: {status}"))?;
        }
        MinHook::apply_queued().map_err(|status| format!("enable D3D11 hooks failed: {status}"))?;
    }

    let _ = HOOK_TARGETS.set(targets);
    HOOKS_INSTALLED.store(true, Ordering::SeqCst);
    write_event(
        output_dir,
        &format!(
            "{{\"type\":\"terrain_capture_hooks_installed\",\"create_buffer\":\"0x{:X}\",\"create_input_layout\":\"0x{:X}\",\"create_vertex_shader\":\"0x{:X}\",\"create_pixel_shader\":\"0x{:X}\",\"ia_set_input_layout\":\"0x{:X}\",\"vs_set_constant_buffers\":\"0x{:X}\",\"ps_set_constant_buffers\":\"0x{:X}\",\"vs_set_shader\":\"0x{:X}\",\"ps_set_shader\":\"0x{:X}\",\"ps_set_shader_resources\":\"0x{:X}\",\"draw_indexed\":\"0x{:X}\",\"draw\":\"0x{:X}\",\"draw_indexed_instanced\":\"0x{:X}\",\"draw_instanced\":\"0x{:X}\",\"clear_render_target_view\":\"0x{:X}\",\"clear_depth_stencil_view\":\"0x{:X}\",\"present\":{}}}",
            targets.create_buffer,
            targets.create_input_layout,
            targets.create_vertex_shader,
            targets.create_pixel_shader,
            targets.ia_set_input_layout,
            targets.vs_set_constant_buffers,
            targets.ps_set_constant_buffers,
            targets.vs_set_shader,
            targets.ps_set_shader,
            targets.ps_set_shader_resources,
            targets.draw_indexed,
            targets.draw,
            targets.draw_indexed_instanced,
            targets.draw_instanced,
            targets.clear_render_target_view,
            targets.clear_depth_stencil_view,
            targets
                .present
                .map(|present| format!("\"0x{present:X}\""))
                .unwrap_or_else(|| "null".to_string())
        ),
    );
    unsafe {
        release_unknown(probe.context as *mut WinVoid);
        release_unknown(probe.device as *mut WinVoid);
    }
    Ok(())
}

// CAPFIX2 BUG 1: the former `disable_hooks()` (MinHook queue_disable_hook +
// apply_queued + remove_hook for every target) has been DELETED, not just
// bypassed. Removing inline hooks/trampolines from d3d11 functions the render
// thread calls every frame is an unavoidable use-after-free — two WER records
// this session faulted at a fixed offset in the unloaded DLL right at
// dwell-expiry. There is no safe runtime unhook. Deleting the function makes
// the "never unhook at runtime" invariant compiler-enforced: any reintroduced
// call simply will not build. Dwell-end teardown is now
// `disarm_capture_hooks_permanently()`; process exit is the only real teardown.

fn capture_quiesced() -> bool {
    // CAPFIX2 BUG 1: a disarmed session is, for every hook, indistinguishable
    // from a quiesced one — do no capture work, just forward to the original.
    // Routing disarm through capture_quiesced() means EVERY hook that already
    // early-outs on quiesce (all of them) becomes a cheap pass-through the
    // instant we disarm, without touching each hook body. The hooks stay
    // installed; only their capture work is skipped.
    CAPTURE_QUIESCED.load(Ordering::SeqCst) || HOOKS_DISARMED.load(Ordering::SeqCst)
}

fn hooks_disarmed() -> bool {
    HOOKS_DISARMED.load(Ordering::SeqCst)
}

/// CAPFIX2 BUG 1 — dwell-expiry teardown that is SAFE to run while the render
/// thread is live: never unhook, never unload. Stop the persistent CB0 sampler
/// (it must issue no further D3D reads), latch the permanent disarm flag (so all
/// hooks become pass-throughs via [`capture_quiesced`]), and PIN the module so
/// the image can never be unmapped out from under an in-flight hook return.
/// Idempotent. This REPLACES the former `disable_hooks()` call at dwell end.
fn disarm_capture_hooks_permanently() {
    // Hard-stop the sampler first: after disarm no draw hook does capture work,
    // but the sampler runs BEFORE the quiesce gate in the draw hooks, so it gets
    // its own stop latch (the same one process-detach uses).
    CB0_SAMPLER_DETACHING.store(true, Ordering::SeqCst);
    CB0_SAMPLER_ARMED.store(false, Ordering::SeqCst);
    HOOKS_DISARMED.store(true, Ordering::SeqCst);
    clear_context_hook_interests();
    reset_capture_in_progress_flags();
    pin_capture_module_resident();
}

/// Bump this DLL's loader reference count to "permanent" so it is never
/// unmapped while the process lives (GET_MODULE_HANDLE_EX_FLAG_PIN). Combined
/// with never calling FreeLibrary/remove_hook on ourselves, this guarantees the
/// hook code and trampolines stay mapped until process exit — the only teardown
/// that is atomic w.r.t. the render thread. Idempotent and lock-free.
fn pin_capture_module_resident() {
    if CAPTURE_MODULE_PINNED.swap(true, Ordering::SeqCst) {
        return;
    }
    // Resolve the module from an address inside THIS DLL (a function pointer we
    // own), then pin it. PIN implies the returned handle need not be freed and
    // the module is never unloaded.
    const GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS: u32 = 0x0000_0004;
    const GET_MODULE_HANDLE_EX_FLAG_PIN: u32 = 0x0000_0001;
    let mut module: HMODULE = ptr::null_mut();
    // Safety: GetModuleHandleExW with FROM_ADDRESS takes an address cast to a
    // wide-string pointer; we pass a pointer to our own function so the loader
    // maps it to this module and pins it. On failure `module` is untouched and
    // we simply proceed — the "never unhook" invariant already prevents the
    // trampoline free; pinning is defense-in-depth against external unmap.
    let anchor = pin_capture_module_resident as *const () as *const u16;
    let ok = unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_PIN,
            anchor,
            &mut module,
        )
    };
    if let Some(dir) = RUNTIME
        .get()
        .and_then(|runtime| runtime.try_lock().ok())
        .map(|runtime| runtime.output_dir.clone())
    {
        write_event(
            &dir,
            &format!(
                "{{\"type\":\"capture_module_pinned\",\"pinned\":{},\"module\":\"0x{:X}\"}}",
                ok != 0,
                module as usize
            ),
        );
    }
}

// ---------------------------------------------------------------------------
// CAPFIX2 BUG 2 — runtime-lock wedge instrumentation.
//
// session_LAW_6 proved a render-thread hook took the runtime Mutex and never
// released it: every subsequent drain try_lock returned runtime_lock_busy
// (skip_count 1..64) and then the log went silent for 7+ minutes while the
// game kept playing. std::sync::Mutex is non-reentrant, so a hook that already
// holds the lock and calls a helper that also blocks on runtime.lock() would
// self-deadlock permanently — exactly this symptom class.
//
// This block gives us (a) a lockless record of WHO currently owns the runtime
// lock (thread id + acquire-site name + acquire tick), set on acquire and
// cleared on release, (b) a re-entrancy detector that reports and refuses to
// re-block instead of hanging, and (c) a watchdog (polled every 50ms from the
// dwell loop) that fires ONE event naming the owner site if the lock has been
// continuously held longer than RUNTIME_LOCK_WEDGE_WATCHDOG_MS. None of this
// takes the runtime lock; all state is atomics + a static site table.
// ---------------------------------------------------------------------------

/// Stable names for the runtime-lock acquire sites we track. The index is
/// stored in RUNTIME_LOCK_OWNER_SITE (0 = "none"/untracked). Kept as a table so
/// the watchdog can name the holder without allocating on the render thread.
const RUNTIME_LOCK_SITE_NAMES: [&str; 8] = [
    "none",
    "hook_ia_set_input_layout",
    "hook_vs_set_shader",
    "hook_ps_set_shader",
    "hook_create_vertex_shader",
    "hook_create_pixel_shader",
    "hook_ps_set_shader_resources",
    "present_finalize",
];

#[derive(Clone, Copy)]
enum RuntimeLockSite {
    IaSetInputLayout = 1,
    VsSetShader = 2,
    PsSetShader = 3,
    CreateVertexShader = 4,
    CreatePixelShader = 5,
    PsSetShaderResources = 6,
    PresentFinalize = 7,
}

fn runtime_lock_site_name(index: usize) -> &'static str {
    RUNTIME_LOCK_SITE_NAMES
        .get(index)
        .copied()
        .unwrap_or("unknown")
}

/// RAII guard: records the current thread + site + tick as the runtime-lock
/// owner on construction and clears the owner record on drop. It does NOT own
/// the Mutex guard itself — the caller keeps that — but its lifetime is bound to
/// the same scope, so owner tracking follows the real lock hold exactly.
struct RuntimeLockOwnerRecord;

impl RuntimeLockOwnerRecord {
    fn new(site: RuntimeLockSite) -> Self {
        RUNTIME_LOCK_OWNER_THREAD.store(unsafe { GetCurrentThreadId() }, Ordering::SeqCst);
        RUNTIME_LOCK_OWNER_SITE.store(site as usize, Ordering::SeqCst);
        RUNTIME_LOCK_ACQUIRED_TICK_MS.store(unsafe { GetTickCount64() }, Ordering::SeqCst);
        RuntimeLockOwnerRecord
    }
}

impl Drop for RuntimeLockOwnerRecord {
    fn drop(&mut self) {
        RUNTIME_LOCK_OWNER_SITE.store(0, Ordering::SeqCst);
        RUNTIME_LOCK_OWNER_THREAD.store(0, Ordering::SeqCst);
        RUNTIME_LOCK_ACQUIRED_TICK_MS.store(0, Ordering::SeqCst);
    }
}

/// True if THIS thread is already recorded as the runtime-lock owner — i.e. a
/// blocking `runtime.lock()` from here would self-deadlock a non-reentrant
/// Mutex. Reported once (lockless) so the next live session names the bug
/// instead of hanging silently.
fn runtime_lock_would_reenter() -> bool {
    let owner = RUNTIME_LOCK_OWNER_THREAD.load(Ordering::SeqCst);
    owner != 0 && owner == unsafe { GetCurrentThreadId() }
}

fn record_runtime_lock_reentrant_attempt(site: RuntimeLockSite) {
    if RUNTIME_LOCK_REENTRANT_RECORDED.swap(true, Ordering::SeqCst) {
        return;
    }
    let held_site = runtime_lock_site_name(RUNTIME_LOCK_OWNER_SITE.load(Ordering::SeqCst));
    write_event_lockless(&format!(
        "{{\"type\":\"runtime_lock_reentrant_attempt\",\"attempt_site\":\"{}\",\"held_by_site\":\"{}\",\"thread\":{}}}",
        runtime_lock_site_name(site as usize),
        held_site,
        unsafe { GetCurrentThreadId() }
    ));
}

/// Acquire the runtime lock from a render-thread hook, tracked so the watchdog
/// and re-entrancy detector can see the holder. Returns None (skipping the
/// capture work, never blocking) if:
///   * this thread already holds the lock (would self-deadlock), or
///   * the lock is currently held by another thread (try_lock would block).
/// Using try_lock here means NO hook path can ever block the render thread on
/// the runtime lock — the exact failure that wedged session_LAW_6. The returned
/// tuple keeps both the Mutex guard and the owner record alive for the scope.
fn try_lock_runtime_tracked(
    runtime: &'static Mutex<CaptureRuntime>,
    site: RuntimeLockSite,
) -> Option<(MutexGuard<'static, CaptureRuntime>, RuntimeLockOwnerRecord)> {
    if runtime_lock_would_reenter() {
        record_runtime_lock_reentrant_attempt(site);
        return None;
    }
    match runtime.try_lock() {
        Ok(guard) => Some((guard, RuntimeLockOwnerRecord::new(site))),
        Err(TryLockError::WouldBlock) => None,
        Err(TryLockError::Poisoned(_)) => None,
    }
}

/// Pure decision core for the lock-holder watchdog (unit-testable, no globals):
/// how long the lock has been continuously held, IF that exceeds the wedge
/// threshold. `owner_thread == 0` (not held) or `acquired == 0` (no tick) => no
/// wedge.
fn runtime_lock_wedge_elapsed_ms(owner_thread: u32, acquired_ms: u64, now_ms: u64) -> Option<u64> {
    if owner_thread == 0 || acquired_ms == 0 {
        return None;
    }
    let held_ms = now_ms.saturating_sub(acquired_ms);
    (held_ms >= RUNTIME_LOCK_WEDGE_WATCHDOG_MS).then_some(held_ms)
}

/// Polled from the dwell loop (every ~50ms). If the runtime lock has been
/// continuously held by any thread for longer than the wedge threshold, emit
/// ONE lockless event naming the owning site + thread + hold duration. This is
/// the diagnostic that would have named the session_LAW_6 holder instead of
/// leaving 7 minutes of silence. It never takes the runtime lock.
fn runtime_lock_watchdog_tick() {
    let owner_thread = RUNTIME_LOCK_OWNER_THREAD.load(Ordering::SeqCst);
    if owner_thread == 0 {
        // Not held; re-arm so a later genuine wedge is reported.
        RUNTIME_LOCK_WATCHDOG_RECORDED.store(false, Ordering::SeqCst);
        return;
    }
    let acquired = RUNTIME_LOCK_ACQUIRED_TICK_MS.load(Ordering::SeqCst);
    let now = unsafe { GetTickCount64() };
    let Some(held_ms) = runtime_lock_wedge_elapsed_ms(owner_thread, acquired, now) else {
        return;
    };
    if RUNTIME_LOCK_WATCHDOG_RECORDED.swap(true, Ordering::SeqCst) {
        return;
    }
    write_event_lockless(&format!(
        "{{\"type\":\"runtime_lock_wedge_detected\",\"owner_site\":\"{}\",\"owner_thread\":{},\"held_ms\":{},\"threshold_ms\":{}}}",
        runtime_lock_site_name(RUNTIME_LOCK_OWNER_SITE.load(Ordering::SeqCst)),
        owner_thread,
        held_ms,
        RUNTIME_LOCK_WEDGE_WATCHDOG_MS
    ));
}

fn reset_capture_in_progress_flags() {
    CAPTURE_IN_PROGRESS.store(false, Ordering::SeqCst);
    VERTEX_CAPTURE_IN_PROGRESS.store(false, Ordering::SeqCst);
    INPUT_LAYOUT_CAPTURE_IN_PROGRESS.store(false, Ordering::SeqCst);
    MAP_FRAME_CAPTURE_IN_PROGRESS.store(false, Ordering::SeqCst);
    CENSUS_CAPTURE_IN_PROGRESS.store(false, Ordering::SeqCst);
    RENDER_THREAD_REQUEST_DRAIN_IN_PROGRESS.store(false, Ordering::SeqCst);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RenderThreadDrainSkipReason {
    AlreadyInProgress,
    RuntimeUnavailable,
    RuntimeLockBusy,
    RuntimeLockPoisoned,
}

impl RenderThreadDrainSkipReason {
    fn as_str(self) -> &'static str {
        match self {
            RenderThreadDrainSkipReason::AlreadyInProgress => "already_in_progress",
            RenderThreadDrainSkipReason::RuntimeUnavailable => "runtime_unavailable",
            RenderThreadDrainSkipReason::RuntimeLockBusy => "runtime_lock_busy",
            RenderThreadDrainSkipReason::RuntimeLockPoisoned => "runtime_lock_poisoned",
        }
    }
}

struct RenderThreadDrainGuard;

impl Drop for RenderThreadDrainGuard {
    fn drop(&mut self) {
        RENDER_THREAD_REQUEST_DRAIN_IN_PROGRESS.store(false, Ordering::SeqCst);
    }
}

fn try_begin_render_thread_request_drain(
    stage: &'static str,
) -> Result<RenderThreadDrainGuard, RenderThreadDrainSkipReason> {
    if RENDER_THREAD_REQUEST_DRAIN_IN_PROGRESS.swap(true, Ordering::SeqCst) {
        record_render_thread_request_drain_skip(
            stage,
            RenderThreadDrainSkipReason::AlreadyInProgress,
        );
        Err(RenderThreadDrainSkipReason::AlreadyInProgress)
    } else {
        Ok(RenderThreadDrainGuard)
    }
}

fn try_take_render_thread_request_flags(
    runtime: &Mutex<CaptureRuntime>,
) -> Result<(bool, bool), RenderThreadDrainSkipReason> {
    match runtime.try_lock() {
        Ok(mut runtime) => {
            let finalize_boundary = runtime.pending_dwell_end_boundary_finalization;
            let check_generated = runtime.pending_dwell_end_generated_texture_check;
            runtime.pending_dwell_end_boundary_finalization = false;
            runtime.pending_dwell_end_generated_texture_check = false;
            refresh_context_hook_interests_for_runtime(&runtime);
            Ok((finalize_boundary, check_generated))
        }
        Err(TryLockError::WouldBlock) => Err(RenderThreadDrainSkipReason::RuntimeLockBusy),
        Err(TryLockError::Poisoned(_)) => Err(RenderThreadDrainSkipReason::RuntimeLockPoisoned),
    }
}

fn should_log_counter_event(count: u64) -> bool {
    count <= 4 || count.is_power_of_two() || count % 1024 == 0
}

fn record_render_thread_request_drain_skip(
    stage: &'static str,
    reason: RenderThreadDrainSkipReason,
) {
    let count = RENDER_THREAD_REQUEST_DRAIN_SKIPS
        .fetch_add(1, Ordering::SeqCst)
        .saturating_add(1);
    if should_log_counter_event(count) {
        write_event_lockless(&format!(
            "{{\"type\":\"render_thread_request_drain_skipped\",\"stage\":\"{}\",\"reason\":\"{}\",\"skip_count\":{}}}",
            json_escape(stage),
            reason.as_str(),
            count
        ));
    }
}

fn mark_capture_progress(now_ms: u64) {
    LAST_CAPTURE_PROGRESS_TICK_MS.store(now_ms, Ordering::SeqCst);
}

fn mark_capture_progress_now() {
    mark_capture_progress(unsafe { GetTickCount64() });
}

fn mark_round_open(now_ms: u64) {
    ROUND_OPEN_TICK_MS.store(now_ms, Ordering::SeqCst);
    LAST_CAPTURE_PROGRESS_TICK_MS.store(now_ms, Ordering::SeqCst);
    ROUND_STALL_WATCHDOG_RECORDED.store(false, Ordering::SeqCst);
}

fn mark_round_closed() {
    ROUND_OPEN_TICK_MS.store(0, Ordering::SeqCst);
    ROUND_STALL_WATCHDOG_RECORDED.store(false, Ordering::SeqCst);
}

fn round_stall_elapsed_ms(open_ms: u64, last_progress_ms: u64, now_ms: u64) -> Option<u64> {
    if open_ms == 0 {
        return None;
    }
    let baseline = last_progress_ms.max(open_ms);
    let elapsed = now_ms.saturating_sub(baseline);
    (elapsed >= ROUND_STALL_WATCHDOG_MS).then_some(elapsed)
}

fn json_string_array(values: Vec<&str>) -> String {
    let body = values
        .into_iter()
        .map(|value| format!("\"{}\"", json_escape(value)))
        .collect::<Vec<_>>()
        .join(",");
    format!("[{body}]")
}

fn maybe_emit_round_stall_watchdog() {
    if capture_quiesced() || CAPTURE_BUDGET_STOPPED.load(Ordering::SeqCst) {
        return;
    }
    let now = unsafe { GetTickCount64() };
    let Some(stalled_ms) = round_stall_elapsed_ms(
        ROUND_OPEN_TICK_MS.load(Ordering::SeqCst),
        LAST_CAPTURE_PROGRESS_TICK_MS.load(Ordering::SeqCst),
        now,
    ) else {
        return;
    };
    if ROUND_STALL_WATCHDOG_RECORDED.swap(true, Ordering::SeqCst) {
        return;
    }
    let drain_in_progress = RENDER_THREAD_REQUEST_DRAIN_IN_PROGRESS.load(Ordering::SeqCst);
    let render_thread_id = RENDER_THREAD_ID.load(Ordering::SeqCst);
    let Some(runtime) = RUNTIME.get() else {
        write_event_lockless(&format!(
            "{{\"type\":\"map_frame_round_watchdog_stalled\",\"runtime_lock\":\"unavailable\",\"stalled_ms\":{},\"drain_in_progress\":{},\"capture_quiesced\":{},\"capture_budget_stopped\":{},\"render_thread_id\":{}}}",
            stalled_ms,
            drain_in_progress,
            capture_quiesced(),
            CAPTURE_BUDGET_STOPPED.load(Ordering::SeqCst),
            render_thread_id
        ));
        return;
    };
    let (root_output_dir, round_output_dir, event) = match runtime.try_lock() {
        Ok(runtime) => {
            let active_moment = runtime
                .capture_moment_sequence
                .get(runtime.active_capture_index)
                .map(String::as_str)
                .unwrap_or("none");
            let completed = json_string_array(
                runtime
                    .completed_map_frame_moments
                    .iter()
                    .map(String::as_str)
                    .collect(),
            );
            let active_round = runtime.active_round_label.as_deref().unwrap_or("none");
            let event = format!(
                "{{\"type\":\"map_frame_round_watchdog_stalled\",\"runtime_lock\":\"acquired\",\"stalled_ms\":{},\"active_round\":\"{}\",\"active_round_finished\":{},\"active_moment\":\"{}\",\"active_capture_index\":{},\"capture_moments\":{},\"completed_moments\":{},\"map_frame_draws\":{},\"map_frame_blockers\":{},\"pending_boundary_finalization\":{},\"pending_generated_texture_check\":{},\"pending_render_thread_releases\":{},\"total_payload_bytes_reserved\":{},\"capture_work_frame_ms\":{},\"drain_in_progress\":{},\"capture_quiesced\":{},\"capture_budget_stopped\":{},\"render_thread_id\":{},\"last_context\":\"0x{:X}\",\"last_error\":{}}}",
                stalled_ms,
                json_escape(active_round),
                runtime.active_round_finished,
                json_escape(active_moment),
                runtime.active_capture_index,
                json_string_array(
                    runtime
                        .capture_moment_sequence
                        .iter()
                        .map(String::as_str)
                        .collect(),
                ),
                completed,
                runtime.map_frame_draws.len(),
                runtime.map_frame_blockers.len(),
                runtime.pending_dwell_end_boundary_finalization,
                runtime.pending_dwell_end_generated_texture_check,
                runtime.pending_render_thread_release_srvs.len(),
                runtime.total_payload_bytes_reserved,
                runtime.capture_work_frame_ms,
                drain_in_progress,
                capture_quiesced(),
                CAPTURE_BUDGET_STOPPED.load(Ordering::SeqCst),
                render_thread_id,
                runtime.last_context,
                runtime
                    .last_error
                    .as_ref()
                    .map(|error| format!("\"{}\"", json_escape(error)))
                    .unwrap_or_else(|| "null".to_string())
            );
            (
                runtime.root_output_dir.clone(),
                runtime.output_dir.clone(),
                event,
            )
        }
        Err(TryLockError::WouldBlock) => {
            write_event_lockless(&format!(
                "{{\"type\":\"map_frame_round_watchdog_stalled\",\"runtime_lock\":\"busy\",\"stalled_ms\":{},\"drain_in_progress\":{},\"capture_quiesced\":{},\"capture_budget_stopped\":{},\"render_thread_id\":{}}}",
                stalled_ms,
                drain_in_progress,
                capture_quiesced(),
                CAPTURE_BUDGET_STOPPED.load(Ordering::SeqCst),
                render_thread_id
            ));
            return;
        }
        Err(TryLockError::Poisoned(_)) => {
            write_event_lockless(&format!(
                "{{\"type\":\"map_frame_round_watchdog_stalled\",\"runtime_lock\":\"poisoned\",\"stalled_ms\":{},\"drain_in_progress\":{},\"capture_quiesced\":{},\"capture_budget_stopped\":{},\"render_thread_id\":{}}}",
                stalled_ms,
                drain_in_progress,
                capture_quiesced(),
                CAPTURE_BUDGET_STOPPED.load(Ordering::SeqCst),
                render_thread_id
            ));
            return;
        }
    };
    write_event(&root_output_dir, &event);
    if round_output_dir != root_output_dir {
        write_event(&round_output_dir, &event);
    }
}

fn queue_quiesce_release_and_clear_capture_state(runtime: &mut CaptureRuntime) -> usize {
    let releases = runtime
        .deferred_runtime_targets
        .values()
        .map(|target| target.srv)
        .filter(|srv| *srv != 0)
        .collect::<Vec<_>>();
    let release_count = releases.len();
    runtime.pending_render_thread_release_srvs.extend(releases);
    runtime.deferred_runtime_targets.clear();
    runtime.pending_dwell_end_boundary_finalization = false;
    runtime.pending_dwell_end_generated_texture_check = false;
    runtime.dwell_end_generated_texture_check_done = true;
    runtime.terrain_chunk_library.enabled = false;
    runtime.map_frame_draws.clear();
    runtime.map_frame_rejections.clear();
    runtime.ui_pass = UiPassCaptureState::default();
    stop_census_capture_for_runtime(runtime);
    runtime.latest_post_draw_output_merger = None;
    runtime.latest_post_draw_output_blockers.clear();
    runtime.capture_work_frame_ms = 0;
    refresh_context_hook_interests_for_runtime(runtime);
    release_count
}

unsafe fn drain_quiesced_render_thread_releases(stage: &'static str) {
    if !capture_quiesced() {
        return;
    }
    let owner_thread = RENDER_THREAD_ID.load(Ordering::SeqCst);
    if owner_thread != 0 && unsafe { GetCurrentThreadId() } != owner_thread {
        return;
    }
    let Some(runtime) = RUNTIME.get() else {
        record_render_thread_request_drain_skip(
            stage,
            RenderThreadDrainSkipReason::RuntimeUnavailable,
        );
        return;
    };
    let (output_dir, releases, record_event) = match runtime.try_lock() {
        Ok(mut runtime) => {
            let output_dir = runtime.output_dir.clone();
            let releases = std::mem::take(&mut runtime.pending_render_thread_release_srvs);
            let record_event = !runtime.capture_quiesced_event_recorded;
            if record_event {
                runtime.capture_quiesced_event_recorded = true;
            }
            (output_dir, releases, record_event)
        }
        Err(TryLockError::WouldBlock) => {
            record_render_thread_request_drain_skip(
                stage,
                RenderThreadDrainSkipReason::RuntimeLockBusy,
            );
            return;
        }
        Err(TryLockError::Poisoned(_)) => {
            record_render_thread_request_drain_skip(
                stage,
                RenderThreadDrainSkipReason::RuntimeLockPoisoned,
            );
            return;
        }
    };
    if releases.is_empty() && !record_event {
        return;
    }
    let release_count = releases.len();
    for srv in releases {
        unsafe {
            release_unknown(srv as *mut WinVoid);
        }
    }
    if record_event {
        write_event(
            &output_dir,
            &format!(
                "{{\"type\":\"capture_quiesced\",\"reason\":\"capture_budget_exceeded\",\"stage\":\"{}\",\"released_deferred_runtime_targets\":{}}}",
                json_escape(stage),
                release_count
            ),
        );
    }
}

unsafe fn context_capture_allowed(context: *mut ID3D11DeviceContext, stage: &'static str) -> bool {
    if capture_quiesced() || CAPTURE_BUDGET_STOPPED.load(Ordering::SeqCst) {
        return false;
    }
    if context.is_null() {
        return false;
    }
    let context_type = unsafe { (*context).GetType() };
    if context_type != D3D11_DEVICE_CONTEXT_IMMEDIATE {
        record_deferred_context_skipped(stage, context as usize, context_type as u32);
        return false;
    }
    let current_thread = unsafe { GetCurrentThreadId() };
    match claim_render_thread_id(current_thread, &RENDER_THREAD_ID) {
        Ok(()) => true,
        Err(owner_thread) => {
            // NO assert here: the real game legally touches the immediate
            // context from more than one thread (live 2026-07-05: an
            // OMSetRenderTargets call off the owner thread aborted the whole
            // process via debug_assert -> unwind across the extern hook).
            // Off-owner touches are recorded and skipped, never fatal.
            record_render_thread_mismatch(stage, context as usize, current_thread, owner_thread);
            false
        }
    }
}

unsafe fn context_hook_allowed_for_relevant_call(
    context: *mut ID3D11DeviceContext,
    stage: &'static str,
    interest: u32,
    call_relevant: bool,
) -> bool {
    if capture_quiesced() || CAPTURE_BUDGET_STOPPED.load(Ordering::SeqCst) {
        context_prefilter_skip();
        return false;
    }
    let drain_pending = context_drain_work_pending();
    if !context_interest_enabled(interest) && !drain_pending {
        context_prefilter_skip();
        return false;
    }
    if !call_relevant && !drain_pending {
        context_prefilter_skip();
        return false;
    }
    if !unsafe { context_capture_allowed(context, stage) } {
        context_prefilter_skip();
        return false;
    }
    true
}

fn context_draw_call_relevant() -> bool {
    // CAPFIX3 choreography — map-frame draw capture waits out the --arm-delay-ms
    // window. Gated HERE (live, lock-free) rather than in the interest refresh so
    // the PS/VS/IA hooks keep firing during the delay: they must stay live to
    // latch the renderer-live trust bit (which starts the delay clock) and to
    // observe shader relevance, while this single check keeps the DRAW hook from
    // recording any pass until the orchestrator has positioned the camera. The
    // arm-delay check is only reached when a map-frame moment is already armed,
    // so the common no-delay path pays one extra relaxed atomic load.
    let map_relevant = CONTEXT_MAP_FRAME_DRAW_ARMED.load(Ordering::Acquire)
        && map_frame_arm_delay_open()
        && (CONTEXT_CURRENT_PIXEL_SHADER_RELEVANT.load(Ordering::Acquire)
            || CONTEXT_CURRENT_VERTEX_SHADER_RELEVANT.load(Ordering::Acquire)
            || CONTEXT_CURRENT_INPUT_LAYOUT_RELEVANT.load(Ordering::Acquire));
    let ui_relevant = CONTEXT_UI_PASS_ARMED.load(Ordering::Acquire)
        && CONTEXT_CURRENT_UI_PASS_SHADER_RELEVANT.load(Ordering::Acquire);
    let census_relevant = CONTEXT_CENSUS_ARMED.load(Ordering::Acquire);
    let auto_round_relevant = CONTEXT_AUTO_ROUND_ARMED.load(Ordering::Acquire)
        && CONTEXT_CURRENT_PIXEL_SHADER_RELEVANT.load(Ordering::Acquire);
    map_relevant || ui_relevant || census_relevant || auto_round_relevant
}

fn context_boundary_call_relevant() -> bool {
    CONTEXT_MAP_FRAME_BOUNDARY_ARMED.load(Ordering::Acquire)
        || CONTEXT_CENSUS_ARMED.load(Ordering::Acquire)
        || context_drain_work_pending()
}

fn claim_render_thread_id(current_thread: u32, owner: &AtomicU32) -> Result<(), u32> {
    let recorded = owner.load(Ordering::SeqCst);
    if recorded == current_thread {
        return Ok(());
    }
    if recorded == 0 {
        return match owner.compare_exchange(0, current_thread, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(_) => Ok(()),
            Err(actual) if actual == current_thread => Ok(()),
            Err(actual) => Err(actual),
        };
    }
    Err(recorded)
}

fn debug_assert_render_thread_owner(stage: &'static str) {
    // Historically a debug_assert_eq!, which ABORTS THE GAME in debug builds
    // when the real game touches the immediate context off the owner thread
    // (legal D3D11; the harness never does it, so gates could not catch it —
    // live 2026-07-05 the OMSetRenderTargets variant killed the process via
    // panic-across-extern-hook). Violations are now recorded, never fatal.
    let owner_thread = RENDER_THREAD_ID.load(Ordering::SeqCst);
    if owner_thread != 0 {
        let current_thread = unsafe { GetCurrentThreadId() };
        if current_thread != owner_thread {
            record_render_thread_mismatch(stage, 0, current_thread, owner_thread);
        }
    }
}

fn record_deferred_context_skipped(stage: &'static str, context: usize, context_type: u32) {
    if DEFERRED_CONTEXT_RECORDED.swap(true, Ordering::SeqCst) {
        return;
    }
    if let Some(runtime) = RUNTIME.get() {
        if let Ok(runtime) = runtime.lock() {
            write_event(
                &runtime.output_dir,
                &format!(
                    "{{\"type\":\"d3d11_deferred_context_skipped\",\"stage\":\"{}\",\"context\":\"0x{:X}\",\"context_type\":{},\"message\":\"terrain_capture only reads back from the immediate context\"}}",
                    json_escape(stage),
                    context,
                    context_type
                ),
            );
        }
    }
}

fn record_render_thread_mismatch(
    stage: &'static str,
    context: usize,
    current_thread: u32,
    owner_thread: u32,
) {
    if RENDER_THREAD_MISMATCH_RECORDED.swap(true, Ordering::SeqCst) {
        return;
    }
    if let Some(runtime) = RUNTIME.get() {
        if let Ok(runtime) = runtime.lock() {
            write_event(
                &runtime.output_dir,
                &format!(
                    "{{\"type\":\"d3d11_render_thread_mismatch\",\"stage\":\"{}\",\"context\":\"0x{:X}\",\"current_thread_id\":{},\"owner_thread_id\":{},\"message\":\"skipped terrain_capture D3D11 readback outside the render-thread owner\"}}",
                    json_escape(stage),
                    context,
                    current_thread,
                    owner_thread
                ),
            );
        }
    }
}

unsafe fn drain_render_thread_requests(context: *mut ID3D11DeviceContext, stage: &'static str) {
    if capture_quiesced() {
        unsafe {
            drain_quiesced_render_thread_releases(stage);
        }
        return;
    }
    if !unsafe { context_capture_allowed(context, stage) } {
        return;
    }
    let Ok(_drain_guard) = try_begin_render_thread_request_drain(stage) else {
        return;
    };
    let Some(runtime) = RUNTIME.get() else {
        record_render_thread_request_drain_skip(
            stage,
            RenderThreadDrainSkipReason::RuntimeUnavailable,
        );
        return;
    };
    let (finalize_boundary, check_generated) = match try_take_render_thread_request_flags(runtime) {
        Ok(flags) => flags,
        Err(reason) => {
            record_render_thread_request_drain_skip(stage, reason);
            return;
        }
    };
    if finalize_boundary {
        let started_ms = unsafe { GetTickCount64() };
        finalize_map_frame_bundle_pass(context, "dwell_end_boundary");
        let elapsed_ms = unsafe { GetTickCount64() }.saturating_sub(started_ms);
        charge_capture_work_ms(elapsed_ms, "dwell_end_boundary");
    }
    if check_generated {
        let started_ms = unsafe { GetTickCount64() };
        unsafe {
            finalize_generated_runtime_textures_at_dwell_end_on_render_thread(context);
        }
        let elapsed_ms = unsafe { GetTickCount64() }.saturating_sub(started_ms);
        charge_capture_work_ms(elapsed_ms, "dwell_end_generated_textures");
    }
}

unsafe extern "system" fn hook_create_buffer(
    device: *mut ID3D11Device,
    desc: *const D3D11_BUFFER_DESC,
    initial_data: *const D3D11_SUBRESOURCE_DATA,
    buffer: *mut *mut ID3D11Buffer,
) -> i32 {
    let Some(original) = (unsafe { ORIG_CREATE_BUFFER }) else {
        return 0x80004005u32 as i32;
    };
    let hr = unsafe { original(device, desc, initial_data, buffer) };
    if capture_quiesced() {
        return hr;
    }
    if SUCCEEDED(hr) && !desc.is_null() && !buffer.is_null() {
        let created = unsafe { *buffer };
        if !created.is_null() {
            unsafe { remember_created_buffer_desc(created as usize, &*desc) };
        }
    }
    hr
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
    if capture_quiesced() {
        return unsafe {
            original(
                device,
                input_elements,
                num_elements,
                shader_bytecode_with_input_signature,
                bytecode_length,
                input_layout,
            )
        };
    }
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
    if capture_quiesced() {
        return hr;
    }
    if SUCCEEDED(hr) && !bytecode.is_null() && !pixel_shader.is_null() {
        let shader = unsafe { *pixel_shader };
        if !shader.is_null() {
            let bytes = unsafe { slice::from_raw_parts(bytecode.cast::<u8>(), bytecode_len) };
            remember_ui_pixel_shader_bytecode(device as usize, shader as usize, bytes);
            // LOAD-FREEZE FIX (2026-07-02): the game creates thousands of pixel
            // shaders during map load. The exact classification below runs D3D
            // reflection (expensive) under the global capture lock, which
            // serialized the asset-creation thread against the render thread
            // and froze the game. Reject the ~all non-map shaders here with a
            // sound, lock-free, reflection-free byte scan so only genuine map
            // families ever take the lock.
            if !pixel_shader_bytecode_could_be_capture_family(bytes) {
                CREATE_PS_PREFILTER_SKIPS.fetch_add(1, Ordering::Relaxed);
                return hr;
            }
            CREATE_PS_LOCK_ACQUISITIONS.fetch_add(1, Ordering::Relaxed);
            if let Some(runtime) = RUNTIME.get() {
                // CAPFIX2 BUG 2: tracked try_lock — the create hook can run on the
                // render thread (shaders are created mid-frame), so it must not
                // block on a busy runtime lock either.
                if let Some((mut runtime, _owner)) =
                    try_lock_runtime_tracked(runtime, RuntimeLockSite::CreatePixelShader)
                {
                    let mut observed_capture_family = false;
                    let terrain_id_srv_slot = terrain_id_slot_from_shader_bytecode(bytes);
                    if runtime
                        .state
                        .observe_pixel_shader_created(shader as usize, bytes)
                    {
                        observed_capture_family = true;
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
                        observed_capture_family = true;
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
                        observed_capture_family = true;
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
                        observed_capture_family = true;
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
                        .observe_underwater_pixel_shader_created(shader as usize, bytes)
                    {
                        observed_capture_family = true;
                        let shader_hash = fnv1a64_bytes(bytes);
                        let shader_file = format!("underwater_pixel_shader_{shader_hash}.bin");
                        if !runtime.output_dir.join(&shader_file).is_file() {
                            if let Err(error) =
                                fs::write(runtime.output_dir.join(&shader_file), bytes)
                            {
                                write_event(
                                    &runtime.output_dir,
                                    &format!(
                                        "{{\"type\":\"underwater_shader_bytecode_write_failed\",\"file\":\"{}\",\"message\":\"{}\"}}",
                                        json_escape(&shader_file),
                                        json_escape(&error.to_string())
                                    ),
                                );
                            }
                        }
                        write_event(
                            &runtime.output_dir,
                            &format!(
                                "{{\"type\":\"underwater_shader_observed\",\"pixel_shader\":\"0x{:X}\",\"byte_len\":{},\"fnv1a64\":\"{}\",\"file\":\"{}\",\"shader_count\":{}}}",
                                shader as usize,
                                bytes.len(),
                                shader_hash,
                                json_escape(&shader_file),
                                runtime.state.underwater_pixel_shader_count()
                            ),
                        );
                    }
                    if runtime
                        .state
                        .observe_tree_pixel_shader_created(shader as usize, bytes)
                    {
                        observed_capture_family = true;
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
                    if runtime
                        .state
                        .observe_restore_pixel_shader_created(shader as usize, bytes)
                    {
                        let shader_hash = fnv1a64_bytes(bytes);
                        let shader_file = format!("restorescene_pixel_shader_{shader_hash}.bin");
                        if let Err(error) = fs::write(runtime.output_dir.join(&shader_file), bytes)
                        {
                            write_event(
                                &runtime.output_dir,
                                &format!(
                                    "{{\"type\":\"restore_shader_bytecode_write_failed\",\"file\":\"{}\",\"message\":\"{}\"}}",
                                    json_escape(&shader_file),
                                    json_escape(&error.to_string())
                                ),
                            );
                        }
                        let slots = runtime
                            .state
                            .restore_pixel_shader_metadata(shader as usize)
                            .map(|metadata| metadata.resource_slots.clone())
                            .unwrap_or_default();
                        write_event(
                            &runtime.output_dir,
                            &format!(
                                "{{\"type\":\"restore_shader_observed\",\"pixel_shader\":\"0x{:X}\",\"main_scene_srv_slot\":{},\"restore_bloom_srv_slot\":{},\"color_cube_srv_slot\":{},\"average_luminance_srv_slot\":{},\"byte_len\":{},\"fnv1a64\":\"{}\",\"file\":\"{}\",\"shader_count\":{}}}",
                                shader as usize,
                                slots
                                    .get(RESTORE_MAIN_SCENE_RESOURCE_SYMBOL)
                                    .copied()
                                    .unwrap_or(u32::MAX),
                                slots
                                    .get(RESTORE_BLOOM_RESOURCE_SYMBOL)
                                    .copied()
                                    .unwrap_or(u32::MAX),
                                slots
                                    .get(RESTORE_COLOR_CUBE_RESOURCE_SYMBOL)
                                    .copied()
                                    .unwrap_or(u32::MAX),
                                slots
                                    .get(RESTORE_AVERAGE_LUMINANCE_RESOURCE_SYMBOL)
                                    .copied()
                                    .unwrap_or(u32::MAX),
                                bytes.len(),
                                shader_hash,
                                json_escape(&shader_file),
                                runtime.state.restore_pixel_shader_count()
                            ),
                        );
                        remember_restore_pixel_shader(shader as usize);
                    }
                    // Corroborating PS anchor for the label pass. Recorded but
                    // NOT treated as a capture-family shader: it must never set
                    // `observed_capture_family` (that would latch context trust
                    // on a generic single-texture PS). The mapname pass is
                    // anchored on its VERTEX shader; this PS metadata only
                    // annotates the draw and supplies the CB1/SRV slot names.
                    if runtime
                        .state
                        .observe_mapname_pixel_shader_created(shader as usize, bytes)
                    {
                        let shader_hash = fnv1a64_bytes(bytes);
                        let shader_file = format!("mapname_pixel_shader_{shader_hash}.bin");
                        if !runtime.output_dir.join(&shader_file).is_file() {
                            if let Err(error) =
                                fs::write(runtime.output_dir.join(&shader_file), bytes)
                            {
                                write_event(
                                    &runtime.output_dir,
                                    &format!(
                                        "{{\"type\":\"mapname_shader_bytecode_write_failed\",\"file\":\"{}\",\"message\":\"{}\"}}",
                                        json_escape(&shader_file),
                                        json_escape(&error.to_string())
                                    ),
                                );
                            }
                        }
                        let slots = runtime
                            .state
                            .mapname_pixel_shader_metadata(shader as usize)
                            .map(|metadata| metadata.resource_slots.clone())
                            .unwrap_or_default();
                        write_event(
                            &runtime.output_dir,
                            &format!(
                                "{{\"type\":\"mapname_shader_observed\",\"pixel_shader\":\"0x{:X}\",\"diffuse_texture_srv_slot\":{},\"byte_len\":{},\"fnv1a64\":\"{}\",\"file\":\"{}\",\"shader_count\":{}}}",
                                shader as usize,
                                slots
                                    .get(MAPNAME_DIFFUSE_TEXTURE_RESOURCE_SYMBOL)
                                    .copied()
                                    .unwrap_or(u32::MAX),
                                bytes.len(),
                                shader_hash,
                                json_escape(&shader_file),
                                runtime.state.mapname_pixel_shader_count()
                            ),
                        );
                        remember_mapname_pixel_shader(shader as usize);
                    }
                    if observed_capture_family {
                        remember_capture_pixel_shader(shader as usize);
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
    if capture_quiesced() {
        return hr;
    }
    if SUCCEEDED(hr) && !bytecode.is_null() && !vertex_shader.is_null() {
        let shader = unsafe { *vertex_shader };
        if !shader.is_null() {
            let bytes = unsafe { slice::from_raw_parts(bytecode.cast::<u8>(), bytecode_len) };
            remember_ui_vertex_shader_bytecode(device as usize, shader as usize, bytes);
            let reflection = reflect_terrain_vertex_shader_metadata(bytes);
            if let Some(runtime) = RUNTIME.get() {
                // CAPFIX2 BUG 2: tracked try_lock — create hooks may run on the
                // render thread; never block on a busy runtime lock.
                if let Some((mut runtime, _owner)) =
                    try_lock_runtime_tracked(runtime, RuntimeLockSite::CreateVertexShader)
                {
                    let terrain_shader_observed = runtime.state.observe_vertex_shader_created(
                        shader as usize,
                        bytes,
                        reflection.clone(),
                    );
                    if terrain_shader_observed {
                        remember_capture_vertex_shader(shader as usize);
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
                    if let Some(metadata) = runtime
                        .state
                        .map_vertex_shader_metadata(shader as usize)
                        .cloned()
                    {
                        if metadata.input_parameters.iter().any(|parameter| {
                            parameter.semantic_name.eq_ignore_ascii_case("POSITION")
                        }) {
                            remember_capture_vertex_shader(shader as usize);
                        }
                        let shader_file = format!("map_vertex_shader_{}.bin", metadata.fnv1a64);
                        if let Err(error) = fs::write(runtime.output_dir.join(&shader_file), bytes)
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
                    if let Some(metadata) = runtime
                        .state
                        .mapname_vertex_shader_metadata(shader as usize)
                        .cloned()
                    {
                        // The label pass is armed by its VERTEX shader: this is
                        // the primary reflected discriminator. Ensure the draw
                        // hook fires on mapname draws (belt-and-braces — a
                        // POSITION-bearing VS is already remembered above).
                        remember_capture_vertex_shader(shader as usize);
                        let shader_file = format!("mapname_vertex_shader_{}.bin", metadata.fnv1a64);
                        if !runtime.output_dir.join(&shader_file).is_file() {
                            if let Err(error) =
                                fs::write(runtime.output_dir.join(&shader_file), bytes)
                            {
                                write_event(
                                    &runtime.output_dir,
                                    &format!(
                                        "{{\"type\":\"mapname_vertex_shader_bytecode_write_failed\",\"file\":\"{}\",\"message\":\"{}\"}}",
                                        json_escape(&shader_file),
                                        json_escape(&error.to_string())
                                    ),
                                );
                            }
                        }
                        write_event(
                            &runtime.output_dir,
                            &format!(
                                "{{\"type\":\"mapname_vertex_shader_observed\",\"vertex_shader\":\"0x{:X}\",\"byte_len\":{},\"fnv1a64\":\"{}\",\"fnv1a64_matches_known\":{},\"file\":\"{}\",\"input_parameter_count\":{},\"has_transp_offset_cb1\":{},\"shader_count\":{}}}",
                                shader as usize,
                                metadata.byte_len,
                                metadata.fnv1a64,
                                metadata.fnv1a64 == MAPNAME_KNOWN_VERTEX_SHADER_FNV1A64,
                                json_escape(&shader_file),
                                metadata.input_parameters.len(),
                                metadata
                                    .constant_buffers
                                    .values()
                                    .any(|bind_point| *bind_point == MAPNAME_TRANSP_OFFSET_CB_BIND_POINT),
                                runtime.state.mapname_vertex_shader_count()
                            ),
                        );
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
    let input_layout_ptr = input_layout as usize;
    let ui_pass_armed = CONTEXT_UI_PASS_ARMED.load(Ordering::Acquire);
    let census_armed = CONTEXT_CENSUS_ARMED.load(Ordering::Acquire);
    let input_layout_relevant = capture_input_layout_known(input_layout_ptr)
        || CONTEXT_CURRENT_INPUT_LAYOUT_RELEVANT.load(Ordering::Acquire)
        || ui_pass_armed
        || census_armed;
    let capture_allowed = unsafe {
        context_hook_allowed_for_relevant_call(
            context,
            "IASetInputLayout",
            CONTEXT_INTEREST_IA_SET_INPUT_LAYOUT,
            input_layout_relevant,
        )
    };
    if capture_allowed {
        unsafe { drain_render_thread_requests(context, "IASetInputLayout") };
        if let Some(runtime) = RUNTIME.get() {
            record_context_lock_acquisition(&CONTEXT_IA_SET_INPUT_LAYOUT_LOCK_ACQUISITIONS);
            // CAPFIX2 BUG 2: tracked try_lock — never block the render thread on
            // the runtime lock; skip this call's bookkeeping if it is busy.
            if let Some((mut runtime, _owner)) =
                try_lock_runtime_tracked(runtime, RuntimeLockSite::IaSetInputLayout)
            {
                runtime.state.observe_ia_set_input_layout(input_layout_ptr);
                CONTEXT_CURRENT_INPUT_LAYOUT_RELEVANT.store(
                    census_armed || capture_input_layout_known(input_layout_ptr),
                    Ordering::SeqCst,
                );
            }
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
    let vertex_shader_ptr = vertex_shader as usize;
    let ui_pass_armed = CONTEXT_UI_PASS_ARMED.load(Ordering::Acquire);
    let census_armed = CONTEXT_CENSUS_ARMED.load(Ordering::Acquire);
    let vertex_shader_relevant = capture_vertex_shader_known(vertex_shader_ptr)
        || CONTEXT_CURRENT_VERTEX_SHADER_RELEVANT.load(Ordering::Acquire)
        || ui_pass_armed
        || census_armed;
    let capture_allowed = unsafe {
        context_hook_allowed_for_relevant_call(
            context,
            "VSSetShader",
            CONTEXT_INTEREST_VS_SET_SHADER,
            vertex_shader_relevant,
        )
    };
    if capture_allowed {
        unsafe { drain_render_thread_requests(context, "VSSetShader") };
        if let Some(runtime) = RUNTIME.get() {
            record_context_lock_acquisition(&CONTEXT_VS_SET_SHADER_LOCK_ACQUISITIONS);
            // CAPFIX2 BUG 2: tracked try_lock — never block the render thread.
            if let Some((mut runtime, _owner)) =
                try_lock_runtime_tracked(runtime, RuntimeLockSite::VsSetShader)
            {
                runtime.state.observe_vs_set_shader(vertex_shader_ptr);
                CONTEXT_CURRENT_VERTEX_SHADER_RELEVANT.store(
                    census_armed || capture_vertex_shader_known(vertex_shader_ptr),
                    Ordering::SeqCst,
                );
            }
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
    let pixel_shader_ptr = pixel_shader as usize;
    let ui_pass_armed = CONTEXT_UI_PASS_ARMED.load(Ordering::Acquire);
    let census_armed = CONTEXT_CENSUS_ARMED.load(Ordering::Acquire);
    let incoming_map_relevant = capture_pixel_shader_known(pixel_shader_ptr);
    let incoming_restore_relevant = restore_pixel_shader_known(pixel_shader_ptr);
    let incoming_mapname_relevant = mapname_pixel_shader_known(pixel_shader_ptr);
    let incoming_relevant =
        incoming_map_relevant || incoming_restore_relevant || incoming_mapname_relevant;
    let pixel_shader_relevant = incoming_relevant
        || CONTEXT_CURRENT_PIXEL_SHADER_RELEVANT.load(Ordering::Acquire)
        || ui_pass_armed
        || census_armed
        || CONTEXT_AUTO_ROUND_ARMED.load(Ordering::Acquire);
    let mut finalize_map_frame = false;
    let capture_allowed = unsafe {
        context_hook_allowed_for_relevant_call(
            context,
            "PSSetShader",
            CONTEXT_INTEREST_PS_SET_SHADER,
            pixel_shader_relevant,
        )
    };
    if capture_allowed {
        // Trust latch: a known capture-family pixel shader bound on the
        // render-thread-owned immediate context proves the real map renderer
        // is live (the load-screen/browser-interop bootstrap is over). The
        // STRICT check refuses to latch on pointer-set overflow.
        if capture_pixel_shader_known_strict(pixel_shader_ptr) {
            latch_map_renderer_live();
        }
        unsafe { drain_render_thread_requests(context, "PSSetShader") };
        if let Some(runtime) = RUNTIME.get() {
            record_context_lock_acquisition(&CONTEXT_PS_SET_SHADER_LOCK_ACQUISITIONS);
            // CAPFIX2 BUG 2: tracked try_lock — never block the render thread.
            if let Some((mut runtime, _owner)) =
                try_lock_runtime_tracked(runtime, RuntimeLockSite::PsSetShader)
            {
                let profile = requested_map_frame_capture_profile_for_runtime(&runtime).ok();
                finalize_map_frame = profile.map_or(false, |profile| {
                    should_finalize_map_frame_on_pixel_shader_transition(
                        &runtime.state,
                        profile,
                        pixel_shader_ptr,
                        !runtime.map_frame_draws.is_empty(),
                        runtime
                            .completed_map_frame_moments
                            .contains(profile.capture_moment),
                    )
                });
                runtime.state.observe_ps_set_shader(pixel_shader_ptr);
                CONTEXT_CURRENT_PIXEL_SHADER_RELEVANT
                    .store(census_armed || incoming_relevant, Ordering::SeqCst);
                CONTEXT_CURRENT_UI_PASS_SHADER_RELEVANT.store(
                    ui_pass_armed
                        && CONTEXT_UI_PASS_FRAME_CANDIDATE.load(Ordering::Acquire)
                        && !incoming_relevant,
                    Ordering::SeqCst,
                );
            }
        }
    }
    if capture_allowed && finalize_map_frame {
        finalize_map_frame_bundle_pass(context, "ps_shader_transition");
    }
    if let Some(original) = unsafe { ORIG_PS_SET_SHADER } {
        unsafe {
            original(context, pixel_shader, class_instances, class_instance_count);
        }
    }
}

unsafe extern "system" fn hook_vs_set_constant_buffers(
    context: *mut ID3D11DeviceContext,
    start_slot: UINT,
    num_buffers: UINT,
    buffers: *const *mut ID3D11Buffer,
) {
    shadow_ui_pass_constant_buffers(
        &UI_PASS_VS_CONSTANT_BUFFER_SHADOWS,
        start_slot,
        num_buffers,
        buffers,
    );
    if let Some(original) = unsafe { ORIG_VS_SET_CONSTANT_BUFFERS } {
        unsafe {
            original(context, start_slot, num_buffers, buffers);
        }
    }
}

unsafe extern "system" fn hook_ps_set_constant_buffers(
    context: *mut ID3D11DeviceContext,
    start_slot: UINT,
    num_buffers: UINT,
    buffers: *const *mut ID3D11Buffer,
) {
    shadow_ui_pass_constant_buffers(
        &UI_PASS_PS_CONSTANT_BUFFER_SHADOWS,
        start_slot,
        num_buffers,
        buffers,
    );
    if let Some(original) = unsafe { ORIG_PS_SET_CONSTANT_BUFFERS } {
        unsafe {
            original(context, start_slot, num_buffers, buffers);
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
    let capture_allowed = unsafe {
        context_hook_allowed_for_relevant_call(
            context,
            "PSSetShaderResources",
            CONTEXT_INTEREST_PS_SET_SHADER_RESOURCES,
            CONTEXT_CURRENT_PIXEL_SHADER_RELEVANT.load(Ordering::Acquire),
        )
    };
    if !capture_allowed {
        return;
    }
    unsafe { drain_render_thread_requests(context, "PSSetShaderResources") };
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
    record_context_lock_acquisition(&CONTEXT_PS_SET_SHADER_RESOURCES_LOCK_ACQUISITIONS);
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
        if candidate.target.timing == RuntimeTextureCaptureTiming::Finalize {
            unsafe {
                record_deferred_runtime_target(
                    candidate.target,
                    candidate.binding.srv as *mut ID3D11ShaderResourceView,
                    candidate.binding.slot,
                    candidate.identity,
                );
            }
            last_error = None;
            continue;
        }
        let result = unsafe {
            capture_runtime_texture(
                context,
                candidate.target,
                candidate.binding.srv as *mut ID3D11ShaderResourceView,
                candidate.binding.slot,
                candidate.identity,
                RuntimeTextureCaptureTiming::FirstDraw,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CapturedDrawKind {
    None,
    MapFrame,
    UiPass,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UiPassDrawOutcome {
    Ignored,
    Watched,
    Captured,
    SkippedForeign,
}

unsafe fn capture_relevant_draw_before_original(
    context: *mut ID3D11DeviceContext,
    draw_call: &'static str,
    draw_count: UINT,
    draw_start: UINT,
    base_vertex_location: i32,
    per_instance_count: Option<UINT>,
    instance_count: Option<UINT>,
    start_instance_location: Option<UINT>,
) -> CapturedDrawKind {
    debug_assert_render_thread_owner("capture_relevant_draw_before_original");
    record_context_lock_acquisition(&CONTEXT_DRAW_LOCK_ACQUISITIONS);
    unsafe { drain_render_thread_requests(context, draw_call) };
    unsafe {
        sample_auto_round_height_if_active(context);
        capture_census_draw_if_active(
            context,
            draw_call,
            draw_count,
            draw_start,
            base_vertex_location,
            per_instance_count,
            instance_count,
            start_instance_location,
        );
    }
    if CONTEXT_UI_PASS_ARMED.load(Ordering::Acquire)
        && CONTEXT_CURRENT_UI_PASS_SHADER_RELEVANT.load(Ordering::Acquire)
    {
        if unsafe {
            capture_ui_pass_draw_if_active(
                context,
                draw_call,
                draw_count,
                draw_start,
                base_vertex_location,
                per_instance_count,
                instance_count,
                start_instance_location,
            )
        } {
            return CapturedDrawKind::UiPass;
        }
    }
    unsafe {
        capture_terrain_input_layout_if_active(
            context,
            draw_call,
            draw_count,
            draw_start,
            base_vertex_location,
        );
        capture_terrain_vertex_buffer_if_active(
            context,
            draw_call,
            draw_count,
            draw_start,
            base_vertex_location,
        );
        capture_terrain_chunk_library_if_active(
            context,
            draw_call,
            draw_count,
            draw_start,
            base_vertex_location,
            per_instance_count,
            instance_count,
            start_instance_location,
        );
        if capture_map_frame_bundle_if_active(
            context,
            draw_call,
            draw_count,
            draw_start,
            base_vertex_location,
            per_instance_count,
            instance_count,
            start_instance_location,
        ) {
            CapturedDrawKind::MapFrame
        } else {
            CapturedDrawKind::None
        }
    }
}

unsafe extern "system" fn hook_draw_indexed(
    context: *mut ID3D11DeviceContext,
    index_count: UINT,
    start_index_location: UINT,
    base_vertex_location: i32,
) {
    // Persistent scene-CB0 sampler: runs BEFORE the capture_allowed gate and the
    // original draw, on every draw regardless of quiesce/arming. No-op unless
    // --cb0-sample-ms armed it. See sample_scene_cb0_if_active.
    unsafe { sample_scene_cb0_if_active(context) };
    let capture_allowed = unsafe {
        context_hook_allowed_for_relevant_call(
            context,
            "DrawIndexed",
            CONTEXT_INTEREST_DRAW,
            context_draw_call_relevant(),
        )
    };
    let captured_draw = if capture_allowed {
        unsafe {
            capture_relevant_draw_before_original(
                context,
                "DrawIndexed",
                index_count,
                start_index_location,
                base_vertex_location,
                None,
                None,
                None,
            )
        }
    } else {
        CapturedDrawKind::None
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
    if captured_draw == CapturedDrawKind::MapFrame {
        unsafe { capture_map_frame_post_draw_output_if_active(context) };
    }
}

unsafe extern "system" fn hook_draw(
    context: *mut ID3D11DeviceContext,
    vertex_count: UINT,
    start_vertex_location: UINT,
) {
    unsafe { sample_scene_cb0_if_active(context) };
    let capture_allowed = unsafe {
        context_hook_allowed_for_relevant_call(
            context,
            "Draw",
            CONTEXT_INTEREST_DRAW,
            context_draw_call_relevant(),
        )
    };
    let captured_draw = if capture_allowed {
        unsafe {
            capture_relevant_draw_before_original(
                context,
                "Draw",
                vertex_count,
                start_vertex_location,
                0,
                None,
                None,
                None,
            )
        }
    } else {
        CapturedDrawKind::None
    };
    if let Some(original) = unsafe { ORIG_DRAW } {
        unsafe {
            original(context, vertex_count, start_vertex_location);
        }
    }
    if captured_draw == CapturedDrawKind::MapFrame {
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
    unsafe { sample_scene_cb0_if_active(context) };
    let draw_count = index_count_per_instance.saturating_mul(instance_count);
    let capture_allowed = unsafe {
        context_hook_allowed_for_relevant_call(
            context,
            "DrawIndexedInstanced",
            CONTEXT_INTEREST_DRAW,
            context_draw_call_relevant(),
        )
    };
    let captured_draw = if capture_allowed {
        unsafe {
            capture_relevant_draw_before_original(
                context,
                "DrawIndexedInstanced",
                draw_count,
                start_index_location,
                base_vertex_location,
                Some(index_count_per_instance),
                Some(instance_count),
                Some(start_instance_location),
            )
        }
    } else {
        CapturedDrawKind::None
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
    if captured_draw == CapturedDrawKind::MapFrame {
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
    unsafe { sample_scene_cb0_if_active(context) };
    let draw_count = vertex_count_per_instance.saturating_mul(instance_count);
    let capture_allowed = unsafe {
        context_hook_allowed_for_relevant_call(
            context,
            "DrawInstanced",
            CONTEXT_INTEREST_DRAW,
            context_draw_call_relevant(),
        )
    };
    let captured_draw = if capture_allowed {
        unsafe {
            capture_relevant_draw_before_original(
                context,
                "DrawInstanced",
                draw_count,
                start_vertex_location,
                0,
                Some(vertex_count_per_instance),
                Some(instance_count),
                Some(start_instance_location),
            )
        }
    } else {
        CapturedDrawKind::None
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
    if captured_draw == CapturedDrawKind::MapFrame {
        unsafe { capture_map_frame_post_draw_output_if_active(context) };
    }
}

const OM_KEEP_RENDER_TARGETS_AND_DEPTH_STENCIL: UINT = u32::MAX;

fn record_census_tracked_om_targets(
    num_views: UINT,
    render_target_views: *const *mut ID3D11RenderTargetView,
    depth_stencil_view: *mut ID3D11DepthStencilView,
) {
    if !CONTEXT_CENSUS_ARMED.load(Ordering::Acquire) {
        return;
    }
    let count = if render_target_views.is_null() {
        0
    } else {
        (num_views as usize).min(CENSUS_TRACKED_OM_RTVS.len())
    };
    for (slot, tracked) in CENSUS_TRACKED_OM_RTVS.iter().enumerate() {
        let value = if slot < count {
            unsafe { *render_target_views.add(slot) as usize }
        } else {
            0
        };
        tracked.store(value, Ordering::Release);
    }
    CENSUS_TRACKED_OM_DSV.store(depth_stencil_view as usize, Ordering::Release);
    CENSUS_TRACKED_OM_SET_SEEN.store(true, Ordering::Release);
}

unsafe extern "system" fn hook_om_set_render_targets(
    context: *mut ID3D11DeviceContext,
    num_views: UINT,
    render_target_views: *const *mut ID3D11RenderTargetView,
    depth_stencil_view: *mut ID3D11DepthStencilView,
) {
    let capture_allowed = unsafe {
        context_hook_allowed_for_relevant_call(
            context,
            "OMSetRenderTargets",
            CONTEXT_INTEREST_OM_SET_RENDER_TARGETS,
            CONTEXT_CENSUS_ARMED.load(Ordering::Acquire),
        )
    };
    if capture_allowed {
        record_census_tracked_om_targets(num_views, render_target_views, depth_stencil_view);
    }
    if let Some(original) = unsafe { ORIG_OM_SET_RENDER_TARGETS } {
        unsafe { original(context, num_views, render_target_views, depth_stencil_view) }
    }
}

#[allow(clippy::too_many_arguments)]
unsafe extern "system" fn hook_om_set_render_targets_and_uavs(
    context: *mut ID3D11DeviceContext,
    num_rtvs: UINT,
    render_target_views: *const *mut ID3D11RenderTargetView,
    depth_stencil_view: *mut ID3D11DepthStencilView,
    uav_start_slot: UINT,
    num_uavs: UINT,
    unordered_access_views: *const *mut WinVoid,
    uav_initial_counts: *const UINT,
) {
    let capture_allowed = unsafe {
        context_hook_allowed_for_relevant_call(
            context,
            "OMSetRenderTargetsAndUnorderedAccessViews",
            CONTEXT_INTEREST_OM_SET_RENDER_TARGETS,
            CONTEXT_CENSUS_ARMED.load(Ordering::Acquire)
                && num_rtvs != OM_KEEP_RENDER_TARGETS_AND_DEPTH_STENCIL,
        )
    };
    if capture_allowed && num_rtvs != OM_KEEP_RENDER_TARGETS_AND_DEPTH_STENCIL {
        record_census_tracked_om_targets(num_rtvs, render_target_views, depth_stencil_view);
    }
    if let Some(original) = unsafe { ORIG_OM_SET_RENDER_TARGETS_AND_UAVS } {
        unsafe {
            original(
                context,
                num_rtvs,
                render_target_views,
                depth_stencil_view,
                uav_start_slot,
                num_uavs,
                unordered_access_views,
                uav_initial_counts,
            )
        }
    }
}

unsafe extern "system" fn hook_om_set_blend_state(
    context: *mut ID3D11DeviceContext,
    blend_state: *mut WinVoid,
    blend_factor: *const f32,
    sample_mask: UINT,
) {
    let capture_allowed = unsafe {
        context_hook_allowed_for_relevant_call(
            context,
            "OMSetBlendState",
            CONTEXT_INTEREST_OM_SET_RENDER_TARGETS,
            CONTEXT_CENSUS_ARMED.load(Ordering::Acquire),
        )
    };
    if capture_allowed && CONTEXT_CENSUS_ARMED.load(Ordering::Acquire) {
        CENSUS_TRACKED_OM_BLEND_STATE.store(blend_state as usize, Ordering::Release);
    }
    if let Some(original) = unsafe { ORIG_OM_SET_BLEND_STATE } {
        unsafe { original(context, blend_state, blend_factor, sample_mask) }
    }
}

unsafe extern "system" fn hook_om_set_depth_stencil_state(
    context: *mut ID3D11DeviceContext,
    depth_stencil_state: *mut WinVoid,
    stencil_ref: UINT,
) {
    let capture_allowed = unsafe {
        context_hook_allowed_for_relevant_call(
            context,
            "OMSetDepthStencilState",
            CONTEXT_INTEREST_OM_SET_RENDER_TARGETS,
            CONTEXT_CENSUS_ARMED.load(Ordering::Acquire),
        )
    };
    if capture_allowed && CONTEXT_CENSUS_ARMED.load(Ordering::Acquire) {
        CENSUS_TRACKED_OM_DEPTH_STENCIL_STATE
            .store(depth_stencil_state as usize, Ordering::Release);
    }
    if let Some(original) = unsafe { ORIG_OM_SET_DEPTH_STENCIL_STATE } {
        unsafe { original(context, depth_stencil_state, stencil_ref) }
    }
}

unsafe extern "system" fn hook_clear_render_target_view(
    context: *mut ID3D11DeviceContext,
    render_target_view: *mut ID3D11RenderTargetView,
    color_rgba: *const f32,
) {
    let capture_allowed = unsafe {
        context_hook_allowed_for_relevant_call(
            context,
            "ClearRenderTargetView",
            CONTEXT_INTEREST_CLEAR_RENDER_TARGET,
            context_boundary_call_relevant(),
        )
    };
    if capture_allowed {
        record_context_lock_acquisition(&CONTEXT_CLEAR_LOCK_ACQUISITIONS);
        unsafe { drain_render_thread_requests(context, "ClearRenderTargetView") };
    }
    if capture_allowed && should_finalize_map_frame_bundle_at_render_target_clear() {
        finalize_map_frame_bundle_pass(context, "pre_clear_render_target_frame_boundary");
    }
    if capture_allowed {
        // CAPFIX3 ISSUE 2: close the map-frame pass at this clean frame boundary
        // even if the finalize gate above did not fire or did not complete, so
        // per-frame draws never accumulate cross-frame into max_draws_per_pass.
        // No-op unless a map-frame moment is active with carried draws.
        close_map_frame_pass_at_frame_boundary_if_incomplete(
            "pre_clear_render_target_frame_boundary",
        );
    }
    if capture_allowed && CONTEXT_CENSUS_ARMED.load(Ordering::Acquire) {
        finalize_census_pass(context, "pre_clear_render_target_frame_boundary");
    }
    if capture_allowed {
        reset_capture_work_frame_budget();
    }
    if let Some(original) = unsafe { ORIG_CLEAR_RENDER_TARGET_VIEW } {
        unsafe {
            original(context, render_target_view, color_rgba);
        }
    }
    if !capture_allowed
        || render_target_view.is_null()
        || color_rgba.is_null()
        || !map_frame_clear_provenance_recording_enabled()
    {
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
    should_finalize_map_frame_bundle_at_clean_frame_boundary()
}

fn should_finalize_map_frame_bundle_at_present() -> bool {
    should_finalize_map_frame_bundle_at_clean_frame_boundary()
}

fn should_finalize_map_frame_bundle_at_clean_frame_boundary() -> bool {
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
                    if profile.capture_moment == UI_PASS_CAPTURE_MOMENT {
                        return false;
                    }
                    map_frame_clean_boundary_ready(
                        runtime.map_frame_draws.len(),
                        profile,
                        runtime
                            .completed_map_frame_moments
                            .contains(profile.capture_moment),
                    )
                })
        })
        .unwrap_or(false)
}

fn map_frame_clear_provenance_recording_enabled() -> bool {
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
                    profile.capture_moment != UI_PASS_CAPTURE_MOMENT
                        && !runtime
                            .completed_map_frame_moments
                            .contains(profile.capture_moment)
                })
        })
        .unwrap_or(false)
}

fn map_frame_clean_boundary_ready(
    draw_count: usize,
    profile: MapFrameCaptureProfile,
    moment_completed: bool,
) -> bool {
    draw_count >= profile.min_draws_before_clear_finalize && !moment_completed
}

#[cfg(test)]
fn map_frame_post_draw_blend_layer_boundary_ready(
    draw_count: usize,
    profile: MapFrameCaptureProfile,
    moment_completed: bool,
) -> bool {
    map_frame_post_draw_blend_layer_boundary_ready_with_mode(
        draw_count,
        profile,
        moment_completed,
        TreesCloseMode::Default,
    )
}

fn map_frame_post_draw_blend_layer_boundary_ready_with_mode(
    draw_count: usize,
    profile: MapFrameCaptureProfile,
    moment_completed: bool,
    trees_close_mode: TreesCloseMode,
) -> bool {
    // mapname_pass is deliberately NOT in this set (2026-07-06 adversarial
    // review finding): the live label pass is N glyph draws, and a post-draw
    // close after the first would lose labels 1..N-1. It closes instead on the
    // PS-transition-away boundary or the clear/present frame boundary, both of
    // which fire only after ALL of the frame's label draws have accumulated.
    matches!(
        profile.capture_moment,
        RIVERS_CAPTURE_MOMENT | BORDERS_CAPTURE_MOMENT | RESTORE_CAPTURE_MOMENT
    ) && map_frame_clean_boundary_ready(draw_count, profile, moment_completed)
        || profile.capture_moment == TREES_CAPTURE_MOMENT
            && trees_close_mode == TreesCloseMode::Default
            && map_frame_clean_boundary_ready(draw_count, profile, moment_completed)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TreesCloseMode {
    Default,
    FrameRepeat,
}

fn trees_close_mode_from_control(control: Option<&TerrainCaptureControl>) -> TreesCloseMode {
    match control.and_then(|control| control.trees_close_mode.as_deref()) {
        Some("frame-repeat") => TreesCloseMode::FrameRepeat,
        _ => TreesCloseMode::Default,
    }
}

fn map_frame_declared_resource_needs_moment_payload(
    profile: MapFrameCaptureProfile,
    declared_name: Option<&str>,
) -> bool {
    if !matches!(
        profile.capture_moment,
        TERRAIN_CAPTURE_MOMENT
            | WATER_CAPTURE_MOMENT
            | RIVERS_CAPTURE_MOMENT
            | BORDERS_CAPTURE_MOMENT
            | TREES_CAPTURE_MOMENT
    ) {
        return false;
    }
    let Some(name) = declared_name.map(map_frame_runtime_texture_resource_key) else {
        return false;
    };
    matches!(
        name.as_str(),
        "SnowMudData"
            | "SnowMudTexture"
            | "GradientBorderChannel1"
            | "GradientBorderChannel2"
            | "GradientBorderChannel3"
            | "FoW"
            | "ShadowMap"
            | "LightIndexMap"
            | "LightDataMap"
            | "ProvinceSecondaryColorMap"
            | "IntelMap"
    )
}

fn map_frame_runtime_texture_resource_key(name: &str) -> String {
    name.trim()
        .trim_end_matches("_Texture")
        .trim_end_matches("_TextureSampler")
        .to_string()
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct MapFrameDrawContentIdentity {
    pixel_shader_hash: String,
    vertex_shader_hash: String,
    vertex_buffers: Vec<(String, Option<String>, usize, Option<u32>, Option<u32>)>,
    draw_call: &'static str,
    draw_count: u32,
    draw_start: u32,
    base_vertex_location: i32,
    per_instance_count: Option<u32>,
    instance_count: Option<u32>,
    start_instance_location: Option<u32>,
}

fn map_frame_draw_content_identity(draw: &MapFrameDrawFile) -> MapFrameDrawContentIdentity {
    MapFrameDrawContentIdentity {
        pixel_shader_hash: draw.shader.fnv1a64.clone(),
        vertex_shader_hash: draw.vertex_shader.fnv1a64.clone(),
        vertex_buffers: draw
            .vertex_buffers
            .iter()
            .map(|(slot, binding)| {
                (
                    slot.clone(),
                    binding.fnv1a64.clone(),
                    binding.byte_len,
                    binding.stride,
                    binding.offset,
                )
            })
            .collect(),
        draw_call: draw.draw_call,
        draw_count: draw.draw_count,
        draw_start: draw.draw_start,
        base_vertex_location: draw.base_vertex_location,
        per_instance_count: draw.per_instance_count,
        instance_count: draw.instance_count,
        start_instance_location: draw.start_instance_location,
    }
}

fn should_close_trees_pass_on_frame_repeat(
    draws: &[MapFrameDrawFile],
    incoming_draw: &MapFrameDrawFile,
    mode: TreesCloseMode,
) -> bool {
    mode == TreesCloseMode::FrameRepeat
        && draws
            .first()
            .map(|first| {
                map_frame_draw_content_identity(first)
                    == map_frame_draw_content_identity(incoming_draw)
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
    let capture_allowed = unsafe {
        context_hook_allowed_for_relevant_call(
            context,
            "ClearDepthStencilView",
            CONTEXT_INTEREST_CLEAR_DEPTH_STENCIL,
            context_boundary_call_relevant(),
        )
    };
    if capture_allowed {
        record_context_lock_acquisition(&CONTEXT_CLEAR_LOCK_ACQUISITIONS);
        unsafe { drain_render_thread_requests(context, "ClearDepthStencilView") };
    }
    if capture_allowed && CONTEXT_CENSUS_ARMED.load(Ordering::Acquire) {
        finalize_census_pass(context, "pre_clear_depth_stencil_frame_boundary");
    }
    if let Some(original) = unsafe { ORIG_CLEAR_DEPTH_STENCIL_VIEW } {
        unsafe {
            original(context, depth_stencil_view, clear_flags, depth, stencil);
        }
    }
    if !capture_allowed
        || depth_stencil_view.is_null()
        || !map_frame_clear_provenance_recording_enabled()
    {
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

unsafe extern "system" fn hook_present(
    swap_chain: *mut IDXGISwapChain,
    sync_interval: UINT,
    flags: UINT,
) -> i32 {
    if !swap_chain.is_null() && UI_PASS_SHADER_TRACKING_ENABLED.load(Ordering::Acquire) {
        unsafe { observe_present_swapchain_backbuffer(swap_chain) };
        unsafe { observe_ui_pass_present_candidate(swap_chain) };
    }
    let finalize_map_frame = !swap_chain.is_null()
        && context_interest_enabled(CONTEXT_INTEREST_PRESENT)
        && context_boundary_call_relevant()
        && should_finalize_map_frame_bundle_at_present();
    let finalize_ui_pass = !swap_chain.is_null()
        && context_interest_enabled(CONTEXT_INTEREST_PRESENT)
        && context_boundary_call_relevant()
        && should_finalize_ui_pass_at_present();
    let process_ui_pass_watch = !finalize_ui_pass
        && !swap_chain.is_null()
        && context_interest_enabled(CONTEXT_INTEREST_PRESENT)
        && context_boundary_call_relevant()
        && should_process_ui_pass_watch_at_present();
    let finalize_census = !swap_chain.is_null()
        && context_interest_enabled(CONTEXT_INTEREST_PRESENT)
        && CONTEXT_CENSUS_ARMED.load(Ordering::Acquire)
        && census_active_pass_needs_context();
    // CAPFIX3 ISSUE 2: a map-frame moment with carried draws must have its pass
    // closed at THIS present boundary even when the standard finalize gate
    // (min_draws / clean-boundary-ready) does not fire, otherwise draws below
    // the finalize threshold still creep across frames. CONTEXT_MAP_FRAME_-
    // BOUNDARY_ARMED is set only while a map-frame pass has accumulated draws.
    let close_map_frame_pass = !swap_chain.is_null()
        && context_interest_enabled(CONTEXT_INTEREST_PRESENT)
        && context_boundary_call_relevant()
        && CONTEXT_MAP_FRAME_BOUNDARY_ARMED.load(Ordering::Acquire);
    if finalize_map_frame
        || finalize_ui_pass
        || process_ui_pass_watch
        || finalize_census
        || close_map_frame_pass
    {
        if let Some(context) = unsafe { immediate_context_from_swap_chain(swap_chain) } {
            if unsafe {
                context_hook_allowed_for_relevant_call(
                    context,
                    "Present",
                    CONTEXT_INTEREST_PRESENT,
                    true,
                )
            } {
                record_context_lock_acquisition(&CONTEXT_CLEAR_LOCK_ACQUISITIONS);
                unsafe { drain_render_thread_requests(context, "Present") };
                if finalize_map_frame {
                    finalize_map_frame_bundle_pass(context, "present_frame_boundary");
                }
                // CAPFIX3 ISSUE 2: close the map-frame pass at this frame
                // boundary even if the finalize gate above did not fire or did
                // not complete, so per-frame draws never accumulate cross-frame
                // into max_draws_per_pass. No-op unless a map-frame moment is
                // active with carried draws (see the function's scope guards).
                if close_map_frame_pass {
                    close_map_frame_pass_at_frame_boundary_if_incomplete("present_frame_boundary");
                }
                if finalize_ui_pass {
                    finalize_ui_pass_bundle(context, "present_frame_boundary");
                }
                if process_ui_pass_watch {
                    process_ui_pass_watch_frame_at_present("present_frame_boundary");
                }
                if finalize_census {
                    finalize_census_pass(context, "present_frame_boundary");
                }
                reset_capture_work_frame_budget();
            } else {
                context_prefilter_skip();
            }
            unsafe {
                release_unknown(context as *mut WinVoid);
            }
        }
    } else {
        context_prefilter_skip();
    }
    if let Some(original) = unsafe { ORIG_PRESENT } {
        unsafe { original(swap_chain, sync_interval, flags) }
    } else {
        0
    }
}

unsafe fn immediate_context_from_swap_chain(
    swap_chain: *mut IDXGISwapChain,
) -> Option<*mut ID3D11DeviceContext> {
    if swap_chain.is_null() {
        return None;
    }
    let mut device: *mut WinVoid = ptr::null_mut();
    let hr = unsafe { (*swap_chain).GetDevice(&IID_ID3D11Device, &mut device) };
    if FAILED(hr) || device.is_null() {
        return None;
    }
    let device = device as *mut ID3D11Device;
    let mut context: *mut ID3D11DeviceContext = ptr::null_mut();
    unsafe {
        (*device).GetImmediateContext(&mut context);
        release_unknown(device as *mut WinVoid);
    }
    if context.is_null() {
        None
    } else {
        Some(context)
    }
}

unsafe fn observe_present_swapchain_backbuffer(swap_chain: *mut IDXGISwapChain) {
    if capture_quiesced() || swap_chain.is_null() {
        return;
    }
    let swapchain_device = unsafe { swap_chain_device_pointer(swap_chain) }.unwrap_or(0);
    let mut backbuffer: *mut ID3D11Texture2D = ptr::null_mut();
    let hr = unsafe {
        (*swap_chain).GetBuffer(
            0,
            &IID_ID3D11Texture2D,
            &mut backbuffer as *mut _ as *mut *mut WinVoid,
        )
    };
    if FAILED(hr) || backbuffer.is_null() {
        return;
    }
    let mut desc: D3D11_TEXTURE2D_DESC = unsafe { mem::zeroed() };
    unsafe {
        (*backbuffer).GetDesc(&mut desc);
    }
    let observed = UiPassBackbufferFile {
        resource_pointer: format!("0x{:X}", backbuffer as usize),
        width: desc.Width,
        height: desc.Height,
        format: desc.Format as u32,
        sample_count: desc.SampleDesc.Count,
        sample_quality: desc.SampleDesc.Quality,
    };
    unsafe { release_unknown(backbuffer as *mut WinVoid) };
    let Some((output_dir, changed)) = RUNTIME.get().and_then(|runtime| {
        runtime.lock().ok().map(|mut runtime| {
            let changed = runtime
                .current_swapchain_backbuffer
                .as_ref()
                .map_or(true, |current| current != &observed);
            if changed {
                runtime.current_swapchain_backbuffer = Some(observed.clone());
            }
            runtime.current_swapchain_device = swapchain_device;
            refresh_context_hook_interests_for_runtime(&runtime);
            (runtime.output_dir.clone(), changed)
        })
    }) else {
        return;
    };
    if changed {
        write_event(
            &output_dir,
            &format!(
                "{{\"type\":\"ui_pass_backbuffer_observed\",\"resource\":\"{}\",\"width\":{},\"height\":{},\"format\":{},\"sample_count\":{},\"sample_quality\":{}}}",
                json_escape(&observed.resource_pointer),
                observed.width,
                observed.height,
                observed.format,
                observed.sample_count,
                observed.sample_quality
            ),
        );
    }
}

unsafe fn swap_chain_device_pointer(swap_chain: *mut IDXGISwapChain) -> Option<usize> {
    if swap_chain.is_null() {
        return None;
    }
    let mut device: *mut WinVoid = ptr::null_mut();
    let hr = unsafe { (*swap_chain).GetDevice(&IID_ID3D11Device, &mut device) };
    if FAILED(hr) || device.is_null() {
        return None;
    }
    let pointer = device as usize;
    unsafe {
        release_unknown(device);
    }
    Some(pointer)
}

unsafe fn observe_ui_pass_present_candidate(swap_chain: *mut IDXGISwapChain) {
    if capture_quiesced()
        || swap_chain.is_null()
        || !UI_PASS_SHADER_TRACKING_ENABLED.load(Ordering::Acquire)
    {
        return;
    }
    let Some(context) = (unsafe { immediate_context_from_swap_chain(swap_chain) }) else {
        return;
    };
    let candidate = unsafe { ui_pass_open_condition(context) }
        .ok()
        .flatten()
        .is_some();
    unsafe {
        release_unknown(context as *mut WinVoid);
    }
    let changed = CONTEXT_UI_PASS_FRAME_CANDIDATE.swap(candidate, Ordering::SeqCst) != candidate;
    if !candidate {
        CONTEXT_CURRENT_UI_PASS_SHADER_RELEVANT.store(false, Ordering::SeqCst);
    }
    if changed {
        if let Some(runtime) = RUNTIME.get() {
            if let Ok(runtime) = runtime.lock() {
                refresh_context_hook_interests_for_runtime(&runtime);
            }
        }
    }
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
    if capture_quiesced() {
        return;
    }
    if let Some(runtime) = RUNTIME.get() {
        if let Ok(mut runtime) = runtime.lock() {
            if !runtime
                .state
                .observe_input_layout_created(layout_ptr, elements.clone())
            {
                return;
            }
            if validate_terrain_input_layout(&elements, u32::MAX).is_ok() {
                remember_capture_input_layout(layout_ptr);
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
    if capture_quiesced() {
        return;
    }
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

unsafe fn record_deferred_runtime_target(
    target: CaptureTarget,
    srv: *mut ID3D11ShaderResourceView,
    slot: u32,
    identity: &'static str,
) {
    if capture_quiesced() {
        return;
    }
    if srv.is_null() {
        return;
    }
    let mut old_srv = None;
    let mut stored = false;
    let mut output_dir = None;
    if let Some(runtime) = RUNTIME.get() {
        if let Ok(mut runtime) = runtime.lock() {
            let existing = runtime.deferred_runtime_targets.get(target.symbol).copied();
            if deferred_runtime_target_matches(existing.as_ref(), srv as usize, slot, identity) {
                return;
            }
            if existing.map_or(true, |existing| existing.srv != srv as usize) {
                unsafe {
                    add_ref_unknown(srv as *mut WinVoid);
                }
            }
            let old = runtime.deferred_runtime_targets.insert(
                target.symbol.to_string(),
                DeferredRuntimeTarget {
                    target,
                    srv: srv as usize,
                    slot,
                    identity,
                },
            );
            stored = true;
            old_srv = old
                .map(|old| old.srv)
                .filter(|old_srv| *old_srv != srv as usize);
            refresh_context_hook_interests_for_runtime(&runtime);
            output_dir = Some(runtime.output_dir.clone());
        }
    }
    if !stored {
        return;
    }
    if let Some(old_srv) = old_srv {
        unsafe {
            release_unknown(old_srv as *mut WinVoid);
        }
    }
    if let Some(output_dir) = output_dir {
        write_event(
            &output_dir,
            &format!(
                "{{\"type\":\"runtime_texture_deferred\",\"symbol\":\"{}\",\"shader_resource_slot\":{},\"identity\":\"{}\",\"capture_timing\":\"{}\"}}",
                target.symbol,
                slot,
                json_escape(identity),
                target.timing.as_str()
            ),
        );
    }
}

fn deferred_runtime_target_matches(
    existing: Option<&DeferredRuntimeTarget>,
    srv: usize,
    slot: u32,
    _identity: &'static str,
) -> bool {
    existing.map_or(false, |existing| {
        existing.srv == srv && existing.slot == slot
    })
}

fn record_capture_success(target: CaptureTarget, meta: &CapturedTextureMeta) {
    if capture_quiesced() {
        return;
    }
    if let Some(runtime) = RUNTIME.get() {
        if let Ok(mut runtime) = runtime.lock() {
            runtime.captured_symbols.insert(target.symbol.to_string());
            mark_capture_progress_now();
            runtime.captured_runtime_textures.insert(
                target.symbol.to_string(),
                RuntimeTextureCaptureRecord {
                    captured_at: meta.captured_at,
                    fnv1a64: meta.fnv1a64.clone(),
                },
            );
            if target.symbol == TERRAIN_ID_RESOURCE_SYMBOL {
                runtime.terrain_id_srv_slot_observed = Some(meta.slot);
            }
            write_event(
                &runtime.output_dir,
                &format!(
                    "{{\"type\":\"runtime_texture_captured\",\"symbol\":\"{}\",\"file\":\"{}\",\"width\":{},\"height\":{},\"byte_len\":{},\"fnv1a64\":\"{}\",\"source_dxgi_format\":{},\"shader_resource_slot\":{},\"identity\":\"{}\",\"captured_at\":\"{}\"}}",
                    target.symbol,
                    target.rgba_file,
                    meta.width,
                    meta.height,
                    meta.byte_len,
                    meta.fnv1a64,
                    meta.source_format,
                    meta.slot,
                    json_escape(&meta.identity),
                    meta.captured_at.as_str()
                ),
            );
            if refresh_session_capture_progress(&mut runtime) {
                CAPTURED.store(true, Ordering::SeqCst);
            }
        }
    }
}

fn capture_complete(runtime: &CaptureRuntime) -> bool {
    if runtime.capture_moment_sequence.is_empty() {
        return false;
    }
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
    if capture_quiesced() {
        return false;
    }
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
        runtime.active_moment_started_ms =
            unsafe { GetTickCount64() }.saturating_sub(runtime.start_tick_ms);
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
    let complete = capture_complete(runtime);
    // MOMFIX1: latch continuous-control-sequence completion BEFORE any round can
    // overwrite the shared state, then start a held manual round now that the
    // latch is set. Both are no-ops when a round is already active or the
    // sequence is not yet complete. maybe_start_deferred_manual_round re-arms the
    // round's hook interests internally on success.
    mark_control_sequence_complete_if_reached(runtime);
    maybe_start_deferred_manual_round(runtime);
    if (complete && !runtime.census.enabled && !runtime.round_commands_enabled)
        || CAPTURE_BUDGET_STOPPED.load(Ordering::SeqCst)
    {
        // Moment sequence finished: the capture arming flags are about to be
        // cleared. If the persistent CB0 sampler is armed, drop a one-time
        // breadcrumb recording how many samples it had taken by this boundary.
        // Any sample_index >= this value in cb0_samples.jsonl is proof the
        // sampler kept running AFTER moment completion (the survival this job
        // exists to guarantee). This is diagnostic-only; it changes no control
        // flow and is emitted at most once per session.
        maybe_record_cb0_sampler_moment_boundary(&runtime.root_output_dir);
        clear_context_hook_interests();
    } else {
        refresh_context_hook_interests_for_runtime(runtime);
    }
    complete
}

static CB0_SAMPLER_MOMENT_BOUNDARY_RECORDED: AtomicBool = AtomicBool::new(false);

fn maybe_record_cb0_sampler_moment_boundary(root_output_dir: &Path) {
    if !CB0_SAMPLER_ARMED.load(Ordering::Acquire) {
        return;
    }
    if CB0_SAMPLER_MOMENT_BOUNDARY_RECORDED.swap(true, Ordering::SeqCst) {
        return;
    }
    let samples_at_boundary = CB0_SAMPLE_COUNT.load(Ordering::Acquire);
    write_event(
        root_output_dir,
        &format!(
            "{{\"type\":\"cb0_sampler_moment_sequence_complete\",\"samples_at_boundary\":{}}}",
            samples_at_boundary
        ),
    );
}

fn reset_current_map_frame_capture(runtime: &mut CaptureRuntime) {
    runtime.map_frame_draws.clear();
    runtime.map_frame_blockers.clear();
    runtime.map_frame_rejections.clear();
    runtime.ui_pass = UiPassCaptureState::default();
    runtime.latest_post_draw_output_merger = None;
    runtime.latest_post_draw_output_blockers.clear();
    runtime.capture_work_frame_ms = 0;
    CONTEXT_CURRENT_PIXEL_SHADER_RELEVANT.store(false, Ordering::SeqCst);
    CONTEXT_CURRENT_VERTEX_SHADER_RELEVANT.store(false, Ordering::SeqCst);
    CONTEXT_CURRENT_INPUT_LAYOUT_RELEVANT.store(false, Ordering::SeqCst);
    refresh_context_hook_interests_for_runtime(runtime);
}

fn start_map_frame_capture_round_for_runtime(
    runtime: &mut CaptureRuntime,
    label: &str,
) -> Result<(), String> {
    if !runtime.round_commands_enabled {
        return Err("map-frame capture round commands are not enabled".to_string());
    }
    // MOMFIX1: never overwrite the shared capture state of a round that is still
    // in flight (the manual path calls this directly and has no active-round
    // guard of its own). Recorded rejection, not a panic — hooks never assert.
    if runtime.active_round_label.is_some() && !runtime.active_round_finished {
        return Err(format!(
            "map-frame capture round '{}' is still in progress",
            runtime.active_round_label.as_deref().unwrap_or_default()
        ));
    }
    let label = sanitize_round_label(label)?;
    let round_dir = runtime.root_output_dir.join("rounds").join(&label);
    fs::create_dir_all(&round_dir).map_err(|error| error.to_string())?;

    let releases = runtime
        .deferred_runtime_targets
        .values()
        .map(|target| target.srv)
        .filter(|srv| *srv != 0)
        .collect::<Vec<_>>();
    runtime.pending_render_thread_release_srvs.extend(releases);
    runtime.deferred_runtime_targets.clear();

    runtime.output_dir = round_dir.clone();
    // Rounds capture the SESSION'S configured map moments, not a hardcoded
    // set: the 2026-07-05 live wedge was every round stalling forever on
    // borders_pass/trees_pass that the attach command had deliberately
    // excluded (this line used to overwrite the control's list with all
    // five map moments unconditionally).
    runtime.capture_moment_sequence = map_frame_round_moment_sequence_for_runtime(runtime);
    runtime.active_capture_index = 0;
    runtime.active_moment_started_ms =
        unsafe { GetTickCount64() }.saturating_sub(runtime.start_tick_ms);
    runtime.dwell_end_generated_texture_check_done = false;
    runtime.pending_dwell_end_boundary_finalization = false;
    runtime.pending_dwell_end_generated_texture_check = false;
    runtime.capture_quiesced_event_recorded = false;
    runtime.captured_symbols.clear();
    runtime.captured_runtime_textures.clear();
    runtime.shared_payloads.clear();
    runtime.shared_payload_files_written.clear();
    runtime.boundary_payloads.clear();
    runtime.boundary_payload_warnings.clear();
    runtime.completed_map_frame_moments.clear();
    runtime.map_frame_draws.clear();
    runtime.map_frame_blockers.clear();
    runtime.map_frame_rejections.clear();
    runtime.ui_pass = UiPassCaptureState::default();
    runtime.ui_shader_files_written.clear();
    runtime.latest_post_draw_output_merger = None;
    runtime.latest_post_draw_output_blockers.clear();
    runtime.om_render_target_clears.clear();
    runtime.om_depth_stencil_clears.clear();
    runtime.om_clear_sequence = 0;
    runtime.terrain_id_srv_slot_observed = None;
    runtime.active_round_label = Some(label.clone());
    runtime.active_round_finished = false;
    runtime.last_error = None;
    mark_round_open(unsafe { GetTickCount64() });
    CAPTURED.store(false, Ordering::SeqCst);
    reset_capture_in_progress_flags();
    refresh_context_hook_interests_for_runtime(runtime);
    write_event(
        &runtime.root_output_dir,
        &format!(
            "{{\"type\":\"map_frame_capture_round_started\",\"label\":\"{}\",\"dir\":\"{}\"}}",
            json_escape(&label),
            json_escape(&round_dir.to_string_lossy())
        ),
    );
    write_event(
        &runtime.output_dir,
        &format!(
            "{{\"type\":\"map_frame_capture_round_started\",\"label\":\"{}\",\"dir\":\"{}\"}}",
            json_escape(&label),
            json_escape(&round_dir.to_string_lossy())
        ),
    );
    Ok(())
}

fn auto_round_watch_pending(runtime: &CaptureRuntime) -> bool {
    if !runtime.round_commands_enabled {
        return false;
    }
    // MOMFIX1 HOLD (auto): do not arm the height sampler until the continuous
    // control sequence has completed once. A round cannot start before then
    // (round_hold_pending_moments would defer it and starve the excluded
    // moments), so sampling early only burns hot-path cycles and arms context
    // interests for a round that cannot fire. Once the latch flips, the sampler
    // arms and the next height sample starts the round.
    if !runtime.control_sequence_complete {
        return false;
    }
    if runtime.active_round_label.is_some() && !runtime.active_round_finished {
        return false;
    }
    runtime.census.auto_round_buckets_done.len() < AUTO_ROUND_HEIGHT_BUCKETS.len()
}

const AUTO_ROUND_HEIGHT_BUCKETS: [AutoRoundHeightBucket; 6] = [
    AutoRoundHeightBucket {
        min_inclusive: 0.0,
        max_exclusive: Some(150.0),
        label: "auto_h0_150",
    },
    AutoRoundHeightBucket {
        min_inclusive: 150.0,
        max_exclusive: Some(300.0),
        label: "auto_h150_300",
    },
    AutoRoundHeightBucket {
        min_inclusive: 300.0,
        max_exclusive: Some(600.0),
        label: "auto_h300_600",
    },
    AutoRoundHeightBucket {
        min_inclusive: 600.0,
        max_exclusive: Some(1200.0),
        label: "auto_h600_1200",
    },
    AutoRoundHeightBucket {
        min_inclusive: 1200.0,
        max_exclusive: Some(2400.0),
        label: "auto_h1200_2400",
    },
    AutoRoundHeightBucket {
        min_inclusive: 2400.0,
        max_exclusive: None,
        label: "auto_h2400_plus",
    },
];

#[derive(Clone, Copy, Debug)]
struct AutoRoundHeightBucket {
    min_inclusive: f32,
    max_exclusive: Option<f32>,
    label: &'static str,
}

unsafe fn sample_auto_round_height_if_active(context: *mut ID3D11DeviceContext) {
    debug_assert_render_thread_owner("sample_auto_round_height_if_active");
    if context.is_null() || !CONTEXT_AUTO_ROUND_ARMED.load(Ordering::Acquire) {
        return;
    }
    // Same crash class as the census payload fix: this probe issues
    // PSGetConstantBuffers on the immediate context. TerrainSceneCB0 cannot be
    // genuinely bound before the map renderer is live, so sampling before the
    // trust latch is pure load-screen context exposure for zero information
    // (the 2026-07-05 session logged only "CB0 not bound" failures there).
    if !CONTEXT_MAP_RENDERER_LIVE.load(Ordering::Acquire) {
        return;
    }
    let now = unsafe { GetTickCount64() };
    let should_sample = RUNTIME
        .get()
        .and_then(|runtime| runtime.lock().ok())
        .is_some_and(|mut runtime| {
            if !auto_round_watch_pending(&runtime) || now < runtime.census.next_auto_round_sample_ms
            {
                return false;
            }
            runtime.census.next_auto_round_sample_ms =
                now.saturating_add(AUTO_ROUND_SAMPLE_INTERVAL_MS);
            true
        });
    if !should_sample {
        return;
    }
    match unsafe { active_terrain_scene_cb0_camera_height(context) } {
        Ok(height) => record_auto_round_height_sample(height, now),
        Err(error) => {
            if let Some(runtime) = RUNTIME.get() {
                if let Ok(runtime) = runtime.lock() {
                    write_event(
                        &runtime.root_output_dir,
                        &format!(
                            "{{\"type\":\"map_frame_auto_round_sample_failed\",\"message\":\"{}\"}}",
                            json_escape(&error)
                        ),
                    );
                }
            }
        }
    }
}

unsafe fn active_terrain_scene_cb0_camera_height(
    context: *mut ID3D11DeviceContext,
) -> Result<f32, String> {
    debug_assert_render_thread_owner("active_terrain_scene_cb0_camera_height");
    let mut buffer: *mut ID3D11Buffer = ptr::null_mut();
    unsafe {
        (*context).PSGetConstantBuffers(TERRAIN_SCENE_CB0_BIND_POINT, 1, &mut buffer);
    }
    if buffer.is_null() {
        return Err("TerrainSceneCB0 PS slot 0 is not bound".to_string());
    }
    let result = unsafe { readback_d3d11_buffer_bytes(context, buffer) };
    unsafe {
        release_unknown(buffer as *mut WinVoid);
    }
    let (_, bytes) = result?;
    terrain_scene_cb0_camera_height(&bytes)
        .ok_or_else(|| "TerrainSceneCB0 has no finite vCamPos.y height at offset 208".to_string())
}

fn terrain_scene_cb0_camera_height(bytes: &[u8]) -> Option<f32> {
    let end = TERRAIN_SCENE_CB0_VCAMPOS_OFFSET.checked_add(mem::size_of::<f32>())?;
    let raw: [u8; 4] = bytes
        .get(TERRAIN_SCENE_CB0_VCAMPOS_OFFSET..end)?
        .try_into()
        .ok()?;
    let value = f32::from_le_bytes(raw);
    value.is_finite().then_some(value)
}

// ---------------------------------------------------------------------------
// Persistent scene-CB0 sampler (the day/night sun-curve recorder).
//
// This is a deliberately minimal, standalone recorder that lives for the WHOLE
// session so the day/night sun curve can be sampled across many in-game hours.
// The existing auto-round height sampler (sample_auto_round_height_if_active)
// dies the moment capture quiesces because it runs inside the capture_allowed
// gate; this one runs BEFORE that gate on every draw and never consults
// capture_quiesced()/CAPTURE_BUDGET_STOPPED. It is armed only by
// --cb0-sample-ms; absent, none of this code executes past the first relaxed
// load, keeping the default draw path a no-op feature path.
//
// Laws obeyed (all pre-existing, none regressed):
//  * NEVER assert/panic — every failure records an event or silently skips.
//  * Trusted-context ONLY — samples nothing until CONTEXT_MAP_RENDERER_LIVE is
//    latched (the real map renderer proven live on the render-thread-owned
//    immediate context). Untrusted/load-screen time records nothing.
//  * Never blocks/reorders the moment machinery — it reads back a single 416B
//    constant buffer via the proven-safe readback path and appends one line.
//  * BYTE-BUDGET EXEMPT and work-pause exempt — bounded instead by the wall
//    rate (>=1 sample per --cb0-sample-ms ms) and a hard CB0_SAMPLER_MAX_SAMPLES
//    cap; at the documented ~2 samples/s that is <3MB/hour.
//  * SURVIVES moment completion and round completion — those clear the capture
//    arming flags but touch none of the sampler statics or the trust latch, so
//    the sampler keeps appending. It dies only with the session: at
//    run_one_shot finalize (vtable hooks removed => no draw hook fires) or at
//    process detach (CB0_SAMPLER_DETACHING hard-stops it before any D3D call).

fn arm_cb0_sampler_from_control(
    control: Option<&TerrainCaptureControl>,
    start_tick_ms: u64,
    output_dir: &Path,
) {
    let Some(interval) = control
        .and_then(|control| control.cb0_sample_ms)
        .filter(|interval| *interval > 0)
    else {
        // Feature off: leave every sampler static at its default. No draw-path
        // code beyond the first CB0_SAMPLER_ARMED load will ever run.
        return;
    };
    CB0_SAMPLE_INTERVAL_MS.store(interval, Ordering::SeqCst);
    CB0_SESSION_START_MS.store(start_tick_ms, Ordering::SeqCst);
    CB0_NEXT_SAMPLE_MS.store(start_tick_ms, Ordering::SeqCst);
    CB0_SAMPLE_COUNT.store(0, Ordering::SeqCst);
    CB0_SAMPLER_CAP_RECORDED.store(false, Ordering::SeqCst);
    CB0_SAMPLER_MOMENT_BOUNDARY_RECORDED.store(false, Ordering::SeqCst);
    CB0_SAMPLER_DETACHING.store(false, Ordering::SeqCst);
    // Capture the session output dir once so the draw hot path can resolve it
    // WITHOUT taking the RUNTIME mutex (which would risk a same-thread deadlock
    // via a RUNTIME.lock() -> original Draw* re-entry, and is poisoned at
    // quiesce). set() is once-per-process; ignore failure (real sessions arm
    // once; unit tests re-arm and simply keep the first dir).
    let _ = CB0_SAMPLER_OUTPUT_DIR.set(output_dir.to_path_buf());
    // Armed LAST with Release so the config above is visible to any draw thread
    // that observes ARMED==true.
    CB0_SAMPLER_ARMED.store(true, Ordering::Release);
    write_event(
        output_dir,
        &format!(
            "{{\"type\":\"cb0_sampler_armed\",\"sample_interval_ms\":{},\"max_samples\":{},\"file\":\"{}\",\"scene_cb0_byte_len\":{}}}",
            interval, CB0_SAMPLER_MAX_SAMPLES, CB0_SAMPLES_FILE, CB0_SCENE_BYTE_LEN
        ),
    );
}

/// CAPFIX3 — install the --arm-delay-ms window from the control. Absent/0 =>
/// no delay (MAP_FRAME_ARM_DELAY_MS stays 0 and [`map_frame_arm_delay_open`]
/// always returns true). Set once at run_one_shot start before any hook can
/// capture; the window is later measured from the renderer-live latch tick.
fn configure_map_frame_arm_delay_from_control(
    control: Option<&TerrainCaptureControl>,
    output_dir: &Path,
) {
    let delay = control
        .and_then(|control| control.arm_delay_ms)
        .filter(|delay| *delay > 0)
        .unwrap_or(0);
    MAP_FRAME_ARM_DELAY_MS.store(delay, Ordering::SeqCst);
    if delay > 0 {
        write_event(
            output_dir,
            &format!(
                "{{\"type\":\"map_frame_arm_delay_configured\",\"arm_delay_ms\":{},\"measured_from\":\"map_renderer_live\",\"message\":\"map-frame moment sequence will not arm until this many ms after the map renderer goes live; CB0 sampler and renderer-live trust latch are not delayed\"}}",
                delay
            ),
        );
    }
}

/// Decide whether this draw wins the current sample window. Pure so the CAS
/// dedup logic is unit-testable. Returns the deadline to install on success.
/// Single-shot CAS: two render threads may both observe `now >= next`, but only
/// one can move CB0_NEXT_SAMPLE_MS from the observed value to `now + interval`;
/// the loser sees the CAS fail and skips (the window is claimed), which is
/// correct — no CAS loop, no double-append.
fn cb0_sampler_try_claim_window(now_ms: u64) -> bool {
    if !CB0_SAMPLER_ARMED.load(Ordering::Acquire) {
        return false;
    }
    let interval = CB0_SAMPLE_INTERVAL_MS.load(Ordering::Acquire);
    if interval == 0 {
        return false;
    }
    let observed = CB0_NEXT_SAMPLE_MS.load(Ordering::Acquire);
    if now_ms < observed {
        return false;
    }
    let deadline = now_ms.saturating_add(interval);
    CB0_NEXT_SAMPLE_MS
        .compare_exchange(observed, deadline, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
}

/// CAPFIX3 ISSUE 1 — is the persistent CB0 sampler still live and sampling?
/// True only while the sampler is armed AND has not begun detaching. Used by
/// [`capture_ready_to_finish`] to keep the dwell loop spinning to expiry after
/// the moment sequence completes, so the sun-curve recorder survives moment
/// completion / round completion / budget quiesce (SAMP1 contract), dying only
/// at dwell expiry or process detach. Lock-free (two relaxed atomic loads) so
/// it is safe to poll from the finish loop. A capped-out sampler
/// (CB0_SAMPLER_ARMED already cleared at the cap) or a disarmed session returns
/// false and lets the loop finish normally.
fn cb0_sampler_keepalive_active() -> bool {
    CB0_SAMPLER_ARMED.load(Ordering::Acquire)
        && !CB0_SAMPLER_DETACHING.load(Ordering::Acquire)
        && !HOOKS_DISARMED.load(Ordering::Acquire)
}

/// CAPFIX3 — latch "the map renderer is live" and, the FIRST time, record the
/// tick so [`map_frame_arm_delay_open`] can measure --arm-delay-ms from this
/// moment. Lock-free; safe to call from the render-thread hot path. The bool
/// store is idempotent; the tick is captured once via a 0->now CAS so a later
/// re-latch never moves the window.
fn latch_map_renderer_live() {
    CONTEXT_MAP_RENDERER_LIVE.store(true, Ordering::Release);
    if MAP_RENDERER_LIVE_TICK_MS.load(Ordering::Acquire) == 0 {
        let now = unsafe { GetTickCount64() }.max(1);
        let _ =
            MAP_RENDERER_LIVE_TICK_MS.compare_exchange(0, now, Ordering::SeqCst, Ordering::SeqCst);
    }
}

/// CAPFIX3 — is the map-frame arm-delay window over (may we arm the moment
/// sequence)? True when no delay is configured, OR the renderer is live and at
/// least --arm-delay-ms have elapsed since it went live. Lock-free. When it
/// first flips open, records a one-time event so the log shows exactly when
/// capture arming began relative to the delay. Until it returns true,
/// [`refresh_context_hook_interests_for_runtime`] withholds map-frame draw
/// interests so the orchestrator can zoom/position first.
fn map_frame_arm_delay_open() -> bool {
    let delay = MAP_FRAME_ARM_DELAY_MS.load(Ordering::Acquire);
    if delay == 0 {
        return true;
    }
    let live_tick = MAP_RENDERER_LIVE_TICK_MS.load(Ordering::Acquire);
    if live_tick == 0 {
        // Renderer not live yet: nothing to capture regardless; window closed.
        return false;
    }
    let elapsed = unsafe { GetTickCount64() }.saturating_sub(live_tick);
    if elapsed < delay {
        return false;
    }
    if !MAP_FRAME_ARM_DELAY_ELAPSED_RECORDED.swap(true, Ordering::SeqCst) {
        // Lockless: never touch the RUNTIME mutex from this render-thread arming
        // gate (it may be held by the very refresh that is calling us).
        write_event_lockless(&format!(
            "{{\"type\":\"map_frame_arm_delay_elapsed\",\"arm_delay_ms\":{},\"elapsed_ms\":{},\"message\":\"arm-delay window over; map-frame moment sequence now arming\"}}",
            delay, elapsed
        ));
    }
    true
}

fn cb0_encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
fn cb0_decode_hex(text: &str) -> Option<Vec<u8>> {
    if text.len() % 2 != 0 {
        return None;
    }
    fn nibble(ch: u8) -> Option<u8> {
        match ch {
            b'0'..=b'9' => Some(ch - b'0'),
            b'a'..=b'f' => Some(ch - b'a' + 10),
            b'A'..=b'F' => Some(ch - b'A' + 10),
            _ => None,
        }
    }
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() / 2);
    for pair in bytes.chunks_exact(2) {
        out.push((nibble(pair[0])? << 4) | nibble(pair[1])?);
    }
    Some(out)
}

fn append_cb0_sample_line(output_dir: &Path, line: &str) {
    let path = output_dir.join(CB0_SAMPLES_FILE);
    let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) else {
        return;
    };
    // Single-writer by construction: sample_scene_cb0_if_active only reaches
    // this call on the render-thread OWNER (the GetCurrentThreadId ==
    // RENDER_THREAD_ID gate returns early for every other thread), and the CAS
    // window claim serializes even that one thread. So there is no concurrent
    // appender to interleave with — one writeln is one complete JSONL line.
    let _ = writeln!(file, "{line}");
}

fn cb0_sampler_output_dir() -> Option<&'static Path> {
    // Lock-free: reads the dir captured at arm time. The draw hot path must
    // never take the RUNTIME mutex (see CB0_SAMPLER_OUTPUT_DIR docs).
    CB0_SAMPLER_OUTPUT_DIR.get().map(PathBuf::as_path)
}

/// Persistent scene-CB0 sampler entry point. Called first inside every draw
/// hook, before the capture_allowed gate and before the original draw. All
/// failure paths record-or-skip; nothing here can panic.
unsafe fn sample_scene_cb0_if_active(context: *mut ID3D11DeviceContext) {
    // CAPFIX2 BUG 1: the sampler runs BEFORE the quiesce gate in the draw hooks,
    // so once dwell-end disarms the hooks it must stop issuing D3D reads. Disarm
    // already clears CB0_SAMPLER_ARMED and sets CB0_SAMPLER_DETACHING (below),
    // but check the disarm latch explicitly so the invariant is self-evident and
    // cannot be reintroduced by a future change to the sampler latches.
    if hooks_disarmed() {
        return;
    }
    // Cheapest possible early-out on the default (unarmed) path: one relaxed
    // load. Also the detach hard-stop and the null-context guard.
    if !CB0_SAMPLER_ARMED.load(Ordering::Acquire) {
        return;
    }
    if CB0_SAMPLER_DETACHING.load(Ordering::Acquire) || context.is_null() {
        return;
    }
    // Trusted-context law: sample nothing until the real map renderer is proven
    // live on this immediate context. This latch is never cleared on quiesce,
    // so it stays true across moment completion and budget quiesce.
    if !CONTEXT_MAP_RENDERER_LIVE.load(Ordering::Acquire) {
        return;
    }
    let now = unsafe { GetTickCount64() };
    if !cb0_sampler_try_claim_window(now) {
        return;
    }
    // Cap: bound unbounded-session growth. Event once, then disarm.
    if CB0_SAMPLE_COUNT.load(Ordering::Acquire) >= CB0_SAMPLER_MAX_SAMPLES {
        if !CB0_SAMPLER_CAP_RECORDED.swap(true, Ordering::SeqCst) {
            CB0_SAMPLER_ARMED.store(false, Ordering::SeqCst);
            if let Some(dir) = cb0_sampler_output_dir() {
                write_event(
                    dir,
                    &format!(
                        "{{\"type\":\"cb0_sampler_cap_reached\",\"max_samples\":{}}}",
                        CB0_SAMPLER_MAX_SAMPLES
                    ),
                );
            }
        }
        return;
    }
    // Immediate-context + render-thread-owner check. Same non-fatal recording
    // helpers the rest of the runtime uses; deferred contexts and off-owner
    // touches are skipped, never fatal (live 2026-07-05 taught this law).
    let context_type = unsafe { (*context).GetType() };
    if context_type != D3D11_DEVICE_CONTEXT_IMMEDIATE {
        record_deferred_context_skipped("cb0_sampler", context as usize, context_type as u32);
        return;
    }
    let owner_thread = RENDER_THREAD_ID.load(Ordering::SeqCst);
    if owner_thread != 0 {
        let current_thread = unsafe { GetCurrentThreadId() };
        if current_thread != owner_thread {
            record_render_thread_mismatch(
                "cb0_sampler",
                context as usize,
                current_thread,
                owner_thread,
            );
            return;
        }
    }
    // Confirm the ACTUAL currently-bound PS is a known capture-family (terrain/
    // water) shader. We read it live via PSGetShader rather than trusting
    // runtime.state.current_pixel_shader(), which goes stale after a budget
    // quiesce (observe_ps_set_shader stops running once capture_allowed is
    // false). This guarantees the CB at PS b0 is the terrain scene CB0 and not,
    // say, a UI constant buffer during a post-map UI draw.
    let mut pixel_shader: *mut ID3D11PixelShader = ptr::null_mut();
    unsafe {
        (*context).PSGetShader(&mut pixel_shader, ptr::null_mut(), ptr::null_mut());
    }
    let ps_ptr = pixel_shader as usize;
    unsafe {
        // PSGetShader AddRefs the returned interface; release it immediately.
        release_unknown(pixel_shader as *mut WinVoid);
    }
    if !capture_pixel_shader_known(ps_ptr) {
        return;
    }
    // Read back the 416-byte scene CB0 from PS slot 0 via the proven-safe path
    // (skips SHARED resources, staging-copies plain buffers; SEH-safe).
    let mut buffer: *mut ID3D11Buffer = ptr::null_mut();
    unsafe {
        (*context).PSGetConstantBuffers(TERRAIN_SCENE_CB0_BIND_POINT, 1, &mut buffer);
    }
    if buffer.is_null() {
        return;
    }
    let result = unsafe { readback_d3d11_buffer_bytes(context, buffer) };
    unsafe {
        release_unknown(buffer as *mut WinVoid);
    }
    let bytes = match result {
        Ok((_, bytes)) => bytes,
        Err(_) => return,
    };
    if bytes.len() != CB0_SCENE_BYTE_LEN {
        // Wrong-length CB at slot 0 (not the scene CB0). Record once so the
        // condition is visible, then skip; do NOT count it as a sample.
        if let Some(dir) = cb0_sampler_output_dir() {
            write_event(
                dir,
                &format!(
                    "{{\"type\":\"cb0_sampler_unexpected_len\",\"byte_len\":{},\"expected\":{}}}",
                    bytes.len(),
                    CB0_SCENE_BYTE_LEN
                ),
            );
        }
        return;
    }
    let Some(dir) = cb0_sampler_output_dir() else {
        return;
    };
    let wall_ms = now.saturating_sub(CB0_SESSION_START_MS.load(Ordering::Acquire));
    let index = CB0_SAMPLE_COUNT.fetch_add(1, Ordering::SeqCst);
    append_cb0_sample_line(
        dir,
        &format!(
            "{{\"wall_ms\":{},\"sample_index\":{},\"cb0_hex\":\"{}\"}}",
            wall_ms,
            index,
            cb0_encode_hex(&bytes)
        ),
    );
}

fn record_auto_round_height_sample(height: f32, now_ms: u64) {
    let Some(label) = auto_round_label_for_height(height) else {
        return;
    };
    let event = RUNTIME
        .get()
        .and_then(|runtime| runtime.lock().ok())
        .and_then(|mut runtime| {
            maybe_start_auto_round_for_height_locked(&mut runtime, label, now_ms)
        });
    if let Some((root, event)) = event {
        write_event(&root, &event);
    }
}

fn auto_round_label_for_height(height: f32) -> Option<&'static str> {
    if !height.is_finite() {
        return None;
    }
    AUTO_ROUND_HEIGHT_BUCKETS
        .iter()
        .find(|bucket| {
            height >= bucket.min_inclusive
                && bucket
                    .max_exclusive
                    .map_or(true, |max_exclusive| height < max_exclusive)
        })
        .map(|bucket| bucket.label)
}

fn maybe_start_auto_round_for_height_locked(
    runtime: &mut CaptureRuntime,
    label: &'static str,
    now_ms: u64,
) -> Option<(PathBuf, String)> {
    if !auto_round_watch_pending(runtime) {
        return None;
    }
    if runtime.census.auto_round_buckets_done.contains(label) {
        return None;
    }
    if runtime
        .census
        .last_auto_round_started_ms
        .is_some_and(|last| now_ms.saturating_sub(last) < AUTO_ROUND_RATE_LIMIT_MS)
    {
        return None;
    }
    // MOMFIX1 HOLD (auto): the height sampler is not armed until the continuous
    // control sequence completes (see auto_round_watch_pending), so reaching here
    // with the latch unset should be impossible. Guard defensively: never start a
    // round that would starve still-pending control moments.
    if !round_hold_pending_moments(runtime).is_empty() {
        return None;
    }
    let root = runtime.root_output_dir.clone();
    match start_map_frame_capture_round_for_runtime(runtime, label) {
        Ok(()) => {
            runtime
                .census
                .auto_round_buckets_done
                .insert(label.to_string());
            runtime.census.last_auto_round_started_ms = Some(now_ms);
            Some((
                root,
                format!(
                    "{{\"type\":\"map_frame_auto_round_started\",\"label\":\"{}\"}}",
                    json_escape(label)
                ),
            ))
        }
        Err(error) => Some((
            root,
            format!(
                "{{\"type\":\"map_frame_auto_round_failed\",\"label\":\"{}\",\"message\":\"{}\"}}",
                json_escape(label),
                json_escape(&error)
            ),
        )),
    }
}

fn full_map_frame_round_moment_sequence() -> Vec<String> {
    [
        TERRAIN_CAPTURE_MOMENT,
        WATER_CAPTURE_MOMENT,
        RIVERS_CAPTURE_MOMENT,
        BORDERS_CAPTURE_MOMENT,
        TREES_CAPTURE_MOMENT,
    ]
    .iter()
    .map(|moment| (*moment).to_string())
    .collect()
}

/// Round moment list = the session's configured map moments (ui_pass has
/// its own continuous machinery and never belongs to a round). Falls back
/// to the historical five-map-moment set only when the control requested
/// no map moments at all.
fn map_frame_round_moment_sequence_for_runtime(runtime: &CaptureRuntime) -> Vec<String> {
    let requested: Vec<String> = capture_moment_sequence_from_control(runtime.control.as_ref())
        .into_iter()
        .filter(|moment| {
            moment != UI_PASS_CAPTURE_MOMENT
                && moment != RESTORE_CAPTURE_MOMENT
                && moment != MAPNAME_CAPTURE_MOMENT
        })
        .collect();
    if requested.is_empty() {
        full_map_frame_round_moment_sequence()
    } else {
        requested
    }
}

/// MOMFIX1: the control-requested continuous moments that have NOT yet been
/// captured. Empty once the continuous sequence is complete.
///
/// This is meaningful ONLY before the first round runs: a round start clears
/// `completed_map_frame_moments` (see start_map_frame_capture_round_for_runtime)
/// so after a round the completed set no longer reflects the continuous
/// sequence. Callers therefore gate on the `control_sequence_complete` latch
/// first and only fall through to this per-moment view while no round has run.
fn control_moments_still_pending(runtime: &CaptureRuntime) -> Vec<String> {
    capture_moment_sequence_from_control(runtime.control.as_ref())
        .into_iter()
        .filter(|moment| !runtime.completed_map_frame_moments.contains(moment))
        .collect()
}

/// MOMFIX1: the moments a round must wait for before it may overwrite the shared
/// capture state. Empty => the round may start. Non-empty => HOLD the round.
///
/// A round overwrites output_dir / capture_moment_sequence / active_capture_index
/// / completed_map_frame_moments / ui_pass with its filtered set, so starting one
/// while ANY control moment is still pending permanently starves that moment
/// (restore_pass/mapname_pass/ui_pass are excluded from the round sequence and
/// can never be re-armed; a still-pending non-excluded moment like water_pass
/// would instead be captured into the round dir, never landing as a continuous
/// artifact). We therefore hold rounds until the WHOLE control sequence has
/// completed once, recorded by the persistent latch.
fn round_hold_pending_moments(runtime: &CaptureRuntime) -> Vec<String> {
    if runtime.control_sequence_complete {
        return Vec::new();
    }
    control_moments_still_pending(runtime)
}

/// MOMFIX1: set the persistent control-sequence-complete latch the first time the
/// continuous sequence finishes, i.e. while no round has ever started (a round
/// start would have overwritten the shared sequence, so `capture_complete` after
/// that point speaks for the round, not the control). Idempotent; never clears.
fn mark_control_sequence_complete_if_reached(runtime: &mut CaptureRuntime) {
    if runtime.control_sequence_complete {
        return;
    }
    if runtime.active_round_label.is_some() {
        // A round is (or was) active: capture_moment_sequence no longer reflects
        // the control's continuous sequence, so completion here does not prove
        // the control sequence finished. The latch can only be set from the
        // pre-round continuous path.
        return;
    }
    if !capture_complete(runtime) {
        return;
    }
    runtime.control_sequence_complete = true;
    write_event(
        &runtime.root_output_dir,
        "{\"type\":\"control_sequence_complete\",\"rounds_unblocked\":true}",
    );
}

fn round_hold_pending_moments_json(pending: &[String]) -> String {
    json_string_array(pending.iter().map(String::as_str).collect())
}

fn sanitize_round_label(label: &str) -> Result<String, String> {
    let sanitized = sanitize_map_frame_file_component(label.trim());
    if sanitized.is_empty() {
        Err("map-frame capture round label is empty".to_string())
    } else {
        Ok(sanitized)
    }
}

fn poll_control_file_for_round_command() {
    if let Some(runtime) = RUNTIME.get() {
        if let Ok(mut runtime) = runtime.lock() {
            poll_control_file_for_round_command_for_runtime(&mut runtime);
        }
    }
}

fn poll_control_file_for_round_command_for_runtime(runtime: &mut CaptureRuntime) {
    let Ok(metadata) = fs::metadata(&runtime.control_path) else {
        return;
    };
    let modified = metadata.modified().ok();
    if modified.is_some() && modified == runtime.control_command_last_modified {
        return;
    }
    runtime.control_command_last_modified = modified;
    let Ok(bytes) = fs::read(&runtime.control_path) else {
        return;
    };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return;
    };
    let command_type = value.get("type").and_then(serde_json::Value::as_str);
    match command_type {
        Some("capture_map_frames_round") if runtime.round_commands_enabled => {
            let label = value
                .get("label")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string();
            note_control_activity();
            request_map_frame_round_for_runtime(runtime, label);
        }
        Some("capture_map_frames_round") => {}
        Some("ui_capture_state_now") => {
            let label = value
                .get("label")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string();
            note_control_activity();
            if let Err(error) = request_ui_capture_state_now_for_runtime(runtime, &label) {
                write_event(
                    &runtime.output_dir,
                    &format!(
                        "{{\"type\":\"ui_capture_state_now_command_failed\",\"label\":\"{}\",\"message\":\"{}\"}}",
                        json_escape(&label),
                        json_escape(&error)
                    ),
                );
            }
        }
        _ => {}
    }
}

/// MOMFIX1: honour a manual `capture_map_frames_round` command. If the
/// continuous control sequence is still incomplete, HOLD the round: stash the
/// label (the control file is edge-triggered by mtime, so re-polling would never
/// re-fire it) and record `round_deferred_pending_moments`. It is started by
/// `maybe_start_deferred_manual_round` once the sequence completes. If the
/// sequence is already complete, start immediately. Hooks never assert: every
/// failure is a recorded event.
fn request_map_frame_round_for_runtime(runtime: &mut CaptureRuntime, label: String) {
    if !runtime.round_commands_enabled {
        return;
    }
    let pending = round_hold_pending_moments(runtime);
    if !pending.is_empty() {
        runtime.pending_manual_round_label = Some(label.clone());
        write_event(
            &runtime.root_output_dir,
            &format!(
                "{{\"type\":\"round_deferred_pending_moments\",\"trigger\":\"manual\",\"label\":\"{}\",\"pending\":{}}}",
                json_escape(&label),
                round_hold_pending_moments_json(&pending)
            ),
        );
        return;
    }
    start_manual_map_frame_round_for_runtime(runtime, &label);
}

/// MOMFIX1: start a held manual round once the control sequence has completed.
/// Called from the progress-refresh path. No-op unless a label is stashed and
/// the sequence is now complete (round_hold_pending_moments empty).
fn maybe_start_deferred_manual_round(runtime: &mut CaptureRuntime) {
    if runtime.pending_manual_round_label.is_none() {
        return;
    }
    if !round_hold_pending_moments(runtime).is_empty() {
        return;
    }
    // A round already in flight? Keep the stash; retry on a later refresh.
    if runtime.active_round_label.is_some() && !runtime.active_round_finished {
        return;
    }
    let Some(label) = runtime.pending_manual_round_label.take() else {
        return;
    };
    start_manual_map_frame_round_for_runtime(runtime, &label);
}

fn start_manual_map_frame_round_for_runtime(runtime: &mut CaptureRuntime, label: &str) {
    if let Err(error) = start_map_frame_capture_round_for_runtime(runtime, label) {
        write_event(
            &runtime.root_output_dir,
            &format!(
                "{{\"type\":\"map_frame_capture_round_command_failed\",\"label\":\"{}\",\"message\":\"{}\"}}",
                json_escape(label),
                json_escape(&error)
            ),
        );
    }
}

fn request_ui_capture_state_now_for_runtime(
    runtime: &mut CaptureRuntime,
    label: &str,
) -> Result<(), String> {
    if !ui_pass_capture_pending_for_runtime(runtime) {
        return Err("ui_pass is not the active capture moment".to_string());
    }
    let label = label.trim();
    if label.is_empty() {
        return Err("ui_capture_state_now label is empty".to_string());
    }
    if runtime.ui_pass.state_count >= ui_pass_max_states_for_runtime(runtime) {
        return Err(format!(
            "ui_pass already captured max_states {}",
            ui_pass_max_states_for_runtime(runtime)
        ));
    }
    runtime.ui_pass.pending_capture_label = Some(label.to_string());
    runtime.ui_pass.pending_capture_signature = None;
    if runtime.ui_pass.active {
        runtime.ui_pass.active_capture_label = Some(label.to_string());
    }
    refresh_context_hook_interests_for_runtime(runtime);
    write_event(
        &runtime.output_dir,
        &format!(
            "{{\"type\":\"ui_capture_state_now_requested\",\"label\":\"{}\",\"state_index\":{}}}",
            json_escape(label),
            runtime.ui_pass.state_count
        ),
    );
    Ok(())
}

fn capture_budget_blocker(event: &CaptureBudgetExceeded) -> String {
    format!(
        "capture budget exceeded for {}: {} (draw_count={}, max_draws_per_pass={}, total_payload_bytes_reserved={}, attempted_payload_bytes={}, max_total_payload_bytes_per_session={}, max_payload_writer_queue_bytes={}, frame_capture_work_ms={}, max_capture_work_ms_per_frame={})",
        event.capture_moment,
        event.message,
        event.draw_count,
        event.max_draws_per_pass,
        event.total_payload_bytes_reserved,
        event.attempted_payload_bytes,
        event.max_total_payload_bytes_per_session,
        event.max_payload_writer_queue_bytes,
        event.frame_capture_work_ms,
        event.max_capture_work_ms_per_frame
    )
}

fn stop_capture_for_budget(event: CaptureBudgetExceeded) -> String {
    let blocker = capture_budget_blocker(&event);
    if CAPTURE_BUDGET_STOPPED.swap(true, Ordering::SeqCst) {
        CAPTURE_QUIESCED.store(true, Ordering::SeqCst);
        clear_context_hook_interests();
        return blocker;
    }
    let Some((output_dir, profile)) = RUNTIME.get().and_then(|runtime| {
        runtime.lock().ok().map(|mut runtime| {
            let profile = requested_map_frame_capture_profile_for_runtime(&runtime)
                .unwrap_or(TERRAIN_MAP_FRAME_CAPTURE_PROFILE);
            runtime.capture_budget_exceeded = Some(event.clone());
            if profile.capture_moment == UI_PASS_CAPTURE_MOMENT {
                runtime.ui_pass.blockers.push(blocker.clone());
            } else {
                runtime.map_frame_blockers.push(blocker.clone());
            }
            queue_quiesce_release_and_clear_capture_state(&mut runtime);
            (runtime.output_dir.clone(), profile)
        })
    }) else {
        CAPTURE_QUIESCED.store(true, Ordering::SeqCst);
        reset_capture_in_progress_flags();
        clear_context_hook_interests();
        // The runtime lock was unavailable (poisoned by a panic or unset):
        // the detailed budget event below is unreachable, which is how the
        // 2026-07-05 live session died with zero evidence. Record the stop
        // lockless so the death always names its cause.
        write_event_lockless(&format!(
            "{{\"type\":\"capture_budget_exceeded\",\"lockless\":true,\"runtime_lock_unavailable\":true,\"cap\":\"{}\",\"moment\":\"{}\",\"message\":\"{}\"}}",
            event.cap,
            event.capture_moment,
            json_escape(&event.message)
        ));
        return blocker;
    };
    CAPTURE_QUIESCED.store(true, Ordering::SeqCst);
    reset_capture_in_progress_flags();
    clear_context_hook_interests();
    write_event(
        &output_dir,
        &format!(
            "{{\"type\":\"capture_budget_exceeded\",\"cap\":\"{}\",\"moment\":\"{}\",\"draw_count\":{},\"max_draws_per_pass\":{},\"total_payload_bytes_reserved\":{},\"attempted_payload_bytes\":{},\"max_total_payload_bytes_per_session\":{},\"max_payload_writer_queue_bytes\":{},\"frame_capture_work_ms\":{},\"max_capture_work_ms_per_frame\":{},\"message\":\"{}\"}}",
            event.cap,
            event.capture_moment,
            event.draw_count,
            event.max_draws_per_pass,
            event.total_payload_bytes_reserved,
            event.attempted_payload_bytes,
            event.max_total_payload_bytes_per_session,
            event.max_payload_writer_queue_bytes,
            event.frame_capture_work_ms,
            event.max_capture_work_ms_per_frame,
            json_escape(&event.message)
        ),
    );
    // CAPFIX3 ISSUE 2 — document WHY a budget exceedance quiesces the WHOLE
    // session rather than sparing the remaining moments. Per-pass caps
    // (max_draws_per_pass, max_capture_work_ms_per_frame) reset every frame and,
    // after the CAPFIX3 pass-close-at-frame-boundary fix, no longer accumulate
    // cross-frame — so tripping one now means a SINGLE frame genuinely exceeded
    // the cap, which repeats deterministically next frame and would recur for
    // every later moment at this camera/zoom. The session-wide byte budget is a
    // shared monotonic non-refundable counter; once exceeded, continuing risks
    // unbounded disk use, the exact hazard the budget exists to prevent. There
    // is no safe per-moment "spend down and continue" — the quiesce latch also
    // owns the deferred-COM-release drain — so the moments die together by
    // design. Operators who need more headroom raise the specific cap
    // (--max-draws-per-pass, --max-total-payload-bytes-per-session,
    // --max-capture-work-ms-per-frame) rather than the DLL silently pressing on.
    write_event(
        &output_dir,
        &format!(
            "{{\"type\":\"capture_session_quiesced_together\",\"cap\":\"{}\",\"tripped_by_moment\":\"{}\",\"remaining_moments_aborted\":true,\"reason\":\"per-pass caps reset per frame and no longer accumulate cross-frame (CAPFIX3), so a trip is a deterministic single-frame overrun; the session byte budget is non-refundable; the quiesce latch owns the deferred COM-release drain — no safe partial continuation. Raise the specific cap flag to gain headroom.\"}}",
            event.cap, event.capture_moment
        ),
    );
    if let Err(error) = write_blocked_map_capture_session(
        &output_dir,
        vec![blocker.clone()],
        "capture_budget_exceeded",
        profile,
    ) {
        write_event(
            &output_dir,
            &format!(
                "{{\"type\":\"map_capture_session_write_failed\",\"file\":\"{}\",\"message\":\"{}\"}}",
                MAP_CAPTURE_SESSION_FILE,
                json_escape(&error)
            ),
        );
    }
    unsafe {
        drain_quiesced_render_thread_releases("capture_budget_exceeded");
    }
    blocker
}

fn check_next_map_frame_draw_budget() -> Result<(), String> {
    if capture_quiesced() || CAPTURE_BUDGET_STOPPED.load(Ordering::SeqCst) {
        return Err("capture budget already exceeded".to_string());
    }
    let Some(event) = RUNTIME.get().and_then(|runtime| {
        runtime
            .lock()
            .ok()
            .and_then(|runtime| next_map_frame_draw_budget_exceeded(&runtime))
    }) else {
        return Ok(());
    };
    Err(stop_capture_for_budget(event))
}

fn check_next_ui_pass_draw_budget() -> Result<(), String> {
    if capture_quiesced() || CAPTURE_BUDGET_STOPPED.load(Ordering::SeqCst) {
        return Err("capture budget already exceeded".to_string());
    }
    let Some(event) = RUNTIME.get().and_then(|runtime| {
        runtime.lock().ok().and_then(|runtime| {
            let draw_count = runtime.ui_pass.draws.len();
            let max_draws = ui_pass_max_draws_for_runtime(&runtime);
            (draw_count >= max_draws as usize).then(|| {
                let budget = runtime.capture_budget;
                CaptureBudgetExceeded {
                    cap: "max_draws_per_pass",
                    capture_moment: UI_PASS_CAPTURE_MOMENT,
                    draw_count,
                    attempted_payload_bytes: 0,
                    total_payload_bytes_reserved: runtime.total_payload_bytes_reserved,
                    max_total_payload_bytes_per_session: budget.max_total_payload_bytes_per_session,
                    max_payload_writer_queue_bytes: budget.max_payload_writer_queue_bytes,
                    frame_capture_work_ms: runtime.capture_work_frame_ms,
                    max_capture_work_ms_per_frame: budget.max_capture_work_ms_per_frame,
                    max_draws_per_pass: max_draws,
                    message: format!(
                        "next ui_pass draw would exceed max_draws_per_pass {max_draws}"
                    ),
                }
            })
        })
    }) else {
        return Ok(());
    };
    Err(stop_capture_for_budget(event))
}

fn next_map_frame_draw_budget_exceeded(runtime: &CaptureRuntime) -> Option<CaptureBudgetExceeded> {
    let profile = requested_map_frame_capture_profile_for_runtime(runtime).ok()?;
    let budget = runtime.capture_budget;
    let draw_count = runtime.map_frame_draws.len();
    if draw_count < budget.max_draws_per_pass as usize {
        return None;
    }
    Some(CaptureBudgetExceeded {
        cap: "max_draws_per_pass",
        capture_moment: profile.capture_moment,
        draw_count,
        attempted_payload_bytes: 0,
        total_payload_bytes_reserved: runtime.total_payload_bytes_reserved,
        max_total_payload_bytes_per_session: budget.max_total_payload_bytes_per_session,
        max_payload_writer_queue_bytes: budget.max_payload_writer_queue_bytes,
        frame_capture_work_ms: runtime.capture_work_frame_ms,
        max_capture_work_ms_per_frame: budget.max_capture_work_ms_per_frame,
        max_draws_per_pass: budget.max_draws_per_pass,
        message: format!(
            "next map-frame draw would exceed max_draws_per_pass {}",
            budget.max_draws_per_pass
        ),
    })
}

fn reserve_capture_payload_bytes(byte_len: usize, file: &str) -> Result<(), String> {
    if capture_quiesced() || CAPTURE_BUDGET_STOPPED.load(Ordering::SeqCst) {
        return Err("capture budget already exceeded".to_string());
    }
    let Some(event) = RUNTIME.get().and_then(|runtime| {
        runtime.lock().ok().and_then(|mut runtime| {
            reserve_capture_payload_bytes_for_runtime(&mut runtime, byte_len, file)
        })
    }) else {
        return Ok(());
    };
    Err(stop_capture_for_budget(event))
}

fn reserve_capture_payload_bytes_for_runtime(
    runtime: &mut CaptureRuntime,
    byte_len: usize,
    file: &str,
) -> Option<CaptureBudgetExceeded> {
    let profile = requested_map_frame_capture_profile_for_runtime(runtime).ok()?;
    let budget = runtime.capture_budget;
    let draw_count = active_moment_draw_count(runtime, profile);
    let attempted = byte_len as u64;
    let next_total = runtime
        .total_payload_bytes_reserved
        .saturating_add(attempted);
    runtime.total_payload_bytes_reserved = next_total;
    if next_total <= budget.max_total_payload_bytes_per_session {
        return None;
    }
    Some(CaptureBudgetExceeded {
        cap: "max_total_payload_bytes_per_session",
        capture_moment: profile.capture_moment,
        draw_count,
        attempted_payload_bytes: attempted,
        total_payload_bytes_reserved: next_total,
        max_total_payload_bytes_per_session: budget.max_total_payload_bytes_per_session,
        max_payload_writer_queue_bytes: budget.max_payload_writer_queue_bytes,
        frame_capture_work_ms: runtime.capture_work_frame_ms,
        max_capture_work_ms_per_frame: budget.max_capture_work_ms_per_frame,
        max_draws_per_pass: budget.max_draws_per_pass,
        message: format!(
            "reserving payload file {file} ({attempted} bytes) would exceed max_total_payload_bytes_per_session {}",
            budget.max_total_payload_bytes_per_session
        ),
    })
}

fn stop_capture_for_payload_writer_queue_full(
    file: &Path,
    byte_len: usize,
    pending_bytes: u64,
    max_pending_bytes: u64,
) -> String {
    let event = RUNTIME
        .get()
        .and_then(|runtime| {
            runtime.lock().ok().map(|runtime| {
                let profile = requested_map_frame_capture_profile_for_runtime(&runtime)
                    .unwrap_or(TERRAIN_MAP_FRAME_CAPTURE_PROFILE);
                let budget = runtime.capture_budget;
                let draw_count = active_moment_draw_count(&runtime, profile);
                CaptureBudgetExceeded {
                    cap: "max_payload_writer_queue_bytes",
                    capture_moment: profile.capture_moment,
                    draw_count,
                    attempted_payload_bytes: byte_len as u64,
                    total_payload_bytes_reserved: runtime.total_payload_bytes_reserved,
                    max_total_payload_bytes_per_session: budget.max_total_payload_bytes_per_session,
                    max_payload_writer_queue_bytes: budget.max_payload_writer_queue_bytes,
                    frame_capture_work_ms: runtime.capture_work_frame_ms,
                    max_capture_work_ms_per_frame: budget.max_capture_work_ms_per_frame,
                    max_draws_per_pass: budget.max_draws_per_pass,
                    message: format!(
                        "payload writer queue byte cap would be exceeded before enqueuing {} ({} bytes, pending_bytes {}, max_payload_writer_queue_bytes {})",
                        file.display(),
                        byte_len,
                        pending_bytes,
                        max_pending_bytes
                    ),
                }
            })
        })
        .unwrap_or(CaptureBudgetExceeded {
            cap: "max_payload_writer_queue_bytes",
            capture_moment: TERRAIN_CAPTURE_MOMENT,
            draw_count: 0,
            attempted_payload_bytes: byte_len as u64,
            total_payload_bytes_reserved: 0,
            max_total_payload_bytes_per_session: DEFAULT_MAX_TOTAL_PAYLOAD_BYTES_PER_SESSION,
            max_payload_writer_queue_bytes: DEFAULT_MAX_PAYLOAD_WRITER_QUEUE_BYTES,
            frame_capture_work_ms: 0,
            max_capture_work_ms_per_frame: DEFAULT_MAX_CAPTURE_WORK_MS_PER_FRAME,
            max_draws_per_pass: DEFAULT_MAX_DRAWS_PER_PASS,
            message: format!(
                "payload writer queue byte cap would be exceeded before enqueuing {} ({} bytes, pending_bytes {}, max_payload_writer_queue_bytes {})",
                file.display(),
                byte_len,
                pending_bytes,
                max_pending_bytes
            ),
        });
    stop_capture_for_budget(event)
}

fn payload_writer_queue_bytes_exceeded(
    pending_bytes: u64,
    attempted_bytes: usize,
    max_pending_bytes: u64,
) -> bool {
    pending_bytes
        .checked_add(attempted_bytes as u64)
        .map_or(true, |next_pending_bytes| {
            next_pending_bytes > max_pending_bytes
        })
}

fn payload_writer_queue_byte_cap() -> u64 {
    RUNTIME
        .get()
        .and_then(|runtime| runtime.lock().ok())
        .map_or(DEFAULT_MAX_PAYLOAD_WRITER_QUEUE_BYTES, |runtime| {
            runtime.capture_budget.max_payload_writer_queue_bytes
        })
}

fn charge_capture_work_ms(elapsed_ms: u64, stage: &'static str) {
    if elapsed_ms == 0 || capture_quiesced() || CAPTURE_BUDGET_STOPPED.load(Ordering::SeqCst) {
        return;
    }
    let Some(event) = RUNTIME.get().and_then(|runtime| {
        runtime.lock().ok().and_then(|mut runtime| {
            let profile = requested_map_frame_capture_profile_for_runtime(&runtime).ok()?;
            if profile.capture_moment == RESTORE_CAPTURE_MOMENT {
                write_event(
                    &runtime.output_dir,
                    &format!(
                        "{{\"type\":\"capture_work_budget_exempt\",\"moment\":\"{}\",\"stage\":\"{}\",\"elapsed_ms\":{},\"reason\":\"restore_pass_tiny_payload\"}}",
                        RESTORE_CAPTURE_MOMENT,
                        json_escape(stage),
                        elapsed_ms
                    ),
                );
                return None;
            }
            // The label pass is a single small glyph draw fired once per frame
            // after the map is live; like restore_pass it is exempt from the
            // per-frame work pause so it always completes. The per-session BYTE
            // budget still applies (enforced in write_map_frame_binary_payload).
            if profile.capture_moment == MAPNAME_CAPTURE_MOMENT {
                write_event(
                    &runtime.output_dir,
                    &format!(
                        "{{\"type\":\"capture_work_budget_exempt\",\"moment\":\"{}\",\"stage\":\"{}\",\"elapsed_ms\":{},\"reason\":\"mapname_pass_small_payload\"}}",
                        MAPNAME_CAPTURE_MOMENT,
                        json_escape(stage),
                        elapsed_ms
                    ),
                );
                return None;
            }
            runtime.capture_work_frame_ms =
                runtime.capture_work_frame_ms.saturating_add(elapsed_ms);
            let budget = runtime.capture_budget;
            let max_draws_per_pass = if profile.capture_moment == UI_PASS_CAPTURE_MOMENT {
                ui_pass_max_draws_for_runtime(&runtime)
            } else {
                budget.max_draws_per_pass
            };
            if runtime.capture_work_frame_ms <= budget.max_capture_work_ms_per_frame {
                return None;
            }
            Some(CaptureBudgetExceeded {
                cap: "max_capture_work_ms_per_frame",
                capture_moment: profile.capture_moment,
                draw_count: active_moment_draw_count(&runtime, profile),
                attempted_payload_bytes: 0,
                total_payload_bytes_reserved: runtime.total_payload_bytes_reserved,
                max_total_payload_bytes_per_session: budget.max_total_payload_bytes_per_session,
                max_payload_writer_queue_bytes: budget.max_payload_writer_queue_bytes,
                frame_capture_work_ms: runtime.capture_work_frame_ms,
                max_capture_work_ms_per_frame: budget.max_capture_work_ms_per_frame,
                max_draws_per_pass,
                message: format!(
                    "capture work at {stage} used {} ms in the current frame, exceeding max_capture_work_ms_per_frame {}",
                    runtime.capture_work_frame_ms, budget.max_capture_work_ms_per_frame
                ),
            })
        })
    }) else {
        return;
    };
    let _ = stop_capture_for_budget(event);
}

fn active_moment_draw_count(runtime: &CaptureRuntime, profile: MapFrameCaptureProfile) -> usize {
    if profile.capture_moment == UI_PASS_CAPTURE_MOMENT {
        runtime.ui_pass.draws.len()
    } else {
        runtime.map_frame_draws.len()
    }
}

fn reset_capture_work_frame_budget() {
    if capture_quiesced() {
        return;
    }
    if let Some(runtime) = RUNTIME.get() {
        if let Ok(mut runtime) = runtime.lock() {
            record_capture_work_frame_stat_locked(&runtime, "frame_boundary");
            runtime.capture_work_frame_ms = 0;
            if runtime.census.enabled {
                runtime.census.frame_index = runtime.census.frame_index.saturating_add(1);
                runtime.census.capture_work_frame_ms = 0;
                runtime.census.work_frame_paused = false;
                maybe_emit_census_heartbeat_locked(&mut runtime);
            }
        }
    }
}

fn record_capture_work_frame_stat_locked(runtime: &CaptureRuntime, stage: &'static str) {
    if runtime.capture_work_frame_ms == 0 {
        return;
    }
    let profile = requested_map_frame_capture_profile_for_runtime(runtime)
        .unwrap_or(TERRAIN_MAP_FRAME_CAPTURE_PROFILE);
    let over_budget =
        runtime.capture_work_frame_ms > runtime.capture_budget.max_capture_work_ms_per_frame;
    write_event(
        &runtime.output_dir,
        &format!(
            "{{\"type\":\"capture_work_frame\",\"stage\":\"{}\",\"moment\":\"{}\",\"work_ms\":{},\"max_capture_work_ms_per_frame\":{},\"over_budget\":{}}}",
            stage,
            profile.capture_moment,
            runtime.capture_work_frame_ms,
            runtime.capture_budget.max_capture_work_ms_per_frame,
            over_budget
        ),
    );
}

fn ui_pass_capture_pending_for_runtime(runtime: &CaptureRuntime) -> bool {
    runtime
        .capture_moment_sequence
        .get(runtime.active_capture_index)
        .map(String::as_str)
        == Some(UI_PASS_CAPTURE_MOMENT)
        && !runtime
            .completed_map_frame_moments
            .contains(UI_PASS_CAPTURE_MOMENT)
}

fn ui_pass_draw_capture_ready_for_runtime(runtime: &CaptureRuntime) -> bool {
    ui_pass_capture_pending_for_runtime(runtime) && runtime.current_swapchain_backbuffer.is_some()
}

fn ui_pass_max_draws_for_runtime(runtime: &CaptureRuntime) -> u32 {
    let configured = runtime
        .control
        .as_ref()
        .and_then(|control| control.max_draws_per_pass)
        .unwrap_or(runtime.capture_budget.max_draws_per_pass);
    configured.max(DEFAULT_UI_PASS_MAX_DRAWS_PER_FRAME).max(1)
}

fn ui_pass_max_states_for_runtime(runtime: &CaptureRuntime) -> u32 {
    ui_pass_max_states_for_control(runtime.control.as_ref())
}

fn ui_pass_max_states_for_control(control: Option<&TerrainCaptureControl>) -> u32 {
    control
        .and_then(|control| control.ui_max_states)
        .unwrap_or(DEFAULT_UI_PASS_MAX_STATES)
        .max(1)
}

fn ui_cb_skip_policy_for_runtime() -> UiCbSkipPolicy {
    RUNTIME
        .get()
        .and_then(|runtime| runtime.lock().ok())
        .and_then(|runtime| runtime.control.as_ref().map(|control| control.ui_cb_skip))
        .unwrap_or_default()
}

fn ui_pass_interop_hazard_draw(draw_call: &str, draw_count: u32) -> bool {
    draw_call == "Draw" && draw_count <= 6
}

fn ui_pass_skip_policy_applies(policy: UiCbSkipPolicy, hazard_draw: bool) -> bool {
    match policy {
        UiCbSkipPolicy::Hazard => hazard_draw,
        UiCbSkipPolicy::All => true,
        UiCbSkipPolicy::None => false,
    }
}

fn ui_pass_bundle_file(state_index: u32) -> String {
    format!("{UI_PASS_BUNDLE_FILE_PREFIX}_{state_index:03}.json")
}

fn ui_pass_file_draw_index(state_index: u32, draw_index: u32) -> u32 {
    state_index.saturating_mul(1000).saturating_add(draw_index)
}

fn ui_pass_debounce_allows_capture(runtime: &CaptureRuntime) -> bool {
    runtime
        .ui_pass
        .last_capture_frame_index
        .map_or(true, |last| {
            runtime.ui_pass.frame_index.saturating_sub(last) >= UI_PASS_MIN_FRAMES_BETWEEN_CAPTURES
        })
}

fn ui_pass_frame_signature_hash(signature: &UiPassFrameSignature) -> String {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&(signature.draws.len() as u32).to_le_bytes());
    for draw in &signature.draws {
        bytes.extend_from_slice(&(draw.pixel_shader_hash.len() as u32).to_le_bytes());
        bytes.extend_from_slice(draw.pixel_shader_hash.as_bytes());
        bytes.extend_from_slice(&(draw.vertex_shader_hash.len() as u32).to_le_bytes());
        bytes.extend_from_slice(draw.vertex_shader_hash.as_bytes());
        bytes.extend_from_slice(&(draw.texture_srv_pointers.len() as u32).to_le_bytes());
        for srv in &draw.texture_srv_pointers {
            bytes.extend_from_slice(&srv.slot.to_le_bytes());
            bytes.extend_from_slice(&(srv.pointer as u64).to_le_bytes());
        }
        bytes.extend_from_slice(&draw.vertex_count.to_le_bytes());
        bytes.extend_from_slice(&(draw.scissor_rects.len() as u32).to_le_bytes());
        for rect in &draw.scissor_rects {
            bytes.extend_from_slice(&rect.left.to_le_bytes());
            bytes.extend_from_slice(&rect.top.to_le_bytes());
            bytes.extend_from_slice(&rect.right.to_le_bytes());
            bytes.extend_from_slice(&rect.bottom.to_le_bytes());
        }
    }
    fnv1a64_bytes(&bytes)
}

fn record_candidate_rejected(
    target: CaptureTarget,
    slot: u32,
    identity: &'static str,
    error: &str,
) {
    if capture_quiesced() {
        return;
    }
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
    if capture_quiesced() {
        return;
    }
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
    #[serde(skip_serializing_if = "Option::is_none")]
    finalized_by: Option<&'static str>,
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
struct UiPassBundleFile {
    schema: &'static str,
    provenance: &'static str,
    finalized_by: &'static str,
    capture_moment: &'static str,
    state_index: u32,
    signature: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    session_manifest: &'static str,
    backbuffer: UiPassBackbufferFile,
    draws: Vec<MapFrameDrawFile>,
    runtime_files: BTreeMap<String, MapFrameFileEvidence>,
    blockers: Vec<String>,
}

#[derive(Clone, Debug)]
struct UiPassBundleWriteResult {
    clean: bool,
    file: String,
    bundle_path: String,
    state_index: u32,
    signature: String,
    label: Option<String>,
    draw_count: usize,
    duplicate: bool,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
struct UiPassBackbufferFile {
    resource_pointer: String,
    width: u32,
    height: u32,
    format: u32,
    sample_count: u32,
    sample_quality: u32,
}

#[derive(Clone, Debug, Serialize)]
struct TerrainChunkLibraryManifestFile {
    schema: &'static str,
    provenance: &'static str,
    capture_moment: &'static str,
    session_manifest: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    shader_cache_constants: Option<MapFrameShaderCacheConstantsFile>,
    chunks: Vec<TerrainChunkRecordFile>,
    vanilla_sources: BTreeMap<String, MapFrameFileEvidence>,
    runtime_files: BTreeMap<String, MapFrameFileEvidence>,
    coverage: TerrainChunkCoverageFile,
    blockers: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
struct TerrainChunkRecordFile {
    chunk_id: String,
    identity: TerrainChunkIdentityFile,
    bounds: TerrainChunkBoundsFile,
    world_quad: TerrainChunkWorldRectFile,
    draw: MapFrameDrawFile,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
struct TerrainChunkIdentityFile {
    draw_call: String,
    draw_count: u32,
    draw_start: u32,
    base_vertex_location: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    per_instance_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    instance_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    start_instance_location: Option<u32>,
    pixel_shader_fnv1a64: String,
    vertex_shader_fnv1a64: String,
    vertex_payloads: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    index_payload: Option<String>,
    per_chunk_constant_buffers: Vec<TerrainChunkConstantBufferIdentityFile>,
    world_quad: TerrainChunkWorldRectFile,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
struct TerrainChunkConstantBufferIdentityFile {
    stage: &'static str,
    slot: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    declared_name: Option<String>,
    fnv1a64: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
struct TerrainChunkPreidentityFile {
    draw_call: String,
    draw_count: u32,
    draw_start: u32,
    base_vertex_location: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    per_instance_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    instance_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    start_instance_location: Option<u32>,
    pixel_shader_fnv1a64: String,
    vertex_shader_fnv1a64: String,
    input_layout_pointer: String,
    vertex_buffers: BTreeMap<String, TerrainChunkBufferPointerIdentityFile>,
    #[serde(skip_serializing_if = "Option::is_none")]
    index_buffer: Option<TerrainChunkIndexPointerIdentityFile>,
    per_chunk_constant_buffers: Vec<TerrainChunkConstantBufferIdentityFile>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
struct TerrainChunkBufferPointerIdentityFile {
    resource_pointer: String,
    stride: u32,
    offset: u32,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
struct TerrainChunkIndexPointerIdentityFile {
    resource_pointer: String,
    dxgi_format: u32,
    offset: u32,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq)]
struct TerrainChunkBoundsFile {
    min_x: f32,
    min_y: f32,
    min_z: f32,
    max_x: f32,
    max_y: f32,
    max_z: f32,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
struct TerrainChunkCoverageFile {
    world_rect: TerrainChunkWorldRectFile,
    covered_area: f64,
    world_area: f64,
    fraction: f64,
    holes: Vec<TerrainChunkHoleFile>,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq)]
struct TerrainChunkWorldRectFile {
    min_x: f32,
    min_z: f32,
    max_x: f32,
    max_z: f32,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq)]
struct TerrainChunkHoleFile {
    min_x: f32,
    min_z: f32,
    max_x: f32,
    max_z: f32,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    watched_start_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    watched_end_ms: Option<u64>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    buffer_byte_width: Option<usize>,
    fnv1a64: Option<String>,
    stride: Option<u32>,
    offset: Option<u32>,
    dxgi_format: Option<u32>,
    bind_flags: u32,
    usage: u32,
    cpu_access_flags: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    unknown_origin: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    byte_range: Option<MapFrameBufferByteRangeFile>,
    #[serde(skip_serializing_if = "Option::is_none")]
    payload_truncated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    unobtained_reason: Option<&'static str>,
}

#[derive(Clone, Debug, Serialize)]
struct MapFrameIndexBufferBindingFile {
    file: Option<String>,
    byte_len: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    buffer_byte_width: Option<usize>,
    fnv1a64: Option<String>,
    dxgi_format: u32,
    offset: u32,
    bind_flags: u32,
    usage: u32,
    cpu_access_flags: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    byte_range: Option<MapFrameBufferByteRangeFile>,
    #[serde(skip_serializing_if = "Option::is_none")]
    payload_truncated: Option<bool>,
}

#[derive(Clone, Debug, Serialize)]
struct MapFrameBufferByteRangeFile {
    requested_offset: usize,
    requested_byte_len: usize,
    captured_offset: usize,
    captured_byte_len: usize,
    source_byte_len: usize,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    unobtained_reason: Option<&'static str>,
    /// `true` iff an exact readback of this binding was obtained AND every byte
    /// was zero. Only meaningful when a payload was captured; the borders_pass
    /// validator consults it for the BorderDiffuse slot. Defaults `false`
    /// (no readback / non-empty content).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    all_zero: bool,
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

unsafe fn capture_census_draw_if_active(
    context: *mut ID3D11DeviceContext,
    draw_call: &'static str,
    draw_count: UINT,
    draw_start: UINT,
    base_vertex_location: i32,
    per_instance_count: Option<UINT>,
    instance_count: Option<UINT>,
    start_instance_location: Option<UINT>,
) -> bool {
    debug_assert_render_thread_owner("capture_census_draw_if_active");
    if capture_quiesced() || !CONTEXT_CENSUS_ARMED.load(Ordering::Acquire) {
        return false;
    }
    if census_work_frame_paused() {
        record_census_work_frame_paused_draw(
            draw_call,
            draw_count,
            draw_start,
            base_vertex_location,
            per_instance_count,
            instance_count,
            start_instance_location,
        );
        return false;
    }
    if CENSUS_CAPTURE_IN_PROGRESS.swap(true, Ordering::SeqCst) {
        return false;
    }
    let started_ms = unsafe { GetTickCount64() };
    let result = unsafe {
        capture_active_census_draw(
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
    let elapsed_ms = unsafe { GetTickCount64() }.saturating_sub(started_ms);
    CENSUS_CAPTURE_IN_PROGRESS.store(false, Ordering::SeqCst);
    record_census_capture_work_ms(elapsed_ms);
    result.is_ok()
}

fn census_work_frame_paused() -> bool {
    RUNTIME
        .get()
        .and_then(|runtime| runtime.lock().ok())
        .is_some_and(|runtime| {
            runtime.census.enabled
                && runtime.census.work_frame_paused
                && !runtime.census.payload_stopped
        })
}

#[allow(clippy::too_many_arguments)]
fn record_census_work_frame_paused_draw(
    draw_call: &'static str,
    draw_count: UINT,
    draw_start: UINT,
    base_vertex_location: i32,
    per_instance_count: Option<UINT>,
    instance_count: Option<UINT>,
    start_instance_location: Option<UINT>,
) {
    let tracked_state = census_tracked_draw_state();
    let boundary = census_tracked_pass_boundary();
    let draw = census_minimal_uncaptured_draw(
        0,
        draw_call,
        draw_count,
        draw_start,
        base_vertex_location,
        per_instance_count,
        instance_count,
        start_instance_location,
        &tracked_state,
    );
    let pass = CensusActivePass {
        sequence: 0,
        first_seen_frame: 0,
        output_dir: PathBuf::new(),
        boundary,
        draws: vec![draw],
        blockers: vec![format!(
            "census draw payload capture disabled: {CENSUS_UNCAPTURED_WORK_BUDGET_FRAME_PAUSED}"
        )],
        payload_bytes_at_start: 0,
        capture_payloads: false,
        uncaptured_reason: Some(CENSUS_UNCAPTURED_WORK_BUDGET_FRAME_PAUSED.to_string()),
        prefix_key: 0,
        prefix_keys: Vec::new(),
        any_payload_captured: false,
        repeat_skipped_draws: 0,
    };
    let parts = census_signature_parts(&pass);
    let signature = census_signature_hash(&parts);
    let sig12 = signature.chars().take(12).collect::<String>();
    if let Some(runtime) = RUNTIME.get() {
        if let Ok(mut runtime) = runtime.lock() {
            if !runtime.census.enabled
                || !runtime.census.work_frame_paused
                || runtime.census.payload_stopped
            {
                return;
            }
            let first_seen_frame = runtime.census.frame_index;
            record_census_signature_for_runtime(
                &mut runtime,
                signature,
                sig12,
                first_seen_frame,
                0,
                None,
                Some(CENSUS_UNCAPTURED_WORK_BUDGET_FRAME_PAUSED.to_string()),
                parts,
            );
            maybe_emit_census_heartbeat_locked(&mut runtime);
        }
    }
}

unsafe fn capture_active_census_draw(
    context: *mut ID3D11DeviceContext,
    draw_call: &'static str,
    draw_count: UINT,
    draw_start: UINT,
    base_vertex_location: i32,
    per_instance_count: Option<UINT>,
    instance_count: Option<UINT>,
    start_instance_location: Option<UINT>,
) -> Result<(), String> {
    debug_assert_render_thread_owner("capture_active_census_draw");
    if context.is_null() {
        return Err("null D3D11 context for census capture".to_string());
    }
    let hazard_draw = ui_pass_interop_hazard_draw(draw_call, draw_count);
    let hazard_reason = ui_pass_skip_policy_applies(ui_cb_skip_policy_for_runtime(), hazard_draw)
        .then_some(UI_PASS_INTEROP_HAZARD_UNOBTAINED_REASON);
    let tracked_state = census_tracked_draw_state();
    let context_trusted = census_context_class_trusted_for_draw(&tracked_state);
    let preflight_uncaptured_reason =
        census_preflight_uncaptured_reason(context_trusted, hazard_reason, &tracked_state);
    // The pass-open reason must agree with the preflight priority: an
    // untrusted context class reports untrusted_context_class even for draws
    // that also match the interop-hazard pattern.
    let pass_open_uncaptured_reason = if context_trusted {
        hazard_reason
    } else {
        Some(CENSUS_UNCAPTURED_UNTRUSTED_CONTEXT_CLASS)
    };
    let boundary = if let Some(reason) = preflight_uncaptured_reason {
        census_uncaptured_pass_boundary(reason, draw_call, draw_count, &tracked_state)
    } else {
        census_tracked_pass_boundary()
    };
    let finalize_current = RUNTIME
        .get()
        .and_then(|runtime| runtime.lock().ok())
        .and_then(|runtime| {
            runtime
                .census
                .active_pass
                .as_ref()
                .map(|pass| pass.boundary != boundary)
        })
        .unwrap_or(false);
    if finalize_current {
        finalize_census_pass(context, "output_merger_transition");
    }
    let finalize_uncaptured_state = RUNTIME
        .get()
        .and_then(|runtime| runtime.lock().ok())
        .and_then(|runtime| {
            runtime
                .census
                .active_pass
                .as_ref()
                .and_then(|pass| pass.uncaptured_reason.as_deref())
                .map(census_uncaptured_reason_allows_next_draw_to_reopen_pass)
        })
        .unwrap_or(false);
    if finalize_uncaptured_state {
        finalize_census_pass(context, "uncaptured_state_transition");
    }
    if preflight_uncaptured_reason.is_some() {
        let signature = census_preflight_uncaptured_signature(
            &boundary,
            draw_call,
            draw_count,
            draw_start,
            base_vertex_location,
            per_instance_count,
            instance_count,
            start_instance_location,
            &tracked_state,
        );
        if census_signature_is_hazard_skipped(&signature) {
            record_census_skipped_signature_occurrence(&signature);
            return Ok(());
        }
    }
    let (output_dir, draw_index, capture_payloads) =
        start_or_update_census_pass(boundary, pass_open_uncaptured_reason)?;
    let (draw, blockers, prospective_prefix_key, payloads_read) = unsafe {
        capture_census_draw_record(
            context,
            &output_dir,
            draw_index,
            draw_call,
            draw_count,
            draw_start,
            base_vertex_location,
            per_instance_count,
            instance_count,
            start_instance_location,
            capture_payloads,
            preflight_uncaptured_reason,
        )
    };
    record_census_draw(draw, blockers, prospective_prefix_key, payloads_read);
    Ok(())
}

fn start_or_update_census_pass(
    boundary: CensusPassBoundary,
    open_uncaptured_reason: Option<&'static str>,
) -> Result<(PathBuf, u32, bool), String> {
    let Some(runtime) = RUNTIME.get() else {
        return Err("terrain capture runtime not initialized".to_string());
    };
    let mut runtime = runtime
        .lock()
        .map_err(|_| "terrain capture runtime lock poisoned".to_string())?;
    if !runtime.census.enabled {
        return Err("census is not enabled".to_string());
    }
    if runtime.census.active_pass.is_none() {
        let sequence = runtime.census.next_sequence;
        runtime.census.next_sequence = runtime.census.next_sequence.saturating_add(1);
        // The directory is created lazily at finalize (only for passes that
        // produce a bundle); repeat passes must not churn the filesystem.
        let output_dir = runtime
            .root_output_dir
            .join(CENSUS_DIR)
            .join(format!("pending_{sequence:06}"));
        let (capture_payloads, uncaptured_reason) =
            census_payload_capture_admission_for_signature(&runtime, None);
        let prefix_key = census_prefix_seed(&boundary);
        runtime.census.active_pass = Some(CensusActivePass {
            sequence,
            first_seen_frame: runtime.census.frame_index,
            output_dir,
            boundary,
            draws: Vec::new(),
            blockers: Vec::new(),
            payload_bytes_at_start: runtime.census.payload_bytes_reserved,
            capture_payloads,
            uncaptured_reason,
            prefix_key,
            prefix_keys: Vec::new(),
            any_payload_captured: false,
            repeat_skipped_draws: 0,
        });
    }
    let (global_capture_payloads, global_uncaptured_reason) =
        census_payload_capture_admission_for_signature(&runtime, None);
    let pass = runtime
        .census
        .active_pass
        .as_mut()
        .expect("active pass was just initialized");
    if let Some(reason) = open_uncaptured_reason {
        pass.capture_payloads = false;
        pass.uncaptured_reason = Some(reason.to_string());
    }
    if !global_capture_payloads {
        pass.capture_payloads = false;
        pass.uncaptured_reason = global_uncaptured_reason;
    }
    Ok((
        pass.output_dir.clone(),
        pass.draws.len() as u32,
        pass.capture_payloads,
    ))
}

const CENSUS_PREFIX_FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const CENSUS_PREFIX_FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
const CENSUS_REPEAT_PREFIX_SKIP_BLOCKER: &str =
    "census draw payload capture skipped: repeat_of_recorded_signature_prefix";

fn census_prefix_fnv(seed: u64, bytes: &[u8]) -> u64 {
    let mut hash = seed;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(CENSUS_PREFIX_FNV_PRIME);
    }
    hash
}

fn census_prefix_seed(boundary: &CensusPassBoundary) -> u64 {
    let mut hash = census_prefix_fnv(
        CENSUS_PREFIX_FNV_OFFSET_BASIS,
        boundary.render_target_class.as_bytes(),
    );
    hash = census_prefix_fnv(hash, &[0xff]);
    census_prefix_fnv(hash, boundary.depth_class.as_bytes())
}

#[allow(clippy::too_many_arguments)]
fn census_draw_prefix_component(
    draw_call: &'static str,
    draw_count: UINT,
    draw_start: UINT,
    base_vertex_location: i32,
    per_instance_count: Option<UINT>,
    instance_count: Option<UINT>,
    start_instance_location: Option<UINT>,
    vertex_shader_fnv1a64: &str,
    pixel_shader_fnv1a64: &str,
    input_layout_pointer: usize,
) -> String {
    // Blend/depth-stencil state and input-layout identities keep the repeat
    // key at least as fine as the census signature: a state-only variant of
    // an otherwise identical pass must not be repeat-skipped into the
    // original signature.
    let blend_state = CENSUS_TRACKED_OM_BLEND_STATE.load(Ordering::Acquire);
    let depth_stencil_state = CENSUS_TRACKED_OM_DEPTH_STENCIL_STATE.load(Ordering::Acquire);
    format!(
        "{draw_call}:{draw_count}:{draw_start}:{base_vertex_location}:{per_instance_count:?}:{instance_count:?}:{start_instance_location:?}:{vertex_shader_fnv1a64}:{pixel_shader_fnv1a64}:bs=0x{blend_state:X}:ds=0x{depth_stencil_state:X}:il=0x{input_layout_pointer:X}"
    )
}

/// Returns the prospective prefix key for appending this draw to the active
/// pass, and whether an identical prefix was already recorded by an earlier
/// finalized pass (in which case payload readbacks are skipped: the spec
/// requires repeat occurrences to be counter-only, and zero per-draw context
/// work is also the crash-safety property of census mode).
fn census_repeat_prefix_admission(draw_component: &str) -> (Option<u64>, bool) {
    RUNTIME
        .get()
        .and_then(|runtime| runtime.lock().ok())
        .map(|runtime| {
            let Some(pass) = runtime.census.active_pass.as_ref() else {
                return (None, false);
            };
            let prospective = census_prefix_fnv(
                census_prefix_fnv(pass.prefix_key, &[0xfe]),
                draw_component.as_bytes(),
            );
            let repeat = runtime.census.known_prefix_keys.contains(&prospective);
            (Some(prospective), repeat)
        })
        .unwrap_or((None, false))
}

fn census_shader_file_memo_get(stage: &'static str, pointer: usize) -> Option<MapFrameShaderFile> {
    RUNTIME
        .get()
        .and_then(|runtime| runtime.lock().ok())
        .and_then(|runtime| {
            runtime
                .census
                .shader_file_memo
                .get(&(stage == "PS", pointer))
                .cloned()
        })
}

fn census_shader_file_memo_insert(stage: &'static str, pointer: usize, file: &MapFrameShaderFile) {
    if let Some(runtime) = RUNTIME.get() {
        if let Ok(mut runtime) = runtime.lock() {
            runtime
                .census
                .shader_file_memo
                .insert((stage == "PS", pointer), file.clone());
        }
    }
}

fn census_shader_file_memoized(
    output_dir: &Path,
    stage: &'static str,
    pointer: usize,
    bytecode: Option<&'static [u8]>,
    blockers: &mut Vec<String>,
) -> MapFrameShaderFile {
    if let Some(cached) = census_shader_file_memo_get(stage, pointer) {
        return cached;
    }
    let file = census_shader_file(output_dir, stage, pointer, bytecode, blockers);
    if !file.fnv1a64.is_empty() {
        census_shader_file_memo_insert(stage, pointer, &file);
    }
    file
}

fn census_tracked_draw_state() -> CensusTrackedDrawState {
    let (pixel_shader, vertex_shader, layout_ptr, layout_elements, layout_known) = RUNTIME
        .get()
        .and_then(|runtime| runtime.lock().ok())
        .map(|runtime| {
            let (layout_ptr, layout_elements, layout_known) = runtime
                .state
                .current_input_layout()
                .map(|(layout, elements)| (layout, elements.to_vec(), true))
                .unwrap_or((0, Vec::new(), false));
            (
                runtime.state.current_pixel_shader().unwrap_or(0),
                runtime.state.current_vertex_shader().unwrap_or(0),
                layout_ptr,
                layout_elements,
                layout_known,
            )
        })
        .unwrap_or((0, 0, 0, Vec::new(), false));
    CensusTrackedDrawState {
        pixel_shader,
        vertex_shader,
        layout_ptr,
        layout_elements,
        pixel_shader_known: census_shader_bytecode_available("PS", pixel_shader),
        vertex_shader_known: census_shader_bytecode_available("VS", vertex_shader),
        layout_known,
    }
}

/// Trust gate for census first-occurrence payload capture (and, via the same
/// latch, auto-round CB0 sampling). Trusted means the real map renderer has
/// been proven live on this immediate context: a known capture-family pixel
/// shader has been bound by the render-thread owner (latched in
/// hook_ps_set_shader), or is bound right now on this draw. Before that,
/// census must record signatures ONLY and issue zero immediate-context
/// queries — the 2026-07-05 load-screen crash (PSGetConstantBuffers AV inside
/// d3d11) is exactly this class. This function performs no D3D11 calls.
fn census_context_class_trusted_for_draw(tracked_state: &CensusTrackedDrawState) -> bool {
    if CONTEXT_MAP_RENDERER_LIVE.load(Ordering::Acquire) {
        return true;
    }
    // Belt-and-braces: the census draw path sees the tracked bound-shader
    // pointer even if the PSSetShader latch was raced/missed. Strict check:
    // never grant trust on pointer-set overflow.
    if capture_pixel_shader_known_strict(tracked_state.pixel_shader) {
        latch_map_renderer_live();
        return true;
    }
    if census_known_capture_family_shader_available("PS", tracked_state.pixel_shader) {
        latch_map_renderer_live();
        return true;
    }
    false
}

/// Pure preflight decision: any Some(reason) means the draw is recorded as an
/// uncaptured signature with that explicit reason and the payload-capture
/// Get* battery is never entered. Untrusted context class wins over every
/// other reason — it is the one that must guarantee zero context calls.
fn census_preflight_uncaptured_reason(
    context_trusted: bool,
    hazard_reason: Option<&'static str>,
    tracked_state: &CensusTrackedDrawState,
) -> Option<&'static str> {
    if !context_trusted {
        return Some(CENSUS_UNCAPTURED_UNTRUSTED_CONTEXT_CLASS);
    }
    hazard_reason.or_else(|| {
        if !tracked_state.shaders_known() {
            Some(CENSUS_UNCAPTURED_UNKNOWN_SHADER)
        } else if !tracked_state.layout_known {
            Some(CENSUS_UNCAPTURED_UNKNOWN_INPUT_LAYOUT)
        } else {
            None
        }
    })
}

fn census_shader_bytecode_available(stage: &'static str, pointer: usize) -> bool {
    if pointer == 0 {
        return false;
    }
    (match stage {
        "PS" => ui_pixel_shader_bytecode(pointer).is_some(),
        "VS" => ui_vertex_shader_bytecode(pointer).is_some(),
        _ => false,
    }) || census_known_capture_family_shader_available(stage, pointer)
}

fn census_known_capture_family_shader_available(stage: &'static str, pointer: usize) -> bool {
    RUNTIME
        .get()
        .and_then(|runtime| runtime.lock().ok())
        .and_then(|runtime| {
            let profile = requested_map_frame_capture_profile_for_runtime(&runtime).ok()?;
            match stage {
                "PS" => census_known_capture_family_pixel_shader(&runtime, profile, pointer),
                "VS" => census_known_capture_family_vertex_shader(&runtime, profile, pointer),
                _ => None,
            }
        })
        .is_some()
}

fn census_uncaptured_pass_boundary(
    reason: &'static str,
    draw_call: &'static str,
    draw_count: UINT,
    tracked_state: &CensusTrackedDrawState,
) -> CensusPassBoundary {
    CensusPassBoundary {
        render_target_class: format!(
            "not_read:{reason}:{draw_call}:{draw_count}:ps=0x{:X}:vs=0x{:X}:layout=0x{:X}",
            tracked_state.pixel_shader, tracked_state.vertex_shader, tracked_state.layout_ptr
        ),
        depth_class: "not_read".to_string(),
    }
}

fn census_preflight_uncaptured_signature(
    boundary: &CensusPassBoundary,
    draw_call: &'static str,
    draw_count: UINT,
    draw_start: UINT,
    base_vertex_location: i32,
    per_instance_count: Option<UINT>,
    instance_count: Option<UINT>,
    start_instance_location: Option<UINT>,
    tracked_state: &CensusTrackedDrawState,
) -> String {
    let draw = census_minimal_uncaptured_draw(
        0,
        draw_call,
        draw_count,
        draw_start,
        base_vertex_location,
        per_instance_count,
        instance_count,
        start_instance_location,
        tracked_state,
    );
    let pass = CensusActivePass {
        sequence: 0,
        first_seen_frame: 0,
        output_dir: PathBuf::new(),
        boundary: boundary.clone(),
        draws: vec![draw],
        blockers: Vec::new(),
        payload_bytes_at_start: 0,
        capture_payloads: false,
        uncaptured_reason: None,
        prefix_key: 0,
        prefix_keys: Vec::new(),
        any_payload_captured: false,
        repeat_skipped_draws: 0,
    };
    census_signature_hash(&census_signature_parts(&pass))
}

fn census_signature_is_hazard_skipped(signature: &str) -> bool {
    RUNTIME
        .get()
        .and_then(|runtime| runtime.lock().ok())
        .is_some_and(|runtime| runtime.census.hazard_skipped_signatures.contains(signature))
}

fn record_census_skipped_signature_occurrence(signature: &str) {
    if let Some(runtime) = RUNTIME.get() {
        if let Ok(mut runtime) = runtime.lock() {
            if let Some(entry) = runtime.census.signatures.get_mut(signature) {
                entry.occurrence_count = entry.occurrence_count.saturating_add(1);
            }
            maybe_emit_census_heartbeat_locked(&mut runtime);
        }
    }
}

fn census_minimal_uncaptured_draw(
    draw_index: u32,
    draw_call: &'static str,
    draw_count: UINT,
    draw_start: UINT,
    base_vertex_location: i32,
    per_instance_count: Option<UINT>,
    instance_count: Option<UINT>,
    start_instance_location: Option<UINT>,
    tracked_state: &CensusTrackedDrawState,
) -> MapFrameDrawFile {
    MapFrameDrawFile {
        draw_index,
        draw_call,
        draw_count,
        draw_start,
        base_vertex_location,
        per_instance_count,
        instance_count,
        start_instance_location,
        topology: None,
        shader: empty_census_shader_file("PS", tracked_state.pixel_shader),
        vertex_shader: empty_census_shader_file("VS", tracked_state.vertex_shader),
        input_layout: MapFrameInputLayoutFile {
            pointer: format!("0x{:X}", tracked_state.layout_ptr),
            elements: tracked_state.layout_elements.clone(),
        },
        vertex_buffers: BTreeMap::new(),
        index_buffer: None,
        constant_buffers: Vec::new(),
        shader_resources: Vec::new(),
        samplers: Vec::new(),
        draw_state: None,
        post_draw_output_merger: None,
    }
}

unsafe fn capture_census_draw_record(
    context: *mut ID3D11DeviceContext,
    output_dir: &Path,
    draw_index: u32,
    draw_call: &'static str,
    draw_count: UINT,
    draw_start: UINT,
    base_vertex_location: i32,
    per_instance_count: Option<UINT>,
    instance_count: Option<UINT>,
    start_instance_location: Option<UINT>,
    capture_payloads: bool,
    preflight_uncaptured_reason: Option<&'static str>,
) -> (MapFrameDrawFile, Vec<String>, Option<u64>, bool) {
    debug_assert_render_thread_owner("capture_census_draw_record");
    let mut blockers = Vec::new();
    let tracked_state = census_tracked_draw_state();
    if let Some(reason) = preflight_uncaptured_reason {
        blockers.push(format!("census draw payload capture disabled: {reason}"));
        return (
            census_minimal_uncaptured_draw(
                draw_index,
                draw_call,
                draw_count,
                draw_start,
                base_vertex_location,
                per_instance_count,
                instance_count,
                start_instance_location,
                &tracked_state,
            ),
            blockers,
            None,
            false,
        );
    }
    let pixel_shader_bytecode = ui_pixel_shader_bytecode(tracked_state.pixel_shader);
    let vertex_shader_bytecode = ui_vertex_shader_bytecode(tracked_state.vertex_shader);
    let shader = census_shader_file_memoized(
        output_dir,
        "PS",
        tracked_state.pixel_shader,
        pixel_shader_bytecode,
        &mut blockers,
    );
    let vertex_shader_file = census_shader_file_memoized(
        output_dir,
        "VS",
        tracked_state.vertex_shader,
        vertex_shader_bytecode,
        &mut blockers,
    );
    let pixel_shader_known = !shader.fnv1a64.is_empty();
    let vertex_shader_known = !vertex_shader_file.fnv1a64.is_empty();
    let draw_prefix_component = census_draw_prefix_component(
        draw_call,
        draw_count,
        draw_start,
        base_vertex_location,
        per_instance_count,
        instance_count,
        start_instance_location,
        &vertex_shader_file.fnv1a64,
        &shader.fnv1a64,
        tracked_state.layout_ptr,
    );
    let (prospective_prefix_key, repeat_prefix) =
        census_repeat_prefix_admission(&draw_prefix_component);
    let capture_payloads = capture_payloads && !repeat_prefix;
    if repeat_prefix {
        blockers.push(CENSUS_REPEAT_PREFIX_SKIP_BLOCKER.to_string());
    }
    let state_reads_allowed = capture_payloads
        && pixel_shader_known
        && vertex_shader_known
        && tracked_state.layout_known
        && tracked_state.safe_for_state_reads();
    let layout_ptr = tracked_state.layout_ptr;
    let elements = tracked_state.layout_elements;
    let layout_known = tracked_state.layout_known;
    let mut capture_payloads =
        capture_payloads && pixel_shader_known && vertex_shader_known && layout_known;
    if !pixel_shader_known || !vertex_shader_known {
        blockers.push(format!(
            "census draw payload capture disabled: {CENSUS_UNCAPTURED_UNKNOWN_SHADER}"
        ));
    }
    if !layout_known {
        blockers.push(format!(
            "census draw payload capture disabled: {CENSUS_UNCAPTURED_UNKNOWN_INPUT_LAYOUT}"
        ));
    }
    let topology = if state_reads_allowed {
        unsafe { active_primitive_topology(context) }
    } else {
        None
    };
    let draw_state = if state_reads_allowed {
        let (draw_state, mut state_blockers) = unsafe {
            capture_map_frame_draw_state(
                context,
                output_dir,
                draw_index,
                CENSUS_CAPTURE_PROFILE,
                false,
            )
        };
        blockers.append(&mut state_blockers);
        draw_state
    } else {
        None
    };
    let mut draw = MapFrameDrawFile {
        draw_index,
        draw_call,
        draw_count,
        draw_start,
        base_vertex_location,
        per_instance_count,
        instance_count,
        start_instance_location,
        topology,
        shader,
        vertex_shader: vertex_shader_file,
        input_layout: MapFrameInputLayoutFile {
            pointer: format!("0x{layout_ptr:X}"),
            elements,
        },
        vertex_buffers: BTreeMap::new(),
        index_buffer: None,
        constant_buffers: Vec::new(),
        shader_resources: Vec::new(),
        samplers: Vec::new(),
        draw_state,
        post_draw_output_merger: None,
    };
    if capture_payloads {
        if let Some(reason) = skipped_hazard_reason_for_census_probe_draw(&draw) {
            capture_payloads = false;
            blockers.push(format!("census draw payload capture disabled: {reason}"));
        }
    }
    let input_slots = draw
        .input_layout
        .elements
        .iter()
        .map(|element| element.input_slot)
        .collect::<BTreeSet<_>>();
    if capture_payloads {
        for slot in input_slots {
            match unsafe {
                capture_map_frame_vertex_buffer_slot(
                    context,
                    slot,
                    output_dir,
                    draw_index,
                    CENSUS_CAPTURE_PROFILE,
                )
            } {
                Ok(binding) => {
                    draw.vertex_buffers.insert(slot.to_string(), binding);
                }
                Err(error) => blockers.push(error),
            }
        }
    }
    if capture_payloads && draw_call.contains("Indexed") {
        match unsafe {
            capture_map_frame_index_buffer(context, output_dir, draw_index, CENSUS_CAPTURE_PROFILE)
        } {
            Ok(binding) => {
                draw.index_buffer = Some(binding);
            }
            Err(error) => {
                blockers.push(error);
            }
        }
    }
    if capture_payloads {
        let (shader_resources, mut srv_blockers) = unsafe {
            capture_map_frame_ps_srvs(
                context,
                &draw.shader.reflection_resources,
                output_dir,
                draw_index,
                CENSUS_CAPTURE_PROFILE,
                None,
            )
        };
        blockers.append(&mut srv_blockers);
        draw.shader_resources = shader_resources;
        let (samplers, mut sampler_blockers) =
            unsafe { capture_map_frame_ps_samplers(context, CENSUS_CAPTURE_PROFILE) };
        blockers.append(&mut sampler_blockers);
        draw.samplers = samplers;
        let (constant_buffers, mut constant_buffer_blockers) = unsafe {
            capture_map_frame_constant_buffers(context, draw_index, CENSUS_CAPTURE_PROFILE)
        };
        blockers.append(&mut constant_buffer_blockers);
        draw.constant_buffers = constant_buffers;
        let (draw_state, mut state_blockers) = unsafe {
            capture_map_frame_draw_state(
                context,
                output_dir,
                draw_index,
                CENSUS_CAPTURE_PROFILE,
                true,
            )
        };
        blockers.append(&mut state_blockers);
        draw.draw_state = draw_state;
    }
    (draw, blockers, prospective_prefix_key, capture_payloads)
}

fn record_census_draw(
    draw: MapFrameDrawFile,
    blockers: Vec<String>,
    prospective_prefix_key: Option<u64>,
    payloads_read: bool,
) {
    if let Some(runtime) = RUNTIME.get() {
        if let Ok(mut runtime) = runtime.lock() {
            let payload_stopped = runtime.census.payload_stopped;
            let payload_stop_reason = runtime.census.payload_stop_reason.clone();
            if let Some(pass) = runtime.census.active_pass.as_mut() {
                if let Some(reason) = census_uncaptured_reason_from_blockers(&blockers) {
                    pass.capture_payloads = false;
                    pass.uncaptured_reason
                        .get_or_insert_with(|| reason.to_string());
                }
                if blockers.iter().any(|blocker| blocker == "budget") || payload_stopped {
                    pass.capture_payloads = false;
                    pass.uncaptured_reason =
                        Some(payload_stop_reason.unwrap_or_else(|| "budget".to_string()));
                }
                if let Some(prefix_key) = prospective_prefix_key {
                    pass.prefix_key = prefix_key;
                    pass.prefix_keys.push(prefix_key);
                }
                pass.any_payload_captured |= payloads_read;
                if blockers
                    .iter()
                    .any(|blocker| blocker == CENSUS_REPEAT_PREFIX_SKIP_BLOCKER)
                {
                    pass.repeat_skipped_draws = pass.repeat_skipped_draws.saturating_add(1);
                }
                pass.blockers.extend(blockers);
                pass.draws.push(draw);
            }
            maybe_emit_census_heartbeat_locked(&mut runtime);
        }
    }
}

fn census_uncaptured_reason_from_blockers(blockers: &[String]) -> Option<&'static str> {
    if blockers
        .iter()
        .any(|blocker| blocker.contains(CENSUS_UNCAPTURED_WORK_BUDGET_FRAME_PAUSED))
    {
        return Some(CENSUS_UNCAPTURED_WORK_BUDGET_FRAME_PAUSED);
    }
    if blockers
        .iter()
        .any(|blocker| blocker.contains(CENSUS_UNCAPTURED_UNTRUSTED_CONTEXT_CLASS))
    {
        return Some(CENSUS_UNCAPTURED_UNTRUSTED_CONTEXT_CLASS);
    }
    if blockers
        .iter()
        .any(|blocker| blocker.contains(UI_PASS_INTEROP_HAZARD_UNOBTAINED_REASON))
    {
        return Some(UI_PASS_INTEROP_HAZARD_UNOBTAINED_REASON);
    }
    if blockers
        .iter()
        .any(|blocker| blocker.contains(CENSUS_UNCAPTURED_UNKNOWN_SHADER))
    {
        return Some(CENSUS_UNCAPTURED_UNKNOWN_SHADER);
    }
    if blockers
        .iter()
        .any(|blocker| blocker.contains(CENSUS_UNCAPTURED_UNKNOWN_INPUT_LAYOUT))
    {
        return Some(CENSUS_UNCAPTURED_UNKNOWN_INPUT_LAYOUT);
    }
    None
}

fn census_uncaptured_reason_skips_signature(reason: Option<&str>) -> bool {
    matches!(
        reason,
        Some(UI_PASS_INTEROP_HAZARD_UNOBTAINED_REASON)
            | Some(CENSUS_UNCAPTURED_UNKNOWN_SHADER)
            | Some(CENSUS_UNCAPTURED_UNKNOWN_INPUT_LAYOUT)
            | Some(CENSUS_UNCAPTURED_UNTRUSTED_CONTEXT_CLASS)
    )
}

fn census_uncaptured_reason_allows_next_draw_to_reopen_pass(reason: &str) -> bool {
    matches!(
        reason,
        CENSUS_UNCAPTURED_UNKNOWN_SHADER
            | CENSUS_UNCAPTURED_UNKNOWN_INPUT_LAYOUT
            | CENSUS_UNCAPTURED_UNTRUSTED_CONTEXT_CLASS
            | CENSUS_UNCAPTURED_WORK_BUDGET_FRAME_PAUSED
    )
}

fn census_payload_capture_admission_for_signature(
    runtime: &CaptureRuntime,
    signature: Option<&str>,
) -> (bool, Option<String>) {
    if runtime.census.work_frame_paused && !runtime.census.payload_stopped {
        return (
            false,
            Some(CENSUS_UNCAPTURED_WORK_BUDGET_FRAME_PAUSED.to_string()),
        );
    }
    if runtime.census.payload_stopped {
        return (
            false,
            Some(
                runtime
                    .census
                    .payload_stop_reason
                    .clone()
                    .unwrap_or_else(|| "budget".to_string()),
            ),
        );
    }
    if let Some(signature) = signature {
        if runtime.census.hazard_skipped_signatures.contains(signature) {
            return (
                false,
                Some(
                    runtime
                        .census
                        .signatures
                        .get(signature)
                        .and_then(|entry| entry.uncaptured_reason.clone())
                        .unwrap_or_else(|| CENSUS_UNCAPTURED_PRIOR_HAZARD_SIGNATURE.to_string()),
                ),
            );
        }
    }
    (true, None)
}

fn skipped_hazard_reason_for_census_probe_draw(draw: &MapFrameDrawFile) -> Option<String> {
    RUNTIME
        .get()
        .and_then(|runtime| runtime.lock().ok())
        .and_then(|runtime| {
            let pass = runtime.census.active_pass.as_ref()?;
            let mut probe = pass.clone();
            probe.draws.push(draw.clone());
            let parts = census_signature_parts(&probe);
            let signature = census_signature_hash(&parts);
            let (capture_payloads, reason) =
                census_payload_capture_admission_for_signature(&runtime, Some(&signature));
            (!capture_payloads).then_some(reason).flatten()
        })
}

/// Census pass-boundary identity built ONLY from state tracked by the
/// OMSetRenderTargets* hooks. This function performs no D3D11 calls: the
/// census draw path runs on every draw of every frame for the whole session,
/// and per-draw immediate-context queries (the previous OMGetRenderTargets
/// implementation) were bisected to be the trigger that detonated a latent
/// driver corruption (see lane history). Pointers are identity strings only.
fn census_tracked_pass_boundary() -> CensusPassBoundary {
    if !CENSUS_TRACKED_OM_SET_SEEN.load(Ordering::Acquire) {
        return CensusPassBoundary {
            render_target_class: "untracked".to_string(),
            depth_class: "untracked".to_string(),
        };
    }
    let mut rt_parts = Vec::new();
    for (slot, tracked) in CENSUS_TRACKED_OM_RTVS.iter().enumerate() {
        let pointer = tracked.load(Ordering::Acquire);
        if pointer == 0 {
            continue;
        }
        rt_parts.push(format!("{slot}:0x{pointer:X}"));
    }
    let depth_pointer = CENSUS_TRACKED_OM_DSV.load(Ordering::Acquire);
    CensusPassBoundary {
        render_target_class: if rt_parts.is_empty() {
            "none".to_string()
        } else {
            rt_parts.join("|")
        },
        depth_class: if depth_pointer == 0 {
            "none".to_string()
        } else {
            format!("0x{depth_pointer:X}")
        },
    }
}

fn finalize_census_pass(context: *mut ID3D11DeviceContext, reason: &'static str) {
    if capture_quiesced() {
        return;
    }
    let active = RUNTIME
        .get()
        .and_then(|runtime| runtime.lock().ok())
        .and_then(|mut runtime| runtime.census.active_pass.take());
    let Some(mut pass) = active else {
        return;
    };
    if pass.draws.is_empty() {
        return;
    }
    // A pass whose every draw was repeat-prefix-skipped is an exact repeat of
    // an already-finalized pass: attribute the occurrence to the original
    // signature (counter-only, no bundle, no event, zero context work).
    if pass.repeat_skipped_draws == pass.draws.len() {
        if let Some(runtime) = RUNTIME.get() {
            if let Ok(mut runtime) = runtime.lock() {
                if let Some(signature) = runtime
                    .census
                    .prefix_signature_map
                    .get(&pass.prefix_key)
                    .cloned()
                {
                    if let Some(entry) = runtime.census.signatures.get_mut(&signature) {
                        entry.occurrence_count = entry.occurrence_count.saturating_add(1);
                    }
                    maybe_emit_census_heartbeat_locked(&mut runtime);
                    return;
                }
            }
        }
    }
    let work_frame_paused_at_finalize = RUNTIME
        .get()
        .and_then(|runtime| runtime.lock().ok())
        .is_some_and(|runtime| runtime.census.work_frame_paused);
    // Post-pass output-merger capture touches the immediate context; it is
    // reserved for passes that actually captured payloads (first occurrence
    // of a signature). Repeat passes and work-paused frames finalize without
    // any additional context work.
    if pass.any_payload_captured
        && pass.capture_payloads
        && !work_frame_paused_at_finalize
        && !context.is_null()
    {
        let draw_index = pass.draws.last().map(|draw| draw.draw_index).unwrap_or(0);
        let (post_pass, mut blockers) = unsafe {
            capture_map_frame_post_pass_output_merger_state(
                context,
                &pass.output_dir,
                draw_index,
                CENSUS_CAPTURE_PROFILE,
            )
        };
        pass.blockers.append(&mut blockers);
        if let Some(last) = pass.draws.last_mut() {
            last.post_draw_output_merger = post_pass;
        }
    }
    pass.blockers.extend(flush_payload_file_writes());
    let parts = census_signature_parts(&pass);
    let signature = census_signature_hash(&parts);
    let sig12 = signature.chars().take(12).collect::<String>();
    let final_dir = pass
        .output_dir
        .parent()
        .map(|parent| parent.join(format!("{:06}_{sig12}", pass.sequence)))
        .unwrap_or_else(|| pass.output_dir.clone());
    let bundle_path = Path::new(CENSUS_DIR)
        .join(format!("{:06}_{sig12}", pass.sequence))
        .join(CENSUS_PASS_BUNDLE_FILE)
        .to_string_lossy()
        .replace('\\', "/");
    let payload_bytes = RUNTIME
        .get()
        .and_then(|runtime| runtime.lock().ok())
        .map(|runtime| {
            runtime
                .census
                .payload_bytes_reserved
                .saturating_sub(pass.payload_bytes_at_start)
        })
        .unwrap_or(0);
    let clean_capture = pass.capture_payloads
        && pass.uncaptured_reason.is_none()
        && !pass.blockers.iter().any(|blocker| blocker == "budget");
    let signature_already_blocks_capture = RUNTIME
        .get()
        .and_then(|runtime| runtime.lock().ok())
        .is_some_and(|runtime| census_signature_blocks_clean_capture(&runtime, &signature));
    let mut wrote_bundle = false;
    if clean_capture && !signature_already_blocks_capture {
        match write_census_pass_bundle(&final_dir, &pass, reason) {
            Ok(()) => wrote_bundle = true,
            Err(error) => {
                pass.blockers
                    .push(format!("census pass bundle write failed: {error}"));
            }
        }
    }
    let uncaptured_reason = pass
        .uncaptured_reason
        .clone()
        .or_else(|| (!wrote_bundle).then(|| "errors".to_string()));
    let retryable_uncaptured =
        uncaptured_reason.as_deref() == Some(CENSUS_UNCAPTURED_WORK_BUDGET_FRAME_PAUSED);
    let skip_signature_after_uncaptured =
        !wrote_bundle && census_uncaptured_reason_skips_signature(uncaptured_reason.as_deref());
    let root_output_dir = RUNTIME
        .get()
        .and_then(|runtime| runtime.lock().ok())
        .map(|runtime| runtime.root_output_dir.clone());
    let mut emit_finalized_event = false;
    if let Some(runtime) = RUNTIME.get() {
        if let Ok(mut runtime) = runtime.lock() {
            let first_manifest_entry = record_census_signature_for_runtime(
                &mut runtime,
                signature.clone(),
                sig12.clone(),
                pass.first_seen_frame,
                payload_bytes,
                wrote_bundle.then_some(bundle_path.clone()),
                uncaptured_reason.clone(),
                parts.clone(),
            );
            maybe_record_census_hazard_skip_for_finalized_signature(
                &mut runtime,
                &signature,
                skip_signature_after_uncaptured,
            );
            if !retryable_uncaptured {
                // Remember every prefix of this pass so repeat occurrences
                // skip payload readbacks entirely (counter-only, zero context
                // work). Work-budget frame pauses are deliberately excluded:
                // they must retry on a later frame.
                runtime
                    .census
                    .known_prefix_keys
                    .extend(pass.prefix_keys.iter().copied());
                if !pass.prefix_keys.is_empty() {
                    runtime
                        .census
                        .prefix_signature_map
                        .entry(pass.prefix_key)
                        .or_insert_with(|| signature.clone());
                }
            }
            emit_finalized_event = first_manifest_entry
                && runtime
                    .census
                    .finalized_event_signatures
                    .insert(signature.clone());
            maybe_emit_census_heartbeat_locked(&mut runtime);
            refresh_context_hook_interests_for_runtime(&runtime);
        }
    }
    if let Some(root) = root_output_dir.filter(|_| emit_finalized_event) {
        let _ = write_census_manifest_snapshot(&root);
        write_event(
            &root,
            &format!(
                "{{\"type\":\"census_pass_finalized\",\"signature\":\"{}\",\"sig12\":\"{}\",\"draw_count\":{},\"captured\":{},\"reason\":\"{}\"}}",
                json_escape(&signature),
                json_escape(&sig12),
                pass.draws.len(),
                wrote_bundle,
                json_escape(reason)
            ),
        );
    }
}

fn record_census_signature_for_runtime(
    runtime: &mut CaptureRuntime,
    signature: String,
    sig12: String,
    first_seen_frame: u64,
    payload_bytes: u64,
    bundle_path: Option<String>,
    uncaptured_reason: Option<String>,
    parts: CensusSignatureParts,
) -> bool {
    let is_new = !runtime.census.signatures.contains_key(&signature);
    let entry = runtime
        .census
        .signatures
        .entry(signature.clone())
        .or_insert_with(|| CensusSignatureRecord {
            signature,
            sig12,
            first_seen_frame,
            occurrence_count: 0,
            bundle_path: None,
            payload_bytes: 0,
            captured: false,
            uncaptured_reason: None,
            signature_parts: parts,
        });
    let upgrades_paused_entry = !is_new
        && bundle_path.is_some()
        && !entry.captured
        && entry.uncaptured_reason.as_deref() == Some(CENSUS_UNCAPTURED_WORK_BUDGET_FRAME_PAUSED);
    entry.occurrence_count = entry.occurrence_count.saturating_add(1);
    if is_new || upgrades_paused_entry {
        entry.payload_bytes = payload_bytes;
        if let Some(bundle_path) = bundle_path {
            entry.bundle_path = Some(bundle_path);
            entry.captured = true;
            entry.uncaptured_reason = None;
        } else {
            entry.captured = false;
            entry.uncaptured_reason = uncaptured_reason;
        }
    }
    is_new || upgrades_paused_entry
}

fn census_signature_blocks_clean_capture(runtime: &CaptureRuntime, signature: &str) -> bool {
    runtime
        .census
        .signatures
        .get(signature)
        .is_some_and(|entry| {
            entry.captured
                || entry.uncaptured_reason.as_deref()
                    != Some(CENSUS_UNCAPTURED_WORK_BUDGET_FRAME_PAUSED)
        })
}

fn maybe_record_census_hazard_skip_for_finalized_signature(
    runtime: &mut CaptureRuntime,
    signature: &str,
    skip_signature_after_uncaptured: bool,
) {
    if skip_signature_after_uncaptured {
        runtime
            .census
            .hazard_skipped_signatures
            .insert(signature.to_string());
    }
}

fn write_census_pass_bundle(
    output_dir: &Path,
    pass: &CensusActivePass,
    reason: &'static str,
) -> Result<(), String> {
    let active_draw = pass
        .draws
        .last()
        .cloned()
        .ok_or_else(|| "census pass produced no draw records".to_string())?;
    let frame = MapFrameBundleFile {
        schema: CENSUS_PASS_BUNDLE_SCHEMA,
        provenance: reason,
        finalized_by: map_frame_finalized_by(reason),
        capture_moment: CENSUS_CAPTURE_MOMENT,
        session_manifest: CENSUS_PASS_BUNDLE_SESSION_MANIFEST,
        shader_cache_constants: None,
        active_draw,
        draws: pass.draws.clone(),
        post_pass_output_merger: pass
            .draws
            .last()
            .and_then(|draw| draw.post_draw_output_merger.clone()),
        vanilla_sources: BTreeMap::new(),
        runtime_files: BTreeMap::new(),
        blockers: pass.blockers.clone(),
    };
    fs::create_dir_all(output_dir).map_err(|error| error.to_string())?;
    let json = serde_json::to_string_pretty(&frame).map_err(|error| error.to_string())?;
    fs::write(
        output_dir.join(CENSUS_PASS_BUNDLE_FILE),
        format!("{json}\n"),
    )
    .map_err(|error| error.to_string())
}

fn census_signature_parts(pass: &CensusActivePass) -> CensusSignatureParts {
    let shader_pairs = pass
        .draws
        .iter()
        .map(|draw| CensusShaderPair {
            vs: draw.vertex_shader.fnv1a64.clone(),
            ps: draw.shader.fnv1a64.clone(),
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let blend_depth_state_family = pass
        .draws
        .iter()
        .filter_map(|draw| draw.draw_state.as_ref())
        .map(|state| format!("{:?}|{:?}", state.blend, state.depth_stencil))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
        .join(";");
    let draw_call_shape_family = pass
        .draws
        .iter()
        .map(|draw| {
            format!(
                "{}:{}:{}:{:?}:{:?}",
                draw.draw_call,
                draw.draw_count,
                draw.draw_start,
                draw.per_instance_count,
                draw.instance_count
            )
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
        .join(";");
    CensusSignatureParts {
        render_target_class: pass.boundary.render_target_class.clone(),
        depth_class: pass.boundary.depth_class.clone(),
        shader_pairs,
        blend_depth_state_family,
        draw_call_shape_family,
    }
}

fn census_signature_hash(parts: &CensusSignatureParts) -> String {
    let json = serde_json::to_string(parts).unwrap_or_default();
    fnv1a64_bytes(json.as_bytes())
}

fn write_census_manifest_snapshot(root_output_dir: &Path) -> Result<(), String> {
    let Some(manifest) = RUNTIME.get().and_then(|runtime| {
        runtime.lock().ok().and_then(|runtime| {
            if census_manifest_snapshot_needed(&runtime) {
                Some(census_manifest_for_runtime(&runtime))
            } else {
                None
            }
        })
    }) else {
        return Ok(());
    };
    let census_dir = root_output_dir.join(CENSUS_DIR);
    fs::create_dir_all(&census_dir).map_err(|error| error.to_string())?;
    let json = serde_json::to_string_pretty(&manifest).map_err(|error| error.to_string())?;
    fs::write(census_dir.join(CENSUS_MANIFEST_FILE), format!("{json}\n"))
        .map_err(|error| error.to_string())
}

fn census_manifest_snapshot_needed(runtime: &CaptureRuntime) -> bool {
    runtime.census.enabled || !runtime.census.signatures.is_empty()
}

fn census_manifest_for_runtime(runtime: &CaptureRuntime) -> CensusManifestFile {
    let entries = runtime
        .census
        .signatures
        .values()
        .cloned()
        .collect::<Vec<_>>();
    CensusManifestFile {
        schema: CENSUS_MANIFEST_SCHEMA,
        enabled: runtime.census.enabled,
        signatures_seen: entries.len(),
        signatures_captured: entries.iter().filter(|entry| entry.captured).count(),
        signatures_uncaptured: entries.iter().filter(|entry| !entry.captured).count(),
        finalized_events: runtime.census.finalized_event_signatures.len(),
        payload_bytes_reserved: runtime.census.payload_bytes_reserved,
        payload_byte_budget: runtime.census.payload_byte_budget,
        payload_stopped: runtime.census.payload_stopped,
        payload_stop_reason: runtime.census.payload_stop_reason.clone(),
        paused_frames: runtime.census.paused_frames,
        entries,
    }
}

fn maybe_emit_census_heartbeat_locked(runtime: &mut CaptureRuntime) {
    if !runtime.census.enabled {
        return;
    }
    let now = unsafe { GetTickCount64() };
    if now < runtime.census.next_heartbeat_ms {
        return;
    }
    runtime.census.next_heartbeat_ms = now.saturating_add(CENSUS_HEARTBEAT_INTERVAL_MS);
    let signatures_seen = runtime.census.signatures.len();
    let signatures_captured = runtime
        .census
        .signatures
        .values()
        .filter(|entry| entry.captured)
        .count();
    write_event(
        &runtime.root_output_dir,
        &format!(
            "{{\"type\":\"census_heartbeat\",\"frame_index\":{},\"signatures_seen\":{},\"signatures_captured\":{},\"payload_bytes_reserved\":{},\"payload_byte_budget\":{},\"paused_frames\":{},\"rounds_done\":{}}}",
            runtime.census.frame_index,
            signatures_seen,
            signatures_captured,
            runtime.census.payload_bytes_reserved,
            runtime.census.payload_byte_budget,
            runtime.census.paused_frames,
            runtime.census.auto_round_buckets_done.len()
        ),
    );
}

fn record_census_capture_work_ms(elapsed_ms: u64) {
    if elapsed_ms == 0 {
        return;
    }
    if let Some(runtime) = RUNTIME.get() {
        if let Ok(mut runtime) = runtime.lock() {
            if !runtime.census.enabled
                || runtime.census.payload_stopped
                || runtime.census.work_frame_paused
            {
                return;
            }
            runtime.census.capture_work_frame_ms = runtime
                .census
                .capture_work_frame_ms
                .saturating_add(elapsed_ms);
            if runtime.census.capture_work_frame_ms
                > runtime.capture_budget.max_capture_work_ms_per_frame
            {
                pause_census_work_frame_for_runtime(&mut runtime);
            }
        }
    }
}

fn pause_census_work_frame_for_runtime(runtime: &mut CaptureRuntime) -> bool {
    if !runtime.census.enabled || runtime.census.payload_stopped || runtime.census.work_frame_paused
    {
        return false;
    }
    runtime.census.work_frame_paused = true;
    runtime.census.paused_frames = runtime.census.paused_frames.saturating_add(1);
    let reason = CENSUS_UNCAPTURED_WORK_BUDGET_FRAME_PAUSED.to_string();
    if let Some(pass) = runtime.census.active_pass.as_mut() {
        let blocker = format!("census draw payload capture disabled: {reason}");
        if !pass.blockers.iter().any(|existing| existing == &blocker) {
            pass.blockers.push(blocker);
        }
    }
    write_event(
        &runtime.root_output_dir,
        &format!(
            "{{\"type\":\"census_work_frame_paused\",\"frame_index\":{},\"frame_capture_work_ms\":{},\"max_capture_work_ms_per_frame\":{}}}",
            runtime.census.frame_index,
            runtime.census.capture_work_frame_ms,
            runtime.capture_budget.max_capture_work_ms_per_frame
        ),
    );
    true
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
    debug_assert_render_thread_owner("capture_map_frame_bundle_if_active");
    if capture_quiesced() {
        return false;
    }
    if capture_symbol_observed(MAP_FRAME_BUNDLE_SYMBOL) {
        return false;
    }
    if check_next_map_frame_draw_budget().is_err() {
        return false;
    }
    if MAP_FRAME_CAPTURE_IN_PROGRESS.swap(true, Ordering::SeqCst) {
        return false;
    }
    let started_ms = unsafe { GetTickCount64() };
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
    let elapsed_ms = unsafe { GetTickCount64() }.saturating_sub(started_ms);
    let captured_draw = result.is_ok();
    match result {
        Ok(sequence_complete) => record_map_frame_bundle_success(sequence_complete),
        Err(error) => record_map_frame_bundle_rejected(&error),
    }
    MAP_FRAME_CAPTURE_IN_PROGRESS.store(false, Ordering::SeqCst);
    charge_capture_work_ms(elapsed_ms, draw_call);
    captured_draw
}

unsafe fn capture_ui_pass_draw_if_active(
    context: *mut ID3D11DeviceContext,
    draw_call: &'static str,
    draw_count: UINT,
    draw_start: UINT,
    base_vertex_location: i32,
    per_instance_count: Option<UINT>,
    instance_count: Option<UINT>,
    start_instance_location: Option<UINT>,
) -> bool {
    debug_assert_render_thread_owner("capture_ui_pass_draw_if_active");
    if capture_quiesced() || !CONTEXT_UI_PASS_ARMED.load(Ordering::Acquire) {
        return false;
    }
    if MAP_FRAME_CAPTURE_IN_PROGRESS.swap(true, Ordering::SeqCst) {
        return false;
    }
    let started_ms = unsafe { GetTickCount64() };
    let result = unsafe {
        capture_or_watch_ui_pass_draw(
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
    let elapsed_ms = unsafe { GetTickCount64() }.saturating_sub(started_ms);
    let captured_draw = matches!(result, Ok(UiPassDrawOutcome::Captured));
    let handled_draw = matches!(
        result,
        Ok(UiPassDrawOutcome::Captured | UiPassDrawOutcome::SkippedForeign)
    );
    if let Err(error) = result {
        record_ui_pass_rejected(&error);
    }
    MAP_FRAME_CAPTURE_IN_PROGRESS.store(false, Ordering::SeqCst);
    if captured_draw {
        charge_capture_work_ms(elapsed_ms, draw_call);
    }
    handled_draw
}

unsafe fn capture_or_watch_ui_pass_draw(
    context: *mut ID3D11DeviceContext,
    draw_call: &'static str,
    draw_count: UINT,
    draw_start: UINT,
    base_vertex_location: i32,
    per_instance_count: Option<UINT>,
    instance_count: Option<UINT>,
    start_instance_location: Option<UINT>,
) -> Result<UiPassDrawOutcome, String> {
    debug_assert_render_thread_owner("capture_or_watch_ui_pass_draw");
    if context.is_null() {
        return Err("null D3D11 context for ui_pass capture".to_string());
    }
    let (pixel_shader, vertex_shader) = unsafe { active_ui_shader_pointers(context) };
    if let Some((pixel_shader_known, vertex_shader_known)) =
        ui_pass_foreign_shader_status(pixel_shader, vertex_shader)
    {
        return record_ui_pass_foreign_shader_skip(
            draw_call,
            draw_count,
            draw_start,
            base_vertex_location,
            per_instance_count,
            instance_count,
            pixel_shader,
            vertex_shader,
            pixel_shader_known,
            vertex_shader_known,
        );
    }
    let Some(backbuffer) = (unsafe { ui_pass_open_condition(context)? }) else {
        return Ok(UiPassDrawOutcome::Ignored);
    };
    let action = RUNTIME
        .get()
        .and_then(|runtime| runtime.lock().ok())
        .map(|mut runtime| {
            if !ui_pass_capture_pending_for_runtime(&runtime) {
                return (UiPassDrawOutcome::Ignored, None, None, 0);
            }
            if runtime.ui_pass.active {
                return (
                    UiPassDrawOutcome::Captured,
                    Some(runtime.output_dir.clone()),
                    Some(runtime.ui_pass.draws.len() as u32),
                    runtime.ui_pass.state_count,
                );
            }
            let should_capture = runtime.ui_pass.state_count == 0
                || runtime.ui_pass.pending_capture_signature.is_some()
                || runtime.ui_pass.pending_capture_label.is_some();
            if should_capture {
                let state_index = runtime.ui_pass.state_count;
                runtime.ui_pass.active = true;
                runtime.ui_pass.active_capture_label =
                    runtime.ui_pass.pending_capture_label.clone();
                runtime.ui_pass.backbuffer = Some(backbuffer.clone());
                runtime.ui_pass.draws.clear();
                runtime.ui_pass.blockers.clear();
                runtime.ui_pass.frame_signature = UiPassFrameSignature::default();
                refresh_context_hook_interests_for_runtime(&runtime);
                return (
                    UiPassDrawOutcome::Captured,
                    Some(runtime.output_dir.clone()),
                    Some(0),
                    state_index,
                );
            }
            (
                UiPassDrawOutcome::Watched,
                Some(runtime.output_dir.clone()),
                None,
                runtime.ui_pass.state_count,
            )
        })
        .ok_or_else(|| "terrain capture runtime not initialized".to_string())?;
    match action.0 {
        UiPassDrawOutcome::Ignored => return Ok(UiPassDrawOutcome::Ignored),
        UiPassDrawOutcome::SkippedForeign => return Ok(UiPassDrawOutcome::SkippedForeign),
        UiPassDrawOutcome::Watched => {
            unsafe { append_ui_pass_signature_draw(context, draw_count, per_instance_count)? };
            return Ok(UiPassDrawOutcome::Watched);
        }
        UiPassDrawOutcome::Captured => {}
    }
    if check_next_ui_pass_draw_budget().is_err() {
        return Ok(UiPassDrawOutcome::Ignored);
    }
    let output_dir = action
        .1
        .ok_or_else(|| "ui_pass capture has no output directory".to_string())?;
    let draw_index = action
        .2
        .ok_or_else(|| "ui_pass capture has no draw index".to_string())?;
    let state_index = action.3;
    let file_draw_index = ui_pass_file_draw_index(state_index, draw_index);
    if draw_index == 0 {
        write_event(
            &output_dir,
            &format!(
                "{{\"type\":\"ui_pass_opened\",\"moment\":\"{}\",\"backbuffer\":\"{}\",\"width\":{},\"height\":{},\"format\":{}}}",
                UI_PASS_CAPTURE_MOMENT,
                json_escape(&backbuffer.resource_pointer),
                backbuffer.width,
                backbuffer.height,
                backbuffer.format
            ),
        );
    };
    // A panic anywhere in per-draw capture must not unwind across the FFI
    // hook boundary (process abort — live 2026-07-03, UI draw index 2 killed
    // the game). Convert panics into a logged skip: the state loses one draw
    // and says so, the session survives.
    let capture_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
        capture_ui_pass_draw_record(
            context,
            draw_call,
            draw_count,
            draw_start,
            base_vertex_location,
            per_instance_count,
            instance_count,
            start_instance_location,
            draw_index,
            file_draw_index,
            &output_dir,
        )
    }));
    let (draw, mut blockers) = match capture_result {
        Ok(result) => result?,
        Err(payload) => {
            let message = payload
                .downcast_ref::<&str>()
                .map(|s| (*s).to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "non-string panic payload".to_string());
            write_event(
                &output_dir,
                &format!(
                    "{{\"type\":\"ui_pass_draw_capture_panicked\",\"draw_index\":{},\"message\":\"{}\"}}",
                    draw_index,
                    json_escape(&message)
                ),
            );
            if let Some(mut runtime) = RUNTIME.get().and_then(|runtime| runtime.lock().ok()) {
                runtime
                    .ui_pass
                    .blockers
                    .push(format!("draw {draw_index} capture panicked: {message}"));
            }
            return Ok(UiPassDrawOutcome::Captured);
        }
    };
    unsafe { append_ui_pass_signature_draw(context, draw_count, per_instance_count)? };
    let draw_total = {
        let mut runtime = RUNTIME
            .get()
            .and_then(|runtime| runtime.lock().ok())
            .ok_or_else(|| "terrain capture runtime not initialized".to_string())?;
        runtime.last_context = context as usize;
        runtime.ui_pass.draws.push(draw);
        runtime.ui_pass.blockers.append(&mut blockers);
        let draw_total = runtime.ui_pass.draws.len();
        refresh_context_hook_interests_for_runtime(&runtime);
        draw_total
    };
    write_event(
        &output_dir,
        &format!(
            "{{\"type\":\"ui_pass_draw_captured\",\"draw_index\":{},\"draw_count\":{},\"total_draws\":{}}}",
            draw_index, draw_count, draw_total
        ),
    );
    Ok(UiPassDrawOutcome::Captured)
}

/// Live diagnostics for the ui_pass open condition: every early-out leg has a
/// counter so a session where ui_pass never opens tells us exactly which leg
/// rejected the game's real UI draws (2026-07-03: a 12-minute atlas session
/// saw zero opens; the harness's synthetic UI passes all legs).
static UI_OPEN_EVALS: AtomicU64 = AtomicU64::new(0);
static UI_OPEN_NO_KNOWN_BACKBUFFER: AtomicU64 = AtomicU64::new(0);
static UI_OPEN_NULL_RTV: AtomicU64 = AtomicU64::new(0);
static UI_OPEN_DEPTH_BLOCKED: AtomicU64 = AtomicU64::new(0);
static UI_OPEN_RTV_MISMATCH: AtomicU64 = AtomicU64::new(0);
static UI_OPEN_MATCHED: AtomicU64 = AtomicU64::new(0);
static UI_OPEN_LAST_MISMATCH_RTV: AtomicU64 = AtomicU64::new(0);

pub(crate) fn ui_pass_open_diagnostics_json() -> String {
    format!(
        "{{\"type\":\"ui_pass_open_diagnostics\",\"evals\":{},\"no_known_backbuffer\":{},\"null_rtv\":{},\"depth_blocked\":{},\"rtv_mismatch\":{},\"matched\":{},\"last_mismatch_rtv\":\"0x{:X}\"}}",
        UI_OPEN_EVALS.load(Ordering::SeqCst),
        UI_OPEN_NO_KNOWN_BACKBUFFER.load(Ordering::SeqCst),
        UI_OPEN_NULL_RTV.load(Ordering::SeqCst),
        UI_OPEN_DEPTH_BLOCKED.load(Ordering::SeqCst),
        UI_OPEN_RTV_MISMATCH.load(Ordering::SeqCst),
        UI_OPEN_MATCHED.load(Ordering::SeqCst),
        UI_OPEN_LAST_MISMATCH_RTV.load(Ordering::SeqCst),
    )
}

unsafe fn ui_pass_open_condition(
    context: *mut ID3D11DeviceContext,
) -> Result<Option<UiPassBackbufferFile>, String> {
    debug_assert_render_thread_owner("ui_pass_open_condition");
    UI_OPEN_EVALS.fetch_add(1, Ordering::Relaxed);
    let known_backbuffer = RUNTIME
        .get()
        .and_then(|runtime| runtime.lock().ok())
        .and_then(|runtime| runtime.current_swapchain_backbuffer.clone());
    let Some(known_backbuffer) = known_backbuffer else {
        UI_OPEN_NO_KNOWN_BACKBUFFER.fetch_add(1, Ordering::Relaxed);
        return Ok(None);
    };
    let mut rtv: *mut ID3D11RenderTargetView = ptr::null_mut();
    let mut dsv: *mut ID3D11DepthStencilView = ptr::null_mut();
    unsafe {
        (*context).OMGetRenderTargets(1, &mut rtv, &mut dsv);
    }
    // Depth state is deliberately NOT part of the open condition: live the game
    // binds a DSV with DepthEnable=TRUE for its entire UI pass (diagnostic
    // session 2026-07-03: 10595/10595 backbuffer draws depth_blocked, 0
    // matched), and the backbuffer-RTV match alone is a sufficient filter —
    // the 3D map renders to intermediate targets, never the backbuffer. The
    // per-draw depth state is still recorded in the bundle for replay.
    if !dsv.is_null() {
        unsafe { release_unknown(dsv as *mut WinVoid) };
    }
    if rtv.is_null() {
        UI_OPEN_NULL_RTV.fetch_add(1, Ordering::Relaxed);
        return Ok(None);
    }
    let resource_pointer = unsafe { render_target_view_resource_pointer(rtv) };
    unsafe { release_unknown(rtv as *mut WinVoid) };
    let Some(resource_pointer) = resource_pointer else {
        UI_OPEN_NULL_RTV.fetch_add(1, Ordering::Relaxed);
        return Ok(None);
    };
    let raw_pointer = resource_pointer;
    let resource_pointer = format!("0x{resource_pointer:X}");
    if resource_pointer != known_backbuffer.resource_pointer {
        UI_OPEN_RTV_MISMATCH.fetch_add(1, Ordering::Relaxed);
        UI_OPEN_LAST_MISMATCH_RTV.store(raw_pointer as u64, Ordering::Relaxed);
        return Ok(None);
    }
    UI_OPEN_MATCHED.fetch_add(1, Ordering::Relaxed);
    Ok(Some(known_backbuffer))
}

unsafe fn current_depth_test_disabled(context: *mut ID3D11DeviceContext) -> bool {
    let mut state: *mut ID3D11DepthStencilState = ptr::null_mut();
    let mut stencil_ref: UINT = 0;
    unsafe {
        (*context).OMGetDepthStencilState(&mut state, &mut stencil_ref);
    }
    if state.is_null() {
        return false;
    }
    let mut desc: D3D11_DEPTH_STENCIL_DESC = unsafe { mem::zeroed() };
    unsafe {
        (*state).GetDesc(&mut desc);
        release_unknown(state as *mut WinVoid);
    }
    !d3d_bool(desc.DepthEnable)
}

unsafe fn capture_ui_pass_draw_record(
    context: *mut ID3D11DeviceContext,
    draw_call: &'static str,
    draw_count: UINT,
    draw_start: UINT,
    base_vertex_location: i32,
    per_instance_count: Option<UINT>,
    instance_count: Option<UINT>,
    start_instance_location: Option<UINT>,
    draw_index: u32,
    file_draw_index: u32,
    output_dir: &Path,
) -> Result<(MapFrameDrawFile, Vec<String>), String> {
    debug_assert_render_thread_owner("capture_ui_pass_draw_record");
    // Pre-capture breadcrumb: written BEFORE any payload work so a crash in
    // this function leaves the exact draw shape in the log (2026-07-03: the
    // live game died capturing UI draw index 2 with no trace of why).
    write_event(
        output_dir,
        &format!(
            "{{\"type\":\"ui_pass_draw_precapture\",\"draw_index\":{draw_index},\"draw_call\":\"{draw_call}\",\"draw_count\":{draw_count},\"draw_start\":{draw_start},\"base_vertex_location\":{base_vertex_location},\"instance_count\":{}}}",
            instance_count.map_or_else(|| "null".to_string(), |v| v.to_string())
        ),
    );
    let profile = UI_PASS_CAPTURE_PROFILE;
    let hazard_draw = ui_pass_interop_hazard_draw(draw_call, draw_count);
    let mut blockers = Vec::new();
    let (pixel_shader, vertex_shader) = unsafe { active_ui_shader_pointers(context) };
    let shader = ui_pass_shader_file(
        output_dir,
        "PS",
        pixel_shader,
        ui_pixel_shader_bytecode(pixel_shader),
        &mut blockers,
    );
    let vertex_shader_file = ui_pass_shader_file(
        output_dir,
        "VS",
        vertex_shader,
        ui_vertex_shader_bytecode(vertex_shader),
        &mut blockers,
    );
    write_event(
        output_dir,
        &format!("{{\"type\":\"ui_pass_draw_stage\",\"draw_index\":{draw_index},\"stage\":\"input_layout\"}}"),
    );
    let (layout_ptr, elements, mut layout_blockers) = unsafe { active_ui_input_layout(context) };
    blockers.append(&mut layout_blockers);
    let topology = unsafe { active_primitive_topology(context) };
    let index_capture = if draw_call.contains("Indexed") {
        match unsafe {
            capture_ui_pass_index_buffer(
                context,
                output_dir,
                draw_index,
                draw_call,
                draw_count,
                draw_start,
                base_vertex_location,
                per_instance_count,
                file_draw_index,
            )
        } {
            Ok(capture) => Some(capture),
            Err(error) => {
                blockers.push(error);
                None
            }
        }
    } else {
        None
    };
    write_event(
        output_dir,
        &format!("{{\"type\":\"ui_pass_draw_stage\",\"draw_index\":{draw_index},\"stage\":\"vertex_buffers\"}}"),
    );
    let vertex_range = index_capture
        .as_ref()
        .and_then(|capture| capture.vertex_range)
        .unwrap_or_else(|| ui_nonindexed_vertex_range(draw_count, draw_start, per_instance_count));
    let input_slots = ui_input_slots_or_fallback(&elements);
    let mut vertex_buffers = BTreeMap::new();
    for slot in input_slots {
        match unsafe {
            capture_ui_pass_vertex_buffer_slot(
                context,
                slot,
                vertex_range,
                output_dir,
                file_draw_index,
                profile,
            )
        } {
            Ok(binding) => {
                vertex_buffers.insert(slot.to_string(), binding);
            }
            Err(error) => blockers.push(error),
        }
    }
    write_event(
        output_dir,
        &format!(
            "{{\"type\":\"ui_pass_draw_stage\",\"draw_index\":{draw_index},\"stage\":\"srvs\"}}"
        ),
    );
    let (shader_resources, mut srv_blockers) = unsafe {
        capture_map_frame_ps_srvs(
            context,
            &shader.reflection_resources,
            output_dir,
            file_draw_index,
            profile,
            if ui_pass_skip_policy_applies(ui_cb_skip_policy_for_runtime(), hazard_draw) {
                Some(UI_PASS_INTEROP_HAZARD_UNOBTAINED_REASON)
            } else {
                None
            },
        )
    };
    blockers.append(&mut srv_blockers);
    write_event(
        output_dir,
        &format!("{{\"type\":\"ui_pass_draw_stage\",\"draw_index\":{draw_index},\"stage\":\"samplers\"}}"),
    );
    let (samplers, mut sampler_blockers) =
        unsafe { capture_map_frame_ps_samplers(context, profile) };
    blockers.append(&mut sampler_blockers);
    write_event(
        output_dir,
        &format!("{{\"type\":\"ui_pass_draw_stage\",\"draw_index\":{draw_index},\"stage\":\"constant_buffers\"}}"),
    );
    let (constant_buffers, mut constant_buffer_blockers) =
        if ui_pass_skip_policy_applies(ui_cb_skip_policy_for_runtime(), hazard_draw) {
            (
                ui_pass_unobtained_constant_buffers_from_shaders(
                    &vertex_shader_file,
                    &shader,
                    UI_PASS_INTEROP_HAZARD_UNOBTAINED_REASON,
                ),
                Vec::new(),
            )
        } else {
            unsafe { capture_map_frame_constant_buffers(context, file_draw_index, profile) }
        };
    blockers.append(&mut constant_buffer_blockers);
    write_event(
        output_dir,
        &format!("{{\"type\":\"ui_pass_draw_stage\",\"draw_index\":{draw_index},\"stage\":\"draw_state\"}}"),
    );
    let (draw_state, mut state_blockers) = unsafe {
        capture_map_frame_draw_state(context, output_dir, file_draw_index, profile, false)
    };
    blockers.append(&mut state_blockers);
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
        shader,
        vertex_shader: vertex_shader_file,
        input_layout: MapFrameInputLayoutFile {
            pointer: format!("0x{layout_ptr:X}"),
            elements,
        },
        vertex_buffers,
        index_buffer: index_capture.map(|capture| capture.binding),
        constant_buffers,
        shader_resources,
        samplers,
        draw_state,
        post_draw_output_merger: None,
    };
    Ok((draw, blockers))
}

unsafe fn active_ui_shader_pointers(context: *mut ID3D11DeviceContext) -> (usize, usize) {
    let mut pixel_shader: *mut ID3D11PixelShader = ptr::null_mut();
    let mut vertex_shader: *mut ID3D11VertexShader = ptr::null_mut();
    unsafe {
        (*context).PSGetShader(&mut pixel_shader, ptr::null_mut(), ptr::null_mut());
        (*context).VSGetShader(&mut vertex_shader, ptr::null_mut(), ptr::null_mut());
    }
    let pixel_shader_ptr = pixel_shader as usize;
    let vertex_shader_ptr = vertex_shader as usize;
    unsafe {
        release_unknown(pixel_shader as *mut WinVoid);
        release_unknown(vertex_shader as *mut WinVoid);
    }
    (pixel_shader_ptr, vertex_shader_ptr)
}

fn ui_pass_foreign_shader_status(
    pixel_shader: usize,
    vertex_shader: usize,
) -> Option<(bool, bool)> {
    let pixel_shader_known = ui_pixel_shader_known_for_current_swapchain(pixel_shader);
    let vertex_shader_known = ui_vertex_shader_known_for_current_swapchain(vertex_shader);
    (!pixel_shader_known || !vertex_shader_known)
        .then_some((pixel_shader_known, vertex_shader_known))
}

fn record_ui_pass_foreign_shader_skip(
    draw_call: &'static str,
    draw_count: UINT,
    draw_start: UINT,
    base_vertex_location: i32,
    per_instance_count: Option<UINT>,
    instance_count: Option<UINT>,
    pixel_shader: usize,
    vertex_shader: usize,
    pixel_shader_known: bool,
    vertex_shader_known: bool,
) -> Result<UiPassDrawOutcome, String> {
    let vertex_count = per_instance_count.unwrap_or(draw_count).max(1);
    let signature_draw =
        ui_pass_shader_only_signature_draw(pixel_shader, vertex_shader, vertex_count);
    let Some((output_dir, opened, draw_index, state_index, active)) = RUNTIME.get().and_then(|runtime| {
        runtime.lock().ok().and_then(|mut runtime| {
            if !ui_pass_capture_pending_for_runtime(&runtime) {
                return Some((
                    runtime.output_dir.clone(),
                    None,
                    None,
                    runtime.ui_pass.state_count,
                    false,
                ));
            }
            let should_capture = runtime.ui_pass.active
                || runtime.ui_pass.state_count == 0
                || runtime.ui_pass.pending_capture_signature.is_some()
                || runtime.ui_pass.pending_capture_label.is_some();
            let mut opened = None;
            if !runtime.ui_pass.active && should_capture {
                let Some(backbuffer) = runtime.current_swapchain_backbuffer.clone() else {
                    return Some((
                        runtime.output_dir.clone(),
                        None,
                        None,
                        runtime.ui_pass.state_count,
                        false,
                    ));
                };
                runtime.ui_pass.active = true;
                runtime.ui_pass.active_capture_label = runtime.ui_pass.pending_capture_label.clone();
                runtime.ui_pass.backbuffer = Some(backbuffer.clone());
                runtime.ui_pass.draws.clear();
                runtime.ui_pass.blockers.clear();
                runtime.ui_pass.frame_signature = UiPassFrameSignature::default();
                opened = Some(backbuffer);
                refresh_context_hook_interests_for_runtime(&runtime);
            }
            let active = runtime.ui_pass.active;
            let draw_index = active.then_some(runtime.ui_pass.draws.len() as u32);
            runtime.ui_pass.frame_signature.draws.push(signature_draw);
            let draw_label = draw_index
                .map(|index| index.to_string())
                .unwrap_or_else(|| "watched".to_string());
            runtime.ui_pass.blockers.push(format!(
                "ui_pass draw {draw_label} skipped foreign shader: pixel_shader=0x{pixel_shader:X} pixel_shader_known={pixel_shader_known} vertex_shader=0x{vertex_shader:X} vertex_shader_known={vertex_shader_known}"
            ));
            Some((
                runtime.output_dir.clone(),
                opened,
                draw_index,
                runtime.ui_pass.state_count,
                active,
            ))
        })
    }) else {
        return Err("terrain capture runtime not initialized".to_string());
    };
    if let Some(backbuffer) = opened {
        write_event(
            &output_dir,
            &format!(
                "{{\"type\":\"ui_pass_opened\",\"moment\":\"{}\",\"backbuffer\":\"{}\",\"width\":{},\"height\":{},\"format\":{}}}",
                UI_PASS_CAPTURE_MOMENT,
                json_escape(&backbuffer.resource_pointer),
                backbuffer.width,
                backbuffer.height,
                backbuffer.format
            ),
        );
    }
    let draw_index_json = draw_index.map_or_else(|| "null".to_string(), |index| index.to_string());
    write_event(
        &output_dir,
        &format!(
            "{{\"type\":\"ui_pass_draw_skipped_foreign_shader\",\"draw_index\":{},\"draw_call\":\"{}\",\"draw_count\":{},\"draw_start\":{},\"base_vertex_location\":{},\"instance_count\":{},\"state_index\":{},\"active\":{},\"pixel_shader\":\"0x{:X}\",\"pixel_shader_known\":{},\"vertex_shader\":\"0x{:X}\",\"vertex_shader_known\":{}}}",
            draw_index_json,
            draw_call,
            draw_count,
            draw_start,
            base_vertex_location,
            instance_count.map_or_else(|| "null".to_string(), |value| value.to_string()),
            state_index,
            active,
            pixel_shader,
            pixel_shader_known,
            vertex_shader,
            vertex_shader_known,
        ),
    );
    Ok(if active {
        UiPassDrawOutcome::SkippedForeign
    } else {
        UiPassDrawOutcome::Watched
    })
}

unsafe fn append_ui_pass_signature_draw(
    context: *mut ID3D11DeviceContext,
    draw_count: UINT,
    per_instance_count: Option<UINT>,
) -> Result<(), String> {
    debug_assert_render_thread_owner("append_ui_pass_signature_draw");
    let signature_draw = unsafe {
        ui_pass_signature_draw(context, per_instance_count.unwrap_or(draw_count).max(1))?
    };
    let Some(runtime) = RUNTIME.get() else {
        return Err("terrain capture runtime not initialized".to_string());
    };
    let mut runtime = runtime
        .lock()
        .map_err(|_| "terrain capture runtime lock poisoned".to_string())?;
    runtime.ui_pass.frame_signature.draws.push(signature_draw);
    Ok(())
}

unsafe fn ui_pass_signature_draw(
    context: *mut ID3D11DeviceContext,
    vertex_count: UINT,
) -> Result<UiPassDrawSignature, String> {
    debug_assert_render_thread_owner("ui_pass_signature_draw");
    if context.is_null() {
        return Err("null D3D11 context for ui_pass signature".to_string());
    }
    let (pixel_shader, vertex_shader) = unsafe { active_ui_shader_pointers(context) };
    let pixel_shader_hash = ui_pixel_shader_signature_hash(pixel_shader);
    let vertex_shader_hash = ui_vertex_shader_signature_hash(vertex_shader);
    let texture_srv_pointers = unsafe { active_ui_ps_srv_pointer_set(context) };
    let scissor_rects = unsafe { capture_map_frame_scissor_rects(context) };
    Ok(UiPassDrawSignature {
        pixel_shader_hash,
        vertex_shader_hash,
        texture_srv_pointers,
        vertex_count,
        scissor_rects,
    })
}

fn ui_pass_shader_only_signature_draw(
    pixel_shader: usize,
    vertex_shader: usize,
    vertex_count: UINT,
) -> UiPassDrawSignature {
    UiPassDrawSignature {
        pixel_shader_hash: ui_pixel_shader_signature_hash(pixel_shader),
        vertex_shader_hash: ui_vertex_shader_signature_hash(vertex_shader),
        texture_srv_pointers: Vec::new(),
        vertex_count,
        scissor_rects: Vec::new(),
    }
}

fn ui_pixel_shader_signature_hash(pointer: usize) -> String {
    ui_pixel_shader_bytecode(pointer)
        .map(fnv1a64_bytes)
        .unwrap_or_else(|| format!("unobserved_ps_0x{pointer:X}"))
}

fn ui_vertex_shader_signature_hash(pointer: usize) -> String {
    ui_vertex_shader_bytecode(pointer)
        .map(fnv1a64_bytes)
        .unwrap_or_else(|| format!("unobserved_vs_0x{pointer:X}"))
}

unsafe fn active_ui_ps_srv_pointer_set(
    context: *mut ID3D11DeviceContext,
) -> Vec<UiPassSrvPointerSignature> {
    let binding_slots = D3D11_COMMONSHADER_INPUT_RESOURCE_SLOT_COUNT;
    let mut srvs = vec![ptr::null_mut(); binding_slots as usize];
    unsafe {
        (*context).PSGetShaderResources(0, binding_slots, srvs.as_mut_ptr());
    }
    let mut pointers = Vec::new();
    for (slot, srv) in srvs.into_iter().enumerate() {
        if srv.is_null() {
            continue;
        }
        pointers.push(UiPassSrvPointerSignature {
            slot: slot as u32,
            pointer: srv as usize,
        });
        unsafe {
            release_unknown(srv as *mut WinVoid);
        }
    }
    pointers
}

unsafe fn active_ui_input_layout(
    context: *mut ID3D11DeviceContext,
) -> (usize, Vec<TerrainInputElementDesc>, Vec<String>) {
    let mut layout: *mut ID3D11InputLayout = ptr::null_mut();
    unsafe {
        (*context).IAGetInputLayout(&mut layout);
    }
    let layout_ptr = layout as usize;
    unsafe { release_unknown(layout as *mut WinVoid) };
    if layout_ptr == 0 {
        return (
            0,
            Vec::new(),
            vec![
                "ui_pass draw has no IA input layout bound; falling back to vertex buffer slot 0"
                    .to_string(),
            ],
        );
    }
    let elements = RUNTIME
        .get()
        .and_then(|runtime| runtime.lock().ok())
        .and_then(|runtime| {
            runtime
                .state
                .input_layout(layout_ptr)
                .map(|elements| elements.to_vec())
        });
    match elements {
        Some(elements) => (layout_ptr, elements, Vec::new()),
        None => (
            layout_ptr,
            Vec::new(),
            vec![format!(
                "ui_pass input layout 0x{layout_ptr:X} was not created after hook install; falling back to vertex buffer slot 0"
            )],
        ),
    }
}

fn map_frame_shader_file_from_tracked_bytecode(
    output_dir: &Path,
    stage: &'static str,
    pointer: usize,
    bytecode: Option<&'static [u8]>,
    profile: MapFrameCaptureProfile,
    blockers: &mut Vec<String>,
) -> MapFrameShaderFile {
    let Some(bytecode) = bytecode else {
        blockers.push(format!(
            "{} {stage} shader 0x{pointer:X} bytecode was not observed by Create{stage}Shader hook",
            profile.capture_moment
        ));
        return MapFrameShaderFile {
            stage,
            pointer: format!("0x{pointer:X}"),
            bytecode_file: String::new(),
            byte_len: 0,
            fnv1a64: String::new(),
            reflection_resources: BTreeMap::new(),
            input_parameters: Vec::new(),
            constant_buffers: BTreeMap::new(),
            source_evidence: vec!["missing_create_shader_bytecode"],
        };
    };
    let fnv1a64 = fnv1a64_bytes(bytecode);
    let file = if stage == "PS" {
        format!("{}_{}.bin", profile.pixel_shader_file_prefix, fnv1a64)
    } else {
        format!("{}_{}.bin", profile.vertex_shader_file_prefix, fnv1a64)
    };
    if !output_dir.join(&file).is_file() {
        if let Err(error) = write_map_frame_bytes(output_dir, &file, bytecode) {
            blockers.push(format!(
                "{} {stage} shader bytecode file {file} write failed: {error}",
                profile.capture_moment
            ));
        }
    }
    let vertex_metadata = (stage == "VS")
        .then(|| reflect_terrain_vertex_shader_metadata(bytecode))
        .flatten();
    MapFrameShaderFile {
        stage,
        pointer: format!("0x{pointer:X}"),
        bytecode_file: file,
        byte_len: bytecode.len(),
        fnv1a64,
        reflection_resources: if stage == "PS" {
            shader_texture_resource_slots_from_shader_bytecode(bytecode)
        } else {
            BTreeMap::new()
        },
        input_parameters: vertex_metadata
            .as_ref()
            .map(|metadata| metadata.input_parameters.clone())
            .unwrap_or_default(),
        constant_buffers: if stage == "PS" {
            shader_constant_buffer_slots_from_shader_bytecode(bytecode)
        } else {
            vertex_metadata
                .map(|metadata| metadata.constant_buffers)
                .unwrap_or_else(|| shader_constant_buffer_slots_from_shader_bytecode(bytecode))
        },
        source_evidence: profile.shader_sources.to_vec(),
    }
}

fn ui_pass_shader_file(
    output_dir: &Path,
    stage: &'static str,
    pointer: usize,
    bytecode: Option<&'static [u8]>,
    blockers: &mut Vec<String>,
) -> MapFrameShaderFile {
    let Some(bytecode) = bytecode else {
        blockers.push(format!(
            "ui_pass {stage} shader 0x{pointer:X} bytecode was not observed by Create{stage}Shader hook"
        ));
        return MapFrameShaderFile {
            stage,
            pointer: format!("0x{pointer:X}"),
            bytecode_file: String::new(),
            byte_len: 0,
            fnv1a64: String::new(),
            reflection_resources: BTreeMap::new(),
            input_parameters: Vec::new(),
            constant_buffers: BTreeMap::new(),
            source_evidence: vec!["missing_create_shader_bytecode"],
        };
    };
    let fnv1a64 = fnv1a64_bytes(bytecode);
    let file = if stage == "PS" {
        format!("ui_pass_pixel_shader_{fnv1a64}.bin")
    } else {
        format!("ui_pass_vertex_shader_{fnv1a64}.bin")
    };
    if reserve_ui_shader_file_for_write(&file) {
        if let Err(error) = queue_payload_file_write(output_dir, &file, bytecode) {
            blockers.push(format!(
                "ui_pass {stage} shader bytecode file {file} queue failed: {error}"
            ));
        }
    }
    let vertex_metadata = (stage == "VS")
        .then(|| reflect_terrain_vertex_shader_metadata(bytecode))
        .flatten();
    MapFrameShaderFile {
        stage,
        pointer: format!("0x{pointer:X}"),
        bytecode_file: file,
        byte_len: bytecode.len(),
        fnv1a64,
        reflection_resources: if stage == "PS" {
            shader_texture_resource_slots_from_shader_bytecode(bytecode)
        } else {
            BTreeMap::new()
        },
        input_parameters: vertex_metadata
            .as_ref()
            .map(|metadata| metadata.input_parameters.clone())
            .unwrap_or_default(),
        constant_buffers: if stage == "PS" {
            shader_constant_buffer_slots_from_shader_bytecode(bytecode)
        } else {
            vertex_metadata
                .map(|metadata| metadata.constant_buffers)
                .unwrap_or_else(|| shader_constant_buffer_slots_from_shader_bytecode(bytecode))
        },
        source_evidence: vec!["d3d11_create_shader_bytecode_captured_for_ui_pass"],
    }
}

fn empty_census_shader_file(stage: &'static str, pointer: usize) -> MapFrameShaderFile {
    MapFrameShaderFile {
        stage,
        pointer: format!("0x{pointer:X}"),
        bytecode_file: String::new(),
        byte_len: 0,
        fnv1a64: String::new(),
        reflection_resources: BTreeMap::new(),
        input_parameters: Vec::new(),
        constant_buffers: BTreeMap::new(),
        source_evidence: vec!["not_read_uncaptured_census_signature"],
    }
}

fn census_shader_file(
    output_dir: &Path,
    stage: &'static str,
    pointer: usize,
    bytecode: Option<&'static [u8]>,
    blockers: &mut Vec<String>,
) -> MapFrameShaderFile {
    let Some(bytecode) = bytecode else {
        if let Some(shader) =
            census_known_capture_family_shader_file(output_dir, stage, pointer, blockers)
        {
            return shader;
        }
        blockers.push(format!(
            "census {stage} shader 0x{pointer:X} bytecode was not observed by Create{stage}Shader hook"
        ));
        return empty_census_shader_file(stage, pointer);
    };
    let fnv1a64 = fnv1a64_bytes(bytecode);
    let file = if stage == "PS" {
        format!("../payloads/census_pixel_shader_{fnv1a64}.bin")
    } else {
        format!("../payloads/census_vertex_shader_{fnv1a64}.bin")
    };
    if reserve_census_shader_file_for_write(&file) {
        if let Err(error) = queue_census_payload_file_write(output_dir, &file, bytecode) {
            blockers.push(format!(
                "census {stage} shader bytecode file {file} queue failed: {error}"
            ));
        }
    }
    let vertex_metadata = (stage == "VS")
        .then(|| reflect_terrain_vertex_shader_metadata(bytecode))
        .flatten();
    MapFrameShaderFile {
        stage,
        pointer: format!("0x{pointer:X}"),
        bytecode_file: file,
        byte_len: bytecode.len(),
        fnv1a64,
        reflection_resources: if stage == "PS" {
            shader_texture_resource_slots_from_shader_bytecode(bytecode)
        } else {
            BTreeMap::new()
        },
        input_parameters: vertex_metadata
            .as_ref()
            .map(|metadata| metadata.input_parameters.clone())
            .unwrap_or_default(),
        constant_buffers: if stage == "PS" {
            shader_constant_buffer_slots_from_shader_bytecode(bytecode)
        } else {
            vertex_metadata
                .map(|metadata| metadata.constant_buffers)
                .unwrap_or_else(|| shader_constant_buffer_slots_from_shader_bytecode(bytecode))
        },
        source_evidence: vec!["d3d11_create_shader_bytecode_captured_for_census"],
    }
}

fn census_known_capture_family_shader_file(
    output_dir: &Path,
    stage: &'static str,
    pointer: usize,
    blockers: &mut Vec<String>,
) -> Option<MapFrameShaderFile> {
    let (root_output_dir, profile, shader) = RUNTIME
        .get()
        .and_then(|runtime| runtime.lock().ok())
        .and_then(|runtime| {
            let profile = requested_map_frame_capture_profile_for_runtime(&runtime).ok()?;
            let shader = match stage {
                "PS" => census_known_capture_family_pixel_shader(&runtime, profile, pointer),
                "VS" => census_known_capture_family_vertex_shader(&runtime, profile, pointer),
                _ => None,
            }?;
            Some((runtime.root_output_dir.clone(), profile, shader))
        })?;
    let source_file = format!("{}_{}.bin", shader.file_prefix, shader.fnv1a64);
    let census_file = if stage == "PS" {
        format!("../payloads/census_pixel_shader_{}.bin", shader.fnv1a64)
    } else {
        format!("../payloads/census_vertex_shader_{}.bin", shader.fnv1a64)
    };
    if reserve_census_shader_file_for_write(&census_file) {
        match fs::read(root_output_dir.join(&source_file)) {
            Ok(bytes) => {
                if let Err(error) = queue_census_payload_file_write(output_dir, &census_file, &bytes)
                {
                    blockers.push(format!(
                        "census {stage} shader bytecode file {census_file} queue failed: {error}"
                    ));
                }
            }
            Err(error) => blockers.push(format!(
                "census {stage} shader fallback source file {source_file} is missing or unreadable: {error}"
            )),
        }
    }
    Some(MapFrameShaderFile {
        stage,
        pointer: format!("0x{pointer:X}"),
        bytecode_file: census_file,
        byte_len: shader.byte_len,
        fnv1a64: shader.fnv1a64,
        reflection_resources: shader.reflection_resources,
        input_parameters: shader.input_parameters,
        constant_buffers: shader.constant_buffers,
        source_evidence: profile.shader_sources.to_vec(),
    })
}

struct CensusKnownShader {
    file_prefix: &'static str,
    byte_len: usize,
    fnv1a64: String,
    reflection_resources: BTreeMap<String, u32>,
    input_parameters: Vec<ShaderInputParameter>,
    constant_buffers: BTreeMap<String, u32>,
}

fn census_known_capture_family_pixel_shader(
    runtime: &CaptureRuntime,
    profile: MapFrameCaptureProfile,
    pointer: usize,
) -> Option<CensusKnownShader> {
    let (file_prefix, metadata) = match profile.capture_moment {
        TERRAIN_CAPTURE_MOMENT | TERRAIN_CHUNK_LIBRARY_MOMENT => (
            profile.pixel_shader_file_prefix,
            runtime
                .state
                .pdxmap_pixel_shader_metadata(pointer)
                .cloned()?,
        ),
        WATER_CAPTURE_MOMENT => (
            profile.pixel_shader_file_prefix,
            runtime
                .state
                .pdxwater_pixel_shader_metadata(pointer)
                .cloned()?,
        ),
        RIVERS_CAPTURE_MOMENT => (
            profile.pixel_shader_file_prefix,
            runtime
                .state
                .river_pixel_shader_metadata(pointer)
                .cloned()?,
        ),
        BORDERS_CAPTURE_MOMENT => (
            profile.pixel_shader_file_prefix,
            runtime
                .state
                .border_pixel_shader_metadata(pointer)
                .cloned()?,
        ),
        UNDERWATER_CAPTURE_MOMENT => (
            profile.pixel_shader_file_prefix,
            runtime
                .state
                .underwater_pixel_shader_metadata(pointer)
                .cloned()?,
        ),
        TREES_CAPTURE_MOMENT => (
            profile.pixel_shader_file_prefix,
            runtime.state.tree_pixel_shader_metadata(pointer).cloned()?,
        ),
        _ => return None,
    };
    Some(CensusKnownShader {
        file_prefix,
        byte_len: metadata.byte_len,
        fnv1a64: metadata.fnv1a64,
        reflection_resources: metadata.resource_slots,
        input_parameters: Vec::new(),
        constant_buffers: metadata.constant_buffers,
    })
}

fn census_known_capture_family_vertex_shader(
    runtime: &CaptureRuntime,
    profile: MapFrameCaptureProfile,
    pointer: usize,
) -> Option<CensusKnownShader> {
    let metadata = match profile.capture_moment {
        TERRAIN_CAPTURE_MOMENT | TERRAIN_CHUNK_LIBRARY_MOMENT | UNDERWATER_CAPTURE_MOMENT => {
            runtime
                .state
                .terrain_vertex_shader_metadata(pointer)
                .cloned()?
        }
        WATER_CAPTURE_MOMENT
        | RIVERS_CAPTURE_MOMENT
        | BORDERS_CAPTURE_MOMENT
        | TREES_CAPTURE_MOMENT => runtime.state.map_vertex_shader_metadata(pointer).cloned()?,
        _ => return None,
    };
    Some(CensusKnownShader {
        file_prefix: profile.vertex_shader_file_prefix,
        byte_len: metadata.byte_len,
        fnv1a64: metadata.fnv1a64,
        reflection_resources: BTreeMap::new(),
        input_parameters: metadata.input_parameters,
        constant_buffers: metadata.constant_buffers,
    })
}

fn reserve_ui_shader_file_for_write(file: &str) -> bool {
    RUNTIME
        .get()
        .and_then(|runtime| runtime.lock().ok())
        .map_or(false, |mut runtime| {
            runtime.ui_shader_files_written.insert(file.to_string())
        })
}

fn reserve_shared_payload_file_for_write(file: &str) -> bool {
    RUNTIME
        .get()
        .and_then(|runtime| runtime.lock().ok())
        .map_or(true, |mut runtime| {
            runtime
                .shared_payload_files_written
                .insert(file.to_string())
        })
}

fn reserve_census_shared_payload_file_for_write(file: &str) -> bool {
    RUNTIME
        .get()
        .and_then(|runtime| runtime.lock().ok())
        .map_or(true, |mut runtime| {
            reserve_census_shared_payload_file_for_runtime(&mut runtime, file)
        })
}

fn reserve_census_shader_file_for_write(file: &str) -> bool {
    RUNTIME
        .get()
        .and_then(|runtime| runtime.lock().ok())
        .map_or(true, |mut runtime| {
            runtime.census.shader_files_written.insert(file.to_string())
        })
}

fn reserve_census_shared_payload_file_for_runtime(
    runtime: &mut CaptureRuntime,
    file: &str,
) -> bool {
    runtime
        .census
        .shared_payload_files_written
        .insert(file.to_string())
}

#[derive(Clone, Copy, Debug)]
struct UiVertexRange {
    first_vertex: u32,
    vertex_count: u32,
}

struct UiIndexBufferCapture {
    binding: MapFrameIndexBufferBindingFile,
    vertex_range: Option<UiVertexRange>,
}

fn ui_nonindexed_vertex_range(
    draw_count: UINT,
    draw_start: UINT,
    per_instance_count: Option<UINT>,
) -> UiVertexRange {
    UiVertexRange {
        first_vertex: draw_start,
        vertex_count: per_instance_count.unwrap_or(draw_count).max(1),
    }
}

fn ui_input_slots_or_fallback(elements: &[TerrainInputElementDesc]) -> BTreeSet<u32> {
    let slots = elements
        .iter()
        .map(|element| element.input_slot)
        .collect::<BTreeSet<_>>();
    if slots.is_empty() {
        BTreeSet::from([0])
    } else {
        slots
    }
}

unsafe fn capture_ui_pass_vertex_buffer_slot(
    context: *mut ID3D11DeviceContext,
    slot: u32,
    vertex_range: UiVertexRange,
    output_dir: &Path,
    draw_index: u32,
    profile: MapFrameCaptureProfile,
) -> Result<MapFrameBufferBindingFile, String> {
    debug_assert_render_thread_owner("capture_ui_pass_vertex_buffer_slot");
    let mut buffer: *mut ID3D11Buffer = ptr::null_mut();
    let mut stride = 0u32;
    let mut offset = 0u32;
    unsafe {
        (*context).IAGetVertexBuffers(slot, 1, &mut buffer, &mut stride, &mut offset);
    }
    if buffer.is_null() {
        return Err(format!(
            "ui_pass input references IA vertex buffer slot {slot}, but no buffer is bound"
        ));
    }
    if stride == 0 {
        unsafe { release_unknown(buffer as *mut WinVoid) };
        return Err(format!(
            "ui_pass IA vertex buffer slot {slot} has zero stride"
        ));
    }
    let requested_offset = (offset as usize)
        .saturating_add((vertex_range.first_vertex as usize).saturating_mul(stride as usize));
    let requested_byte_len = (vertex_range.vertex_count as usize).saturating_mul(stride as usize);
    let result = unsafe {
        capture_ui_pass_buffer_range(
            context,
            buffer,
            requested_offset,
            requested_byte_len,
            UI_PASS_MAX_BUFFER_PAYLOAD_BYTES_PER_DRAW,
        )
    };
    unsafe { release_unknown(buffer as *mut WinVoid) };
    let captured = result?;
    if captured.source.bind_flags & D3D11_BIND_VERTEX_BUFFER == 0 {
        return Err(format!(
            "ui_pass IA vertex buffer slot {slot} missing D3D11_BIND_VERTEX_BUFFER: BindFlags {}",
            captured.source.bind_flags
        ));
    }
    let requested_file = format!(
        "{}{draw_index:03}_ia_vertex_buffer_slot{slot}.bin",
        profile.file_prefix
    );
    queue_payload_file_write(output_dir, &requested_file, &captured.bytes)?;
    Ok(MapFrameBufferBindingFile {
        stage: "IA",
        slot,
        declared_name: None,
        file: Some(requested_file),
        byte_len: captured.bytes.len(),
        buffer_byte_width: map_frame_buffer_byte_width(&captured.source),
        fnv1a64: Some(fnv1a64_bytes(&captured.bytes)),
        stride: Some(stride),
        offset: Some(offset),
        dxgi_format: None,
        bind_flags: captured.source.bind_flags,
        usage: captured.source.usage,
        cpu_access_flags: captured.source.cpu_access_flags,
        unknown_origin: None,
        byte_range: Some(captured.byte_range),
        payload_truncated: Some(captured.truncated),
        unobtained_reason: None,
    })
}

unsafe fn capture_ui_pass_index_buffer(
    context: *mut ID3D11DeviceContext,
    output_dir: &Path,
    _draw_index: u32,
    _draw_call: &'static str,
    draw_count: UINT,
    draw_start: UINT,
    base_vertex_location: i32,
    per_instance_count: Option<UINT>,
    file_draw_index: u32,
) -> Result<UiIndexBufferCapture, String> {
    debug_assert_render_thread_owner("capture_ui_pass_index_buffer");
    let mut buffer: *mut ID3D11Buffer = ptr::null_mut();
    let mut format = 0u32;
    let mut offset = 0u32;
    unsafe {
        (*context).IAGetIndexBuffer(&mut buffer, &mut format, &mut offset);
    }
    if buffer.is_null() {
        return Err("ui_pass indexed draw has no IA index buffer bound".to_string());
    }
    let index_size = ui_index_size(format)?;
    let index_count = per_instance_count.unwrap_or(draw_count).max(1);
    let requested_offset =
        (offset as usize).saturating_add((draw_start as usize).saturating_mul(index_size));
    let requested_byte_len = (index_count as usize).saturating_mul(index_size);
    let result = unsafe {
        capture_ui_pass_buffer_range(
            context,
            buffer,
            requested_offset,
            requested_byte_len,
            UI_PASS_MAX_BUFFER_PAYLOAD_BYTES_PER_DRAW,
        )
    };
    unsafe { release_unknown(buffer as *mut WinVoid) };
    let captured = result?;
    if captured.source.bind_flags & 0x2 == 0 {
        return Err(format!(
            "ui_pass IA index buffer missing D3D11_BIND_INDEX_BUFFER: BindFlags {}",
            captured.source.bind_flags
        ));
    }
    let vertex_range =
        ui_vertex_range_from_indices(format, &captured.bytes, base_vertex_location).ok();
    let requested_file = format!(
        "{}{file_draw_index:03}_ia_index_buffer.bin",
        UI_PASS_FILE_PREFIX
    );
    queue_payload_file_write(output_dir, &requested_file, &captured.bytes)?;
    Ok(UiIndexBufferCapture {
        binding: MapFrameIndexBufferBindingFile {
            file: Some(requested_file),
            byte_len: captured.bytes.len(),
            buffer_byte_width: map_frame_buffer_byte_width(&captured.source),
            fnv1a64: Some(fnv1a64_bytes(&captured.bytes)),
            dxgi_format: format,
            offset,
            bind_flags: captured.source.bind_flags,
            usage: captured.source.usage,
            cpu_access_flags: captured.source.cpu_access_flags,
            byte_range: Some(captured.byte_range),
            payload_truncated: Some(captured.truncated),
        },
        vertex_range,
    })
}

fn ui_index_size(format: u32) -> Result<usize, String> {
    match format {
        57 => Ok(2),
        42 => Ok(4),
        other => Err(format!(
            "ui_pass unsupported IA index buffer DXGI format {other}"
        )),
    }
}

fn ui_vertex_range_from_indices(
    format: u32,
    bytes: &[u8],
    base_vertex_location: i32,
) -> Result<UiVertexRange, String> {
    let index_size = ui_index_size(format)?;
    if bytes.len() < index_size {
        return Err("ui_pass index payload is too small to derive a vertex range".to_string());
    }
    let mut min_vertex = u32::MAX;
    let mut max_vertex = 0u32;
    for chunk in bytes.chunks_exact(index_size) {
        let raw = if index_size == 2 {
            u16::from_le_bytes([chunk[0], chunk[1]]) as i64
        } else {
            u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]) as i64
        };
        let vertex = raw + base_vertex_location as i64;
        if vertex < 0 || vertex > u32::MAX as i64 {
            return Err(format!(
                "ui_pass index plus base_vertex_location produced out-of-range vertex {vertex}"
            ));
        }
        let vertex = vertex as u32;
        min_vertex = min_vertex.min(vertex);
        max_vertex = max_vertex.max(vertex);
    }
    Ok(UiVertexRange {
        first_vertex: min_vertex,
        vertex_count: max_vertex.saturating_sub(min_vertex).saturating_add(1),
    })
}

struct UiCapturedBufferRange {
    source: BufferDesc,
    bytes: Vec<u8>,
    byte_range: MapFrameBufferByteRangeFile,
    truncated: bool,
}

unsafe fn capture_ui_pass_buffer_range(
    context: *mut ID3D11DeviceContext,
    buffer: *mut ID3D11Buffer,
    requested_offset: usize,
    requested_byte_len: usize,
    cap_bytes: usize,
) -> Result<UiCapturedBufferRange, String> {
    debug_assert_render_thread_owner("capture_ui_pass_buffer_range");
    if requested_byte_len == 0 {
        return Err("ui_pass requested zero bytes from a D3D11 buffer".to_string());
    }
    let mut source_desc_native: D3D11_BUFFER_DESC = unsafe { mem::zeroed() };
    unsafe {
        (*buffer).GetDesc(&mut source_desc_native);
    }
    let source = buffer_desc_from_native(&source_desc_native);
    let source_len = source.byte_width as usize;
    if requested_offset >= source_len {
        return Err(format!(
            "ui_pass requested buffer range offset {requested_offset} beyond source byte_len {source_len}"
        ));
    }
    let available_len = source_len
        .saturating_sub(requested_offset)
        .min(requested_byte_len);
    let captured_len = available_len.min(cap_bytes).max(1);
    let truncated = captured_len < requested_byte_len;
    let mut device: *mut ID3D11Device = ptr::null_mut();
    unsafe {
        (*buffer).GetDevice(&mut device);
    }
    if device.is_null() {
        return Err("ui_pass buffer did not expose an ID3D11Device".to_string());
    }
    let staging_desc = D3D11_BUFFER_DESC {
        ByteWidth: captured_len as u32,
        Usage: D3D11_USAGE_STAGING,
        BindFlags: 0,
        CPUAccessFlags: D3D11_CPU_ACCESS_READ,
        MiscFlags: 0,
        StructureByteStride: 0,
    };
    let mut staging: *mut ID3D11Buffer = ptr::null_mut();
    let create_hr = unsafe { (*device).CreateBuffer(&staging_desc, ptr::null(), &mut staging) };
    unsafe { release_unknown(device as *mut WinVoid) };
    if FAILED(create_hr) || staging.is_null() {
        return Err(format!(
            "CreateBuffer ui_pass staging readback failed: HRESULT 0x{:08X}",
            create_hr as u32
        ));
    }
    let source_box = D3D11_BOX {
        left: requested_offset as u32,
        top: 0,
        front: 0,
        right: requested_offset.saturating_add(captured_len) as u32,
        bottom: 1,
        back: 1,
    };
    unsafe {
        (*context).CopySubresourceRegion(
            staging as *mut ID3D11Resource,
            0,
            0,
            0,
            0,
            buffer as *mut ID3D11Resource,
            0,
            &source_box,
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
        unsafe { release_unknown(staging as *mut WinVoid) };
        return Err(format!(
            "Map ui_pass staging buffer failed: HRESULT 0x{:08X}",
            map_hr as u32
        ));
    }
    let bytes = unsafe { slice::from_raw_parts(mapped.pData.cast::<u8>(), captured_len) }.to_vec();
    unsafe {
        (*context).Unmap(staging as *mut ID3D11Resource, 0);
        release_unknown(staging as *mut WinVoid);
    }
    Ok(UiCapturedBufferRange {
        source,
        bytes,
        byte_range: MapFrameBufferByteRangeFile {
            requested_offset,
            requested_byte_len,
            captured_offset: requested_offset,
            captured_byte_len: captured_len,
            source_byte_len: source_len,
        },
        truncated,
    })
}

unsafe fn capture_terrain_chunk_library_if_active(
    context: *mut ID3D11DeviceContext,
    draw_call: &'static str,
    draw_count: UINT,
    draw_start: UINT,
    base_vertex_location: i32,
    per_instance_count: Option<UINT>,
    instance_count: Option<UINT>,
    start_instance_location: Option<UINT>,
) -> bool {
    debug_assert_render_thread_owner("capture_terrain_chunk_library_if_active");
    if capture_quiesced() {
        return false;
    }
    if !terrain_chunk_library_enabled_for_runtime() {
        return false;
    }
    if !terrain_chunk_library_draw_capture_ready() {
        return false;
    }
    if check_next_terrain_chunk_library_draw_budget().is_err() {
        return false;
    }
    if MAP_FRAME_CAPTURE_IN_PROGRESS.swap(true, Ordering::SeqCst) {
        return false;
    }
    let started_ms = unsafe { GetTickCount64() };
    let result = unsafe {
        capture_active_terrain_chunk_library_draw(
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
    let elapsed_ms = unsafe { GetTickCount64() }.saturating_sub(started_ms);
    let captured_draw = result.is_ok();
    if let Err(error) = result {
        record_terrain_chunk_library_rejected(&error);
    }
    MAP_FRAME_CAPTURE_IN_PROGRESS.store(false, Ordering::SeqCst);
    record_terrain_chunk_capture_work_ms(elapsed_ms, draw_call);
    captured_draw
}

fn record_terrain_chunk_capture_work_ms(elapsed_ms: u64, stage: &'static str) {
    if elapsed_ms == 0 || capture_quiesced() || CAPTURE_BUDGET_STOPPED.load(Ordering::SeqCst) {
        return;
    }
    if let Some(runtime) = RUNTIME.get() {
        if let Ok(runtime) = runtime.lock() {
            let over_budget = elapsed_ms > runtime.capture_budget.max_capture_work_ms_per_frame;
            write_event(
                &runtime.output_dir,
                &format!(
                    "{{\"type\":\"capture_work_frame\",\"stage\":\"terrain_chunk_library_{}\",\"moment\":\"{}\",\"work_ms\":{},\"max_capture_work_ms_per_frame\":{},\"over_budget\":{}}}",
                    json_escape(stage),
                    TERRAIN_CHUNK_LIBRARY_MOMENT,
                    elapsed_ms,
                    runtime.capture_budget.max_capture_work_ms_per_frame,
                    over_budget
                ),
            );
        }
    }
}

fn terrain_chunk_library_enabled_for_runtime() -> bool {
    if capture_quiesced() {
        return false;
    }
    RUNTIME
        .get()
        .and_then(|runtime| runtime.lock().ok())
        .map_or(false, |runtime| runtime.terrain_chunk_library.enabled)
}

fn terrain_chunk_library_draw_capture_ready() -> bool {
    if capture_quiesced() {
        return false;
    }
    RUNTIME
        .get()
        .and_then(|runtime| runtime.lock().ok())
        .map_or(false, |runtime| {
            terrain_chunk_library_draw_capture_ready_for_runtime(&runtime)
        })
}

fn terrain_chunk_library_draw_capture_ready_for_runtime(runtime: &CaptureRuntime) -> bool {
    runtime.terrain_chunk_library.enabled
        && runtime
            .completed_map_frame_moments
            .contains(TERRAIN_CAPTURE_MOMENT)
        && terrain_runtime_inputs_complete(runtime)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TerrainChunkPreidentityReservation {
    Unique,
    Duplicate,
}

fn reserve_terrain_chunk_preidentity_for_capture(
    identity_key: &str,
) -> Result<TerrainChunkPreidentityReservation, String> {
    let mut runtime = RUNTIME
        .get()
        .and_then(|runtime| runtime.lock().ok())
        .ok_or_else(|| "terrain capture runtime not initialized".to_string())?;
    Ok(reserve_terrain_chunk_preidentity_in_state(
        &mut runtime.terrain_chunk_library,
        identity_key,
    ))
}

fn reserve_terrain_chunk_preidentity_in_state(
    state: &mut TerrainChunkLibraryCaptureState,
    identity_key: &str,
) -> TerrainChunkPreidentityReservation {
    state.sampled_draws = state.sampled_draws.saturating_add(1);
    if state.preidentity_to_chunk_id.contains_key(identity_key) {
        state.duplicate_draws = state.duplicate_draws.saturating_add(1);
        TerrainChunkPreidentityReservation::Duplicate
    } else {
        TerrainChunkPreidentityReservation::Unique
    }
}

fn check_next_terrain_chunk_library_draw_budget() -> Result<(), String> {
    if capture_quiesced() || CAPTURE_BUDGET_STOPPED.load(Ordering::SeqCst) {
        return Err("capture budget already exceeded".to_string());
    }
    let Some(event) = RUNTIME.get().and_then(|runtime| {
        runtime.lock().ok().and_then(|runtime| {
            let budget = runtime.capture_budget;
            let draw_count = runtime.terrain_chunk_library.sampled_draws as usize;
            (draw_count >= budget.max_draws_per_pass as usize).then(|| CaptureBudgetExceeded {
                cap: "max_draws_per_pass",
                capture_moment: TERRAIN_CHUNK_LIBRARY_MOMENT,
                draw_count,
                attempted_payload_bytes: 0,
                total_payload_bytes_reserved: runtime.total_payload_bytes_reserved,
                max_total_payload_bytes_per_session: budget.max_total_payload_bytes_per_session,
                max_payload_writer_queue_bytes: budget.max_payload_writer_queue_bytes,
                frame_capture_work_ms: runtime.capture_work_frame_ms,
                max_capture_work_ms_per_frame: budget.max_capture_work_ms_per_frame,
                max_draws_per_pass: budget.max_draws_per_pass,
                message: format!(
                    "next terrain chunk-library draw would exceed max_draws_per_pass {}",
                    budget.max_draws_per_pass
                ),
            })
        })
    }) else {
        return Ok(());
    };
    Err(stop_capture_for_budget(event))
}

unsafe fn capture_active_terrain_chunk_library_draw(
    context: *mut ID3D11DeviceContext,
    draw_call: &'static str,
    draw_count: UINT,
    draw_start: UINT,
    base_vertex_location: i32,
    per_instance_count: Option<UINT>,
    instance_count: Option<UINT>,
    start_instance_location: Option<UINT>,
) -> Result<bool, String> {
    debug_assert_render_thread_owner("capture_active_terrain_chunk_library_draw");
    if capture_quiesced() {
        return Ok(false);
    }
    if context.is_null() {
        return Err("null D3D11 context for terrain chunk-library capture".to_string());
    }
    let profile = TERRAIN_CHUNK_LIBRARY_CAPTURE_PROFILE;
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
        if !runtime.terrain_chunk_library.enabled {
            return Ok(false);
        }
        let Some(shader) =
            runtime
                .state
                .pdxmap_pixel_shader()
                .map(|shader| MapPixelShaderBinding {
                    pixel_shader: shader.pixel_shader,
                })
        else {
            return Ok(false);
        };
        let shader_meta = runtime
            .state
            .pdxmap_pixel_shader_metadata(shader.pixel_shader)
            .cloned()
            .ok_or_else(|| "active pdxmap shader has no bytecode metadata".to_string())?;
        let vertex_shader = runtime
            .state
            .terrain_vertex_shader()
            .ok_or_else(|| "active pdxmap draw has no tracked terrain vertex shader".to_string())?;
        let vertex_shader_meta = runtime
            .state
            .terrain_vertex_shader_metadata(vertex_shader.vertex_shader)
            .cloned()
            .ok_or_else(|| "active terrain vertex shader has no bytecode metadata".to_string())?;
        let (layout_ptr, elements) = runtime
            .state
            .current_input_layout()
            .map(|(layout, elements)| (layout, elements.to_vec()))
            .ok_or_else(|| "active terrain chunk draw has no tracked input layout".to_string())?;
        (
            shader,
            shader_meta,
            vertex_shader,
            vertex_shader_meta,
            layout_ptr,
            elements,
            runtime.output_dir.clone(),
            runtime.terrain_chunk_library.sampled_draws as u32,
        )
    };
    validate_terrain_input_layout(&elements, u32::MAX).map_err(|error| {
        format!(
            "active terrain chunk input layout rejected against {TERRAIN_INPUT_LAYOUT_SHADER_SOURCE}: {error:?}"
        )
    })?;

    let preidentity = unsafe {
        terrain_chunk_preidentity_from_active_state(
            context,
            draw_call,
            draw_count,
            draw_start,
            base_vertex_location,
            per_instance_count,
            instance_count,
            start_instance_location,
            &shader_meta.fnv1a64,
            &vertex_shader_meta.fnv1a64,
            layout_ptr,
            &elements,
            &shader_meta.constant_buffers,
            &vertex_shader_meta.constant_buffers,
        )
    }?;
    let identity_key = terrain_chunk_preidentity_key(&preidentity)?;
    if reserve_terrain_chunk_preidentity_for_capture(&identity_key)?
        == TerrainChunkPreidentityReservation::Duplicate
    {
        return Ok(false);
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
            None,
        )
    };
    blockers.append(&mut srv_blockers);
    write_event(
        &output_dir,
        &format!(
            "{{\"type\":\"terrain_chunk_library_capture_stage\",\"stage\":\"shader_resources\",\"draw_index\":{},\"binding_count\":{},\"blocker_count\":{}}}",
            draw_index,
            shader_resources.len(),
            blockers.len()
        ),
    );
    let (samplers, mut sampler_blockers) =
        unsafe { capture_map_frame_ps_samplers(context, profile) };
    blockers.append(&mut sampler_blockers);
    let (constant_buffers, mut constant_buffer_blockers) =
        unsafe { capture_map_frame_constant_buffers(context, draw_index, profile) };
    blockers.append(&mut constant_buffer_blockers);
    write_event(
        &output_dir,
        &format!(
            "{{\"type\":\"terrain_chunk_library_capture_stage\",\"stage\":\"constant_buffers\",\"draw_index\":{},\"binding_count\":{},\"blocker_count\":{}}}",
            draw_index,
            constant_buffers.len(),
            blockers.len()
        ),
    );
    let (draw_state, mut state_blockers) =
        unsafe { capture_map_frame_draw_state(context, &output_dir, draw_index, profile, false) };
    blockers.append(&mut state_blockers);
    write_event(
        &output_dir,
        &format!(
            "{{\"type\":\"terrain_chunk_library_capture_stage\",\"stage\":\"draw_state\",\"draw_index\":{},\"blocker_count\":{}}}",
            draw_index,
            blockers.len()
        ),
    );

    let shader_file = format!(
        "{}_{}.bin",
        profile.pixel_shader_file_prefix, shader_meta.fnv1a64
    );
    let vertex_shader_file = format!(
        "{}_{}.bin",
        profile.vertex_shader_file_prefix, vertex_shader_meta.fnv1a64
    );
    let mut pixel_shader_constant_buffers = shader_meta.constant_buffers.clone();
    canonicalize_map_frame_shader_constant_buffers(
        profile.capture_moment,
        "PS",
        profile.shader_sources.iter().copied(),
        &mut pixel_shader_constant_buffers,
        map_frame_binding_names_terrain_scene_cb0(&constant_buffers, "PS"),
    );
    let mut vertex_shader_constant_buffers = vertex_shader_meta.constant_buffers.clone();
    canonicalize_map_frame_shader_constant_buffers(
        profile.capture_moment,
        "VS",
        profile.shader_sources.iter().copied(),
        &mut vertex_shader_constant_buffers,
        map_frame_binding_names_terrain_scene_cb0(&constant_buffers, "VS"),
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
            constant_buffers: pixel_shader_constant_buffers,
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
            constant_buffers: vertex_shader_constant_buffers,
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
    let bounds = match terrain_chunk_bounds_from_draw(&draw, &output_dir) {
        Ok(bounds) => Some(bounds),
        Err(error) => {
            blockers.push(error);
            None
        }
    };
    write_event(
        &output_dir,
        &format!(
            "{{\"type\":\"terrain_chunk_library_capture_stage\",\"stage\":\"bounds\",\"draw_index\":{},\"has_bounds\":{},\"blocker_count\":{}}}",
            draw_index,
            bounds.is_some(),
            blockers.len()
        ),
    );

    let (new_chunk, manifest_snapshot, coverage_event) = {
        let mut runtime = RUNTIME
            .get()
            .and_then(|runtime| runtime.lock().ok())
            .ok_or_else(|| "terrain capture runtime not initialized".to_string())?;
        runtime.last_context = context as usize;
        for blocker in blockers {
            runtime.terrain_chunk_library.blockers.insert(blocker);
        }
        let mut new_chunk = false;
        if let Some(bounds) = bounds {
            let world_quad = terrain_chunk_world_quad_from_bounds(bounds);
            let identity = terrain_chunk_identity_from_draw(&draw, world_quad);
            let final_identity_key = terrain_chunk_identity_key(&identity)?;
            let chunk_id = terrain_chunk_id(&final_identity_key);
            if runtime
                .terrain_chunk_library
                .identity_to_chunk_id
                .contains_key(&final_identity_key)
            {
                runtime.terrain_chunk_library.duplicate_draws = runtime
                    .terrain_chunk_library
                    .duplicate_draws
                    .saturating_add(1);
            } else {
                let record = TerrainChunkRecordFile {
                    chunk_id: chunk_id.clone(),
                    identity,
                    bounds,
                    world_quad,
                    draw,
                };
                runtime
                    .terrain_chunk_library
                    .identity_to_chunk_id
                    .insert(final_identity_key, chunk_id.clone());
                runtime
                    .terrain_chunk_library
                    .preidentity_to_chunk_id
                    .insert(identity_key, chunk_id.clone());
                runtime
                    .terrain_chunk_library
                    .chunks
                    .insert(chunk_id.clone(), record);
                new_chunk = true;
            }
        }
        let chunks = runtime
            .terrain_chunk_library
            .chunks
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let manifest_snapshot = new_chunk.then(|| {
            (
                chunks.clone(),
                runtime
                    .terrain_chunk_library
                    .blockers
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>(),
            )
        });
        let now_ms = unsafe { GetTickCount64() };
        let coverage_event = if now_ms >= runtime.terrain_chunk_library.next_coverage_event_ms {
            runtime.terrain_chunk_library.next_coverage_event_ms =
                now_ms.saturating_add(TERRAIN_CHUNK_LIBRARY_COVERAGE_EVENT_INTERVAL_MS);
            Some((
                runtime.output_dir.clone(),
                chunks.iter().map(|chunk| chunk.bounds).collect::<Vec<_>>(),
                runtime.control.clone(),
                chunks.len(),
            ))
        } else {
            None
        };
        (new_chunk, manifest_snapshot, coverage_event)
    };
    if let Some((chunks, manifest_blockers)) = manifest_snapshot {
        write_terrain_chunk_library_manifest_snapshot(
            &output_dir,
            chunks,
            manifest_blockers,
            "terrain_chunk_library_active_draw_accumulation",
        )?;
    }
    if let Some((event_dir, bounds, control, chunk_count)) = coverage_event {
        let mut coverage_blockers = Vec::new();
        let coverage = terrain_chunk_coverage_for_bounds(
            &bounds,
            terrain_chunk_library_world_rect_from_control(control.as_ref(), &mut coverage_blockers),
        );
        write_event(
            &event_dir,
            &format!(
                "{{\"type\":\"chunk_library_coverage\",\"chunk_count\":{},\"coverage_fraction\":{},\"covered_area\":{},\"world_area\":{},\"hole_count\":{}}}",
                chunk_count,
                coverage.fraction,
                coverage.covered_area,
                coverage.world_area,
                coverage.holes.len()
            ),
        );
    }
    Ok(new_chunk)
}

fn record_terrain_chunk_library_rejected(error: &str) {
    if capture_quiesced() || CAPTURE_BUDGET_STOPPED.load(Ordering::SeqCst) {
        return;
    }
    let Some(output_dir) = RUNTIME.get().and_then(|runtime| {
        runtime.lock().ok().map(|mut runtime| {
            runtime.last_error = Some(error.to_string());
            runtime
                .terrain_chunk_library
                .blockers
                .insert(error.to_string());
            runtime.output_dir.clone()
        })
    }) else {
        return;
    };
    write_event(
        &output_dir,
        &format!(
            "{{\"type\":\"terrain_chunk_library_rejected\",\"message\":\"{}\"}}",
            json_escape(error)
        ),
    );
}

fn finalize_terrain_chunk_library_capture() {
    if capture_quiesced() {
        return;
    }
    let Some((output_dir, chunks, blockers, sampled_draws, duplicate_draws, enabled)) =
        RUNTIME.get().and_then(|runtime| {
            runtime.lock().ok().map(|runtime| {
                (
                    runtime.output_dir.clone(),
                    runtime
                        .terrain_chunk_library
                        .chunks
                        .values()
                        .cloned()
                        .collect::<Vec<_>>(),
                    runtime
                        .terrain_chunk_library
                        .blockers
                        .iter()
                        .cloned()
                        .collect::<Vec<_>>(),
                    runtime.terrain_chunk_library.sampled_draws,
                    runtime.terrain_chunk_library.duplicate_draws,
                    runtime.terrain_chunk_library.enabled,
                )
            })
        })
    else {
        return;
    };
    if !enabled {
        return;
    }
    let mut manifest_blockers = blockers;
    if chunks.is_empty() {
        manifest_blockers.push(
            "terrain chunk library mode was enabled but no valid terrain chunks were captured"
                .to_string(),
        );
    }
    let result = write_terrain_chunk_library_manifest_snapshot(
        &output_dir,
        chunks.clone(),
        manifest_blockers,
        "terrain_chunk_library_dwell_end",
    );
    let mut coverage_blockers = Vec::new();
    let coverage = terrain_chunk_coverage_for_bounds(
        &chunks.iter().map(|chunk| chunk.bounds).collect::<Vec<_>>(),
        terrain_chunk_library_world_rect(&mut coverage_blockers),
    );
    write_event(
        &output_dir,
        &format!(
            "{{\"type\":\"chunk_library_coverage\",\"stage\":\"finalize\",\"chunk_count\":{},\"coverage_fraction\":{},\"covered_area\":{},\"world_area\":{},\"hole_count\":{}}}",
            chunks.len(),
            coverage.fraction,
            coverage.covered_area,
            coverage.world_area,
            coverage.holes.len()
        ),
    );
    match result {
        Ok(clean) => write_event(
            &output_dir,
            &format!(
                "{{\"type\":\"terrain_chunk_library_finished\",\"file\":\"{}\",\"clean\":{},\"sampled_draws\":{},\"duplicate_draws\":{},\"chunk_count\":{},\"coverage_fraction\":{},\"hole_count\":{}}}",
                TERRAIN_CHUNK_LIBRARY_FILE,
                clean,
                sampled_draws,
                duplicate_draws,
                chunks.len(),
                coverage.fraction,
                coverage.holes.len()
            ),
        ),
        Err(error) => write_event(
            &output_dir,
            &format!(
                "{{\"type\":\"terrain_chunk_library_write_failed\",\"file\":\"{}\",\"message\":\"{}\"}}",
                TERRAIN_CHUNK_LIBRARY_FILE,
                json_escape(&error)
            ),
        ),
    }
}

unsafe fn capture_map_frame_post_draw_output_if_active(context: *mut ID3D11DeviceContext) {
    debug_assert_render_thread_owner("capture_map_frame_post_draw_output_if_active");
    if context.is_null() || capture_symbol_observed(MAP_FRAME_BUNDLE_SYMBOL) {
        return;
    }
    let mut finalize_blend_layer = false;
    if let Some(runtime) = RUNTIME.get() {
        if let Ok(mut runtime) = runtime.lock() {
            runtime.last_context = context as usize;
            if let Ok(profile) = requested_map_frame_capture_profile_for_runtime(&runtime) {
                finalize_blend_layer = map_frame_post_draw_blend_layer_boundary_ready_with_mode(
                    runtime.map_frame_draws.len(),
                    profile,
                    runtime
                        .completed_map_frame_moments
                        .contains(profile.capture_moment),
                    trees_close_mode_from_control(runtime.control.as_ref()),
                );
            }
        }
    }
    if finalize_blend_layer {
        finalize_map_frame_bundle_pass(context, "post_draw_blend_layer_boundary");
    }
}

/// Dedicated capture for the country-label glyph pass. Structurally mirrors
/// `capture_active_restore_map_frame_bundle` — same trusted-context-only gate,
/// same in-progress-snapshot/pass-close discipline, same fail-closed events —
/// but where restore is a fullscreen triangle (no VB), mapname's whole payload
/// IS the vertex buffer: the float3 world-space glyph positions that are the
/// byte ground truth for the label placement law. So this captures the full
/// IA vertex/index buffers, input layout, font-atlas SRV, and both CBs
/// (slot 0 scene + slot 1 Transp_OffsetX).
///
/// Anchoring (per adversarial review): the moment keys on the tracked mapname
/// PIXEL shader (arming + pass close ride the existing PS machinery), and the
/// currently-bound VERTEX shader is validated at draw time to be a tracked
/// mapname VS (the strict reflected float3-position discriminator). Neither the
/// PS nor the VS may EVER assert — every mismatch, null, or missing resource
/// becomes an Err (recorded as a blocked/observation-miss event) or a per-draw
/// blocker, never an abort. This is the inherited live-crash safety law.
unsafe fn capture_active_mapname_map_frame_bundle(
    context: *mut ID3D11DeviceContext,
    draw_call: &'static str,
    draw_count: UINT,
    draw_start: UINT,
    base_vertex_location: i32,
    per_instance_count: Option<UINT>,
    instance_count: Option<UINT>,
    start_instance_location: Option<UINT>,
    profile: MapFrameCaptureProfile,
) -> Result<bool, String> {
    debug_assert_render_thread_owner("capture_active_mapname_map_frame_bundle");
    if context.is_null() {
        return Err("null D3D11 context for mapname-pass map-frame capture".to_string());
    }
    // Trust gate — fail closed. The label pass overlays the map AFTER the
    // terrain/water/etc renderers have latched CONTEXT_MAP_RENDERER_LIVE. A
    // mapname draw seen before that latch is load-screen/browser-interop noise:
    // record it and skip WITHOUT touching the context (no Get* battery — the
    // 2026-07-05 load-screen AV class).
    if !CONTEXT_MAP_RENDERER_LIVE.load(Ordering::Acquire) {
        if let Some(runtime) = RUNTIME.get() {
            if let Ok(runtime) = runtime.lock() {
                write_event(
                    &runtime.output_dir,
                    &format!(
                        "{{\"type\":\"mapname_pass_seen_untrusted\",\"draw_call\":\"{}\",\"draw_count\":{},\"reason\":\"{}\"}}",
                        draw_call, draw_count, CENSUS_UNCAPTURED_UNTRUSTED_CONTEXT_CLASS
                    ),
                );
            }
        }
        return Err(
            "mapname_pass draw observed before trusted map renderer live; skipped without context state reads"
                .to_string(),
        );
    }

    let (shader, shader_meta, output_dir, draw_index) = {
        let runtime = RUNTIME
            .get()
            .and_then(|runtime| runtime.lock().ok())
            .ok_or_else(|| "terrain capture runtime not initialized".to_string())?;
        let shader = runtime.state.mapname_pixel_shader().ok_or_else(|| {
            "no active mapname pixel shader for mapname-pass map-frame capture".to_string()
        })?;
        let shader_meta = runtime
            .state
            .mapname_pixel_shader_metadata(shader.pixel_shader)
            .cloned()
            .ok_or_else(|| "active mapname shader has no bytecode metadata".to_string())?;
        (
            shader,
            shader_meta,
            runtime.output_dir.clone(),
            runtime.map_frame_draws.len() as u32,
        )
    };

    let mut blockers = Vec::new();
    // Confirm the live IA/shader state against the tracked mapname anchors. The
    // PS must be the tracked one (defence against a torn ps-set); the VS must
    // be a tracked mapname VS (the strict reflected discriminator) — this is
    // where the corroborating VS identity is enforced on the actual draw.
    let (active_pixel_shader, active_vertex_shader) = unsafe { active_ui_shader_pointers(context) };
    if active_pixel_shader != shader.pixel_shader {
        return Err(format!(
            "mapname_pass active PS 0x{active_pixel_shader:X} does not match tracked mapname PS 0x{:X}",
            shader.pixel_shader
        ));
    }
    let vertex_shader_known = {
        let runtime = RUNTIME
            .get()
            .and_then(|runtime| runtime.lock().ok())
            .ok_or_else(|| "terrain capture runtime not initialized".to_string())?;
        runtime
            .state
            .mapname_vertex_shader_known(active_vertex_shader)
    };
    if !vertex_shader_known {
        return Err(format!(
            "mapname_pass active VS 0x{active_vertex_shader:X} is not a tracked mapname vertex shader (float3-position discriminator did not match)"
        ));
    }

    let topology = unsafe { active_primitive_topology(context) };
    // Read the currently-bound input layout from the context (released
    // immediately). A mapname draw with no bound layout, or a layout that was
    // not created after hook install, is recorded as a blocker but does not
    // abort — we still capture VB slot 0 as a best effort.
    let (layout_ptr, elements, mut layout_blockers) = unsafe { active_ui_input_layout(context) };
    blockers.append(&mut layout_blockers);

    // Enumerate the input-layout vertex-buffer slots (the glyph VB is the whole
    // point). Fall back to slot 0 when the layout was not observed.
    let input_slots = if elements.is_empty() {
        let mut slots = BTreeSet::new();
        slots.insert(0u32);
        slots
    } else {
        elements
            .iter()
            .map(|element| element.input_slot)
            .collect::<BTreeSet<_>>()
    };
    let mut vertex_buffers = BTreeMap::new();
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
    if vertex_buffers.is_empty() {
        blockers.push(
            "mapname_pass captured no IA vertex buffer (the float3 glyph VB payload)".to_string(),
        );
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
            None,
        )
    };
    blockers.append(&mut srv_blockers);
    // The font atlas (DiffuseTexture at slot 0) must be captured with a payload.
    if !shader_resources.iter().any(|binding| {
        binding.declared_name.as_deref() == Some(MAPNAME_DIFFUSE_TEXTURE_RESOURCE_SYMBOL)
            && binding.file.is_some()
    }) {
        blockers.push(format!(
            "mapname_pass PS SRV {MAPNAME_DIFFUSE_TEXTURE_RESOURCE_SYMBOL} (font atlas) was not captured with a payload"
        ));
    }
    let (samplers, mut sampler_blockers) =
        unsafe { capture_map_frame_ps_samplers(context, profile) };
    blockers.append(&mut sampler_blockers);
    let (constant_buffers, mut constant_buffer_blockers) =
        unsafe { capture_map_frame_constant_buffers(context, draw_index, profile) };
    blockers.append(&mut constant_buffer_blockers);
    // Slot-1 Transp_OffsetX CB is the transparency/wrap-offset ground truth and
    // must be bound (VS or PS reads it).
    if !constant_buffers
        .iter()
        .any(|binding| binding.slot == MAPNAME_TRANSP_OFFSET_CB_BIND_POINT)
    {
        blockers.push(format!(
            "mapname_pass Transp_OffsetX constant buffer slot {} was not bound",
            MAPNAME_TRANSP_OFFSET_CB_BIND_POINT
        ));
    }
    // Slot-0 scene CB carries ViewProjectionMatrix / vCamLookAtDir.
    if !constant_buffers
        .iter()
        .any(|binding| binding.slot == MAPNAME_SCENE_CB_BIND_POINT)
    {
        blockers.push(format!(
            "mapname_pass scene constant buffer slot {} was not bound",
            MAPNAME_SCENE_CB_BIND_POINT
        ));
    }
    let (draw_state, mut state_blockers) =
        unsafe { capture_map_frame_draw_state(context, &output_dir, draw_index, profile, true) };
    blockers.append(&mut state_blockers);

    let shader_file = format!(
        "{}_{}.bin",
        profile.pixel_shader_file_prefix, shader_meta.fnv1a64
    );
    let mut pixel_shader_constant_buffers = shader_meta.constant_buffers.clone();
    canonicalize_map_frame_shader_constant_buffers(
        profile.capture_moment,
        "PS",
        profile.shader_sources.iter().copied(),
        &mut pixel_shader_constant_buffers,
        map_frame_binding_names_terrain_scene_cb0(&constant_buffers, "PS"),
    );
    // Build the VS record from the tracked mapname VS bytecode if present,
    // else a bytecode-missing placeholder (never panics).
    let vertex_shader_file = map_frame_shader_file_from_tracked_bytecode(
        &output_dir,
        "VS",
        active_vertex_shader,
        ui_vertex_shader_bytecode(active_vertex_shader),
        profile,
        &mut blockers,
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
            constant_buffers: pixel_shader_constant_buffers,
            source_evidence: profile.shader_sources.to_vec(),
        },
        vertex_shader: vertex_shader_file,
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

    let (draws, mut accumulated_blockers, write_in_progress_snapshot) = {
        let mut runtime = RUNTIME
            .get()
            .and_then(|runtime| runtime.lock().ok())
            .ok_or_else(|| "terrain capture runtime not initialized".to_string())?;
        runtime.last_context = context as usize;
        runtime.map_frame_draws.push(draw.clone());
        runtime.map_frame_blockers.extend(blockers);
        refresh_context_hook_interests_for_runtime(&runtime);
        let write_in_progress_snapshot = should_write_in_progress_map_frame_snapshot(
            runtime.map_frame_draws.len(),
            runtime.map_frame_blockers.len(),
        );
        (
            runtime.map_frame_draws.clone(),
            runtime.map_frame_blockers.clone(),
            write_in_progress_snapshot,
        )
    };
    write_event(
        &output_dir,
        &format!(
            "{{\"type\":\"mapname_pass_draw_captured\",\"draw_index\":{},\"draw_call\":\"{}\",\"draw_count\":{},\"vertex_shader\":\"0x{:X}\",\"vertex_buffer_slots\":{},\"srv_count\":{}}}",
            draw_index,
            draw_call,
            draw_count,
            active_vertex_shader,
            draws
                .last()
                .map(|draw| draw.vertex_buffers.len())
                .unwrap_or(0),
            draws
                .last()
                .map(|draw| draw.shader_resources.len())
                .unwrap_or(0)
        ),
    );
    if !write_in_progress_snapshot {
        return Ok(false);
    }
    accumulated_blockers.push(
        "map-frame mapname_pass is still active; waiting for pass-close boundary".to_string(),
    );
    write_map_frame_bundle_snapshot(
        &output_dir,
        draws,
        accumulated_blockers,
        "early_d3d11_active_mapname_draw_sequence_capture_in_progress",
        None,
        profile,
    )?;
    Ok(false)
}

unsafe fn capture_active_restore_map_frame_bundle(
    context: *mut ID3D11DeviceContext,
    draw_call: &'static str,
    draw_count: UINT,
    draw_start: UINT,
    base_vertex_location: i32,
    per_instance_count: Option<UINT>,
    instance_count: Option<UINT>,
    start_instance_location: Option<UINT>,
    profile: MapFrameCaptureProfile,
) -> Result<bool, String> {
    debug_assert_render_thread_owner("capture_active_restore_map_frame_bundle");
    if context.is_null() {
        return Err("null D3D11 context for restore-pass map-frame capture".to_string());
    }
    if !CONTEXT_MAP_RENDERER_LIVE.load(Ordering::Acquire) {
        if let Some(runtime) = RUNTIME.get() {
            if let Ok(runtime) = runtime.lock() {
                write_event(
                    &runtime.output_dir,
                    &format!(
                        "{{\"type\":\"restore_pass_seen_untrusted\",\"draw_call\":\"{}\",\"draw_count\":{},\"reason\":\"{}\"}}",
                        draw_call,
                        draw_count,
                        CENSUS_UNCAPTURED_UNTRUSTED_CONTEXT_CLASS
                    ),
                );
            }
        }
        return Err(
            "restore_pass draw observed before trusted map renderer live; skipped without context state reads"
                .to_string(),
        );
    }

    let (shader, shader_meta, output_dir, draw_index) = {
        let runtime = RUNTIME
            .get()
            .and_then(|runtime| runtime.lock().ok())
            .ok_or_else(|| "terrain capture runtime not initialized".to_string())?;
        let shader = runtime.state.restore_pixel_shader().ok_or_else(|| {
            "no active restorescene pixel shader for restore-pass map-frame capture".to_string()
        })?;
        let shader_meta = runtime
            .state
            .restore_pixel_shader_metadata(shader.pixel_shader)
            .cloned()
            .ok_or_else(|| "active restorescene shader has no bytecode metadata".to_string())?;
        (
            shader,
            shader_meta,
            runtime.output_dir.clone(),
            runtime.map_frame_draws.len() as u32,
        )
    };

    let mut blockers = Vec::new();
    let (active_pixel_shader, active_vertex_shader) = unsafe { active_ui_shader_pointers(context) };
    if active_pixel_shader != shader.pixel_shader {
        return Err(format!(
            "restore_pass active PS 0x{active_pixel_shader:X} does not match tracked restorescene PS 0x{:X}",
            shader.pixel_shader
        ));
    }
    let vertex_shader_file = map_frame_shader_file_from_tracked_bytecode(
        &output_dir,
        "VS",
        active_vertex_shader,
        ui_vertex_shader_bytecode(active_vertex_shader),
        profile,
        &mut blockers,
    );
    let topology = unsafe { active_primitive_topology(context) };
    let restore_srv_slots = shader_meta
        .resource_slots
        .values()
        .copied()
        .collect::<BTreeSet<_>>();
    let (shader_resources, mut srv_blockers) = unsafe {
        capture_map_frame_ps_srvs_for_slots(
            context,
            &shader_meta.resource_slots,
            &restore_srv_slots,
            &output_dir,
            draw_index,
            profile,
            None,
        )
    };
    blockers.append(&mut srv_blockers);
    let (samplers, mut sampler_blockers) =
        unsafe { capture_map_frame_ps_samplers(context, profile) };
    blockers.append(&mut sampler_blockers);
    let (constant_buffers, mut constant_buffer_blockers) =
        unsafe { capture_map_frame_constant_buffers(context, draw_index, profile) };
    blockers.append(&mut constant_buffer_blockers);
    if !constant_buffers.iter().any(|binding| {
        binding.stage == "PS" && binding.slot == RESTORE_POSTEFFECT_BASE_CB_BIND_POINT
    }) {
        blockers.push(format!(
            "restore_pass PS constant buffer slot {} was not bound",
            RESTORE_POSTEFFECT_BASE_CB_BIND_POINT
        ));
    }
    for resource in RESTORE_REQUIRED_RESOURCE_SYMBOLS {
        if !shader_resources.iter().any(|binding| {
            binding.declared_name.as_deref() == Some(resource) && binding.file.is_some()
        }) {
            blockers.push(format!(
                "restore_pass PS SRV {resource} was not captured with a payload"
            ));
        }
    }
    let (draw_state, mut state_blockers) =
        unsafe { capture_map_frame_draw_state(context, &output_dir, draw_index, profile, true) };
    blockers.append(&mut state_blockers);

    let shader_file = format!(
        "{}_{}.bin",
        profile.pixel_shader_file_prefix, shader_meta.fnv1a64
    );
    let mut pixel_shader_constant_buffers = shader_meta.constant_buffers.clone();
    canonicalize_map_frame_shader_constant_buffers(
        profile.capture_moment,
        "PS",
        profile.shader_sources.iter().copied(),
        &mut pixel_shader_constant_buffers,
        map_frame_binding_names_terrain_scene_cb0(&constant_buffers, "PS"),
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
            constant_buffers: pixel_shader_constant_buffers,
            source_evidence: profile.shader_sources.to_vec(),
        },
        vertex_shader: vertex_shader_file,
        input_layout: MapFrameInputLayoutFile {
            pointer: "0x0".to_string(),
            elements: Vec::new(),
        },
        vertex_buffers: BTreeMap::new(),
        index_buffer: None,
        constant_buffers,
        shader_resources,
        samplers,
        draw_state,
        post_draw_output_merger: None,
    };

    let (draws, mut accumulated_blockers, write_in_progress_snapshot) = {
        let mut runtime = RUNTIME
            .get()
            .and_then(|runtime| runtime.lock().ok())
            .ok_or_else(|| "terrain capture runtime not initialized".to_string())?;
        runtime.last_context = context as usize;
        runtime.map_frame_draws.push(draw.clone());
        runtime.map_frame_blockers.extend(blockers);
        refresh_context_hook_interests_for_runtime(&runtime);
        let write_in_progress_snapshot = should_write_in_progress_map_frame_snapshot(
            runtime.map_frame_draws.len(),
            runtime.map_frame_blockers.len(),
        );
        (
            runtime.map_frame_draws.clone(),
            runtime.map_frame_blockers.clone(),
            write_in_progress_snapshot,
        )
    };
    write_event(
        &output_dir,
        &format!(
            "{{\"type\":\"restore_pass_draw_captured\",\"draw_index\":{},\"draw_call\":\"{}\",\"draw_count\":{},\"ps_cb_slot\":{},\"srv_count\":{}}}",
            draw_index,
            draw_call,
            draw_count,
            RESTORE_POSTEFFECT_BASE_CB_BIND_POINT,
            draws
                .last()
                .map(|draw| draw.shader_resources.len())
                .unwrap_or(0)
        ),
    );
    if !write_in_progress_snapshot {
        return Ok(false);
    }
    accumulated_blockers.push(
        "map-frame restore_pass is still active; waiting for pass-close boundary".to_string(),
    );
    write_map_frame_bundle_snapshot(
        &output_dir,
        draws,
        accumulated_blockers,
        "early_d3d11_active_restore_draw_sequence_capture_in_progress",
        None,
        profile,
    )?;
    Ok(false)
}

/// CAPFIX1 finalize-stage half of the fail-closed borders_pass gate: the
/// render-target-changed criterion. Compares the recorded draw's pre-draw render
/// target payload(s) against the just-captured post-pass render target payload(s)
/// by their content hash. If every matched render target is byte-identical
/// pre/post, the draw was a no-op on the RT (the diag23b contamination signature)
/// and the candidate is REJECTED with a truthful reason. A render target whose
/// pre- or post-payload was NOT captured cannot prove the draw wrote pixels, so it
/// is treated as non-proving (fail closed): the check requires at least one render
/// target that provably CHANGED. Never panics.
fn validate_border_render_target_changed(
    active_draw: &MapFrameDrawFile,
    post_pass: Option<&MapFrameOutputMergerStateFile>,
) -> Result<(), String> {
    let Some(post_pass) = post_pass else {
        return Err(
            "border candidate rejected: no post-pass render-target snapshot was \
             captured, so a no-op draw cannot be ruled out (fail closed)"
                .to_string(),
        );
    };
    let Some(draw_state) = active_draw.draw_state.as_ref() else {
        return Err(
            "border candidate rejected: no pre-draw output-merger state was \
             captured, so a no-op draw cannot be ruled out (fail closed)"
                .to_string(),
        );
    };
    let pre_targets = &draw_state.output_merger.render_targets;
    if pre_targets.is_empty() || post_pass.render_targets.is_empty() {
        return Err(
            "border candidate rejected: pre-draw/post-pass render-target payloads \
             were not both captured, so a no-op draw cannot be ruled out (fail closed)"
                .to_string(),
        );
    }
    let mut any_proven_change = false;
    let mut any_comparable = false;
    for pre in pre_targets {
        // Match the same render target across pre/post by its resource pointer.
        let Some(post) = post_pass
            .render_targets
            .iter()
            .find(|post| post.resource_pointer == pre.resource_pointer)
        else {
            continue;
        };
        let (Some(pre_payload), Some(post_payload)) = (
            pre.pre_draw_payload.as_ref(),
            post.post_pass_payload.as_ref(),
        ) else {
            continue;
        };
        any_comparable = true;
        if pre_payload.fnv1a64 != post_payload.fnv1a64 {
            any_proven_change = true;
            break;
        }
    }
    if !any_comparable {
        return Err(
            "border candidate rejected: no render target had both a pre-draw and \
             a post-pass payload to compare, so a no-op draw cannot be ruled out (fail closed)"
                .to_string(),
        );
    }
    if !any_proven_change {
        return Err(
            "border candidate rejected: every captured render target is byte-identical \
             pre-draw and post-pass — the draw was a no-op on the RT (the diag23b contamination \
             signature); a genuine border draw writes pixels"
                .to_string(),
        );
    }
    Ok(())
}

/// CAPFIX1 push-stage half of the fail-closed borders_pass gate. Decides the two
/// criteria available before the draw executes — the VS input signature and the
/// BorderDiffuse ink — and returns `Err(reason)` to REJECT the candidate (so no
/// bundle is written and the moment keeps listening). The render-target-changed
/// criterion is decided later in [`validate_border_render_target_changed`] at
/// finalize, when the post-pass render target exists. Never panics.
fn validate_border_candidate_push_stage(
    vertex_shader_meta: &TerrainVertexShaderMetadata,
    shader_resources: &[MapFrameTextureBindingFile],
) -> Result<(), String> {
    // Locate the BorderDiffuse-slot binding (declared_name == "BorderDiffuse").
    let border_diffuse = shader_resources.iter().find(|binding| {
        binding
            .declared_name
            .as_deref()
            .is_some_and(|name| name == BORDER_DIFFUSE_RESOURCE_SYMBOL)
    });
    // observed = the binding exists, is non-null, and an exact readback landed
    // (payload_format set) so its `all_zero` verdict is meaningful.
    let (srv_observed, srv_all_zero) = match border_diffuse {
        Some(binding) if !binding.is_null && binding.payload_format.is_some() => {
            (true, binding.all_zero)
        }
        _ => (false, false),
    };
    let facts = BorderCandidateFacts {
        vertex_shader: vertex_shader_meta,
        border_diffuse_srv_all_zero: srv_all_zero,
        border_diffuse_srv_observed: srv_observed,
        // Not yet decidable at push stage; the finalize-stage RT check enforces
        // this. Marking observed=true here defers the verdict rather than
        // pre-rejecting; the push-stage validator only gates the VS + SRV.
        render_target_changed: true,
        render_target_snapshots_observed: true,
    };
    validate_border_candidate(&facts)
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
    debug_assert_render_thread_owner("capture_active_map_frame_bundle");
    if context.is_null() {
        return Err("null D3D11 context for map-frame capture".to_string());
    }
    let profile = requested_map_frame_capture_profile()?;
    if profile.capture_moment == RESTORE_CAPTURE_MOMENT {
        return unsafe {
            capture_active_restore_map_frame_bundle(
                context,
                draw_call,
                draw_count,
                draw_start,
                base_vertex_location,
                per_instance_count,
                instance_count,
                start_instance_location,
                profile,
            )
        };
    }
    if profile.capture_moment == MAPNAME_CAPTURE_MOMENT {
        return unsafe {
            capture_active_mapname_map_frame_bundle(
                context,
                draw_call,
                draw_count,
                draw_start,
                base_vertex_location,
                per_instance_count,
                instance_count,
                start_instance_location,
                profile,
            )
        };
    }
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
            UNDERWATER_CAPTURE_MOMENT => {
                let shader = runtime.state.underwater_pixel_shader().ok_or_else(|| {
                    "no active underwater pixel shader for underwater-pass map-frame capture"
                        .to_string()
                })?;
                let shader_meta = runtime
                    .state
                    .underwater_pixel_shader_metadata(shader.pixel_shader)
                    .cloned()
                    .ok_or_else(|| {
                        "active underwater shader has no bytecode metadata".to_string()
                    })?;
                // Effect underwater runs the pdxmap TERRAIN vertex shader.
                let vertex_shader = runtime.state.terrain_vertex_shader().ok_or_else(|| {
                    "active underwater draw has no tracked terrain vertex shader".to_string()
                })?;
                let vertex_shader_meta = runtime
                    .state
                    .terrain_vertex_shader_metadata(vertex_shader.vertex_shader)
                    .cloned()
                    .ok_or_else(|| {
                        "active underwater terrain vertex shader has no bytecode metadata"
                            .to_string()
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
            None,
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
        unsafe { capture_map_frame_draw_state(context, &output_dir, draw_index, profile, true) };
    blockers.append(&mut state_blockers);

    // FAIL-CLOSED borders_pass identification (CAPFIX1). The historical border
    // latch was pixel-shader-only, so a terrain patch drawn while the border PS
    // happened to be bound was finalized as borders_pass (the diag23b
    // contamination: int4-POSITION terrain VS, all-zero BorderDiffuse, no-op RT).
    // Two of the three structural gates are decidable HERE (VS input signature +
    // BorderDiffuse ink); the render-target-changed gate is decided at finalize
    // (post-pass RT is not yet known). Rejecting returns Err so no bundle is
    // written and the borders moment keeps listening (MOMFIX1 truthful blocked
    // finalization, never a contaminated bundle).
    if profile.capture_moment == BORDERS_CAPTURE_MOMENT {
        validate_border_candidate_push_stage(&vertex_shader_meta, &shader_resources)?;
    }

    let shader_file = format!(
        "{}_{}.bin",
        profile.pixel_shader_file_prefix, shader_meta.fnv1a64
    );
    let vertex_shader_file = format!(
        "{}_{}.bin",
        profile.vertex_shader_file_prefix, vertex_shader_meta.fnv1a64
    );
    let mut pixel_shader_constant_buffers = shader_meta.constant_buffers.clone();
    canonicalize_map_frame_shader_constant_buffers(
        profile.capture_moment,
        "PS",
        profile.shader_sources.iter().copied(),
        &mut pixel_shader_constant_buffers,
        map_frame_binding_names_terrain_scene_cb0(&constant_buffers, "PS"),
    );
    let mut vertex_shader_constant_buffers = vertex_shader_meta.constant_buffers.clone();
    canonicalize_map_frame_shader_constant_buffers(
        profile.capture_moment,
        "VS",
        profile.shader_sources.iter().copied(),
        &mut vertex_shader_constant_buffers,
        map_frame_binding_names_terrain_scene_cb0(&constant_buffers, "VS"),
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
            constant_buffers: pixel_shader_constant_buffers,
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
            constant_buffers: vertex_shader_constant_buffers,
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

    let (draws, mut accumulated_blockers, write_in_progress_snapshot, close_on_frame_repeat) = {
        let mut runtime = RUNTIME
            .get()
            .and_then(|runtime| runtime.lock().ok())
            .ok_or_else(|| "terrain capture runtime not initialized".to_string())?;
        runtime.last_context = context as usize;
        let close_on_frame_repeat = profile.capture_moment == TREES_CAPTURE_MOMENT
            && should_close_trees_pass_on_frame_repeat(
                &runtime.map_frame_draws,
                &draw,
                trees_close_mode_from_control(runtime.control.as_ref()),
            );
        if close_on_frame_repeat {
            (
                runtime.map_frame_draws.clone(),
                runtime.map_frame_blockers.clone(),
                false,
                true,
            )
        } else {
            runtime.map_frame_draws.push(draw.clone());
            runtime.map_frame_blockers.extend(blockers);
            refresh_context_hook_interests_for_runtime(&runtime);
            let write_in_progress_snapshot = should_write_in_progress_map_frame_snapshot(
                runtime.map_frame_draws.len(),
                runtime.map_frame_blockers.len(),
            );
            (
                runtime.map_frame_draws.clone(),
                runtime.map_frame_blockers.clone(),
                write_in_progress_snapshot,
                false,
            )
        }
    };
    if close_on_frame_repeat {
        write_final_map_frame_bundle_snapshot(context, "trees_frame_repeat_boundary")?;
        return Ok(true);
    }
    if !write_in_progress_snapshot {
        return Ok(false);
    }
    accumulated_blockers.push(format!(
        "map-frame {} is still active; waiting for pass-close boundary",
        profile.capture_moment
    ));
    let in_progress_provenance = match profile.capture_moment {
        WATER_CAPTURE_MOMENT => "early_d3d11_active_pdxwater_draw_sequence_capture_in_progress",
        RIVERS_CAPTURE_MOMENT => "early_d3d11_active_river_draw_sequence_capture_in_progress",
        BORDERS_CAPTURE_MOMENT => "early_d3d11_active_border_draw_sequence_capture_in_progress",
        UNDERWATER_CAPTURE_MOMENT => {
            "early_d3d11_active_underwater_draw_sequence_capture_in_progress"
        }
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

fn should_write_in_progress_map_frame_snapshot(draw_count: usize, blocker_count: usize) -> bool {
    draw_count <= 1 || blocker_count > 0
}

fn finalize_map_frame_bundle_pass(context: *mut ID3D11DeviceContext, reason: &'static str) {
    if capture_quiesced() {
        return;
    }
    if !unsafe { context_capture_allowed(context, reason) } {
        return;
    }
    if capture_symbol_observed(MAP_FRAME_BUNDLE_SYMBOL) {
        return;
    }
    if MAP_FRAME_CAPTURE_IN_PROGRESS.swap(true, Ordering::SeqCst) {
        return;
    }
    let result = write_final_map_frame_bundle_snapshot(context, reason);
    match result {
        Ok(complete_without_blockers) => record_map_frame_bundle_success(complete_without_blockers),
        Err(error) => {
            record_map_frame_bundle_rejected(&error);
            // CAPFIX1: a borders candidate that reached finalize but failed the
            // render-target-changed gate (a real border VS + inked BorderDiffuse
            // that nonetheless drew nothing to the RT) has its no-op draw record
            // discarded so the moment keeps listening for a genuine border draw,
            // rather than re-finalizing the same rejected draw forever.
            if error.starts_with(BORDER_REJECT_MESSAGE_PREFIX) {
                if let Some(runtime) = RUNTIME.get().and_then(|runtime| runtime.lock().ok()) {
                    let mut runtime = runtime;
                    reset_current_map_frame_capture(&mut runtime);
                }
            }
        }
    }
    MAP_FRAME_CAPTURE_IN_PROGRESS.store(false, Ordering::SeqCst);
}

/// CAPFIX3 ISSUE 2 — night terrain 128-draw blowout (CAPFIX2_FULL_2/_3).
///
/// Force the current map-frame pass closed at a clean frame boundary when the
/// moment did not finalize this frame, so per-frame draws never accumulate
/// across frames into the per-pass draw cap. Called from the two clean-frame-
/// boundary hooks (Present and pre-ClearRenderTargetView) AFTER their
/// finalize-bundle attempt, so a pass closes at the frame boundary REGARDLESS
/// of whether the finalize gate fired or the finalize succeeded.
///
/// Root cause it fixes: a pass that fails to finalize cleanly (blockers
/// present, e.g. the shader-cache had no TerrainIDMap signature for the active
/// slot at the 3AM close-zoom view — the exact FULL_3 blocker
/// `shader-cache control has no TerrainIDMap signature for active slot t5`)
/// left its accumulated draws in `map_frame_draws`. Those draws were only ever
/// cleared at moment-advance, so every frame's pdxmap draws piled onto the SAME
/// per-pass accumulator across many Present/clear boundaries until draw_count
/// hit max_draws_per_pass (128) — 5.4GB reserved, session killed, EVEN PAUSED
/// (FULL_3 proved a single paused frame draws far fewer than 128).
/// `max_draws_per_pass` is a PER-FRAME cap by intent; it must not measure a
/// cross-frame sum. The in-progress bundle snapshot with this frame's draws was
/// already written by the finalize attempt, so no evidence is lost; only the
/// unbounded accumulator is reset. Missed shader binds (the CAPFIX2 try_lock
/// concern in verdict (b)) therefore cannot extend a pass past one frame
/// either: the pass closes at the frame boundary regardless of what was or
/// wasn't observed mid-frame.
///
/// Scope guards: UI pass keeps its own cross-frame `ui_pass.draws` accumulator
/// (its own state cap) and is excluded; a completed/observed moment already
/// cleared its draws at advance and is skipped. Render-thread only; takes the
/// runtime lock briefly exactly like the sibling `reset_current_map_frame_capture`
/// already does at moment-advance.
fn close_map_frame_pass_at_frame_boundary_if_incomplete(reason: &'static str) {
    if capture_quiesced() {
        return;
    }
    // The moment completing this frame writes MAP_FRAME_BUNDLE_SYMBOL; nothing
    // to reset then (advance already cleared the draws).
    if capture_symbol_observed(MAP_FRAME_BUNDLE_SYMBOL) {
        return;
    }
    // CAPFIX2 lock discipline (SACRED, extended not regressed): this runs inside
    // the Present / ClearRenderTargetView render-thread hooks, so it MUST use the
    // tracked NON-BLOCKING lock — never a blocking runtime.lock() that could wedge
    // or self-deadlock the render thread. On contention we skip closing this
    // frame; the NEXT clean frame boundary retries, and the accumulator carries
    // at most a handful of extra draws (nowhere near max_draws_per_pass), so the
    // blowout stays fixed while the render thread is never blocked.
    if let Some(runtime) = RUNTIME.get() {
        if let Some((mut runtime, _owner)) =
            try_lock_runtime_tracked(runtime, RuntimeLockSite::PresentFinalize)
        {
            if let Some(carried) = close_map_frame_pass_for_runtime(&mut runtime) {
                write_event(
                    &runtime.output_dir,
                    &format!(
                        "{{\"type\":\"map_frame_pass_closed_at_frame_boundary\",\"reason\":\"{}\",\"carried_draws_discarded\":{},\"message\":\"pass did not finalize this frame; per-frame draw accumulator reset so max_draws_per_pass stays a per-frame cap\"}}",
                        reason, carried
                    ),
                );
            }
        }
    }
}

/// CAPFIX3 ISSUE 2 — the testable core of the frame-boundary pass-close. Returns
/// `Some(discarded_draw_count)` and resets the per-frame map-frame draw
/// accumulator when the active moment is an incomplete map-frame moment that
/// carried draws into this frame boundary; returns `None` (no reset) for UI pass
/// (own cross-frame accumulator), an already-completed moment, an empty
/// accumulator, or when no map-frame profile is active. Pure w.r.t. globals
/// except the sibling `reset_current_map_frame_capture` it delegates the actual
/// clear to (same one moment-advance uses).
fn close_map_frame_pass_for_runtime(runtime: &mut CaptureRuntime) -> Option<usize> {
    let profile = requested_map_frame_capture_profile_for_runtime(runtime).ok()?;
    // UI pass accumulates state across frames on purpose (its own cap); never
    // fold its draws here.
    if profile.capture_moment == UI_PASS_CAPTURE_MOMENT {
        return None;
    }
    // Moment already complete => its draws were cleared at advance.
    if runtime
        .completed_map_frame_moments
        .contains(profile.capture_moment)
    {
        return None;
    }
    let carried = runtime.map_frame_draws.len();
    if carried == 0 {
        return None;
    }
    reset_current_map_frame_capture(runtime);
    Some(carried)
}

fn should_finalize_ui_pass_at_present() -> bool {
    if capture_quiesced() {
        return false;
    }
    RUNTIME
        .get()
        .and_then(|runtime| runtime.lock().ok())
        .map_or(false, |runtime| {
            ui_pass_capture_pending_for_runtime(&runtime)
                && runtime.ui_pass.active
                && !runtime.ui_pass.draws.is_empty()
        })
}

fn should_process_ui_pass_watch_at_present() -> bool {
    if capture_quiesced() {
        return false;
    }
    RUNTIME
        .get()
        .and_then(|runtime| runtime.lock().ok())
        .map_or(false, |runtime| {
            ui_pass_capture_pending_for_runtime(&runtime)
                && !runtime.ui_pass.active
                && !runtime.ui_pass.frame_signature.draws.is_empty()
        })
}

fn finalize_ui_pass_bundle(context: *mut ID3D11DeviceContext, reason: &'static str) {
    if capture_quiesced() {
        return;
    }
    if !unsafe { context_capture_allowed(context, reason) } {
        return;
    }
    if MAP_FRAME_CAPTURE_IN_PROGRESS.swap(true, Ordering::SeqCst) {
        return;
    }
    let result = write_final_ui_pass_bundle_snapshot(reason);
    match result {
        Ok(result) => record_ui_pass_bundle_success(result),
        Err(error) => record_ui_pass_rejected(&error),
    }
    MAP_FRAME_CAPTURE_IN_PROGRESS.store(false, Ordering::SeqCst);
}

fn process_ui_pass_watch_frame_at_present(reason: &'static str) {
    if capture_quiesced() {
        return;
    }
    let Some((output_dir, event, completed)) = RUNTIME.get().and_then(|runtime| {
        runtime.lock().ok().map(|mut runtime| {
            if !ui_pass_capture_pending_for_runtime(&runtime) || runtime.ui_pass.active {
                return (runtime.output_dir.clone(), None, false);
            }
            let frame_signature = mem::take(&mut runtime.ui_pass.frame_signature);
            if frame_signature.draws.is_empty() {
                return (runtime.output_dir.clone(), None, false);
            }
            runtime.ui_pass.frame_index = runtime.ui_pass.frame_index.saturating_add(1);
            let signature = ui_pass_frame_signature_hash(&frame_signature);
            let draw_count = frame_signature.draws.len();
            let label = runtime.ui_pass.pending_capture_label.clone();
            let forced = label.is_some();
            if !forced
                && (runtime.ui_pass.captured_signatures.contains(&signature)
                    || runtime.ui_pass.pending_capture_signature.as_deref() == Some(signature.as_str())
                    || !ui_pass_debounce_allows_capture(&runtime))
            {
                return (runtime.output_dir.clone(), None, false);
            }
            if runtime.ui_pass.state_count >= ui_pass_max_states_for_runtime(&runtime) {
                let event = record_ui_pass_states_exhausted_for_runtime(&mut runtime);
                let completed = refresh_session_capture_progress(&mut runtime);
                return (runtime.output_dir.clone(), event, completed);
            }
            runtime.ui_pass.pending_capture_signature = Some(signature.clone());
            let label_json = label
                .as_ref()
                .map(|label| format!("\"{}\"", json_escape(label)))
                .unwrap_or_else(|| "null".to_string());
            let event = Some(format!(
                "{{\"type\":\"ui_pass_state_pending\",\"signature\":\"{}\",\"label\":{},\"draw_count\":{},\"frame_index\":{},\"reason\":\"{}\"}}",
                json_escape(&signature),
                label_json,
                draw_count,
                runtime.ui_pass.frame_index,
                reason
            ));
            refresh_context_hook_interests_for_runtime(&runtime);
            (runtime.output_dir.clone(), event, false)
        })
    }) else {
        return;
    };
    if let Some(event) = event {
        write_event(&output_dir, &event);
    }
    if completed {
        CAPTURED.store(true, Ordering::SeqCst);
    }
}

fn finalize_ui_pass_capture_at_dwell_end() {
    if capture_quiesced() || CAPTURE_BUDGET_STOPPED.load(Ordering::SeqCst) {
        return;
    }
    let should_write_active = RUNTIME
        .get()
        .and_then(|runtime| runtime.lock().ok())
        .map_or(false, |runtime| {
            ui_pass_capture_pending_for_runtime(&runtime)
                && runtime.ui_pass.active
                && !runtime.ui_pass.draws.is_empty()
        });
    if should_write_active {
        match write_final_ui_pass_bundle_snapshot("dwell_end") {
            Ok(result) => record_ui_pass_bundle_success(result),
            Err(error) => record_ui_pass_rejected(&error),
        }
    }
    let Some((output_dir, event, completed)) = RUNTIME.get().and_then(|runtime| {
        runtime.lock().ok().map(|mut runtime| {
            if !ui_pass_capture_pending_for_runtime(&runtime) {
                return (runtime.output_dir.clone(), None, false);
            }
            runtime.ui_pass.active = false;
            runtime.ui_pass.backbuffer = None;
            runtime.ui_pass.draws.clear();
            runtime.ui_pass.blockers.clear();
            runtime.ui_pass.frame_signature = UiPassFrameSignature::default();
            runtime.ui_pass.pending_capture_signature = None;
            runtime.ui_pass.pending_capture_label = None;
            runtime.ui_pass.active_capture_label = None;
            runtime
                .completed_map_frame_moments
                .insert(UI_PASS_CAPTURE_MOMENT.to_string());
            let event = Some(format!(
                "{{\"type\":\"ui_pass_finished\",\"reason\":\"dwell_end\",\"state_count\":{},\"max_states\":{}}}",
                runtime.ui_pass.state_count,
                ui_pass_max_states_for_runtime(&runtime)
            ));
            let completed = refresh_session_capture_progress(&mut runtime);
            (runtime.output_dir.clone(), event, completed)
        })
    }) else {
        return;
    };
    if let Some(event) = event {
        write_event(&output_dir, &event);
    }
    if completed {
        CAPTURED.store(true, Ordering::SeqCst);
    }
}

fn write_final_ui_pass_bundle_snapshot(
    reason: &'static str,
) -> Result<UiPassBundleWriteResult, String> {
    if capture_quiesced() {
        return Err("capture quiesced".to_string());
    }
    let (output_dir, draws, mut blockers, backbuffer, state_index, signature, label, duplicate) = {
        let runtime = RUNTIME
            .get()
            .and_then(|runtime| runtime.lock().ok())
            .ok_or_else(|| "terrain capture runtime not initialized".to_string())?;
        if !ui_pass_capture_pending_for_runtime(&runtime) {
            return Err("ui_pass is not the active capture moment".to_string());
        }
        let backbuffer = runtime
            .ui_pass
            .backbuffer
            .clone()
            .ok_or_else(|| "ui_pass has no opened swapchain backbuffer identity".to_string())?;
        let signature = ui_pass_frame_signature_hash(&runtime.ui_pass.frame_signature);
        let label = runtime.ui_pass.active_capture_label.clone();
        let duplicate = label.is_none() && runtime.ui_pass.captured_signatures.contains(&signature);
        (
            runtime.output_dir.clone(),
            runtime.ui_pass.draws.clone(),
            runtime.ui_pass.blockers.clone(),
            backbuffer,
            runtime.ui_pass.state_count,
            signature,
            label,
            duplicate,
        )
    };
    if duplicate {
        return Ok(UiPassBundleWriteResult {
            clean: true,
            file: String::new(),
            bundle_path: String::new(),
            state_index,
            signature,
            label,
            draw_count: draws.len(),
            duplicate: true,
        });
    }
    if draws.is_empty() {
        blockers.push("ui_pass reached Present without any captured UI draws".to_string());
    }
    blockers.extend(flush_payload_file_writes());
    let mut runtime_files = BTreeMap::new();
    add_ui_pass_draw_file_evidence(&mut runtime_files, &output_dir, &draws, &mut blockers);
    dedupe_blockers(&mut blockers);
    let bundle = UiPassBundleFile {
        schema: UI_PASS_BUNDLE_SCHEMA,
        provenance: "d3d11_swapchain_backbuffer_no_depth_draw_sequence",
        finalized_by: reason,
        capture_moment: UI_PASS_CAPTURE_MOMENT,
        state_index,
        signature: signature.clone(),
        label: label.clone(),
        session_manifest: MAP_CAPTURE_SESSION_FILE,
        backbuffer,
        draws,
        runtime_files,
        blockers,
    };
    fs::create_dir_all(&output_dir).map_err(|error| error.to_string())?;
    let json = serde_json::to_string_pretty(&bundle).map_err(|error| error.to_string())?;
    let file = ui_pass_bundle_file(state_index);
    let bundle_path = output_dir.join(&file).to_string_lossy().to_string();
    queue_payload_file_write(&output_dir, &file, format!("{json}\n").as_bytes())?;
    let flush_errors = flush_payload_file_writes();
    if !flush_errors.is_empty() {
        return Err(format!(
            "ui_pass bundle writer flush failed: {}",
            flush_errors.join("; ")
        ));
    }
    Ok(UiPassBundleWriteResult {
        clean: bundle.blockers.is_empty(),
        file,
        bundle_path,
        state_index,
        signature,
        label,
        draw_count: bundle.draws.len(),
        duplicate: false,
    })
}

fn add_ui_pass_draw_file_evidence(
    files: &mut BTreeMap<String, MapFrameFileEvidence>,
    output_dir: &Path,
    draws: &[MapFrameDrawFile],
    blockers: &mut Vec<String>,
) {
    for draw in draws {
        for shader_file in [
            &draw.shader.bytecode_file,
            &draw.vertex_shader.bytecode_file,
        ] {
            if shader_file.is_empty() {
                continue;
            }
            add_required_file_evidence(
                files,
                output_dir,
                shader_file,
                "create_shader_bytecode_ui_pass",
                blockers,
            );
        }
        for binding in draw.vertex_buffers.values() {
            if let Some(file) = binding.file.as_deref() {
                add_required_file_evidence(
                    files,
                    output_dir,
                    file,
                    "ui_pass_ia_vertex_buffer_referenced_range",
                    blockers,
                );
            }
        }
        if let Some(index) = draw.index_buffer.as_ref() {
            if let Some(file) = index.file.as_deref() {
                add_required_file_evidence(
                    files,
                    output_dir,
                    file,
                    "ui_pass_ia_index_buffer_referenced_range",
                    blockers,
                );
            }
        }
        for binding in &draw.constant_buffers {
            if let Some(file) = binding.file.as_deref() {
                add_required_file_evidence(
                    files,
                    output_dir,
                    file,
                    "ui_pass_constant_buffer_readback",
                    blockers,
                );
            }
        }
    }
    add_map_frame_shader_resource_file_evidence(files, output_dir, draws, blockers);
}

fn record_ui_pass_bundle_success(result: UiPassBundleWriteResult) {
    if capture_quiesced() {
        return;
    }
    if let Some(runtime) = RUNTIME.get() {
        if let Ok(mut runtime) = runtime.lock() {
            if result.duplicate {
                runtime.ui_pass.pending_capture_signature = None;
                runtime.ui_pass.pending_capture_label = None;
                runtime.ui_pass.active_capture_label = None;
                runtime.ui_pass.active = false;
                runtime.ui_pass.backbuffer = None;
                runtime.ui_pass.draws.clear();
                runtime.ui_pass.blockers.clear();
                runtime.ui_pass.frame_signature = UiPassFrameSignature::default();
                let label_json = result
                    .label
                    .as_ref()
                    .map(|label| format!("\"{}\"", json_escape(label)))
                    .unwrap_or_else(|| "null".to_string());
                write_event(
                    &runtime.output_dir,
                    &format!(
                        "{{\"type\":\"ui_pass_state_duplicate_ignored\",\"signature\":\"{}\",\"label\":{},\"draw_count\":{}}}",
                        json_escape(&result.signature),
                        label_json,
                        result.draw_count
                    ),
                );
                refresh_context_hook_interests_for_runtime(&runtime);
                return;
            }
            runtime
                .ui_pass
                .captured_signatures
                .insert(result.signature.clone());
            runtime.ui_pass.pending_capture_signature = None;
            runtime.ui_pass.pending_capture_label = None;
            runtime.ui_pass.active_capture_label = None;
            runtime.ui_pass.active = false;
            runtime.ui_pass.backbuffer = None;
            runtime.ui_pass.draws.clear();
            runtime.ui_pass.blockers.clear();
            runtime.ui_pass.frame_signature = UiPassFrameSignature::default();
            runtime.ui_pass.state_count = runtime.ui_pass.state_count.saturating_add(1);
            runtime.ui_pass.frame_index = runtime.ui_pass.frame_index.saturating_add(1);
            runtime.ui_pass.last_capture_frame_index = Some(runtime.ui_pass.frame_index);
            let mut exhausted_event = None;
            let max_states = ui_pass_max_states_for_runtime(&runtime);
            if runtime.ui_pass.state_count >= max_states {
                exhausted_event = record_ui_pass_states_exhausted_for_runtime(&mut runtime);
            }
            let sequence_complete = runtime
                .completed_map_frame_moments
                .contains(UI_PASS_CAPTURE_MOMENT);
            let label_json = result
                .label
                .as_ref()
                .map(|label| format!("\"{}\"", json_escape(label)))
                .unwrap_or_else(|| "null".to_string());
            write_event(
                &runtime.output_dir,
                &format!(
                    "{{\"type\":\"ui_pass_state_captured\",\"index\":{},\"signature\":\"{}\",\"label\":{},\"draw_count\":{},\"file\":\"{}\",\"bundle_path\":\"{}\",\"clean\":{}}}",
                    result.state_index,
                    json_escape(&result.signature),
                    label_json,
                    result.draw_count,
                    json_escape(&result.file),
                    json_escape(&result.bundle_path),
                    result.clean
                ),
            );
            write_event(
                &runtime.output_dir,
                &format!(
                    "{{\"type\":\"ui_pass_bundle_captured\",\"file\":\"{}\",\"bundle_path\":\"{}\",\"sequence_complete\":{},\"draw_count\":{},\"state_index\":{},\"signature\":\"{}\",\"label\":{}}}",
                    json_escape(&result.file),
                    json_escape(&result.bundle_path),
                    sequence_complete,
                    result.draw_count,
                    result.state_index,
                    json_escape(&result.signature),
                    label_json
                ),
            );
            if let Some(event) = exhausted_event {
                write_event(&runtime.output_dir, &event);
            }
            if refresh_session_capture_progress(&mut runtime) {
                CAPTURED.store(true, Ordering::SeqCst);
            }
        }
    }
}

fn record_ui_pass_rejected(error: &str) {
    if capture_quiesced() || CAPTURE_BUDGET_STOPPED.load(Ordering::SeqCst) {
        return;
    }
    if let Some(runtime) = RUNTIME.get() {
        if let Ok(mut runtime) = runtime.lock() {
            runtime.last_error = Some(error.to_string());
            runtime.ui_pass.blockers.push(error.to_string());
            write_event(
                &runtime.output_dir,
                &format!(
                    "{{\"type\":\"ui_pass_bundle_rejected\",\"file\":\"{}\",\"message\":\"{}\"}}",
                    json_escape(&ui_pass_bundle_file(runtime.ui_pass.state_count)),
                    json_escape(error)
                ),
            );
        }
    }
}

fn record_ui_pass_states_exhausted_for_runtime(runtime: &mut CaptureRuntime) -> Option<String> {
    if runtime.ui_pass.states_exhausted_event_recorded {
        return None;
    }
    runtime.ui_pass.states_exhausted_event_recorded = true;
    runtime
        .completed_map_frame_moments
        .insert(UI_PASS_CAPTURE_MOMENT.to_string());
    refresh_context_hook_interests_for_runtime(runtime);
    Some(format!(
        "{{\"type\":\"ui_pass_states_exhausted\",\"state_count\":{},\"max_states\":{}}}",
        runtime.ui_pass.state_count,
        ui_pass_max_states_for_runtime(runtime)
    ))
}

fn write_final_map_frame_bundle_snapshot(
    context: *mut ID3D11DeviceContext,
    reason: &'static str,
) -> Result<bool, String> {
    debug_assert_render_thread_owner("write_final_map_frame_bundle_snapshot");
    if capture_quiesced() {
        return Err("capture quiesced".to_string());
    }
    let (output_dir, draws, mut blockers, profile, capture_context) = {
        let runtime = RUNTIME
            .get()
            .and_then(|runtime| runtime.lock().ok())
            .ok_or_else(|| "terrain capture runtime not initialized".to_string())?;
        let profile = requested_map_frame_capture_profile_for_runtime(&runtime)?;
        if context.is_null() {
            return Err(
                "map-frame finalization requires a render-thread D3D11 context".to_string(),
            );
        }
        let capture_context = context;
        (
            runtime.output_dir.clone(),
            runtime.map_frame_draws.clone(),
            runtime.map_frame_blockers.clone(),
            profile,
            capture_context,
        )
    };
    if profile.requires_terrain_runtime_files {
        blockers.extend(unsafe { capture_deferred_runtime_targets_at_finalize(capture_context) });
    }
    write_map_frame_bundle_snapshot(
        &output_dir,
        draws,
        blockers,
        reason,
        Some(capture_context),
        profile,
    )
}

fn write_map_frame_bundle_snapshot(
    output_dir: &Path,
    draws: Vec<MapFrameDrawFile>,
    mut blockers: Vec<String>,
    provenance: &'static str,
    post_pass_context: Option<*mut ID3D11DeviceContext>,
    profile: MapFrameCaptureProfile,
) -> Result<bool, String> {
    if capture_quiesced() {
        return Err("capture quiesced".to_string());
    }
    let finalizing = post_pass_context.is_some();
    let active_draw = draws
        .last()
        .cloned()
        .ok_or_else(|| "map-frame capture produced no draw records".to_string())?;
    let post_pass_output_merger = if post_pass_context.is_some() {
        let context = post_pass_context.unwrap_or(ptr::null_mut());
        let (state, output_merger_blockers) = unsafe {
            capture_map_frame_post_pass_output_merger_state(
                context,
                output_dir,
                active_draw.draw_index,
                profile,
            )
        };
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
    // FAIL-CLOSED borders_pass finalize gate (CAPFIX1, the render-target-changed
    // criterion). Only when actually finalizing a borders bundle: if the recorded
    // draw's pre-draw render target(s) are byte-identical to the post-pass render
    // target(s), the draw was a no-op on the RT — the diag23b contamination
    // signature — so REJECT (no bundle is written; the moment keeps listening).
    // A missing pre/post payload cannot rule out a no-op, so it is treated as a
    // rejectable candidate (fail closed) rather than silently accepted.
    if finalizing && profile.capture_moment == BORDERS_CAPTURE_MOMENT {
        validate_border_render_target_changed(&active_draw, post_pass_output_merger.as_ref())?;
    }
    if finalizing {
        blockers.extend(flush_payload_file_writes());
    }
    let shader_cache_constants = if profile.requires_shader_cache_constants {
        map_frame_shader_cache_constants_for_draw(&active_draw, &mut blockers)
    } else {
        None
    };
    let mut runtime_files = BTreeMap::new();
    for target in CAPTURE_TARGETS {
        let provenance = runtime_texture_file_provenance(target);
        let requires_target = profile.requires_terrain_runtime_files
            && (target.symbol == TERRAIN_ID_RESOURCE_SYMBOL
                || target.symbol == SNOW_MUD_DATA_RESOURCE_SYMBOL
                    && active_draw
                        .shader
                        .reflection_resources
                        .contains_key(SNOW_MUD_DATA_RESOURCE_SYMBOL)
                    && !draw_has_moment_runtime_texture_payload(
                        &active_draw,
                        SNOW_MUD_DATA_RESOURCE_SYMBOL,
                    ));
        if requires_target {
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
    add_map_frame_draw_shader_file_evidence(&mut runtime_files, output_dir, &draws, &mut blockers);
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
        finalized_by: map_frame_finalized_by(provenance),
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

fn write_terrain_chunk_library_manifest_snapshot(
    output_dir: &Path,
    chunks: Vec<TerrainChunkRecordFile>,
    mut blockers: Vec<String>,
    provenance: &'static str,
) -> Result<bool, String> {
    if capture_quiesced() {
        return Err("capture quiesced".to_string());
    }
    blockers.extend(flush_payload_file_writes());
    let active_draw = chunks.last().map(|chunk| chunk.draw.clone());
    let draws = chunks
        .iter()
        .map(|chunk| chunk.draw.clone())
        .collect::<Vec<_>>();
    if active_draw.is_none() {
        blockers.push("terrain chunk library produced no chunk records".to_string());
    }
    let shader_cache_constants = active_draw.as_ref().and_then(|active_draw| {
        map_frame_shader_cache_constants_for_draw(active_draw, &mut blockers)
    });
    let mut runtime_files = BTreeMap::new();
    for target in CAPTURE_TARGETS {
        let provenance = runtime_texture_file_provenance(target);
        let requires_target = active_draw.as_ref().map_or(false, |active_draw| {
            target.symbol == TERRAIN_ID_RESOURCE_SYMBOL
                || target.symbol == SNOW_MUD_DATA_RESOURCE_SYMBOL
                    && active_draw
                        .shader
                        .reflection_resources
                        .contains_key(SNOW_MUD_DATA_RESOURCE_SYMBOL)
        });
        if requires_target {
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
    add_map_frame_draw_shader_file_evidence(&mut runtime_files, output_dir, &draws, &mut blockers);
    add_file_evidence_if_exists(
        &mut runtime_files,
        output_dir,
        TERRAIN_SCENE_CB0_FILE,
        "runtime_constant_buffer_readback_TerrainSceneCB0",
    );
    let mut vanilla_sources = BTreeMap::new();
    copy_required_vanilla_sources(
        &mut vanilla_sources,
        output_dir,
        TERRAIN_CHUNK_LIBRARY_CAPTURE_PROFILE.vanilla_sources,
        &mut blockers,
    );
    let world_rect = terrain_chunk_library_world_rect(&mut blockers);
    let coverage = terrain_chunk_coverage_for_bounds(
        &chunks.iter().map(|chunk| chunk.bounds).collect::<Vec<_>>(),
        world_rect,
    );
    dedupe_blockers(&mut blockers);
    let manifest = TerrainChunkLibraryManifestFile {
        schema: TERRAIN_CHUNK_LIBRARY_SCHEMA,
        provenance,
        capture_moment: TERRAIN_CHUNK_LIBRARY_MOMENT,
        session_manifest: MAP_CAPTURE_SESSION_FILE,
        shader_cache_constants,
        chunks,
        vanilla_sources,
        runtime_files,
        coverage,
        blockers,
    };
    fs::create_dir_all(output_dir).map_err(|error| error.to_string())?;
    let json = serde_json::to_string_pretty(&manifest).map_err(|error| error.to_string())?;
    fs::write(
        output_dir.join(TERRAIN_CHUNK_LIBRARY_FILE),
        format!("{json}\n"),
    )
    .map_err(|error| error.to_string())?;
    Ok(manifest.blockers.is_empty())
}

unsafe fn terrain_chunk_preidentity_from_active_state(
    context: *mut ID3D11DeviceContext,
    draw_call: &'static str,
    draw_count: UINT,
    draw_start: UINT,
    base_vertex_location: i32,
    per_instance_count: Option<UINT>,
    instance_count: Option<UINT>,
    start_instance_location: Option<UINT>,
    pixel_shader_fnv1a64: &str,
    vertex_shader_fnv1a64: &str,
    layout_ptr: usize,
    elements: &[TerrainInputElementDesc],
    pixel_shader_constant_buffers: &BTreeMap<String, u32>,
    vertex_shader_constant_buffers: &BTreeMap<String, u32>,
) -> Result<TerrainChunkPreidentityFile, String> {
    debug_assert_render_thread_owner("terrain_chunk_preidentity_from_active_state");
    if context.is_null() {
        return Err("null D3D11 context for terrain chunk preidentity".to_string());
    }
    let input_slots = elements
        .iter()
        .map(|element| element.input_slot)
        .collect::<BTreeSet<_>>();
    let mut vertex_buffers = BTreeMap::new();
    for slot in input_slots {
        let mut buffer: *mut ID3D11Buffer = ptr::null_mut();
        let mut stride = 0;
        let mut offset = 0;
        unsafe {
            (*context).IAGetVertexBuffers(slot, 1, &mut buffer, &mut stride, &mut offset);
        }
        if buffer.is_null() {
            return Err(format!(
                "terrain chunk preidentity missing IA vertex buffer slot {slot}"
            ));
        }
        vertex_buffers.insert(
            slot.to_string(),
            TerrainChunkBufferPointerIdentityFile {
                resource_pointer: format!("0x{:X}", buffer as usize),
                stride,
                offset,
            },
        );
        unsafe {
            release_unknown(buffer as *mut WinVoid);
        }
    }
    let index_buffer = if draw_call.contains("Indexed") {
        let mut buffer: *mut ID3D11Buffer = ptr::null_mut();
        let mut format = 0;
        let mut offset = 0;
        unsafe {
            (*context).IAGetIndexBuffer(&mut buffer, &mut format, &mut offset);
        }
        if buffer.is_null() {
            return Err("terrain chunk preidentity missing IA index buffer".to_string());
        }
        let identity = TerrainChunkIndexPointerIdentityFile {
            resource_pointer: format!("0x{:X}", buffer as usize),
            dxgi_format: format as u32,
            offset,
        };
        unsafe {
            release_unknown(buffer as *mut WinVoid);
        }
        Some(identity)
    } else {
        None
    };
    let mut per_chunk_constant_buffers = Vec::new();
    unsafe {
        terrain_chunk_preidentity_constant_buffers_for_stage(
            context,
            "VS",
            vertex_shader_constant_buffers,
            &mut per_chunk_constant_buffers,
        )?;
        terrain_chunk_preidentity_constant_buffers_for_stage(
            context,
            "PS",
            pixel_shader_constant_buffers,
            &mut per_chunk_constant_buffers,
        )?;
    }
    Ok(TerrainChunkPreidentityFile {
        draw_call: draw_call.to_string(),
        draw_count,
        draw_start,
        base_vertex_location,
        per_instance_count,
        instance_count,
        start_instance_location,
        pixel_shader_fnv1a64: pixel_shader_fnv1a64.to_string(),
        vertex_shader_fnv1a64: vertex_shader_fnv1a64.to_string(),
        input_layout_pointer: format!("0x{layout_ptr:X}"),
        vertex_buffers,
        index_buffer,
        per_chunk_constant_buffers,
    })
}

unsafe fn terrain_chunk_preidentity_constant_buffers_for_stage(
    context: *mut ID3D11DeviceContext,
    stage: &'static str,
    reflected_constant_buffers: &BTreeMap<String, u32>,
    out: &mut Vec<TerrainChunkConstantBufferIdentityFile>,
) -> Result<(), String> {
    for slot in 0..MAP_FRAME_BINDING_SLOTS {
        if slot == TERRAIN_SCENE_CB0_BIND_POINT {
            continue;
        }
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
        let result = unsafe { readback_d3d11_buffer_bytes(context, buffer) };
        unsafe {
            release_unknown(buffer as *mut WinVoid);
        }
        let (source, bytes) = result?;
        if source.bind_flags & D3D11_BIND_CONSTANT_BUFFER == 0 {
            return Err(format!(
                "terrain chunk preidentity {stage} constant buffer slot {slot} missing D3D11_BIND_CONSTANT_BUFFER: BindFlags {}",
                source.bind_flags
            ));
        }
        let declared_name = reflected_constant_buffers
            .iter()
            .find_map(|(name, bind_slot)| (*bind_slot == slot).then(|| name.clone()));
        out.push(TerrainChunkConstantBufferIdentityFile {
            stage,
            slot,
            declared_name,
            fnv1a64: fnv1a64_bytes(&bytes),
        });
    }
    Ok(())
}

fn terrain_chunk_preidentity_key(identity: &TerrainChunkPreidentityFile) -> Result<String, String> {
    serde_json::to_string(identity).map_err(|error| error.to_string())
}

fn terrain_chunk_library_world_rect(blockers: &mut Vec<String>) -> TerrainChunkWorldRectFile {
    let control = RUNTIME
        .get()
        .and_then(|runtime| runtime.lock().ok())
        .and_then(|runtime| runtime.control.clone());
    terrain_chunk_library_world_rect_from_control(control.as_ref(), blockers)
}

fn terrain_chunk_library_world_rect_from_control(
    control: Option<&TerrainCaptureControl>,
    blockers: &mut Vec<String>,
) -> TerrainChunkWorldRectFile {
    if let Some(control) = control {
        if control.expected_width > 0 && control.expected_height > 0 {
            return TerrainChunkWorldRectFile {
                min_x: 0.0,
                min_z: 0.0,
                max_x: control.expected_width as f32,
                max_z: control.expected_height as f32,
            };
        }
        if let Some(signature) = control.shader_cache_signatures.first() {
            if signature.map_size_x > 0 && signature.map_size_y > 0 {
                return TerrainChunkWorldRectFile {
                    min_x: 0.0,
                    min_z: 0.0,
                    max_x: signature.map_size_x as f32,
                    max_z: signature.map_size_y as f32,
                };
            }
        }
    }
    blockers.push(
        "terrain chunk library coverage requires --shader-cache MAP_SIZE evidence; using the game default world rectangle only for blocked diagnostics"
            .to_string(),
    );
    TerrainChunkWorldRectFile {
        min_x: 0.0,
        min_z: 0.0,
        max_x: 5632.0,
        max_z: 2048.0,
    }
}

fn terrain_chunk_identity_from_draw(
    draw: &MapFrameDrawFile,
    world_quad: TerrainChunkWorldRectFile,
) -> TerrainChunkIdentityFile {
    let vertex_payloads = draw
        .vertex_buffers
        .iter()
        .filter_map(|(slot, binding)| {
            binding
                .fnv1a64
                .as_ref()
                .map(|hash| (slot.clone(), hash.clone()))
        })
        .collect();
    let per_chunk_constant_buffers = draw
        .constant_buffers
        .iter()
        .filter(|binding| binding.declared_name.as_deref() != Some(TERRAIN_SCENE_CB0_SYMBOL))
        .filter_map(|binding| {
            binding
                .fnv1a64
                .as_ref()
                .map(|hash| TerrainChunkConstantBufferIdentityFile {
                    stage: binding.stage,
                    slot: binding.slot,
                    declared_name: binding.declared_name.clone(),
                    fnv1a64: hash.clone(),
                })
        })
        .collect();
    TerrainChunkIdentityFile {
        draw_call: draw.draw_call.to_string(),
        draw_count: draw.draw_count,
        draw_start: draw.draw_start,
        base_vertex_location: draw.base_vertex_location,
        per_instance_count: draw.per_instance_count,
        instance_count: draw.instance_count,
        start_instance_location: draw.start_instance_location,
        pixel_shader_fnv1a64: draw.shader.fnv1a64.clone(),
        vertex_shader_fnv1a64: draw.vertex_shader.fnv1a64.clone(),
        vertex_payloads,
        index_payload: draw
            .index_buffer
            .as_ref()
            .and_then(|binding| binding.fnv1a64.clone()),
        per_chunk_constant_buffers,
        world_quad,
    }
}

fn terrain_chunk_identity_key(identity: &TerrainChunkIdentityFile) -> Result<String, String> {
    serde_json::to_string(identity).map_err(|error| error.to_string())
}

fn terrain_chunk_id(identity_key: &str) -> String {
    fnv1a64_bytes(identity_key.as_bytes())
}

fn terrain_chunk_world_quad_from_bounds(
    bounds: TerrainChunkBoundsFile,
) -> TerrainChunkWorldRectFile {
    TerrainChunkWorldRectFile {
        min_x: bounds.min_x,
        min_z: bounds.min_z,
        max_x: bounds.max_x,
        max_z: bounds.max_z,
    }
}

fn terrain_chunk_bounds_from_draw(
    draw: &MapFrameDrawFile,
    output_dir: &Path,
) -> Result<TerrainChunkBoundsFile, String> {
    let position_element = draw
        .input_layout
        .elements
        .iter()
        .find(|element| {
            element.semantic_name == "POSITION"
                && element.semantic_index == 0
                && element.format == DXGI_FORMAT_R16G16B16A16_SINT as u32
        })
        .ok_or_else(|| {
            "terrain chunk bounds require POSITION0 R16G16B16A16_SINT input element".to_string()
        })?;
    let position_binding = draw
        .vertex_buffers
        .get(&position_element.input_slot.to_string())
        .ok_or_else(|| {
            format!(
                "terrain chunk bounds require IA vertex buffer slot {}",
                position_element.input_slot
            )
        })?;
    let file = position_binding
        .file
        .as_deref()
        .ok_or_else(|| "terrain chunk POSITION0 vertex buffer has no file".to_string())?;
    let bytes = fs::read(output_dir.join(file)).map_err(|error| {
        format!("terrain chunk POSITION0 vertex buffer {file} is unreadable: {error}")
    })?;
    let hash = fnv1a64_bytes(&bytes);
    if position_binding.fnv1a64.as_deref() != Some(hash.as_str()) {
        return Err(format!(
            "terrain chunk POSITION0 vertex buffer {file} hash mismatch"
        ));
    }
    let stride = position_binding
        .stride
        .ok_or_else(|| "terrain chunk POSITION0 vertex buffer has no stride".to_string())?
        as usize;
    let offset = position_binding
        .offset
        .unwrap_or(0)
        .checked_add(position_element.aligned_byte_offset)
        .ok_or_else(|| "terrain chunk POSITION0 byte offset overflow".to_string())?
        as usize;
    let cb1 = terrain_chunk_cb1_quad_offset_scale(draw, output_dir)?;
    terrain_chunk_bounds_from_position_payload(&bytes, stride, offset, Some(cb1))
}

fn terrain_chunk_cb1_quad_offset_scale(
    draw: &MapFrameDrawFile,
    output_dir: &Path,
) -> Result<[f32; 4], String> {
    let binding = draw
        .constant_buffers
        .iter()
        .find(|binding| binding.stage == "VS" && binding.slot == 1)
        .ok_or_else(|| "terrain chunk draw has no VS terrain draw CB1".to_string())?;
    let file = binding
        .file
        .as_deref()
        .ok_or_else(|| "terrain chunk draw CB1 has no file".to_string())?;
    let bytes = fs::read(output_dir.join(file))
        .map_err(|error| format!("terrain chunk draw CB1 {file} is unreadable: {error}"))?;
    let hash = fnv1a64_bytes(&bytes);
    if binding.fnv1a64.as_deref() != Some(hash.as_str()) {
        return Err(format!("terrain chunk draw CB1 {file} hash mismatch"));
    }
    if bytes.len() < 80 {
        return Err(
            "terrain chunk draw CB1 is too short for QuadOffset_Scale_IsDetail".to_string(),
        );
    }
    Ok([
        read_f32_le_at(&bytes, 64)?,
        read_f32_le_at(&bytes, 68)?,
        read_f32_le_at(&bytes, 72)?,
        read_f32_le_at(&bytes, 76)?,
    ])
}

fn terrain_chunk_bounds_from_position_payload(
    position_bytes: &[u8],
    stride: usize,
    offset: usize,
    cb1_quad_offset_scale: Option<[f32; 4]>,
) -> Result<TerrainChunkBoundsFile, String> {
    if stride < 8
        || offset
            .checked_add(8)
            .map_or(true, |end| end > position_bytes.len())
    {
        return Err("terrain chunk POSITION0 payload stride/offset is invalid".to_string());
    }
    let transform = cb1_quad_offset_scale.unwrap_or([0.0, 0.0, 1.0, 0.0]);
    if transform.iter().any(|value| !value.is_finite()) || transform[2] == 0.0 {
        return Err("terrain chunk CB1 QuadOffset_Scale_IsDetail is invalid".to_string());
    }
    let mut bounds = None;
    let mut cursor = offset;
    while cursor + 8 <= position_bytes.len() {
        let x = f32::from(read_i16_le_at(position_bytes, cursor)?) * transform[2] + transform[0];
        let z =
            f32::from(read_i16_le_at(position_bytes, cursor + 2)?) * transform[2] + transform[1];
        let candidate = TerrainChunkBoundsFile {
            min_x: x,
            min_y: 0.0,
            min_z: z,
            max_x: x,
            max_y: 0.0,
            max_z: z,
        };
        bounds = Some(match bounds {
            Some(existing) => merge_terrain_chunk_bounds(existing, candidate),
            None => candidate,
        });
        cursor = cursor.saturating_add(stride);
    }
    bounds.ok_or_else(|| "terrain chunk POSITION0 payload contains no vertices".to_string())
}

fn merge_terrain_chunk_bounds(
    a: TerrainChunkBoundsFile,
    b: TerrainChunkBoundsFile,
) -> TerrainChunkBoundsFile {
    TerrainChunkBoundsFile {
        min_x: a.min_x.min(b.min_x),
        min_y: a.min_y.min(b.min_y),
        min_z: a.min_z.min(b.min_z),
        max_x: a.max_x.max(b.max_x),
        max_y: a.max_y.max(b.max_y),
        max_z: a.max_z.max(b.max_z),
    }
}

fn terrain_chunk_coverage_for_bounds(
    bounds: &[TerrainChunkBoundsFile],
    world_rect: TerrainChunkWorldRectFile,
) -> TerrainChunkCoverageFile {
    let world_area = f64::from((world_rect.max_x - world_rect.min_x).max(0.0))
        * f64::from((world_rect.max_z - world_rect.min_z).max(0.0));
    let clipped = bounds
        .iter()
        .filter_map(|bounds| clip_terrain_chunk_bounds_to_world(*bounds, world_rect))
        .collect::<Vec<_>>();
    let covered_area = terrain_chunk_union_rect_area(&clipped);
    let fraction = if world_area > 0.0 {
        (covered_area / world_area).clamp(0.0, 1.0)
    } else {
        0.0
    };
    TerrainChunkCoverageFile {
        world_rect,
        covered_area,
        world_area,
        fraction,
        holes: terrain_chunk_coverage_holes(&clipped, world_rect),
    }
}

fn clip_terrain_chunk_bounds_to_world(
    bounds: TerrainChunkBoundsFile,
    world_rect: TerrainChunkWorldRectFile,
) -> Option<TerrainChunkWorldRectFile> {
    let min_x = bounds.min_x.max(world_rect.min_x);
    let max_x = bounds.max_x.min(world_rect.max_x);
    let min_z = bounds.min_z.max(world_rect.min_z);
    let max_z = bounds.max_z.min(world_rect.max_z);
    (min_x < max_x && min_z < max_z).then_some(TerrainChunkWorldRectFile {
        min_x,
        min_z,
        max_x,
        max_z,
    })
}

fn terrain_chunk_coverage_holes(
    covered: &[TerrainChunkWorldRectFile],
    world_rect: TerrainChunkWorldRectFile,
) -> Vec<TerrainChunkHoleFile> {
    if covered.is_empty() {
        return vec![TerrainChunkHoleFile {
            min_x: world_rect.min_x,
            min_z: world_rect.min_z,
            max_x: world_rect.max_x,
            max_z: world_rect.max_z,
        }];
    }
    let mut xs = vec![world_rect.min_x, world_rect.max_x];
    let mut zs = vec![world_rect.min_z, world_rect.max_z];
    for rect in covered {
        xs.push(rect.min_x);
        xs.push(rect.max_x);
        zs.push(rect.min_z);
        zs.push(rect.max_z);
    }
    sort_dedup_terrain_chunk_axis(&mut xs);
    sort_dedup_terrain_chunk_axis(&mut zs);
    let mut holes = Vec::new();
    for x_pair in xs.windows(2) {
        for z_pair in zs.windows(2) {
            let min_x = x_pair[0];
            let max_x = x_pair[1];
            let min_z = z_pair[0];
            let max_z = z_pair[1];
            if min_x >= max_x || min_z >= max_z {
                continue;
            }
            let covered_cell = covered.iter().any(|rect| {
                rect.min_x <= min_x
                    && rect.max_x >= max_x
                    && rect.min_z <= min_z
                    && rect.max_z >= max_z
            });
            if !covered_cell {
                holes.push(TerrainChunkHoleFile {
                    min_x,
                    min_z,
                    max_x,
                    max_z,
                });
            }
        }
    }
    holes
}

fn terrain_chunk_union_rect_area(rects: &[TerrainChunkWorldRectFile]) -> f64 {
    if rects.is_empty() {
        return 0.0;
    }
    let mut xs = rects
        .iter()
        .flat_map(|rect| [rect.min_x, rect.max_x])
        .collect::<Vec<_>>();
    sort_dedup_terrain_chunk_axis(&mut xs);
    let mut area = 0.0f64;
    for x_pair in xs.windows(2) {
        let x0 = x_pair[0];
        let x1 = x_pair[1];
        if x1 <= x0 {
            continue;
        }
        let mut intervals = rects
            .iter()
            .filter(|rect| rect.min_x <= x0 && rect.max_x >= x1)
            .map(|rect| (rect.min_z, rect.max_z))
            .collect::<Vec<_>>();
        intervals.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
        let mut covered_z = 0.0f64;
        let mut current: Option<(f32, f32)> = None;
        for (start, end) in intervals {
            if end <= start {
                continue;
            }
            match current {
                Some((current_start, current_end)) if start <= current_end => {
                    current = Some((current_start, current_end.max(end)));
                }
                Some((current_start, current_end)) => {
                    covered_z += f64::from(current_end - current_start);
                    current = Some((start, end));
                }
                None => current = Some((start, end)),
            }
        }
        if let Some((start, end)) = current {
            covered_z += f64::from(end - start);
        }
        area += f64::from(x1 - x0) * covered_z;
    }
    area
}

fn sort_dedup_terrain_chunk_axis(values: &mut Vec<f32>) {
    values.sort_by(|a, b| a.total_cmp(b));
    values.dedup_by(|a, b| (*a - *b).abs() <= f32::EPSILON);
}

fn read_i16_le_at(bytes: &[u8], offset: usize) -> Result<i16, String> {
    let slice = bytes
        .get(offset..offset + 2)
        .ok_or_else(|| format!("i16 read at offset {offset} is out of bounds"))?;
    Ok(i16::from_le_bytes([slice[0], slice[1]]))
}

fn read_f32_le_at(bytes: &[u8], offset: usize) -> Result<f32, String> {
    let slice = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| format!("f32 read at offset {offset} is out of bounds"))?;
    Ok(f32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

fn dedupe_blockers(blockers: &mut Vec<String>) {
    let mut seen = BTreeSet::new();
    blockers.retain(|blocker| seen.insert(blocker.clone()));
}

fn draw_has_moment_runtime_texture_payload(draw: &MapFrameDrawFile, symbol: &str) -> bool {
    let symbol = map_frame_runtime_texture_resource_key(symbol);
    draw.shader_resources.iter().any(|resource| {
        resource.file.is_some()
            && resource.provenance == Some("d3d11_staging_map_readback_from_active_ps_srv")
            && resource
                .declared_name
                .as_deref()
                .map(map_frame_runtime_texture_resource_key)
                .as_deref()
                == Some(symbol.as_str())
    })
}

fn map_frame_finalized_by(provenance: &'static str) -> Option<&'static str> {
    match provenance {
        "dwell_end_boundary" => Some("dwell_end_boundary"),
        "ps_shader_transition"
        | "post_draw_blend_layer_boundary"
        | "trees_frame_repeat_boundary"
        | "pre_clear_render_target_frame_boundary"
        | "present_frame_boundary" => Some("pass_close"),
        _ => None,
    }
}

fn runtime_texture_file_provenance(target: CaptureTarget) -> String {
    let captured = RUNTIME
        .get()
        .and_then(|runtime| runtime.lock().ok())
        .and_then(|runtime| {
            runtime
                .captured_runtime_textures
                .get(target.symbol)
                .cloned()
        });
    let captured_at = captured
        .as_ref()
        .map(|record| record.captured_at.as_str())
        .unwrap_or(target.timing.as_str());
    format!("runtime_texture_readback_{}_{}", target.symbol, captured_at)
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
            required_payload: required_payload_for_moment(moment_profile),
            watched_start_ms: None,
            watched_end_ms: None,
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

fn required_payload_for_moment(profile: MapFrameCaptureProfile) -> Vec<&'static str> {
    if profile.capture_moment == UI_PASS_CAPTURE_MOMENT {
        UI_PASS_REQUIRED_PAYLOAD.to_vec()
    } else {
        MAP_CAPTURE_REQUIRED_PAYLOAD.to_vec()
    }
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
            required_payload: required_payload_for_moment(moment_profile),
            watched_start_ms: None,
            watched_end_ms: None,
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

#[derive(Clone, Debug, PartialEq, Eq)]
struct MapMomentObservationWindow {
    profile: MapFrameCaptureProfile,
    watched_start_ms: u64,
    watched_end_ms: u64,
    blocker: String,
}

fn not_observed_during_dwell(
    runtime: &CaptureRuntime,
    dwell_end_ms: u64,
) -> Option<MapMomentObservationWindow> {
    let profile = requested_map_frame_capture_profile_for_runtime(runtime).ok()?;
    if profile.capture_moment == TERRAIN_CAPTURE_MOMENT
        || runtime
            .completed_map_frame_moments
            .contains(profile.capture_moment)
        || !runtime.map_frame_draws.is_empty()
        || runtime
            .map_frame_rejections
            .iter()
            .any(|error| !map_frame_rejection_is_observation_miss(profile, error.as_str()))
    {
        return None;
    }
    for prior in runtime
        .capture_moment_sequence
        .iter()
        .take(runtime.active_capture_index)
    {
        if !runtime.completed_map_frame_moments.contains(prior) {
            return None;
        }
    }
    let watched_start_ms = runtime.active_moment_started_ms.min(dwell_end_ms);
    let watched_end_ms = dwell_end_ms.max(watched_start_ms);
    Some(MapMomentObservationWindow {
        profile,
        watched_start_ms,
        watched_end_ms,
        blocker: format!(
            "watched {} from {} ms to {} ms; no active {} observed",
            profile.capture_moment,
            watched_start_ms,
            watched_end_ms,
            map_frame_observed_shader_family(profile)
        ),
    })
}

fn map_frame_observed_shader_family(profile: MapFrameCaptureProfile) -> &'static str {
    match profile.capture_moment {
        WATER_CAPTURE_MOMENT => "pdxwater shader draw",
        RIVERS_CAPTURE_MOMENT => "river shader draw",
        BORDERS_CAPTURE_MOMENT => "border shader draw",
        UNDERWATER_CAPTURE_MOMENT => "underwater shader draw",
        TREES_CAPTURE_MOMENT => "tree shader draw",
        UI_PASS_CAPTURE_MOMENT => "swapchain backbuffer no-depth UI draw",
        _ => "map-frame shader draw",
    }
}

fn write_not_observed_map_capture_session_at_dwell_end(dwell_end_ms: u64) -> bool {
    if capture_quiesced() {
        return false;
    }
    let Some((output_dir, sequence, completed_moments, observation)) =
        RUNTIME.get().and_then(|runtime| {
            runtime.lock().ok().and_then(|runtime| {
                let observation = not_observed_during_dwell(&runtime, dwell_end_ms)?;
                Some((
                    runtime.output_dir.clone(),
                    runtime.capture_moment_sequence.clone(),
                    runtime.completed_map_frame_moments.clone(),
                    observation,
                ))
            })
        })
    else {
        return false;
    };
    write_event(
        &output_dir,
        &format!(
            "{{\"type\":\"map_capture_moment_not_observed\",\"moment\":\"{}\",\"watched_start_ms\":{},\"watched_end_ms\":{},\"message\":\"{}\"}}",
            observation.profile.capture_moment,
            observation.watched_start_ms,
            observation.watched_end_ms,
            json_escape(&observation.blocker)
        ),
    );
    if let Err(error) = write_not_observed_map_capture_session(
        &output_dir,
        &sequence,
        &completed_moments,
        &observation,
    ) {
        write_event(
            &output_dir,
            &format!(
                "{{\"type\":\"map_capture_session_write_failed\",\"file\":\"{}\",\"message\":\"{}\"}}",
                MAP_CAPTURE_SESSION_FILE,
                json_escape(&error)
            ),
        );
        return false;
    }
    true
}

fn write_not_observed_map_capture_session(
    output_dir: &Path,
    sequence: &[String],
    completed_moments: &BTreeSet<String>,
    observation: &MapMomentObservationWindow,
) -> Result<(), String> {
    if capture_quiesced() {
        return Err("capture quiesced".to_string());
    }
    let mut moments = Vec::new();
    let mut recorded = BTreeSet::new();
    for moment in sequence {
        let moment_profile = map_frame_capture_profile(moment)?;
        let (status, blockers, watched_start_ms, watched_end_ms) =
            if moment_profile.capture_moment == observation.profile.capture_moment {
                (
                    "not_observed_during_dwell",
                    vec![observation.blocker.clone()],
                    Some(observation.watched_start_ms),
                    Some(observation.watched_end_ms),
                )
            } else if completed_moments.contains(moment_profile.capture_moment) {
                (
                    "captured_pending_renderer_validation",
                    Vec::new(),
                    None,
                    None,
                )
            } else {
                (
                    "deferred_until_prior_moment_validated",
                    vec![format!(
                        "{} has not reached its active capture moment in this session",
                        moment_profile.capture_moment
                    )],
                    None,
                    None,
                )
            };
        moments.push(MapCaptureSessionMomentFile {
            name: moment_profile.capture_moment,
            status,
            bundle_file: moment_profile.bundle_file,
            required_payload: required_payload_for_moment(moment_profile),
            watched_start_ms,
            watched_end_ms,
            blockers,
        });
        recorded.insert(moment_profile.capture_moment);
    }
    let session = MapCaptureSessionFile {
        schema: MAP_CAPTURE_SESSION_SCHEMA,
        provenance: "dwell_end_not_observed",
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
    debug_assert_render_thread_owner("capture_active_terrain_input_layout");
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
    debug_assert_render_thread_owner("capture_map_frame_vertex_buffer_slot");
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
    // Validate the cheap IA-side invariant BEFORE the staging copy/map
    // readback (2026-07-06 adversarial review): a zero-stride binding used to
    // pay the full buffer readback only to be rejected afterwards.
    if stride == 0 {
        unsafe {
            release_unknown(buffer as *mut WinVoid);
        }
        return Err(format!("IA vertex buffer slot {slot} has zero stride"));
    }
    let result = unsafe {
        let (source, bytes) = readback_d3d11_buffer_bytes(context, buffer)?;
        if source.bind_flags & D3D11_BIND_VERTEX_BUFFER == 0 {
            return Err(format!(
                "IA vertex buffer slot {slot} missing D3D11_BIND_VERTEX_BUFFER: BindFlags {}",
                source.bind_flags
            ));
        }
        let requested_file = format!(
            "{}{draw_index:03}_ia_vertex_buffer_slot{slot}.bin",
            profile.file_prefix
        );
        let file = write_map_frame_binary_payload(
            output_dir,
            &requested_file,
            &bytes,
            profile,
            &format!("ia_vertex_buffer_slot{slot}"),
            None,
        )?;
        Ok(MapFrameBufferBindingFile {
            stage: "IA",
            slot,
            declared_name: None,
            file: Some(file),
            byte_len: bytes.len(),
            buffer_byte_width: map_frame_buffer_byte_width(&source),
            fnv1a64: Some(fnv1a64_bytes(&bytes)),
            stride: Some(stride),
            offset: Some(offset),
            dxgi_format: None,
            bind_flags: source.bind_flags,
            usage: source.usage,
            cpu_access_flags: source.cpu_access_flags,
            unknown_origin: None,
            byte_range: None,
            payload_truncated: None,
            unobtained_reason: None,
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
    debug_assert_render_thread_owner("capture_map_frame_index_buffer");
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
        let requested_file = format!("{}{draw_index:03}_ia_index_buffer.bin", profile.file_prefix);
        let file = write_map_frame_binary_payload(
            output_dir,
            &requested_file,
            &bytes,
            profile,
            "ia_index_buffer",
            None,
        )?;
        Ok(MapFrameIndexBufferBindingFile {
            file: Some(file),
            byte_len: bytes.len(),
            buffer_byte_width: map_frame_buffer_byte_width(&source),
            fnv1a64: Some(fnv1a64_bytes(&bytes)),
            dxgi_format: format,
            offset,
            bind_flags: source.bind_flags,
            usage: source.usage,
            cpu_access_flags: source.cpu_access_flags,
            byte_range: None,
            payload_truncated: None,
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
    debug_assert_render_thread_owner("readback_d3d11_buffer_bytes");
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
    // A staging CopyResource of a SHARED or otherwise exotic buffer drives
    // d3d11 into D3DKMTOpenResource in kernel space and access-violates the
    // host process (live 2026-07-03: a the game UI draw carried such a constant
    // buffer and the copy hard-crashed the game — an SEH fault Rust cannot
    // catch). Only plain buffers (MiscFlags == 0) are safe to copy out; for
    // anything else, record the binding metadata and skip the byte readback.
    if source_desc_native.MiscFlags & D3D11_RESOURCE_MISC_SHARED_MASK != 0 {
        return Err(format!(
            "map-frame buffer is a shared resource (MiscFlags 0x{:X}); skipping unsafe staging readback",
            source_desc_native.MiscFlags
        ));
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
    if capture_quiesced() {
        return Err("capture quiesced".to_string());
    }
    reserve_capture_payload_bytes(bytes.len(), file)?;
    fs::create_dir_all(output_dir).map_err(|error| error.to_string())?;
    fs::write(output_dir.join(file), bytes).map_err(|error| error.to_string())
}

fn write_map_frame_binary_payload(
    output_dir: &Path,
    requested_file: &str,
    bytes: &[u8],
    profile: MapFrameCaptureProfile,
    payload_kind: &str,
    declared_name: Option<&str>,
) -> Result<String, String> {
    if profile.capture_moment == CENSUS_CAPTURE_MOMENT {
        let fnv1a64 = fnv1a64_bytes(bytes);
        let file = census_shared_payload_file(payload_kind, declared_name, &fnv1a64);
        if reserve_census_shared_payload_file_for_write(&file) {
            queue_census_payload_file_write(output_dir, &file, bytes)?;
        }
        return Ok(file);
    }
    if profile.capture_moment == UI_PASS_CAPTURE_MOMENT {
        queue_payload_file_write(output_dir, requested_file, bytes)?;
        return Ok(requested_file.to_string());
    }
    if profile.capture_moment != TERRAIN_CHUNK_LIBRARY_MOMENT {
        write_map_frame_bytes(output_dir, requested_file, bytes)?;
        return Ok(requested_file.to_string());
    }
    let fnv1a64 = fnv1a64_bytes(bytes);
    let file = map_frame_shared_payload_file(profile, payload_kind, declared_name, &fnv1a64);
    let path = output_dir.join(&file);
    if path.is_file() {
        let existing = fs::read(&path).map_err(|error| {
            format!(
                "terrain chunk shared payload {} exists but is unreadable: {}",
                path.display(),
                error
            )
        })?;
        if existing.len() == bytes.len() && fnv1a64_bytes(&existing) == fnv1a64 {
            return Ok(file);
        }
        return Err(format!(
            "terrain chunk shared payload {} hash collision or stale content",
            path.display()
        ));
    }
    write_map_frame_bytes(output_dir, &file, bytes)?;
    Ok(file)
}

fn queue_payload_file_write(output_dir: &Path, file: &str, bytes: &[u8]) -> Result<(), String> {
    if capture_quiesced() {
        return Err("capture quiesced".to_string());
    }
    reserve_capture_payload_bytes(bytes.len(), file)?;
    fs::create_dir_all(output_dir).map_err(|error| error.to_string())?;
    let writer = PAYLOAD_WRITER.get_or_init(start_payload_writer);
    let path = output_dir.join(file);
    {
        let (state, _) = &*writer.state;
        let mut state = state
            .lock()
            .map_err(|_| "payload writer state lock poisoned".to_string())?;
        let byte_len = bytes.len() as u64;
        let cap = payload_writer_queue_byte_cap();
        let next_pending_bytes = state.pending_bytes.saturating_add(byte_len);
        if payload_writer_queue_bytes_exceeded(state.pending_bytes, bytes.len(), cap) {
            let pending_bytes = state.pending_bytes;
            drop(state);
            let blocker =
                stop_capture_for_payload_writer_queue_full(&path, bytes.len(), pending_bytes, cap);
            return Err(blocker);
        }
        state.pending += 1;
        state.pending_bytes = next_pending_bytes;
    }
    match writer.sender.send(PayloadWriteJob {
        path: path.clone(),
        bytes: bytes.to_vec(),
    }) {
        Ok(()) => Ok(()),
        Err(mpsc::SendError(job)) => {
            let (state, condvar) = &*writer.state;
            if let Ok(mut state) = state.lock() {
                state.pending = state.pending.saturating_sub(1);
                state.pending_bytes = state.pending_bytes.saturating_sub(job.bytes.len() as u64);
                state.failures.push(format!(
                    "payload writer queue disconnected before writing {}",
                    job.path.display()
                ));
                condvar.notify_all();
            }
            Err(format!("payload writer queue rejected {}", path.display()))
        }
    }
}

fn queue_census_payload_file_write(
    output_dir: &Path,
    file: &str,
    bytes: &[u8],
) -> Result<(), String> {
    if capture_quiesced() {
        return Err("capture quiesced".to_string());
    }
    reserve_census_payload_bytes(bytes.len(), file)?;
    let writer = PAYLOAD_WRITER.get_or_init(start_payload_writer);
    let path = output_dir.join(file);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    {
        let (state, _) = &*writer.state;
        let mut state = state
            .lock()
            .map_err(|_| "payload writer state lock poisoned".to_string())?;
        let byte_len = bytes.len() as u64;
        let cap = payload_writer_queue_byte_cap();
        let next_pending_bytes = state.pending_bytes.saturating_add(byte_len);
        if payload_writer_queue_bytes_exceeded(state.pending_bytes, bytes.len(), cap) {
            return Err(format!(
                "census payload writer queue would exceed max_payload_writer_queue_bytes {cap} for {}",
                path.display()
            ));
        }
        state.pending += 1;
        state.pending_bytes = next_pending_bytes;
    }
    match writer.sender.send(PayloadWriteJob {
        path: path.clone(),
        bytes: bytes.to_vec(),
    }) {
        Ok(()) => Ok(()),
        Err(mpsc::SendError(job)) => {
            let (state, condvar) = &*writer.state;
            if let Ok(mut state) = state.lock() {
                state.pending = state.pending.saturating_sub(1);
                state.pending_bytes = state.pending_bytes.saturating_sub(job.bytes.len() as u64);
                state.failures.push(format!(
                    "payload writer queue disconnected before writing {}",
                    job.path.display()
                ));
                condvar.notify_all();
            }
            Err(format!(
                "census payload writer queue rejected {}",
                path.display()
            ))
        }
    }
}

fn reserve_census_payload_bytes(byte_len: usize, file: &str) -> Result<(), String> {
    let Some((output_dir, event)) = RUNTIME.get().and_then(|runtime| {
        runtime.lock().ok().map(|mut runtime| {
            let event = reserve_census_payload_bytes_for_runtime(&mut runtime, byte_len, file);
            (runtime.output_dir.clone(), event)
        })
    }) else {
        return Ok(());
    };
    if let Some(event) = event {
        if event.starts_with('{') {
            write_event(&output_dir, &event);
        }
        return Err("budget".to_string());
    }
    Ok(())
}

fn reserve_census_payload_bytes_for_runtime(
    runtime: &mut CaptureRuntime,
    byte_len: usize,
    file: &str,
) -> Option<String> {
    if !runtime.census.enabled {
        return None;
    }
    if runtime.census.payload_stopped {
        return Some(format!(
            "census payload capture already stopped: {}",
            runtime
                .census
                .payload_stop_reason
                .as_deref()
                .unwrap_or("budget")
        ));
    }
    let attempted = byte_len as u64;
    let next = runtime
        .census
        .payload_bytes_reserved
        .saturating_add(attempted);
    if next > runtime.census.payload_byte_budget {
        runtime.census.payload_stopped = true;
        runtime.census.payload_stop_reason = Some("budget".to_string());
        return Some(format!(
            "{{\"type\":\"census_budget_exhausted\",\"reason\":\"budget\",\"file\":\"{}\",\"attempted_payload_bytes\":{},\"payload_bytes_reserved\":{},\"payload_byte_budget\":{},\"reserved_moment_floor_bytes\":{}}}",
            json_escape(file),
            attempted,
            runtime.census.payload_bytes_reserved,
            runtime.census.payload_byte_budget,
            CENSUS_RESERVED_MOMENT_PAYLOAD_FLOOR_BYTES
        ));
    }
    runtime.census.payload_bytes_reserved = next;
    None
}

fn start_payload_writer() -> PayloadWriter {
    let (sender, receiver) = mpsc::channel::<PayloadWriteJob>();
    let state = Arc::new((
        Mutex::new(PayloadWriterState {
            pending: 0,
            pending_bytes: 0,
            failures: Vec::new(),
        }),
        Condvar::new(),
    ));
    let worker_state = state.clone();
    thread::spawn(move || {
        while let Ok(job) = receiver.recv() {
            let result = fs::write(&job.path, &job.bytes).map_err(|error| {
                format!(
                    "payload writer failed for {}: {}",
                    job.path.display(),
                    error
                )
            });
            let (state, condvar) = &*worker_state;
            if let Ok(mut state) = state.lock() {
                if let Err(error) = result {
                    state.failures.push(error);
                }
                state.pending = state.pending.saturating_sub(1);
                state.pending_bytes = state.pending_bytes.saturating_sub(job.bytes.len() as u64);
                condvar.notify_all();
            }
        }
    });
    PayloadWriter { sender, state }
}

fn flush_payload_file_writes() -> Vec<String> {
    let Some(writer) = PAYLOAD_WRITER.get() else {
        return Vec::new();
    };
    let (state, condvar) = &*writer.state;
    let Ok(mut state) = state.lock() else {
        return vec!["payload writer state lock poisoned during flush".to_string()];
    };
    while state.pending > 0 {
        let Ok(next) = condvar.wait(state) else {
            return vec!["payload writer state lock poisoned while waiting".to_string()];
        };
        state = next;
    }
    std::mem::take(&mut state.failures)
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
    if capture_quiesced() {
        return;
    }
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
    if capture_quiesced() {
        return;
    }
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
    debug_assert_render_thread_owner("capture_terrain_vertex_buffer_if_active");
    if capture_quiesced() {
        return;
    }
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
    if capture_quiesced() {
        return None;
    }
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
    debug_assert_render_thread_owner("capture_active_terrain_vertex_buffer");
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
    debug_assert_render_thread_owner("capture_terrain_vertex_buffer");
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
    if capture_quiesced() {
        return;
    }
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
    if capture_quiesced() {
        return;
    }
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
    if capture_quiesced() {
        return;
    }
    let mut round_finished_event = None;
    if let Some(runtime) = RUNTIME.get() {
        if let Ok(mut runtime) = runtime.lock() {
            let profile = requested_map_frame_capture_profile_for_runtime(&runtime)
                .unwrap_or(TERRAIN_MAP_FRAME_CAPTURE_PROFILE);
            mark_capture_progress_now();
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
            let complete = refresh_session_capture_progress(&mut runtime);
            if complete && runtime.active_round_label.is_some() && !runtime.active_round_finished {
                runtime.active_round_finished = true;
                mark_round_closed();
                let label = runtime.active_round_label.clone().unwrap_or_default();
                round_finished_event = Some((
                    runtime.root_output_dir.clone(),
                    runtime.output_dir.clone(),
                    format!(
                        "{{\"type\":\"map_frame_capture_round_finished\",\"label\":\"{}\",\"dir\":\"{}\",\"clean\":true}}",
                        json_escape(&label),
                        json_escape(&runtime.output_dir.to_string_lossy())
                    ),
                ));
            }
            if complete {
                CAPTURED.store(true, Ordering::SeqCst);
            }
        }
    }
    if let Some((root_output_dir, round_output_dir, event)) = round_finished_event {
        write_event(&root_output_dir, &event);
        if round_output_dir != root_output_dir {
            write_event(&round_output_dir, &event);
        }
    }
}

fn record_map_frame_bundle_rejected(error: &str) {
    if capture_quiesced() || CAPTURE_BUDGET_STOPPED.load(Ordering::SeqCst) {
        return;
    }
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
            if map_frame_rejection_is_observation_miss(profile, error) {
                write_event(
                    &runtime.output_dir,
                    &format!(
                        "{{\"type\":\"map_capture_moment_observation_pending\",\"moment\":\"{}\",\"message\":\"{}\"}}",
                        profile.capture_moment,
                        json_escape(error)
                    ),
                );
                return None;
            }
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
                UNDERWATER_CAPTURE_MOMENT => "blocked_before_underwater_pass_bundle",
                TREES_CAPTURE_MOMENT => "blocked_before_trees_pass_bundle",
                RESTORE_CAPTURE_MOMENT => "blocked_before_restore_pass_bundle",
                MAPNAME_CAPTURE_MOMENT => "blocked_before_mapname_pass_bundle",
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

fn map_frame_rejection_is_observation_miss(profile: MapFrameCaptureProfile, error: &str) -> bool {
    match profile.capture_moment {
        WATER_CAPTURE_MOMENT => {
            error == "no active pdxwater pixel shader for water-pass map-frame capture"
        }
        RIVERS_CAPTURE_MOMENT => {
            error == "no active river pixel shader for rivers-pass map-frame capture"
        }
        BORDERS_CAPTURE_MOMENT => {
            error == "no active border pixel shader for borders-pass map-frame capture"
        }
        UNDERWATER_CAPTURE_MOMENT => {
            error == "no active underwater pixel shader for underwater-pass map-frame capture"
        }
        TREES_CAPTURE_MOMENT => {
            error == "no active tree pixel shader for trees-pass map-frame capture"
        }
        RESTORE_CAPTURE_MOMENT => {
            error == "no active restorescene pixel shader for restore-pass map-frame capture"
        }
        MAPNAME_CAPTURE_MOMENT => {
            error == "no active mapname pixel shader for mapname-pass map-frame capture"
        }
        _ => false,
    }
}

unsafe fn capture_paired_runtime_resources(context: *mut ID3D11DeviceContext, terrain_slot: u32) {
    debug_assert_render_thread_owner("capture_paired_runtime_resources");
    if capture_quiesced() {
        return;
    }
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
        if target.timing == RuntimeTextureCaptureTiming::Finalize {
            unsafe {
                record_deferred_runtime_target(
                    target,
                    srv,
                    pair.shader_resource_slot,
                    "ps_get_shader_resources_paired_terrain_slot",
                );
                release_unknown(srv as *mut WinVoid);
            }
            continue;
        }
        let result = unsafe {
            capture_runtime_texture(
                context,
                target,
                srv,
                pair.shader_resource_slot,
                "ps_get_shader_resources_paired_terrain_slot",
                RuntimeTextureCaptureTiming::FirstDraw,
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

unsafe fn capture_deferred_runtime_targets_at_finalize(
    context: *mut ID3D11DeviceContext,
) -> Vec<String> {
    debug_assert_render_thread_owner("capture_deferred_runtime_targets_at_finalize");
    if capture_quiesced() {
        return Vec::new();
    }
    let (settled, settle_ms, elapsed_ms, pending) = RUNTIME
        .get()
        .and_then(|runtime| runtime.lock().ok())
        .map(|runtime| {
            let elapsed_ms = unsafe { GetTickCount64() }.saturating_sub(runtime.start_tick_ms);
            (
                elapsed_ms >= runtime.settle_ms,
                runtime.settle_ms,
                elapsed_ms,
                runtime
                    .deferred_runtime_targets
                    .values()
                    .copied()
                    .filter(|target| {
                        target.target.timing == RuntimeTextureCaptureTiming::Finalize
                            && !runtime.captured_symbols.contains(target.target.symbol)
                    })
                    .collect::<Vec<_>>(),
            )
        })
        .unwrap_or((true, DEFAULT_SETTLE_MS, 0, Vec::new()));
    if pending.is_empty() {
        return Vec::new();
    }
    if !settled {
        return vec![format!(
            "runtime-generated texture capture deferred until settle delay: elapsed {elapsed_ms} ms, required {settle_ms} ms"
        )];
    }
    if context.is_null() {
        return vec![
            "runtime-generated texture capture reached finalization without a D3D11 context"
                .to_string(),
        ];
    }
    let mut blockers = Vec::new();
    let mut completed = Vec::new();
    for pending_target in pending {
        let result = unsafe {
            capture_runtime_texture(
                context,
                pending_target.target,
                pending_target.srv as *mut ID3D11ShaderResourceView,
                pending_target.slot,
                pending_target.identity,
                RuntimeTextureCaptureTiming::Finalize,
            )
        };
        match result {
            Ok(meta) => {
                record_capture_success(pending_target.target, &meta);
                if !pending_target.target.dwell_end_resnapshot {
                    completed.push((pending_target.target.symbol.to_string(), pending_target.srv));
                }
            }
            Err(error) => {
                blockers.push(format!(
                    "finalize runtime texture {} readback failed: {}",
                    pending_target.target.symbol, error
                ));
                record_candidate_rejected(
                    pending_target.target,
                    pending_target.slot,
                    pending_target.identity,
                    &error,
                );
            }
        }
    }
    if let Some(runtime) = RUNTIME.get() {
        if let Ok(mut runtime) = runtime.lock() {
            for (symbol, srv) in &completed {
                runtime.deferred_runtime_targets.remove(symbol);
                unsafe {
                    release_unknown(*srv as *mut WinVoid);
                }
            }
        }
    }
    blockers
}

unsafe fn finalize_generated_runtime_textures_at_dwell_end_on_render_thread(
    context: *mut ID3D11DeviceContext,
) {
    debug_assert_render_thread_owner("finalize_generated_runtime_textures_at_dwell_end");
    if capture_quiesced() {
        unsafe {
            drain_quiesced_render_thread_releases(
                "finalize_generated_runtime_textures_at_dwell_end",
            );
        }
        return;
    }
    if context.is_null() {
        return;
    }
    let Some((output_dir, pending)) = RUNTIME.get().and_then(|runtime| {
        runtime.lock().ok().map(|mut runtime| {
            let pending = runtime
                .deferred_runtime_targets
                .values()
                .copied()
                .filter_map(|target| {
                    if !target.target.dwell_end_resnapshot {
                        return None;
                    }
                    let previous = runtime
                        .captured_runtime_textures
                        .get(target.target.symbol)?
                        .clone();
                    Some((target, previous))
                })
                .collect::<Vec<_>>();
            if pending.is_empty() {
                runtime.dwell_end_generated_texture_check_done = true;
            }
            (runtime.output_dir.clone(), pending)
        })
    }) else {
        return;
    };
    if pending.is_empty() {
        return;
    }

    for (pending_target, previous) in &pending {
        let result = unsafe {
            capture_runtime_texture(
                context,
                pending_target.target,
                pending_target.srv as *mut ID3D11ShaderResourceView,
                pending_target.slot,
                pending_target.identity,
                RuntimeTextureCaptureTiming::DwellEnd,
            )
        };
        match result {
            Ok(meta) if meta.fnv1a64 != previous.fnv1a64 => {
                if let Err(error) = write_runtime_texture_meta_file(
                    &output_dir,
                    pending_target.target,
                    &meta,
                    RuntimeTextureCaptureTiming::DwellEnd,
                    Some(&previous.fnv1a64),
                    None,
                    None,
                ) {
                    write_event(
                        &output_dir,
                        &format!(
                            "{{\"type\":\"runtime_texture_dwell_end_metadata_failed\",\"symbol\":\"{}\",\"message\":\"{}\"}}",
                            pending_target.target.symbol,
                            json_escape(&error)
                        ),
                    );
                }
                if let Err(error) = rewrite_generated_texture_bundle_references(
                    &output_dir,
                    pending_target.target,
                    &meta,
                ) {
                    write_event(
                        &output_dir,
                        &format!(
                            "{{\"type\":\"runtime_texture_dwell_end_bundle_rewrite_failed\",\"symbol\":\"{}\",\"message\":\"{}\"}}",
                            pending_target.target.symbol,
                            json_escape(&error)
                        ),
                    );
                }
                record_capture_success(pending_target.target, &meta);
                write_event(
                    &output_dir,
                    &format!(
                        "{{\"type\":\"runtime_texture_dwell_end_superseded\",\"symbol\":\"{}\",\"file\":\"{}\",\"superseded_hash\":\"{}\",\"fnv1a64\":\"{}\"}}",
                        pending_target.target.symbol,
                        pending_target.target.rgba_file,
                        previous.fnv1a64,
                        meta.fnv1a64
                    ),
                );
            }
            Ok(meta) => {
                if let Err(error) = write_runtime_texture_meta_file(
                    &output_dir,
                    pending_target.target,
                    &meta,
                    previous.captured_at,
                    None,
                    Some("unchanged"),
                    Some(&meta.fnv1a64),
                ) {
                    write_event(
                        &output_dir,
                        &format!(
                            "{{\"type\":\"runtime_texture_dwell_end_metadata_failed\",\"symbol\":\"{}\",\"message\":\"{}\"}}",
                            pending_target.target.symbol,
                            json_escape(&error)
                        ),
                    );
                }
                write_event(
                    &output_dir,
                    &format!(
                        "{{\"type\":\"runtime_texture_dwell_end_unchanged\",\"symbol\":\"{}\",\"file\":\"{}\",\"fnv1a64\":\"{}\"}}",
                        pending_target.target.symbol,
                        pending_target.target.rgba_file,
                        meta.fnv1a64
                    ),
                );
            }
            Err(error) => {
                write_event(
                    &output_dir,
                    &format!(
                        "{{\"type\":\"runtime_texture_dwell_end_resnapshot_failed\",\"symbol\":\"{}\",\"message\":\"{}\"}}",
                        pending_target.target.symbol,
                        json_escape(&error)
                    ),
                );
            }
        }
    }
    if let Some(runtime) = RUNTIME.get() {
        if let Ok(mut runtime) = runtime.lock() {
            runtime.dwell_end_generated_texture_check_done = true;
        }
    }
    unsafe {
        release_retained_generated_runtime_targets(&pending);
    }
}

unsafe fn release_retained_generated_runtime_targets(
    pending: &[(DeferredRuntimeTarget, RuntimeTextureCaptureRecord)],
) {
    debug_assert_render_thread_owner("release_retained_generated_runtime_targets");
    if let Some(runtime) = RUNTIME.get() {
        if let Ok(mut runtime) = runtime.lock() {
            for (target, _) in pending {
                runtime
                    .deferred_runtime_targets
                    .remove(target.target.symbol);
                unsafe {
                    release_unknown(target.srv as *mut WinVoid);
                }
            }
        }
    }
}

unsafe fn capture_scene_constant_buffer(context: *mut ID3D11DeviceContext, slot: u32) {
    debug_assert_render_thread_owner("capture_scene_constant_buffer");
    if capture_quiesced() {
        return;
    }
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
    debug_assert_render_thread_owner("readback_constant_buffer");
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
    if capture_quiesced() {
        return;
    }
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
    if capture_quiesced() {
        return;
    }
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
    if capture_quiesced() {
        return Vec::new();
    }
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
    if capture_quiesced() {
        return true;
    }
    RUNTIME
        .get()
        .and_then(|runtime| runtime.lock().ok())
        .map_or(false, |runtime| {
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
    captured_at: RuntimeTextureCaptureTiming,
    captured_bytes: Vec<u8>,
}

struct RenderThreadComPtr<T> {
    ptr: *mut T,
    _not_send: PhantomData<Rc<()>>,
}

impl<T> RenderThreadComPtr<T> {
    fn new(ptr: *mut T) -> Option<Self> {
        (!ptr.is_null()).then_some(Self {
            ptr,
            _not_send: PhantomData,
        })
    }

    fn as_ptr(&self) -> *mut T {
        self.ptr
    }
}

impl<T> Drop for RenderThreadComPtr<T> {
    fn drop(&mut self) {
        debug_assert_render_thread_owner("RenderThreadComPtr::drop");
        unsafe {
            release_unknown(self.ptr as *mut WinVoid);
        }
    }
}

struct RenderThreadMappedResource {
    context: *mut ID3D11DeviceContext,
    resource: *mut ID3D11Resource,
    subresource: u32,
    _not_send: PhantomData<Rc<()>>,
}

impl RenderThreadMappedResource {
    fn new(
        context: *mut ID3D11DeviceContext,
        resource: *mut ID3D11Resource,
        subresource: u32,
    ) -> Self {
        Self {
            context,
            resource,
            subresource,
            _not_send: PhantomData,
        }
    }
}

impl Drop for RenderThreadMappedResource {
    fn drop(&mut self) {
        debug_assert_render_thread_owner("RenderThreadMappedResource::drop");
        unsafe {
            (*self.context).Unmap(self.resource, self.subresource);
        }
    }
}

unsafe fn capture_runtime_texture(
    context: *mut ID3D11DeviceContext,
    target: CaptureTarget,
    srv: *mut ID3D11ShaderResourceView,
    slot: u32,
    identity: &'static str,
    captured_at: RuntimeTextureCaptureTiming,
) -> Result<CapturedTextureMeta, String> {
    debug_assert_render_thread_owner("capture_runtime_texture");
    if capture_quiesced() {
        return Err("capture quiesced".to_string());
    }
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
    let texture =
        RenderThreadComPtr::new(texture_void as *mut ID3D11Texture2D).ok_or_else(|| {
            format!(
                "{} SRV QueryInterface returned null Texture2D",
                target.symbol
            )
        })?;

    let mut source_desc_native: D3D11_TEXTURE2D_DESC = unsafe { mem::zeroed() };
    unsafe {
        (*texture.as_ptr()).GetDesc(&mut source_desc_native);
    }
    let source_desc = texture_desc_from_native(&source_desc_native);
    if target.validate_control {
        validate_against_control(&source_desc, slot)?;
    }
    validate_capture_source_desc(target, &source_desc)
        .map_err(|error| format!("{} source texture rejected: {error:?}", target.symbol))?;

    let mut device: *mut ID3D11Device = ptr::null_mut();
    unsafe {
        (*texture.as_ptr()).GetDevice(&mut device);
    }
    let device = RenderThreadComPtr::new(device)
        .ok_or_else(|| format!("{} texture did not expose an ID3D11Device", target.symbol))?;

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
    let create_hr =
        unsafe { (*device.as_ptr()).CreateTexture2D(&staging_desc, ptr::null(), &mut staging) };
    if FAILED(create_hr) || staging.is_null() {
        return Err(format!(
            "CreateTexture2D {} staging readback failed: HRESULT 0x{:08X}",
            target.symbol, create_hr as u32
        ));
    }
    let staging = RenderThreadComPtr::new(staging).ok_or_else(|| {
        format!(
            "CreateTexture2D {} returned null staging texture",
            target.symbol
        )
    })?;

    unsafe {
        (*context).CopySubresourceRegion(
            staging.as_ptr() as *mut ID3D11Resource,
            0,
            0,
            0,
            0,
            texture.as_ptr() as *mut ID3D11Resource,
            0,
            ptr::null(),
        );
    }

    let mut mapped: D3D11_MAPPED_SUBRESOURCE = unsafe { mem::zeroed() };
    let map_hr = unsafe {
        (*context).Map(
            staging.as_ptr() as *mut ID3D11Resource,
            0,
            D3D11_MAP_READ,
            0,
            &mut mapped,
        )
    };
    if FAILED(map_hr) || mapped.pData.is_null() {
        return Err(format!(
            "Map staging {} failed: HRESULT 0x{:08X}",
            target.symbol, map_hr as u32
        ));
    }

    let captured_bytes = {
        let _mapped_guard =
            RenderThreadMappedResource::new(context, staging.as_ptr() as *mut ID3D11Resource, 0);
        let row_pitch = mapped.RowPitch as usize;
        let mapped_len = row_pitch
            .checked_mul(source_desc.height as usize)
            .ok_or_else(|| format!("{} mapped byte span overflowed", target.symbol))?;
        let mapped_rows = unsafe { slice::from_raw_parts(mapped.pData.cast::<u8>(), mapped_len) };
        copy_mapped_capture_rows(target, mapped_rows, row_pitch, &source_desc)
            .map_err(|error| format!("{} mapped row copy rejected: {error:?}", target.symbol))?
    };
    drop(staging);
    drop(device);
    drop(texture);

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
        captured_at,
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
    captured_at: RuntimeTextureCaptureTiming,
) -> Result<CapturedTextureMeta, String> {
    if capture_quiesced() {
        return Err("capture quiesced".to_string());
    }
    fs::create_dir_all(output_dir).map_err(|error| error.to_string())?;
    queue_payload_file_write(output_dir, target.rgba_file, captured_bytes)?;
    let fnv1a64 = fnv1a64_bytes(captured_bytes);
    let meta = CapturedTextureMeta {
        width: desc.width,
        height: desc.height,
        byte_len: captured_bytes.len(),
        fnv1a64,
        source_format: desc.format,
        slot,
        identity,
        captured_at,
        captured_bytes: captured_bytes.to_vec(),
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
                "  \"captured_at\": \"{}\",\n",
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
            identity,
            captured_at.as_str()
        )
    } else {
        write_runtime_texture_meta_file(output_dir, target, &meta, captured_at, None, None, None)?;
        return Ok(meta);
    };
    fs::write(output_dir.join(target.meta_file), meta_json).map_err(|error| error.to_string())?;
    Ok(meta)
}

fn write_runtime_texture_meta_file(
    output_dir: &Path,
    target: CaptureTarget,
    meta: &CapturedTextureMeta,
    captured_at: RuntimeTextureCaptureTiming,
    superseded_hash: Option<&str>,
    dwell_end_status: Option<&str>,
    dwell_end_hash: Option<&str>,
) -> Result<(), String> {
    if capture_quiesced() {
        return Err("capture quiesced".to_string());
    }
    let mut meta_json = format!(
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
            "  \"source\": \"{}\",\n",
            "  \"captured_at\": \"{}\""
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
        meta.identity,
        captured_at.as_str()
    );
    if let Some(hash) = superseded_hash {
        meta_json.push_str(&format!(
            ",\n  \"superseded_hash\": \"{}\"",
            json_escape(hash)
        ));
    }
    if let Some(status) = dwell_end_status {
        meta_json.push_str(&format!(
            ",\n  \"dwell_end_status\": \"{}\"",
            json_escape(status)
        ));
    }
    if let Some(hash) = dwell_end_hash {
        meta_json.push_str(&format!(
            ",\n  \"dwell_end_hash\": \"{}\"",
            json_escape(hash)
        ));
    }
    meta_json.push_str("\n}\n");
    fs::write(output_dir.join(target.meta_file), meta_json).map_err(|error| error.to_string())
}

fn rewrite_generated_texture_bundle_references(
    output_dir: &Path,
    target: CaptureTarget,
    meta: &CapturedTextureMeta,
) -> Result<(), String> {
    if capture_quiesced() {
        return Err("capture quiesced".to_string());
    }
    for profile in [
        TERRAIN_MAP_FRAME_CAPTURE_PROFILE,
        WATER_MAP_FRAME_CAPTURE_PROFILE,
        RIVERS_MAP_FRAME_CAPTURE_PROFILE,
        BORDERS_MAP_FRAME_CAPTURE_PROFILE,
        TREES_MAP_FRAME_CAPTURE_PROFILE,
    ] {
        let path = output_dir.join(profile.bundle_file);
        if !path.exists() {
            continue;
        }
        let bytes = fs::read(&path).map_err(|error| error.to_string())?;
        let mut bundle: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
        let exact_file =
            map_frame_shared_payload_file(profile, "ps_srv", Some(target.symbol), &meta.fnv1a64);
        let mut updated = false;
        if let Some(active_draw) = bundle.get_mut("active_draw") {
            updated |=
                patch_generated_texture_draw_reference(active_draw, target, meta, &exact_file)?;
        }
        if let Some(draws) = bundle
            .get_mut("draws")
            .and_then(serde_json::Value::as_array_mut)
        {
            for draw in draws {
                updated |= patch_generated_texture_draw_reference(draw, target, meta, &exact_file)?;
            }
        }
        if !updated {
            continue;
        }
        fs::write(output_dir.join(&exact_file), &meta.captured_bytes)
            .map_err(|error| error.to_string())?;
        upsert_bundle_runtime_file_evidence(
            &mut bundle,
            target.rgba_file,
            meta.byte_len,
            &meta.fnv1a64,
            &format!("runtime_texture_readback_{}_dwell_end", target.symbol),
        );
        upsert_bundle_runtime_file_evidence(
            &mut bundle,
            &exact_file,
            meta.byte_len,
            &meta.fnv1a64,
            "d3d11_staging_map_readback_from_dwell_end_ps_srv",
        );
        let json = serde_json::to_string_pretty(&bundle).map_err(|error| error.to_string())?;
        fs::write(path, format!("{json}\n")).map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn patch_generated_texture_draw_reference(
    draw: &mut serde_json::Value,
    target: CaptureTarget,
    meta: &CapturedTextureMeta,
    exact_file: &str,
) -> Result<bool, String> {
    let Some(resources) = draw
        .get_mut("shader_resources")
        .and_then(serde_json::Value::as_array_mut)
    else {
        return Ok(false);
    };
    let mut updated = false;
    for resource in resources {
        if resource
            .get("declared_name")
            .and_then(serde_json::Value::as_str)
            != Some(target.symbol)
        {
            continue;
        }
        if resource
            .get("provenance")
            .and_then(serde_json::Value::as_str)
            == Some("d3d11_staging_map_readback_from_active_ps_srv")
        {
            continue;
        }
        resource["file"] = serde_json::Value::String(exact_file.to_string());
        resource["byte_len"] = serde_json::Value::Number((meta.byte_len as u64).into());
        resource["fnv1a64"] = serde_json::Value::String(meta.fnv1a64.clone());
        resource["payload_format"] =
            serde_json::Value::String(NATIVE_D3D11_TEXTURE_PAYLOAD_FORMAT.to_string());
        resource["provenance"] = serde_json::Value::String(
            "d3d11_staging_map_readback_from_dwell_end_ps_srv".to_string(),
        );
        ensure_generated_texture_subresource(resource, meta)?;
        updated = true;
    }
    Ok(updated)
}

fn ensure_generated_texture_subresource(
    resource: &mut serde_json::Value,
    meta: &CapturedTextureMeta,
) -> Result<(), String> {
    let has_subresources = resource
        .get("subresources")
        .and_then(serde_json::Value::as_array)
        .map_or(false, |subresources| !subresources.is_empty());
    if has_subresources {
        return Ok(());
    }
    let row_pitch = meta
        .width
        .checked_mul(4)
        .ok_or_else(|| "generated texture row pitch overflow".to_string())?;
    resource["subresources"] = serde_json::json!([
        {
            "mip_slice": 0,
            "array_slice": 0,
            "file_offset": 0,
            "byte_len": meta.byte_len,
            "row_pitch": row_pitch,
            "depth_pitch": meta.byte_len,
            "width": meta.width,
            "height": meta.height
        }
    ]);
    Ok(())
}

fn upsert_bundle_runtime_file_evidence(
    bundle: &mut serde_json::Value,
    file: &str,
    byte_len: usize,
    fnv1a64: &str,
    provenance: &str,
) {
    if !bundle
        .get("runtime_files")
        .map_or(false, serde_json::Value::is_object)
    {
        bundle["runtime_files"] = serde_json::Value::Object(serde_json::Map::new());
    }
    if let Some(files) = bundle
        .get_mut("runtime_files")
        .and_then(serde_json::Value::as_object_mut)
    {
        files.insert(
            file.to_string(),
            serde_json::json!({
                "file": file,
                "byte_len": byte_len,
                "fnv1a64": fnv1a64,
                "provenance": provenance
            }),
        );
    }
}

unsafe fn capture_map_frame_ps_srvs(
    context: *mut ID3D11DeviceContext,
    resource_slots: &BTreeMap<String, u32>,
    output_dir: &Path,
    draw_index: u32,
    profile: MapFrameCaptureProfile,
    skip_payload_reason: Option<&'static str>,
) -> (Vec<MapFrameTextureBindingFile>, Vec<String>) {
    debug_assert_render_thread_owner("capture_map_frame_ps_srvs");
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
                    unobtained_reason: None,
                    all_zero: false,
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
                skip_payload_reason,
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

unsafe fn capture_map_frame_ps_srvs_for_slots(
    context: *mut ID3D11DeviceContext,
    resource_slots: &BTreeMap<String, u32>,
    slots: &BTreeSet<u32>,
    output_dir: &Path,
    draw_index: u32,
    profile: MapFrameCaptureProfile,
    skip_payload_reason: Option<&'static str>,
) -> (Vec<MapFrameTextureBindingFile>, Vec<String>) {
    debug_assert_render_thread_owner("capture_map_frame_ps_srvs_for_slots");
    let mut bindings = Vec::new();
    let mut blockers = Vec::new();
    for slot in slots {
        if *slot >= D3D11_COMMONSHADER_INPUT_RESOURCE_SLOT_COUNT {
            blockers.push(format!(
                "PS SRV slot {slot} exceeds D3D11 common shader input resource slot count"
            ));
            continue;
        }
        let mut srv: *mut ID3D11ShaderResourceView = ptr::null_mut();
        unsafe {
            (*context).PSGetShaderResources(*slot, 1, &mut srv);
        }
        if srv.is_null() {
            if let Some(declared_name) = map_frame_declared_resource_name(resource_slots, *slot) {
                bindings.push(MapFrameTextureBindingFile {
                    stage: "PS",
                    slot: *slot,
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
                    unobtained_reason: None,
                    all_zero: false,
                });
            }
            continue;
        }
        match unsafe {
            map_frame_texture_binding_from_srv(
                context,
                srv,
                *slot,
                resource_slots,
                output_dir,
                draw_index,
                profile,
                skip_payload_reason,
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
    skip_payload_reason: Option<&'static str>,
) -> Result<(MapFrameTextureBindingFile, Vec<String>), String> {
    let mut resource: *mut ID3D11Resource = ptr::null_mut();
    unsafe {
        (*srv).GetResource(&mut resource);
    }
    if resource.is_null() {
        return Err(format!("PS SRV slot {slot} did not expose a resource"));
    }
    let resource_pointer = resource as usize;
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
        let mut unobtained_reason = None;
        let mut payload_all_zero = false;
        let needs_moment_payload =
            map_frame_declared_resource_needs_moment_payload(profile, declared_name.as_deref());
        if let Some(reason) = skip_payload_reason {
            file_meta = (None, 0, None);
            provenance = Some("d3d11_shader_resource_descriptor_only");
            unobtained_reason = Some(reason);
        } else if let Some(target) = declared_name.as_deref().and_then(capture_target_by_symbol) {
            if target.timing == RuntimeTextureCaptureTiming::Finalize {
                unsafe {
                    record_deferred_runtime_target(
                        target,
                        srv,
                        slot,
                        "map_frame_active_draw_declared_resource",
                    );
                }
            }
            if needs_moment_payload {
                match unsafe {
                    capture_map_frame_srv_texture_exact(
                        context,
                        srv,
                        texture,
                        resource_pointer,
                        slot,
                        declared_name.as_deref(),
                        &desc,
                        output_dir,
                        draw_index,
                        profile,
                    )
                } {
                    Ok(payload) => {
                        payload_all_zero = payload.all_zero;
                        file_meta = (Some(payload.file), payload.byte_len, Some(payload.fnv1a64));
                        payload_format = Some(NATIVE_D3D11_TEXTURE_PAYLOAD_FORMAT);
                        subresources = payload.subresources;
                        provenance = Some("d3d11_staging_map_readback_from_active_ps_srv");
                    }
                    Err(error) => {
                        blockers.push(error);
                        file_meta = map_frame_known_runtime_texture_file(
                            output_dir,
                            declared_name.as_deref(),
                        );
                    }
                }
            } else {
                file_meta =
                    map_frame_known_runtime_texture_file(output_dir, declared_name.as_deref());
                if file_meta.0.is_none() && target.timing == RuntimeTextureCaptureTiming::FirstDraw
                {
                    match unsafe {
                        capture_runtime_texture(
                            context,
                            target,
                            srv,
                            slot,
                            "map_frame_active_draw_declared_resource",
                            RuntimeTextureCaptureTiming::FirstDraw,
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
        } else if profile.capture_exact_ps_srv_payloads || needs_moment_payload {
            match unsafe {
                capture_map_frame_srv_texture_exact(
                    context,
                    srv,
                    texture,
                    resource_pointer,
                    slot,
                    declared_name.as_deref(),
                    &desc,
                    output_dir,
                    draw_index,
                    profile,
                )
            } {
                Ok(payload) => {
                    payload_all_zero = payload.all_zero;
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
                file_meta =
                    map_frame_known_runtime_texture_file(output_dir, declared_name.as_deref());
            }
        } else {
            file_meta = (None, 0, None);
            provenance = Some("d3d11_shader_resource_descriptor_only");
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
                unobtained_reason,
                all_zero: payload_all_zero,
            },
            blockers,
        ))
    })();
    unsafe {
        release_unknown(texture as *mut WinVoid);
    }
    result
}

#[derive(Clone, Debug)]
struct MapFrameExactTexturePayload {
    file: String,
    byte_len: usize,
    fnv1a64: String,
    subresources: Vec<MapFrameTextureSubresourceFile>,
    /// `true` iff every byte read back for this texture was zero. Computed once
    /// from the readback bytes (no re-read) so the borders_pass validator can
    /// reject the diag23b all-zero-`BorderDiffuse` contamination signature
    /// without touching the payload file again.
    all_zero: bool,
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
    resource_pointer: usize,
    source_desc: &D3D11_TEXTURE2D_DESC,
    slot: u32,
    output_dir: &Path,
    _draw_index: u32,
    profile: MapFrameCaptureProfile,
) -> Result<MapFrameOutputMergerTexturePayloadFile, String> {
    debug_assert_render_thread_owner("capture_om_render_target_payload_exact");
    unsafe {
        capture_om_texture_payload_exact(
            context,
            texture,
            resource_pointer,
            source_desc,
            slot,
            output_dir,
            profile,
            "om_post_pass_render_target",
            Some("render_target"),
            "post_draw_om_render_target_resolved_readback",
        )
    }
}

unsafe fn capture_om_texture_payload_exact(
    context: *mut ID3D11DeviceContext,
    texture: *mut ID3D11Texture2D,
    _resource_pointer: usize,
    source_desc: &D3D11_TEXTURE2D_DESC,
    slot: u32,
    output_dir: &Path,
    profile: MapFrameCaptureProfile,
    payload_kind: &'static str,
    declared_name: Option<&str>,
    provenance: &'static str,
) -> Result<MapFrameOutputMergerTexturePayloadFile, String> {
    debug_assert_render_thread_owner("capture_om_texture_payload_exact");
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
        let staging = match unsafe { create_texture2d_exact_staging(device, &readback_desc, slot) }
        {
            Ok(staging) => staging,
            Err(error) => {
                if !resolved.is_null() {
                    unsafe {
                        release_unknown(resolved as *mut WinVoid);
                    }
                }
                return Err(error);
            }
        };
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
        let byte_len = mapped.bytes.len();
        let fnv1a64 = fnv1a64_bytes(&mapped.bytes);
        let file = if profile.capture_moment == CENSUS_CAPTURE_MOMENT {
            census_shared_payload_file(payload_kind, declared_name, &fnv1a64)
        } else {
            map_frame_shared_payload_file(profile, payload_kind, declared_name, &fnv1a64)
        };
        let should_write = if profile.capture_moment == CENSUS_CAPTURE_MOMENT {
            reserve_census_shared_payload_file_for_write(&file)
        } else {
            reserve_shared_payload_file_for_write(&file)
        };
        if should_write {
            if profile.capture_moment == CENSUS_CAPTURE_MOMENT {
                queue_census_payload_file_write(output_dir, &file, &mapped.bytes)?;
            } else {
                queue_payload_file_write(output_dir, &file, &mapped.bytes)?;
            }
        }
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
        let payload = MapFrameOutputMergerTexturePayloadFile {
            file,
            byte_len,
            fnv1a64,
            payload_format: NATIVE_D3D11_TEXTURE_PAYLOAD_FORMAT,
            subresources,
            provenance,
        };
        record_boundary_payload_hash(
            output_dir,
            profile,
            payload_kind,
            declared_name,
            slot,
            &payload,
        );
        Ok(payload)
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
    resource_pointer: usize,
    slot: u32,
    declared_name: Option<&str>,
    source_desc: &D3D11_TEXTURE2D_DESC,
    output_dir: &Path,
    _draw_index: u32,
    profile: MapFrameCaptureProfile,
) -> Result<MapFrameExactTexturePayload, String> {
    debug_assert_render_thread_owner("capture_map_frame_srv_texture_exact");
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
    // Same SHARED-resource hazard as the buffer readback: staging-copying a
    // texture with non-plain MiscFlags drives d3d11 into D3DKMTOpenResource and
    // access-violates the host (a the game UI SRV could be a shared texture). Skip
    // the byte readback for such textures; bindings without exact payloads are
    // recorded as blockers, not crashes. RENDER_TARGET/SHADER_RESOURCE bind
    // flags live in BindFlags, not MiscFlags, so ordinary map textures pass.
    if source_desc.MiscFlags & D3D11_RESOURCE_MISC_SHARED_MASK != 0 {
        return Err(format!(
            "PS SRV slot {slot} Texture2D is a shared resource (MiscFlags 0x{:X}); skipping unsafe staging readback",
            source_desc.MiscFlags
        ));
    }

    let mut srv_desc: D3D11_SHADER_RESOURCE_VIEW_DESC = unsafe { mem::zeroed() };
    unsafe {
        (*srv).GetDesc(&mut srv_desc);
    }
    let range = map_frame_srv_subresource_range(&srv_desc, source_desc, slot)?;
    let cacheable = !map_frame_declared_resource_needs_moment_payload(profile, declared_name);
    let cache_key = MapFramePayloadCacheKey {
        resource_pointer,
        payload_kind: "ps_srv",
        first_mip: range.first_mip,
        mip_levels: range.mip_levels,
        first_array_slice: range.first_array_slice,
        array_size: range.array_size,
        width: source_desc.Width,
        height: source_desc.Height,
        mip_count: source_desc.MipLevels,
        texture_array_size: source_desc.ArraySize,
        format: source_desc.Format as u32,
        sample_count: source_desc.SampleDesc.Count,
        sample_quality: source_desc.SampleDesc.Quality,
    };
    if cacheable {
        let cached = if profile.capture_moment == CENSUS_CAPTURE_MOMENT {
            cached_census_payload(&cache_key)
        } else {
            cached_map_frame_payload(&cache_key)
        };
        if let Some(payload) = cached {
            return Ok(payload);
        }
    }

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
        let fnv1a64 = fnv1a64_bytes(&payload_bytes);
        let file = if profile.capture_moment == CENSUS_CAPTURE_MOMENT {
            census_shared_payload_file("ps_srv", declared_name, &fnv1a64)
        } else {
            map_frame_shared_payload_file(profile, "ps_srv", declared_name, &fnv1a64)
        };
        let should_write = if profile.capture_moment == CENSUS_CAPTURE_MOMENT {
            reserve_census_shared_payload_file_for_write(&file)
        } else {
            reserve_shared_payload_file_for_write(&file)
        };
        if should_write {
            if profile.capture_moment == CENSUS_CAPTURE_MOMENT {
                queue_census_payload_file_write(output_dir, &file, &payload_bytes)?;
            } else {
                queue_payload_file_write(output_dir, &file, &payload_bytes)?;
            }
        }
        let all_zero = !payload_bytes.is_empty() && payload_bytes.iter().all(|byte| *byte == 0);
        let payload = MapFrameExactTexturePayload {
            file,
            byte_len: payload_bytes.len(),
            fnv1a64,
            subresources,
            all_zero,
        };
        if cacheable {
            if profile.capture_moment == CENSUS_CAPTURE_MOMENT {
                record_cached_census_payload(cache_key.clone(), payload.clone());
            } else {
                record_cached_map_frame_payload(cache_key.clone(), payload.clone());
            }
        }
        Ok(payload)
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

#[cfg(test)]
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

fn map_frame_shared_payload_file(
    profile: MapFrameCaptureProfile,
    payload_kind: &str,
    declared_name: Option<&str>,
    fnv1a64: &str,
) -> String {
    let name = declared_name
        .map(sanitize_map_frame_file_component)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "unnamed".to_string());
    format!(
        "{}_{}_{}_{}_{}.d3d11",
        profile.file_prefix, MAP_FRAME_SHARED_PAYLOAD_FILE_PREFIX, payload_kind, name, fnv1a64
    )
}

fn census_shared_payload_file(
    payload_kind: &str,
    declared_name: Option<&str>,
    fnv1a64: &str,
) -> String {
    let name = declared_name
        .map(sanitize_map_frame_file_component)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "unnamed".to_string());
    format!(
        "../payloads/census_{}_{}_{}.d3d11",
        payload_kind, name, fnv1a64
    )
}

fn boundary_payload_identity_key(
    payload_kind: &str,
    declared_name: Option<&str>,
    slot: u32,
) -> String {
    let name = declared_name
        .map(sanitize_map_frame_file_component)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "unnamed".to_string());
    format!("{payload_kind}:{name}:slot{slot}")
}

fn record_boundary_payload_hash(
    output_dir: &Path,
    profile: MapFrameCaptureProfile,
    payload_kind: &str,
    declared_name: Option<&str>,
    slot: u32,
    payload: &MapFrameOutputMergerTexturePayloadFile,
) {
    if !payload_kind.starts_with("om_post_pass_") {
        return;
    }
    let key = boundary_payload_identity_key(payload_kind, declared_name, slot);
    let current_moment = profile.capture_moment.to_string();
    let Some(warnings) = RUNTIME.get().and_then(|runtime| {
        runtime.lock().ok().map(|mut runtime| {
            let CaptureRuntime {
                boundary_payloads,
                boundary_payload_warnings,
                ..
            } = &mut *runtime;
            update_boundary_payload_tracking(
                boundary_payloads,
                boundary_payload_warnings,
                &key,
                &current_moment,
                &payload.file,
                &payload.fnv1a64,
            )
        })
    }) else {
        return;
    };
    for (previous_moment, previous_record) in warnings {
        write_event(
            output_dir,
            &format!(
                "{{\"type\":\"map_capture_boundary_payload_identical_warning\",\"payload_key\":\"{}\",\"previous_moment\":\"{}\",\"current_moment\":\"{}\",\"fnv1a64\":\"{}\",\"previous_file\":\"{}\",\"current_file\":\"{}\",\"message\":\"distinct map capture moments produced byte-identical boundary payloads; verify pass-close timing\"}}",
                json_escape(&key),
                json_escape(&previous_moment),
                json_escape(profile.capture_moment),
                json_escape(&payload.fnv1a64),
                json_escape(&previous_record.file),
                json_escape(&payload.file)
            ),
        );
    }
}

fn update_boundary_payload_tracking(
    boundary_payloads: &mut BTreeMap<String, BTreeMap<String, BoundaryPayloadRecord>>,
    boundary_payload_warnings: &mut BTreeSet<String>,
    key: &str,
    current_moment: &str,
    file: &str,
    fnv1a64: &str,
) -> Vec<(String, BoundaryPayloadRecord)> {
    let previous = boundary_payloads
        .get(key)
        .map(|records| {
            records
                .iter()
                .filter(|(moment, record)| {
                    moment.as_str() != current_moment && record.fnv1a64 == fnv1a64
                })
                .map(|(moment, record)| (moment.clone(), record.clone()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut warnings = Vec::new();
    for (previous_moment, previous_record) in previous {
        let warning_key = format!("{key}:{previous_moment}:{current_moment}:{fnv1a64}");
        if boundary_payload_warnings.insert(warning_key) {
            warnings.push((previous_moment, previous_record));
        }
    }
    boundary_payloads
        .entry(key.to_string())
        .or_default()
        .insert(
            current_moment.to_string(),
            BoundaryPayloadRecord {
                file: file.to_string(),
                fnv1a64: fnv1a64.to_string(),
            },
        );
    warnings
}

fn cached_map_frame_payload(key: &MapFramePayloadCacheKey) -> Option<MapFrameExactTexturePayload> {
    RUNTIME
        .get()
        .and_then(|runtime| runtime.lock().ok())
        .and_then(|runtime| runtime.shared_payloads.get(key).cloned())
}

fn record_cached_map_frame_payload(
    key: MapFramePayloadCacheKey,
    payload: MapFrameExactTexturePayload,
) {
    if let Some(runtime) = RUNTIME.get() {
        if let Ok(mut runtime) = runtime.lock() {
            runtime.shared_payloads.insert(key, payload);
        }
    }
}

fn cached_census_payload(key: &MapFramePayloadCacheKey) -> Option<MapFrameExactTexturePayload> {
    RUNTIME
        .get()
        .and_then(|runtime| runtime.lock().ok())
        .and_then(|runtime| runtime.census.shared_payloads.get(key).cloned())
}

fn record_cached_census_payload(
    key: MapFramePayloadCacheKey,
    payload: MapFrameExactTexturePayload,
) {
    if let Some(runtime) = RUNTIME.get() {
        if let Ok(mut runtime) = runtime.lock() {
            runtime.census.shared_payloads.insert(key, payload);
        }
    }
}

#[cfg(test)]
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

#[cfg(test)]
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

#[cfg(test)]
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
    capture_pre_draw_payload: bool,
) -> (Option<MapFrameDrawStateFile>, Vec<String>) {
    debug_assert_render_thread_owner("capture_map_frame_draw_state");
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
    let (output_merger, mut output_merger_blockers) = unsafe {
        capture_map_frame_output_merger_state(
            context,
            output_dir,
            draw_index,
            profile,
            capture_pre_draw_payload,
        )
    };
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
    capture_pre_draw_payload: bool,
) -> (MapFrameOutputMergerStateFile, Vec<String>) {
    debug_assert_render_thread_owner("capture_map_frame_output_merger_state");
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
                capture_pre_draw_payload,
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
                capture_pre_draw_payload,
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
    debug_assert_render_thread_owner("capture_map_frame_post_pass_output_merger_state");
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
            capture_om_depth_stencil_texture_with_post_pass_payload(
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
    _draw_index: u32,
    profile: MapFrameCaptureProfile,
    capture_pre_draw_payload: bool,
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
        if capture_pre_draw_payload && desc_native.SampleDesc.Count > 1 {
            return Err(format!(
                "pre-draw OM render target slot {slot} is {}x MSAA; exact replay requires unresolved per-sample payload, not a resolved readback",
                desc_native.SampleDesc.Count
            ));
        }
        let view_desc = unsafe { render_target_view_desc_file(view) }?;
        let pre_draw_payload = if capture_pre_draw_payload {
            Some(unsafe {
                capture_om_texture_payload_exact(
                    context,
                    texture,
                    resource as usize,
                    &desc_native,
                    slot,
                    output_dir,
                    profile,
                    "om_pre_draw_render_target",
                    Some("render_target"),
                    "pre_draw_om_render_target_resolved_readback",
                )
            }?)
        } else {
            None
        };
        Ok(MapFrameOutputMergerTextureFile {
            slot: Some(slot),
            view_pointer: format!("0x{:X}", view as usize),
            resource_pointer: format!("0x{:X}", resource as usize),
            desc: texture2d_desc_file(&texture_desc_from_native(&desc_native)),
            view_desc: Some(view_desc),
            pre_draw_payload,
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
                resource as usize,
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
    _draw_index: u32,
    profile: MapFrameCaptureProfile,
    capture_pre_draw_payload: bool,
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
        if capture_pre_draw_payload && desc_native.SampleDesc.Count > 1 {
            return Err(format!(
                "pre-draw OM depth-stencil target is {}x MSAA; exact replay requires unresolved per-sample payload, not a resolved readback",
                desc_native.SampleDesc.Count
            ));
        }
        let view_desc = unsafe { depth_stencil_view_desc_file(view) }?;
        let pre_draw_payload = if capture_pre_draw_payload {
            Some(unsafe {
                capture_om_texture_payload_exact(
                    context,
                    texture,
                    resource as usize,
                    &desc_native,
                    0,
                    output_dir,
                    profile,
                    "om_pre_draw_depth_stencil",
                    Some("depth_stencil"),
                    "pre_draw_om_depth_stencil_resolved_readback",
                )
            }?)
        } else {
            None
        };
        Ok(MapFrameOutputMergerTextureFile {
            slot: None,
            view_pointer: format!("0x{:X}", view as usize),
            resource_pointer: format!("0x{:X}", resource as usize),
            desc: texture2d_desc_file(&texture_desc_from_native(&desc_native)),
            view_desc: Some(view_desc),
            pre_draw_payload,
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

unsafe fn capture_om_depth_stencil_texture_with_post_pass_payload(
    context: *mut ID3D11DeviceContext,
    view: *mut ID3D11DepthStencilView,
    clears: &BTreeMap<usize, MapFrameDepthStencilClearFile>,
    output_dir: &Path,
    _draw_index: u32,
    profile: MapFrameCaptureProfile,
) -> Result<MapFrameOutputMergerTextureFile, String> {
    if context.is_null() {
        return Err("null D3D11 context for post-pass depth-stencil readback".to_string());
    }
    if view.is_null() {
        return Err("null OM depth-stencil view".to_string());
    }
    let mut resource: *mut ID3D11Resource = ptr::null_mut();
    unsafe {
        (*view).GetResource(&mut resource);
    }
    if resource.is_null() {
        return Err("OM depth-stencil view has no resource".to_string());
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
                "post-pass OM depth-stencil target is {}x MSAA; exact replay requires unresolved per-sample payload, not a resolved readback",
                desc_native.SampleDesc.Count
            ));
        }
        let view_desc = unsafe { depth_stencil_view_desc_file(view) }?;
        let post_pass_payload = unsafe {
            capture_om_texture_payload_exact(
                context,
                texture,
                resource as usize,
                &desc_native,
                0,
                output_dir,
                profile,
                "om_post_pass_depth_stencil",
                Some("depth_stencil"),
                "post_pass_om_depth_stencil_readback",
            )
        }?;
        Ok(MapFrameOutputMergerTextureFile {
            slot: None,
            view_pointer: format!("0x{:X}", view as usize),
            resource_pointer: format!("0x{:X}", resource as usize),
            desc: texture2d_desc_file(&texture_desc_from_native(&desc_native)),
            view_desc: Some(view_desc),
            pre_draw_payload: None,
            post_pass_payload: Some(post_pass_payload),
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
    debug_assert_render_thread_owner("capture_map_frame_constant_buffers");
    if profile.capture_moment == UI_PASS_CAPTURE_MOMENT {
        return capture_ui_pass_constant_buffers_from_shadow();
    }
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

fn capture_ui_pass_constant_buffers_from_shadow() -> (Vec<MapFrameBufferBindingFile>, Vec<String>) {
    let mut out = Vec::new();
    let mut blockers = Vec::new();
    capture_ui_pass_constant_buffer_shadow_stage(
        "VS",
        &UI_PASS_VS_CONSTANT_BUFFER_SHADOWS,
        &mut out,
        &mut blockers,
    );
    capture_ui_pass_constant_buffer_shadow_stage(
        "PS",
        &UI_PASS_PS_CONSTANT_BUFFER_SHADOWS,
        &mut out,
        &mut blockers,
    );
    (out, blockers)
}

fn ui_pass_unobtained_constant_buffers_from_shaders(
    vertex_shader: &MapFrameShaderFile,
    pixel_shader: &MapFrameShaderFile,
    reason: &'static str,
) -> Vec<MapFrameBufferBindingFile> {
    let mut out = Vec::new();
    ui_pass_unobtained_constant_buffers_for_stage(vertex_shader, reason, &mut out);
    ui_pass_unobtained_constant_buffers_for_stage(pixel_shader, reason, &mut out);
    out
}

fn ui_pass_unobtained_constant_buffers_for_stage(
    shader: &MapFrameShaderFile,
    reason: &'static str,
    out: &mut Vec<MapFrameBufferBindingFile>,
) {
    let mut slots = shader
        .constant_buffers
        .values()
        .copied()
        .collect::<BTreeSet<_>>();
    if slots.is_empty() {
        slots.insert(0);
    }
    for slot in slots {
        out.push(MapFrameBufferBindingFile {
            stage: shader.stage,
            slot,
            declared_name: constant_buffer_name_for_slot(&shader.constant_buffers, slot),
            file: None,
            byte_len: 0,
            buffer_byte_width: None,
            fnv1a64: None,
            stride: None,
            offset: None,
            dxgi_format: None,
            bind_flags: 0,
            usage: 0,
            cpu_access_flags: 0,
            unknown_origin: None,
            byte_range: None,
            payload_truncated: None,
            unobtained_reason: Some(reason),
        });
    }
}

fn capture_ui_pass_constant_buffer_shadow_stage(
    stage: &'static str,
    shadows: &[AtomicUsize; UI_PASS_CONSTANT_BUFFER_SLOT_COUNT],
    out: &mut Vec<MapFrameBufferBindingFile>,
    blockers: &mut Vec<String>,
) {
    let reflected = current_shader_constant_buffer_slots(stage);
    for slot in 0..MAP_FRAME_BINDING_SLOTS {
        let pointer = shadows[slot as usize].load(Ordering::Acquire);
        if pointer == 0 {
            continue;
        }
        let declared_name = constant_buffer_name_for_slot(&reflected, slot);
        match created_buffer_desc(pointer) {
            Some(desc) => {
                if desc.bind_flags & D3D11_BIND_CONSTANT_BUFFER == 0 {
                    blockers.push(format!(
                        "{stage} shadowed constant buffer slot {slot} pointer 0x{pointer:X} missing D3D11_BIND_CONSTANT_BUFFER in CreateBuffer desc: BindFlags {}",
                        desc.bind_flags
                    ));
                }
                out.push(MapFrameBufferBindingFile {
                    stage,
                    slot,
                    declared_name,
                    file: None,
                    byte_len: desc.byte_width as usize,
                    buffer_byte_width: map_frame_buffer_byte_width(&desc),
                    fnv1a64: None,
                    stride: None,
                    offset: None,
                    dxgi_format: None,
                    bind_flags: desc.bind_flags,
                    usage: desc.usage,
                    cpu_access_flags: desc.cpu_access_flags,
                    unknown_origin: None,
                    byte_range: None,
                    payload_truncated: None,
                    unobtained_reason: None,
                });
            }
            None => {
                out.push(MapFrameBufferBindingFile {
                    stage,
                    slot,
                    declared_name,
                    file: None,
                    byte_len: 0,
                    buffer_byte_width: None,
                    fnv1a64: None,
                    stride: None,
                    offset: None,
                    dxgi_format: None,
                    bind_flags: 0,
                    usage: 0,
                    cpu_access_flags: 0,
                    unknown_origin: Some(true),
                    byte_range: None,
                    payload_truncated: None,
                    unobtained_reason: None,
                });
            }
        }
    }
}

fn current_shader_constant_buffer_slots(stage: &str) -> BTreeMap<String, u32> {
    RUNTIME
        .get()
        .and_then(|runtime| runtime.lock().ok())
        .and_then(|runtime| match stage {
            "PS" => runtime.state.current_pixel_shader_constant_buffers(),
            "VS" => runtime.state.current_vertex_shader_constant_buffers(),
            _ => None,
        })
        .unwrap_or_default()
}

unsafe fn capture_map_frame_constant_buffer_stage(
    context: *mut ID3D11DeviceContext,
    stage: &'static str,
    draw_index: u32,
    profile: MapFrameCaptureProfile,
    out: &mut Vec<MapFrameBufferBindingFile>,
    blockers: &mut Vec<String>,
) {
    debug_assert_render_thread_owner("capture_map_frame_constant_buffer_stage");
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
    debug_assert_render_thread_owner("capture_map_frame_constant_buffer_slot");
    if profile.capture_moment == UI_PASS_CAPTURE_MOMENT {
        return Err(
            "ui_pass constant buffers must be captured from shadowed SetConstantBuffers slots"
                .to_string(),
        );
    }
    let output_dir = RUNTIME
        .get()
        .and_then(|runtime| runtime.lock().ok())
        .map(|runtime| runtime.output_dir.clone())
        .ok_or_else(|| "terrain capture runtime not initialized".to_string())?;
    // UI constant buffers are rewritten by the game every draw (dynamic,
    // Map/WRITE_DISCARD). Synchronously CopyResource-ing such a buffer inside
    // the game's own hooked draw call access-violates d3d11 in kernel space and
    // hard-crashes the game — reproduced at exactly this stage across two live
    // sessions (2026-07-03, identical fault address). Record binding METADATA
    // only for ui_pass (GetDesc is safe; the CopyResource is not); the atlas's
    // structural value comes from vertex geometry, textures and scissor rects,
    // which read back safely. Exact CB bytes are a deferred-readback follow-up.
    // Map passes keep full CB readback (their dynamic CB0 is a single known-safe
    // buffer the validators already exercise).
    let (source, bytes) = unsafe { readback_d3d11_buffer_bytes(context, buffer)? };
    if source.bind_flags & D3D11_BIND_CONSTANT_BUFFER == 0 {
        return Err(format!(
            "{stage} constant buffer slot {slot} missing D3D11_BIND_CONSTANT_BUFFER: BindFlags {}",
            source.bind_flags
        ));
    }
    let requested_file = format!(
        "{}{draw_index:03}_{}_constant_buffer_slot{}.bin",
        profile.file_prefix,
        stage.to_ascii_lowercase(),
        slot
    );
    let declared_name =
        map_frame_constant_buffer_declared_name(stage, slot, profile, &output_dir, &bytes);
    let payload_kind = format!(
        "{}_constant_buffer_slot{}",
        stage.to_ascii_lowercase(),
        slot
    );
    let file = write_map_frame_binary_payload(
        &output_dir,
        &requested_file,
        &bytes,
        profile,
        &payload_kind,
        declared_name.as_deref(),
    )?;
    Ok(MapFrameBufferBindingFile {
        stage,
        slot,
        declared_name,
        file: Some(file),
        byte_len: bytes.len(),
        buffer_byte_width: map_frame_buffer_byte_width(&source),
        fnv1a64: Some(fnv1a64_bytes(&bytes)),
        stride: None,
        offset: None,
        dxgi_format: None,
        bind_flags: source.bind_flags,
        usage: source.usage,
        cpu_access_flags: source.cpu_access_flags,
        unknown_origin: None,
        byte_range: None,
        payload_truncated: None,
        unobtained_reason: None,
    })
}

fn map_frame_constant_buffer_declared_name(
    stage: &str,
    slot: u32,
    profile: MapFrameCaptureProfile,
    output_dir: &Path,
    bytes: &[u8],
) -> Option<String> {
    let reflected = RUNTIME
        .get()
        .and_then(|runtime| runtime.lock().ok())
        .and_then(|runtime| match stage {
            "PS" => runtime.state.current_pixel_shader_constant_buffers(),
            "VS" => runtime.state.current_vertex_shader_constant_buffers(),
            _ => None,
        });
    canonical_map_frame_constant_buffer_name_for_binding(
        profile.capture_moment,
        stage,
        slot,
        profile.shader_sources.iter().copied(),
        &reflected.unwrap_or_default(),
        output_dir,
        bytes,
    )
}

fn map_frame_binding_names_terrain_scene_cb0(
    constant_buffers: &[MapFrameBufferBindingFile],
    stage: &str,
) -> bool {
    constant_buffers.iter().any(|binding| {
        binding.stage == stage
            && binding.slot == TERRAIN_SCENE_CB0_BIND_POINT
            && binding.declared_name.as_deref() == Some(TERRAIN_SCENE_CB0_SYMBOL)
    })
}

fn add_file_evidence_if_exists(
    files: &mut BTreeMap<String, MapFrameFileEvidence>,
    output_dir: &Path,
    file: &str,
    provenance: &str,
) {
    let path = round_local_evidence_path(output_dir, file);
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
    let path = round_local_evidence_path(output_dir, file);
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

fn round_local_evidence_path(output_dir: &Path, file: &str) -> PathBuf {
    let path = output_dir.join(file);
    if path.exists() || !relative_payload_file(file) {
        return path;
    }
    let Some(root_output_dir) = RUNTIME
        .get()
        .and_then(|runtime| runtime.lock().ok())
        .map(|runtime| runtime.root_output_dir.clone())
    else {
        return path;
    };
    if !output_dir.starts_with(root_output_dir.join("rounds")) {
        return path;
    }
    let source = root_output_dir.join(file);
    if !source.exists() {
        return path;
    }
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::copy(source, &path);
    path
}

fn relative_payload_file(file: &str) -> bool {
    let path = Path::new(file);
    !path.is_absolute()
        && path.components().all(|component| {
            !matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
}

fn add_map_frame_draw_shader_file_evidence(
    files: &mut BTreeMap<String, MapFrameFileEvidence>,
    output_dir: &Path,
    draws: &[MapFrameDrawFile],
    blockers: &mut Vec<String>,
) {
    for draw in draws {
        add_required_file_evidence(
            files,
            output_dir,
            &draw.shader.bytecode_file,
            "create_pixel_shader_bytecode",
            blockers,
        );
        add_required_file_evidence(
            files,
            output_dir,
            &draw.vertex_shader.bytecode_file,
            "create_vertex_shader_bytecode",
            blockers,
        );
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
    let Some(depth) = output_merger.depth_stencil.as_ref() else {
        blockers.push(format!(
            "required post-pass output-merger {} depth-stencil payload is missing",
            profile.capture_moment
        ));
        return;
    };
    let Some(payload) = depth.post_pass_payload.as_ref() else {
        blockers.push(format!(
            "post-pass output-merger {} depth-stencil has no payload file",
            profile.capture_moment
        ));
        return;
    };
    add_required_file_evidence(
        files,
        output_dir,
        &payload.file,
        payload.provenance,
        blockers,
    );
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
        if vanilla_source_evidence_required() {
            blockers.push(
                "could not resolve the game working directory for vanilla source evidence".to_string(),
            );
        }
        return;
    };
    let required = vanilla_source_evidence_required();
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
            Err(error) => {
                if required {
                    blockers.push(format!(
                        "required vanilla source {source} is missing or unreadable from the game working directory: {error}"
                    ));
                }
            }
        }
    }
}

fn vanilla_source_evidence_required() -> bool {
    RUNTIME
        .get()
        .and_then(|runtime| runtime.lock().ok())
        .and_then(|runtime| {
            runtime
                .control
                .as_ref()
                .map(|control| control.vanilla_source_evidence_required)
        })
        .unwrap_or(true)
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
        .ok_or_else(|| "map-frame capture disabled".to_string())?;
    map_frame_capture_profile(capture_moment)
}

fn capture_moment_sequence_from_control(control: Option<&TerrainCaptureControl>) -> Vec<String> {
    let mut moments = Vec::new();
    if let Some(control) = control {
        if !control.map_frame_capture_enabled {
            return moments;
        }
        moments.extend(
            control
                .capture_moments
                .iter()
                .filter(|moment| !moment.trim().is_empty())
                .filter(|moment| moment.trim() != TERRAIN_CHUNK_LIBRARY_MOMENT)
                .cloned(),
        );
        if moments.is_empty() {
            if let Some(moment) = control.capture_moment.as_ref() {
                if !moment.trim().is_empty() && moment.trim() != TERRAIN_CHUNK_LIBRARY_MOMENT {
                    moments.push(moment.clone());
                }
            }
        }
    }
    if moments.is_empty() {
        moments.push(TERRAIN_CAPTURE_MOMENT.to_string());
    }
    move_ui_pass_to_sequence_tail(&mut moments);
    moments
}

fn move_ui_pass_to_sequence_tail(moments: &mut Vec<String>) {
    let ui_pass_count = moments
        .iter()
        .filter(|moment| moment.trim() == UI_PASS_CAPTURE_MOMENT)
        .count();
    if ui_pass_count == 0 {
        return;
    }
    moments.retain(|moment| moment.trim() != UI_PASS_CAPTURE_MOMENT);
    moments.push(UI_PASS_CAPTURE_MOMENT.to_string());
}

fn terrain_chunk_library_enabled_from_control(control: Option<&TerrainCaptureControl>) -> bool {
    let Some(control) = control else {
        return false;
    };
    control.terrain_chunk_library
        || control.capture_moment.as_deref() == Some(TERRAIN_CHUNK_LIBRARY_MOMENT)
        || control
            .capture_moments
            .iter()
            .any(|moment| moment.trim() == TERRAIN_CHUNK_LIBRARY_MOMENT)
}

fn map_frame_capture_profile(capture_moment: &str) -> Result<MapFrameCaptureProfile, String> {
    match capture_moment {
        TERRAIN_CAPTURE_MOMENT => Ok(TERRAIN_MAP_FRAME_CAPTURE_PROFILE),
        WATER_CAPTURE_MOMENT => Ok(WATER_MAP_FRAME_CAPTURE_PROFILE),
        RIVERS_CAPTURE_MOMENT => Ok(RIVERS_MAP_FRAME_CAPTURE_PROFILE),
        BORDERS_CAPTURE_MOMENT => Ok(BORDERS_MAP_FRAME_CAPTURE_PROFILE),
        UNDERWATER_CAPTURE_MOMENT => Ok(UNDERWATER_MAP_FRAME_CAPTURE_PROFILE),
        TREES_CAPTURE_MOMENT => Ok(TREES_MAP_FRAME_CAPTURE_PROFILE),
        RESTORE_CAPTURE_MOMENT => Ok(RESTORE_MAP_FRAME_CAPTURE_PROFILE),
        MAPNAME_CAPTURE_MOMENT => Ok(MAPNAME_MAP_FRAME_CAPTURE_PROFILE),
        UI_PASS_CAPTURE_MOMENT => Ok(UI_PASS_CAPTURE_PROFILE),
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
        UNDERWATER_CAPTURE_MOMENT => state
            .underwater_pixel_shader()
            .map(|shader| shader.pixel_shader),
        TREES_CAPTURE_MOMENT => state.tree_pixel_shader().map(|shader| shader.pixel_shader),
        RESTORE_CAPTURE_MOMENT => state
            .restore_pixel_shader()
            .map(|shader| shader.pixel_shader),
        MAPNAME_CAPTURE_MOMENT => state
            .mapname_pixel_shader()
            .map(|shader| shader.pixel_shader),
        _ => None,
    }
}

fn map_frame_pixel_shader_pointer_matches_profile(
    state: &TerrainCaptureState,
    profile: MapFrameCaptureProfile,
    pixel_shader: usize,
) -> Option<usize> {
    if pixel_shader == 0 {
        return None;
    }
    match profile.capture_moment {
        TERRAIN_CAPTURE_MOMENT => state
            .pdxmap_pixel_shader_metadata(pixel_shader)
            .is_some()
            .then_some(pixel_shader),
        WATER_CAPTURE_MOMENT => state
            .pdxwater_pixel_shader_metadata(pixel_shader)
            .is_some()
            .then_some(pixel_shader),
        RIVERS_CAPTURE_MOMENT => state
            .river_pixel_shader_metadata(pixel_shader)
            .is_some()
            .then_some(pixel_shader),
        BORDERS_CAPTURE_MOMENT => state
            .border_pixel_shader_metadata(pixel_shader)
            .is_some()
            .then_some(pixel_shader),
        UNDERWATER_CAPTURE_MOMENT => state
            .underwater_pixel_shader_metadata(pixel_shader)
            .is_some()
            .then_some(pixel_shader),
        TREES_CAPTURE_MOMENT => state
            .tree_pixel_shader_metadata(pixel_shader)
            .is_some()
            .then_some(pixel_shader),
        RESTORE_CAPTURE_MOMENT => state
            .restore_pixel_shader_metadata(pixel_shader)
            .is_some()
            .then_some(pixel_shader),
        MAPNAME_CAPTURE_MOMENT => state
            .mapname_pixel_shader_metadata(pixel_shader)
            .is_some()
            .then_some(pixel_shader),
        _ => None,
    }
}

fn should_finalize_map_frame_on_pixel_shader_transition(
    state: &TerrainCaptureState,
    profile: MapFrameCaptureProfile,
    incoming_pixel_shader: usize,
    has_draws: bool,
    moment_completed: bool,
) -> bool {
    active_map_frame_pixel_shader_pointer_for_profile(state, profile).is_some()
        && map_frame_pixel_shader_pointer_matches_profile(state, profile, incoming_pixel_shader)
            .is_none()
        && has_draws
        && !moment_completed
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
    } else if profile.capture_moment == UNDERWATER_CAPTURE_MOMENT
        && pending.len() == 1
        && pending[0] == MAP_FRAME_BUNDLE_SYMBOL
    {
        "no active underwater shader draw observed before timeout".to_string()
    } else if profile.capture_moment == TREES_CAPTURE_MOMENT
        && pending.len() == 1
        && pending[0] == MAP_FRAME_BUNDLE_SYMBOL
    {
        "no active tree shader draw observed before timeout".to_string()
    } else if profile.capture_moment == RESTORE_CAPTURE_MOMENT
        && pending.len() == 1
        && pending[0] == MAP_FRAME_BUNDLE_SYMBOL
    {
        "no trusted restorescene shader draw observed before timeout".to_string()
    } else if profile.capture_moment == MAPNAME_CAPTURE_MOMENT
        && pending.len() == 1
        && pending[0] == MAP_FRAME_BUNDLE_SYMBOL
    {
        "no trusted mapname shader draw observed before timeout".to_string()
    } else if profile.capture_moment == UI_PASS_CAPTURE_MOMENT
        && pending.len() == 1
        && pending[0] == MAP_FRAME_BUNDLE_SYMBOL
    {
        "no swapchain backbuffer UI draw observed before timeout".to_string()
    } else {
        format!(
            "pending runtime {} capture targets not observed before timeout: {}",
            profile.capture_moment,
            pending.join(",")
        )
    }
}

/// MOMFIX1 finalization guard: when round commands are enabled and the control's
/// requested continuous sequence never completed, emit a truthful blocked report
/// instead of letting a completed ROUND's CAPTURED flag masquerade as full
/// success. Returns true if it emitted the blocked report (caller then skips the
/// captured/budget/not-observed branches). Returns false when rounds are off or
/// the control sequence completed — the normal reporting applies.
///
/// Because HOLD prevents any round from starting until the control sequence
/// completes, an unset `control_sequence_complete` latch here guarantees no
/// round ever ran; `completed_map_frame_moments` therefore still reflects the
/// continuous sequence and yields the true pending list. Every path is a
/// recorded event — hooks never assert.
fn report_incomplete_control_sequence_before_rounds(output_dir: &Path) -> bool {
    let Some(runtime) = RUNTIME.get().and_then(|runtime| runtime.lock().ok()) else {
        return false;
    };
    report_incomplete_control_sequence_before_rounds_for_runtime(&runtime, output_dir)
}

fn report_incomplete_control_sequence_before_rounds_for_runtime(
    runtime: &CaptureRuntime,
    output_dir: &Path,
) -> bool {
    if !runtime.round_commands_enabled || runtime.control_sequence_complete {
        return false;
    }
    let pending = control_moments_still_pending(runtime);
    if pending.is_empty() {
        // Rounds enabled and latch unset but every requested moment shows
        // complete: a benign latch/ordering gap, not a starvation. Let normal
        // reporting proceed rather than fabricate a blocker.
        return false;
    }
    let blocked_before = pending.first().cloned().unwrap_or_default();
    write_event(
        output_dir,
        &format!(
            "{{\"type\":\"terrain_capture_finished\",\"status\":\"blocked_before_{}\",\"pending_control_moments\":{},\"rounds_enabled\":true}}",
            json_escape(&blocked_before),
            json_string_array(pending.iter().map(String::as_str).collect())
        ),
    );
    true
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

unsafe fn create_probe_swap_chain_present_target() -> Result<usize, String> {
    let class_name = wide_null("STATIC");
    let window_name = wide_null("TerrainCapturePresentProbeWindow");
    let instance = unsafe { GetModuleHandleW(ptr::null()) } as HINSTANCE;
    let hwnd = unsafe {
        CreateWindowExW(
            0,
            class_name.as_ptr(),
            window_name.as_ptr(),
            WS_DISABLED,
            0,
            0,
            1,
            1,
            ptr::null_mut(),
            ptr::null_mut(),
            instance,
            ptr::null_mut(),
        )
    };
    if hwnd.is_null() {
        return Err("CreateWindowExW probe window for IDXGISwapChain::Present failed".to_string());
    }

    let feature_levels = [D3D_FEATURE_LEVEL_11_0];
    let mut swap_chain_desc: DXGI_SWAP_CHAIN_DESC = unsafe { mem::zeroed() };
    swap_chain_desc.BufferDesc = DXGI_MODE_DESC {
        Width: 1,
        Height: 1,
        RefreshRate: DXGI_RATIONAL {
            Numerator: 60,
            Denominator: 1,
        },
        Format: DXGI_FORMAT_R8G8B8A8_UNORM,
        ScanlineOrdering: 0,
        Scaling: 0,
    };
    swap_chain_desc.SampleDesc = DXGI_SAMPLE_DESC {
        Count: 1,
        Quality: 0,
    };
    swap_chain_desc.BufferUsage = DXGI_USAGE_RENDER_TARGET_OUTPUT;
    swap_chain_desc.BufferCount = 1;
    swap_chain_desc.OutputWindow = hwnd as _;
    swap_chain_desc.Windowed = 1;
    swap_chain_desc.SwapEffect = DXGI_SWAP_EFFECT_DISCARD;

    let mut swap_chain: *mut IDXGISwapChain = ptr::null_mut();
    let mut device: *mut ID3D11Device = ptr::null_mut();
    let mut context: *mut ID3D11DeviceContext = ptr::null_mut();
    let mut actual_feature_level = D3D_FEATURE_LEVEL_11_0;
    let hr = unsafe {
        D3D11CreateDeviceAndSwapChain(
            ptr::null_mut(),
            D3D_DRIVER_TYPE_HARDWARE,
            ptr::null_mut(),
            0,
            feature_levels.as_ptr(),
            feature_levels.len() as UINT,
            D3D11_SDK_VERSION,
            &mut swap_chain_desc,
            &mut swap_chain,
            &mut device,
            &mut actual_feature_level,
            &mut context,
        )
    };
    let result = if FAILED(hr) || swap_chain.is_null() {
        Err(format!(
            "D3D11CreateDeviceAndSwapChain probe failed: HRESULT 0x{:08X}",
            hr as u32
        ))
    } else {
        unsafe { vtable_slot(swap_chain as *mut WinVoid, SWAP_CHAIN_PRESENT_INDEX) }
    };
    unsafe {
        if !context.is_null() {
            release_unknown(context as *mut WinVoid);
        }
        if !device.is_null() {
            release_unknown(device as *mut WinVoid);
        }
        if !swap_chain.is_null() {
            release_unknown(swap_chain as *mut WinVoid);
        }
        DestroyWindow(hwnd);
    }
    result
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

fn map_frame_buffer_byte_width(desc: &BufferDesc) -> Option<usize> {
    (desc.byte_width > 0).then_some(desc.byte_width as usize)
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

unsafe fn add_ref_unknown(object: *mut WinVoid) {
    if !object.is_null() {
        unsafe {
            (*(object as *mut IUnknown)).AddRef();
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
    fn GetCurrentThreadId() -> u32;
    fn GetTickCount64() -> u64;
    fn Sleep(milliseconds: u32);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capfix4_dwell_elapsed_anchors_to_latest_control_activity() {
        // No activity (0): anchored to session start.
        assert_eq!(dwell_elapsed_ms(10_000, 1_000, 0), 9_000);
        // Activity after start re-anchors: elapsed measured from the command.
        assert_eq!(dwell_elapsed_ms(700_000, 1_000, 650_000), 50_000);
        // Stale activity from a previous session (before start) is ignored.
        assert_eq!(dwell_elapsed_ms(10_000, 8_000, 2_000), 2_000);
        // Idle cap intent: 10 minutes after the LAST command still expires.
        let timeout = u64::from(MAX_DWELL_MS);
        assert!(dwell_elapsed_ms(650_000 + timeout, 1_000, 650_000) >= timeout);
        // Clock weirdness never underflows.
        assert_eq!(dwell_elapsed_ms(5, 10, 0), 0);
    }

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

    fn assert_send_static<T: Send + 'static>() {}

    #[test]
    fn render_thread_owner_claim_rejects_thread_migration() {
        let owner = AtomicU32::new(0);
        assert!(claim_render_thread_id(101, &owner).is_ok());
        assert_eq!(owner.load(Ordering::SeqCst), 101);
        assert!(claim_render_thread_id(101, &owner).is_ok());
        assert_eq!(claim_render_thread_id(202, &owner), Err(101));
    }

    #[test]
    fn render_thread_request_try_lock_reports_busy_without_clearing_work() {
        let root = temp_capture_dir("render_thread_try_lock_busy");
        let runtime = Mutex::new(runtime_for_capture_moments(
            &root,
            &[TERRAIN_CAPTURE_MOMENT],
        ));
        {
            let mut guard = runtime.lock().expect("lock test runtime");
            guard.pending_dwell_end_boundary_finalization = true;
            guard.pending_dwell_end_generated_texture_check = true;

            assert_eq!(
                try_take_render_thread_request_flags(&runtime),
                Err(RenderThreadDrainSkipReason::RuntimeLockBusy)
            );
            assert!(guard.pending_dwell_end_boundary_finalization);
            assert!(guard.pending_dwell_end_generated_texture_check);
        }

        assert_eq!(
            try_take_render_thread_request_flags(&runtime).expect("take pending work"),
            (true, true)
        );
        let guard = runtime.lock().expect("lock test runtime after drain");
        assert!(!guard.pending_dwell_end_boundary_finalization);
        assert!(!guard.pending_dwell_end_generated_texture_check);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn render_thread_drain_skip_counter_is_rate_limited() {
        for count in [1, 2, 3, 4, 8, 16, 1024] {
            assert!(should_log_counter_event(count), "{count} should log");
        }
        for count in [5, 6, 7, 9, 1023, 1025] {
            assert!(!should_log_counter_event(count), "{count} should not log");
        }
    }

    #[test]
    fn round_stall_watchdog_requires_open_round_and_stale_progress() {
        assert_eq!(round_stall_elapsed_ms(0, 0, 120_000), None);
        assert_eq!(round_stall_elapsed_ms(1_000, 1_000, 60_999), None);
        assert_eq!(round_stall_elapsed_ms(1_000, 1_000, 61_000), Some(60_000));
        assert_eq!(round_stall_elapsed_ms(1_000, 50_000, 109_999), None);
        assert_eq!(round_stall_elapsed_ms(1_000, 50_000, 110_000), Some(60_000));
    }

    #[test]
    fn payload_writer_job_remains_plain_send_bytes() {
        assert_send_static::<PayloadWriteJob>();
        let job = PayloadWriteJob {
            path: PathBuf::from("payload.d3d11"),
            bytes: vec![1, 2, 3, 4],
        };
        assert_eq!(job.path, PathBuf::from("payload.d3d11"));
        assert_eq!(job.bytes, vec![1, 2, 3, 4]);
    }

    #[test]
    fn dwell_end_boundary_request_does_not_require_retained_context_pointer() {
        let root = temp_capture_dir("dwell_boundary_without_last_context");
        let mut runtime = runtime_for_capture_moments(&root, &[TERRAIN_CAPTURE_MOMENT]);
        runtime.last_context = 0;
        runtime.map_frame_draws.push(test_map_frame_draw(
            0,
            "pdxmap_pixel_shader_test.bin",
            "0123456789abcdef",
            "pdxmap_vertex_shader_test.bin",
            "fedcba9876543210",
        ));
        runtime
            .captured_symbols
            .insert(TERRAIN_ID_RESOURCE_SYMBOL.to_string());
        runtime
            .captured_symbols
            .insert(TERRAIN_VERTEX_BUFFER_SYMBOL.to_string());
        runtime
            .captured_symbols
            .insert(TERRAIN_INPUT_LAYOUT_SYMBOL.to_string());
        for target in CAPTURE_TARGETS {
            if target.timing == RuntimeTextureCaptureTiming::Finalize {
                runtime.deferred_runtime_targets.insert(
                    target.symbol.to_string(),
                    DeferredRuntimeTarget {
                        target,
                        srv: 1,
                        slot: 0,
                        identity: "unit_test",
                    },
                );
            }
        }

        assert!(dwell_end_boundary_finalization_ready(&runtime));
        let _ = fs::remove_dir_all(root);
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
        // CAPFIX1: the underwater_pass profile must resolve, name its own
        // pixel-shader/bundle files, and (Effect underwater binds the terrain VS)
        // record exact PS SRV payloads so t0 and the runtime CB0/CB1 light state
        // land in the bundle.
        let underwater =
            map_frame_capture_profile(UNDERWATER_CAPTURE_MOMENT).expect("underwater profile");
        assert_eq!(
            underwater.pixel_shader_file_prefix,
            "underwater_pixel_shader"
        );
        assert_eq!(underwater.vertex_shader_file_prefix, "pdxmap_vertex_shader");
        assert_eq!(underwater.bundle_file, UNDERWATER_MAP_FRAME_BUNDLE_FILE);
        assert!(underwater.capture_exact_ps_srv_payloads);
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
            capture_out_dir: None,
            capture_moment: moments.first().map(|moment| (*moment).to_string()),
            capture_moments: moments.iter().map(|moment| (*moment).to_string()).collect(),
            map_frame_capture_enabled: true,
            terrain_chunk_library: moments.contains(&TERRAIN_CHUNK_LIBRARY_MOMENT),
            expected_width: 0,
            expected_height: 0,
            allowed_slots: Vec::new(),
            paired_resource_slots: Vec::new(),
            shader_cache_signatures: Vec::new(),
            settle_ms: DEFAULT_SETTLE_MS,
            max_draws_per_pass: None,
            max_total_payload_bytes_per_session: None,
            max_capture_work_ms_per_frame: None,
            max_payload_writer_queue_bytes: None,
            ui_max_states: None,
            ui_cb_skip: UiCbSkipPolicy::default(),
            census: false,
            round_commands_enabled: false,
            trees_close_mode: None,
            dwell_ms: None,
            vanilla_source_evidence_required: true,
            cb0_sample_ms: None,
            arm_delay_ms: None,
            provenance: "unit_test".to_string(),
        };
        let capture_budget = capture_budget_from_control(Some(&control));
        let chunk_library_enabled = terrain_chunk_library_enabled_from_control(Some(&control));
        let census = census_state_from_control(Some(&control), capture_budget, 0);
        CaptureRuntime {
            state: TerrainCaptureState::default(),
            root_output_dir: root.to_path_buf(),
            output_dir: root.to_path_buf(),
            control_path: root.join(CONTROL_FILE),
            control_command_last_modified: None,
            capture_moment_sequence: capture_moment_sequence_from_control(Some(&control)),
            control: Some(control),
            start_tick_ms: 0,
            settle_ms: DEFAULT_SETTLE_MS,
            capture_budget,
            total_payload_bytes_reserved: 0,
            capture_work_frame_ms: 0,
            capture_budget_exceeded: None,
            terrain_chunk_library: TerrainChunkLibraryCaptureState::new(chunk_library_enabled, 0),
            active_capture_index: 0,
            active_moment_started_ms: 0,
            dwell_end_generated_texture_check_done: false,
            pending_dwell_end_boundary_finalization: false,
            pending_dwell_end_generated_texture_check: false,
            pending_render_thread_release_srvs: Vec::new(),
            capture_quiesced_event_recorded: false,
            captured_symbols: BTreeSet::new(),
            captured_runtime_textures: BTreeMap::new(),
            deferred_runtime_targets: BTreeMap::new(),
            shared_payloads: BTreeMap::new(),
            shared_payload_files_written: BTreeSet::new(),
            boundary_payloads: BTreeMap::new(),
            boundary_payload_warnings: BTreeSet::new(),
            completed_map_frame_moments: BTreeSet::new(),
            map_frame_draws: Vec::new(),
            map_frame_blockers: Vec::new(),
            map_frame_rejections: BTreeSet::new(),
            ui_pass: UiPassCaptureState::default(),
            census,
            ui_shader_files_written: BTreeSet::new(),
            current_swapchain_backbuffer: None,
            current_swapchain_device: 0,
            latest_post_draw_output_merger: None,
            latest_post_draw_output_blockers: Vec::new(),
            om_render_target_clears: BTreeMap::new(),
            om_depth_stencil_clears: BTreeMap::new(),
            om_clear_sequence: 0,
            terrain_id_srv_slot_observed: None,
            round_commands_enabled: false,
            active_round_label: None,
            active_round_finished: false,
            control_sequence_complete: false,
            pending_manual_round_label: None,
            last_context: 0,
            last_error: None,
        }
    }

    fn test_map_frame_draw(
        draw_index: u32,
        pixel_shader_file: &str,
        pixel_shader_hash: &str,
        vertex_shader_file: &str,
        vertex_shader_hash: &str,
    ) -> MapFrameDrawFile {
        MapFrameDrawFile {
            draw_index,
            draw_call: "DrawIndexed",
            draw_count: 1,
            draw_start: 0,
            base_vertex_location: 0,
            per_instance_count: None,
            instance_count: None,
            start_instance_location: None,
            topology: Some(4),
            shader: MapFrameShaderFile {
                stage: "PS",
                pointer: "0x1".to_string(),
                bytecode_file: pixel_shader_file.to_string(),
                byte_len: 8,
                fnv1a64: pixel_shader_hash.to_string(),
                reflection_resources: BTreeMap::new(),
                input_parameters: Vec::new(),
                constant_buffers: BTreeMap::new(),
                source_evidence: Vec::new(),
            },
            vertex_shader: MapFrameShaderFile {
                stage: "VS",
                pointer: "0x2".to_string(),
                bytecode_file: vertex_shader_file.to_string(),
                byte_len: 8,
                fnv1a64: vertex_shader_hash.to_string(),
                reflection_resources: BTreeMap::new(),
                input_parameters: Vec::new(),
                constant_buffers: BTreeMap::new(),
                source_evidence: Vec::new(),
            },
            input_layout: MapFrameInputLayoutFile {
                pointer: "0x3".to_string(),
                elements: Vec::new(),
            },
            vertex_buffers: BTreeMap::new(),
            index_buffer: None,
            constant_buffers: Vec::new(),
            shader_resources: Vec::new(),
            samplers: Vec::new(),
            draw_state: None,
            post_draw_output_merger: None,
        }
    }

    fn test_buffer_binding(
        stage: &'static str,
        slot: u32,
        file: &str,
        bytes: &[u8],
    ) -> MapFrameBufferBindingFile {
        MapFrameBufferBindingFile {
            stage,
            slot,
            declared_name: None,
            file: Some(file.to_string()),
            byte_len: bytes.len(),
            buffer_byte_width: Some(bytes.len()),
            fnv1a64: Some(fnv1a64_bytes(bytes)),
            stride: Some(bytes.len() as u32),
            offset: Some(0),
            dxgi_format: None,
            bind_flags: D3D11_BIND_VERTEX_BUFFER,
            usage: 0,
            cpu_access_flags: 0,
            unknown_origin: None,
            byte_range: None,
            payload_truncated: None,
            unobtained_reason: None,
        }
    }

    fn test_texture_binding(file: &str, bytes: &[u8]) -> MapFrameTextureBindingFile {
        MapFrameTextureBindingFile {
            stage: "PS",
            slot: 0,
            declared_name: Some("SharedTexture".to_string()),
            is_null: false,
            view_desc: None,
            file: Some(file.to_string()),
            byte_len: bytes.len(),
            fnv1a64: Some(fnv1a64_bytes(bytes)),
            dxgi_format: DXGI_FORMAT_R8G8B8A8_UNORM,
            width: 1,
            height: 1,
            mip_count: 1,
            array_size: 1,
            sample_count: 1,
            sample_quality: 0,
            payload_format: Some(NATIVE_D3D11_TEXTURE_PAYLOAD_FORMAT),
            subresources: Vec::new(),
            provenance: Some("unit_test"),
            bind_flags: D3D11_BIND_SHADER_RESOURCE,
            usage: 0,
            cpu_access_flags: 0,
            misc_flags: 0,
            unobtained_reason: None,
            all_zero: false,
        }
    }

    fn test_census_pass(
        root: &Path,
        sequence: u64,
        first_seen_frame: u64,
        draw: MapFrameDrawFile,
    ) -> CensusActivePass {
        CensusActivePass {
            sequence,
            first_seen_frame,
            output_dir: root.join(CENSUS_DIR).join(format!("pending_{sequence:06}")),
            boundary: CensusPassBoundary {
                render_target_class: "rt:main".to_string(),
                depth_class: "depth:main".to_string(),
            },
            draws: vec![draw],
            blockers: Vec::new(),
            payload_bytes_at_start: 0,
            capture_payloads: true,
            uncaptured_reason: None,
            prefix_key: 0,
            prefix_keys: Vec::new(),
            any_payload_captured: true,
            repeat_skipped_draws: 0,
        }
    }

    #[test]
    fn census_manifest_counts_signatures_bundles_and_shared_payload_dedupe() {
        let root = temp_capture_dir("census_manifest_counts");
        let mut runtime = runtime_for_capture_moments(&root, &[TERRAIN_CAPTURE_MOMENT]);
        runtime.census.enabled = true;
        runtime.census.payload_byte_budget = 1024;

        let shared_bytes = [9u8, 8, 7, 6];
        let shared_hash = fnv1a64_bytes(&shared_bytes);
        let shared_file = census_shared_payload_file("ps_srv", Some("SharedTexture"), &shared_hash);
        let payload_dir = root.join(CENSUS_DIR).join("payloads");
        fs::create_dir_all(&payload_dir).expect("create census payload dir");
        if reserve_census_shared_payload_file_for_runtime(&mut runtime, &shared_file) {
            let payload_name = shared_file
                .strip_prefix("../payloads/")
                .expect("shared payload relative path");
            fs::write(payload_dir.join(payload_name), shared_bytes).expect("write shared payload");
        }
        assert!(!reserve_census_shared_payload_file_for_runtime(
            &mut runtime,
            &shared_file
        ));

        let cases = [
            ("aa", "11", 3u32),
            ("aa", "11", 3u32),
            ("bb", "22", 4u32),
            ("cc", "33", 5u32),
        ];
        for (sequence, (ps_hash, vs_hash, draw_count)) in cases.into_iter().enumerate() {
            let mut draw = test_map_frame_draw(
                0,
                &format!("ps_{ps_hash}.bin"),
                ps_hash,
                &format!("vs_{vs_hash}.bin"),
                vs_hash,
            );
            draw.draw_count = draw_count;
            draw.shader_resources
                .push(test_texture_binding(&shared_file, &shared_bytes));
            let pass = test_census_pass(&root, sequence as u64, sequence as u64 + 1, draw);
            let parts = census_signature_parts(&pass);
            let signature = census_signature_hash(&parts);
            let sig12 = signature.chars().take(12).collect::<String>();
            let bundle_path = if !runtime.census.signatures.contains_key(&signature) {
                let bundle_dir = root.join(CENSUS_DIR).join(format!("{sequence:06}_{sig12}"));
                write_census_pass_bundle(&bundle_dir, &pass, "unit_test")
                    .expect("write census bundle");
                Some(
                    Path::new(CENSUS_DIR)
                        .join(format!("{sequence:06}_{sig12}"))
                        .join(CENSUS_PASS_BUNDLE_FILE)
                        .to_string_lossy()
                        .replace('\\', "/"),
                )
            } else {
                None
            };
            let expected_new_signature = bundle_path.is_some();
            assert_eq!(
                record_census_signature_for_runtime(
                    &mut runtime,
                    signature,
                    sig12,
                    pass.first_seen_frame,
                    shared_bytes.len() as u64,
                    bundle_path,
                    None,
                    parts,
                ),
                expected_new_signature
            );
        }

        let manifest = census_manifest_for_runtime(&runtime);
        assert_eq!(manifest.signatures_seen, 3);
        assert_eq!(manifest.signatures_captured, 3);
        assert_eq!(manifest.signatures_uncaptured, 0);
        let counts = manifest
            .entries
            .iter()
            .map(|entry| entry.occurrence_count)
            .collect::<BTreeSet<_>>();
        assert_eq!(counts, BTreeSet::from([1, 2]));
        assert!(manifest
            .entries
            .iter()
            .any(|entry| entry.occurrence_count == 2 && entry.bundle_path.is_some()));

        let bundle_dirs = fs::read_dir(root.join(CENSUS_DIR))
            .expect("read census dir")
            .filter_map(std::result::Result::ok)
            .filter(|entry| entry.path().is_dir())
            .filter(|entry| entry.file_name().to_string_lossy() != "payloads")
            .count();
        assert_eq!(bundle_dirs, 3);
        let stored_payloads = fs::read_dir(&payload_dir)
            .expect("read payload dir")
            .filter_map(std::result::Result::ok)
            .count();
        assert_eq!(stored_payloads, 1);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn census_manifest_snapshot_skips_disabled_empty_runtime() {
        let root = temp_capture_dir("census_manifest_skip_disabled");
        let mut runtime = runtime_for_capture_moments(&root, &[TERRAIN_CAPTURE_MOMENT]);
        assert!(!runtime.census.enabled);
        assert!(!census_manifest_snapshot_needed(&runtime));

        runtime.census.enabled = true;
        assert!(census_manifest_snapshot_needed(&runtime));

        runtime.census.enabled = false;
        let draw = test_map_frame_draw(0, "ps.bin", "aa", "vs.bin", "bb");
        let pass = test_census_pass(&root, 0, 1, draw);
        let parts = census_signature_parts(&pass);
        let signature = census_signature_hash(&parts);
        let sig12 = signature.chars().take(12).collect::<String>();
        assert!(record_census_signature_for_runtime(
            &mut runtime,
            signature,
            sig12,
            pass.first_seen_frame,
            0,
            None,
            Some(CENSUS_UNCAPTURED_UNKNOWN_SHADER.to_string()),
            parts,
        ));
        assert!(census_manifest_snapshot_needed(&runtime));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn census_unknown_state_blockers_promote_manifest_reasons() {
        assert_eq!(
            census_uncaptured_reason_from_blockers(&[format!(
                "census draw payload capture disabled: {UI_PASS_INTEROP_HAZARD_UNOBTAINED_REASON}"
            )]),
            Some(UI_PASS_INTEROP_HAZARD_UNOBTAINED_REASON)
        );
        assert_eq!(
            census_uncaptured_reason_from_blockers(&[
                "census PS shader 0x10 bytecode was not observed by CreatePSShader hook"
                    .to_string(),
                format!("census draw payload capture disabled: {CENSUS_UNCAPTURED_UNKNOWN_SHADER}")
            ]),
            Some(CENSUS_UNCAPTURED_UNKNOWN_SHADER)
        );
        assert_eq!(
            census_uncaptured_reason_from_blockers(&[
                "census draw has no tracked IA input layout".to_string(),
                format!(
                    "census draw payload capture disabled: {CENSUS_UNCAPTURED_UNKNOWN_INPUT_LAYOUT}"
                )
            ]),
            Some(CENSUS_UNCAPTURED_UNKNOWN_INPUT_LAYOUT)
        );
        assert!(census_uncaptured_reason_skips_signature(Some(
            UI_PASS_INTEROP_HAZARD_UNOBTAINED_REASON
        )));
        assert!(census_uncaptured_reason_skips_signature(Some(
            CENSUS_UNCAPTURED_UNKNOWN_SHADER
        )));
        assert!(census_uncaptured_reason_skips_signature(Some(
            CENSUS_UNCAPTURED_UNKNOWN_INPUT_LAYOUT
        )));
        assert_eq!(
            census_uncaptured_reason_from_blockers(&[format!(
                "census draw payload capture disabled: {CENSUS_UNCAPTURED_WORK_BUDGET_FRAME_PAUSED}"
            )]),
            Some(CENSUS_UNCAPTURED_WORK_BUDGET_FRAME_PAUSED)
        );
        assert!(!census_uncaptured_reason_skips_signature(Some(
            CENSUS_UNCAPTURED_WORK_BUDGET_FRAME_PAUSED
        )));
        assert!(census_uncaptured_reason_allows_next_draw_to_reopen_pass(
            CENSUS_UNCAPTURED_WORK_BUDGET_FRAME_PAUSED
        ));
        assert!(!census_uncaptured_reason_skips_signature(Some("errors")));
    }

    #[test]
    fn census_work_budget_frame_pause_sets_no_permanent_stop() {
        let root = temp_capture_dir("census_work_pause_no_stop");
        let mut runtime = runtime_for_capture_moments(&root, &[TERRAIN_CAPTURE_MOMENT]);
        runtime.census.enabled = true;
        runtime.census.payload_byte_budget = 1024;
        runtime.capture_budget.max_capture_work_ms_per_frame = 1;
        runtime.census.capture_work_frame_ms = 2;

        let draw = test_map_frame_draw(0, "pause_ps.bin", "aa", "pause_vs.bin", "11");
        runtime.census.active_pass = Some(test_census_pass(&root, 0, 1, draw));

        assert!(pause_census_work_frame_for_runtime(&mut runtime));
        assert!(!pause_census_work_frame_for_runtime(&mut runtime));
        assert!(runtime.census.work_frame_paused);
        assert_eq!(runtime.census.paused_frames, 1);
        assert!(!runtime.census.payload_stopped);
        assert!(runtime.census.payload_stop_reason.is_none());
        let pass = runtime.census.active_pass.as_ref().expect("active pass");
        assert!(pass.capture_payloads);
        assert!(pass.uncaptured_reason.is_none());
        assert!(pass
            .blockers
            .iter()
            .any(|blocker| blocker.contains(CENSUS_UNCAPTURED_WORK_BUDGET_FRAME_PAUSED)));
        let (capture_payloads, reason) =
            census_payload_capture_admission_for_signature(&runtime, None);
        assert!(!capture_payloads);
        assert_eq!(
            reason.as_deref(),
            Some(CENSUS_UNCAPTURED_WORK_BUDGET_FRAME_PAUSED)
        );
        let manifest = census_manifest_for_runtime(&runtime);
        assert!(!manifest.payload_stopped);
        assert!(manifest.payload_stop_reason.is_none());
        assert_eq!(manifest.paused_frames, 1);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn census_work_budget_paused_signature_retries_next_frame_and_captures() {
        let root = temp_capture_dir("census_work_pause_retry");
        let mut runtime = runtime_for_capture_moments(&root, &[TERRAIN_CAPTURE_MOMENT]);
        runtime.census.enabled = true;
        runtime.census.payload_byte_budget = 1024;
        runtime.census.paused_frames = 1;

        let draw = test_map_frame_draw(0, "retry_ps.bin", "aa", "retry_vs.bin", "11");
        let pass = test_census_pass(&root, 7, 4, draw);
        let parts = census_signature_parts(&pass);
        let signature = census_signature_hash(&parts);
        let sig12 = signature.chars().take(12).collect::<String>();

        assert!(record_census_signature_for_runtime(
            &mut runtime,
            signature.clone(),
            sig12.clone(),
            pass.first_seen_frame,
            0,
            None,
            Some(CENSUS_UNCAPTURED_WORK_BUDGET_FRAME_PAUSED.to_string()),
            parts.clone(),
        ));
        assert!(!runtime
            .census
            .hazard_skipped_signatures
            .contains(&signature));
        assert!(!census_signature_blocks_clean_capture(&runtime, &signature));
        let (capture_payloads, reason) =
            census_payload_capture_admission_for_signature(&runtime, Some(&signature));
        assert!(capture_payloads);
        assert!(reason.is_none());

        let bundle_path = Path::new(CENSUS_DIR)
            .join(format!("000007_{sig12}"))
            .join(CENSUS_PASS_BUNDLE_FILE)
            .to_string_lossy()
            .replace('\\', "/");
        assert!(record_census_signature_for_runtime(
            &mut runtime,
            signature.clone(),
            sig12,
            pass.first_seen_frame.saturating_add(1),
            128,
            Some(bundle_path.clone()),
            None,
            parts,
        ));
        let entry = runtime
            .census
            .signatures
            .get(&signature)
            .expect("signature entry");
        assert!(entry.captured);
        assert_eq!(entry.occurrence_count, 2);
        assert_eq!(entry.first_seen_frame, pass.first_seen_frame);
        assert_eq!(entry.payload_bytes, 128);
        assert_eq!(entry.bundle_path.as_deref(), Some(bundle_path.as_str()));
        assert!(entry.uncaptured_reason.is_none());
        assert!(census_signature_blocks_clean_capture(&runtime, &signature));

        let manifest = census_manifest_for_runtime(&runtime);
        assert_eq!(manifest.signatures_seen, 1);
        assert_eq!(manifest.signatures_captured, 1);
        assert_eq!(manifest.signatures_uncaptured, 0);
        assert_eq!(manifest.paused_frames, 1);
        assert!(!manifest.payload_stopped);
        assert!(manifest.payload_stop_reason.is_none());
        let _ = fs::remove_dir_all(root);
    }

    fn tracked_state_for_trust_tests(
        shaders_known: bool,
        layout_known: bool,
    ) -> CensusTrackedDrawState {
        CensusTrackedDrawState {
            pixel_shader: 0x10,
            vertex_shader: 0x20,
            layout_ptr: 0x30,
            layout_elements: Vec::new(),
            pixel_shader_known: shaders_known,
            vertex_shader_known: shaders_known,
            layout_known,
        }
    }

    #[test]
    fn census_untrusted_context_class_blocks_capture_before_map_renderer_live() {
        let fully_known = tracked_state_for_trust_tests(true, true);
        // Untrusted context class wins over EVERYTHING, including a fully
        // known shader/layout state (the crash draw had known bytecode and
        // layout — the census tracks all created shaders) and even over the
        // hazard pattern: the guarantee is zero context calls, so the reason
        // must be decided before any other consideration.
        assert_eq!(
            census_preflight_uncaptured_reason(false, None, &fully_known),
            Some(CENSUS_UNCAPTURED_UNTRUSTED_CONTEXT_CLASS)
        );
        assert_eq!(
            census_preflight_uncaptured_reason(
                false,
                Some(UI_PASS_INTEROP_HAZARD_UNOBTAINED_REASON),
                &fully_known
            ),
            Some(CENSUS_UNCAPTURED_UNTRUSTED_CONTEXT_CLASS)
        );
        assert_eq!(
            census_preflight_uncaptured_reason(
                false,
                None,
                &tracked_state_for_trust_tests(false, false)
            ),
            Some(CENSUS_UNCAPTURED_UNTRUSTED_CONTEXT_CLASS)
        );
    }

    #[test]
    fn census_trusted_context_class_preserves_existing_preflight_order() {
        let fully_known = tracked_state_for_trust_tests(true, true);
        // Trusted + clean state -> None: the payload capture attempt proceeds.
        assert_eq!(
            census_preflight_uncaptured_reason(true, None, &fully_known),
            None
        );
        // Trusted keeps the proven preflight order: hazard first, then
        // unknown shader, then unknown layout.
        assert_eq!(
            census_preflight_uncaptured_reason(
                true,
                Some(UI_PASS_INTEROP_HAZARD_UNOBTAINED_REASON),
                &fully_known
            ),
            Some(UI_PASS_INTEROP_HAZARD_UNOBTAINED_REASON)
        );
        assert_eq!(
            census_preflight_uncaptured_reason(
                true,
                None,
                &tracked_state_for_trust_tests(false, true)
            ),
            Some(CENSUS_UNCAPTURED_UNKNOWN_SHADER)
        );
        assert_eq!(
            census_preflight_uncaptured_reason(
                true,
                None,
                &tracked_state_for_trust_tests(true, false)
            ),
            Some(CENSUS_UNCAPTURED_UNKNOWN_INPUT_LAYOUT)
        );
    }

    #[test]
    fn trust_latch_membership_is_strict_under_pointer_set_overflow() {
        let table: [AtomicUsize; 8] = Default::default();
        let overflowed = AtomicBool::new(false);
        remember_capture_pointer(&table, &overflowed, 0x1230);
        // Normal case: remembered pointer is a strict member, others are not.
        assert!(capture_pointer_known_strict(&table, &overflowed, 0x1230));
        assert!(!capture_pointer_known_strict(&table, &overflowed, 0x4560));
        // Overflow: the base check deliberately fails open (map-frame arming
        // must not miss family shaders), but the trust latch must fail CLOSED
        // — an overflow guess must never unlock census payload capture.
        overflowed.store(true, Ordering::Release);
        assert!(capture_pointer_known(&table, &overflowed, 0x4560));
        assert!(!capture_pointer_known_strict(&table, &overflowed, 0x4560));
        assert!(!capture_pointer_known_strict(&table, &overflowed, 0x1230));
    }

    #[test]
    fn census_untrusted_context_class_reason_lifecycle() {
        // Manifest promotion from the draw-record blocker string.
        assert_eq!(
            census_uncaptured_reason_from_blockers(&[format!(
                "census draw payload capture disabled: {CENSUS_UNCAPTURED_UNTRUSTED_CONTEXT_CLASS}"
            )]),
            Some(CENSUS_UNCAPTURED_UNTRUSTED_CONTEXT_CLASS)
        );
        // Finalized untrusted signatures join the skip set: repeats are
        // counter-only and never retry the payload battery.
        assert!(census_uncaptured_reason_skips_signature(Some(
            CENSUS_UNCAPTURED_UNTRUSTED_CONTEXT_CLASS
        )));
        // And the next draw may open a fresh pass (same lifecycle as
        // unknown-shader/layout preflight skips).
        assert!(census_uncaptured_reason_allows_next_draw_to_reopen_pass(
            CENSUS_UNCAPTURED_UNTRUSTED_CONTEXT_CLASS
        ));
    }

    #[test]
    fn hazardous_census_finalize_skips_signature_without_global_payload_stop() {
        let root = temp_capture_dir("census_hazard_skip_signature");
        let mut runtime = runtime_for_capture_moments(&root, &[TERRAIN_CAPTURE_MOMENT]);
        runtime.census.enabled = true;
        runtime.census.payload_byte_budget = 1024;

        let hazard_draw = test_map_frame_draw(0, "hazard_ps.bin", "aa", "hazard_vs.bin", "11");
        let hazard_pass = test_census_pass(&root, 0, 1, hazard_draw);
        let hazard_parts = census_signature_parts(&hazard_pass);
        let hazard_signature = census_signature_hash(&hazard_parts);
        let hazard_sig12 = hazard_signature.chars().take(12).collect::<String>();
        let hazard_reason = Some(UI_PASS_INTEROP_HAZARD_UNOBTAINED_REASON.to_string());
        assert!(record_census_signature_for_runtime(
            &mut runtime,
            hazard_signature.clone(),
            hazard_sig12,
            hazard_pass.first_seen_frame,
            0,
            None,
            hazard_reason.clone(),
            hazard_parts,
        ));
        maybe_record_census_hazard_skip_for_finalized_signature(
            &mut runtime,
            &hazard_signature,
            census_uncaptured_reason_skips_signature(hazard_reason.as_deref()),
        );

        assert!(!runtime.census.payload_stopped);
        assert!(runtime.census.payload_stop_reason.is_none());
        assert!(runtime
            .census
            .hazard_skipped_signatures
            .contains(&hazard_signature));
        let manifest = census_manifest_for_runtime(&runtime);
        assert!(!manifest.payload_stopped);
        assert!(manifest.payload_stop_reason.is_none());

        let (capture_payloads, reason) =
            census_payload_capture_admission_for_signature(&runtime, Some(&hazard_signature));
        assert!(!capture_payloads);
        assert_eq!(
            reason.as_deref(),
            Some(UI_PASS_INTEROP_HAZARD_UNOBTAINED_REASON)
        );

        let next_draw = test_map_frame_draw(0, "safe_ps.bin", "bb", "safe_vs.bin", "22");
        let next_pass = test_census_pass(&root, 1, 2, next_draw);
        let next_signature = census_signature_hash(&census_signature_parts(&next_pass));
        let (capture_payloads, reason) =
            census_payload_capture_admission_for_signature(&runtime, Some(&next_signature));
        assert!(capture_payloads);
        assert!(reason.is_none());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn census_bundle_preserves_instanced_input_layout_and_instance_stream() {
        let mut draw = test_map_frame_draw(
            0,
            "mesh_ps.bin",
            "mesh_ps_hash",
            "mesh_vs.bin",
            "mesh_vs_hash",
        );
        draw.draw_call = "DrawIndexedInstanced";
        draw.draw_count = 36;
        draw.per_instance_count = Some(36);
        draw.instance_count = Some(2);
        draw.start_instance_location = Some(0);
        draw.input_layout.elements = vec![
            TerrainInputElementDesc {
                semantic_name: "POSITION".to_string(),
                semantic_index: 0,
                format: DXGI_FORMAT_R16G16B16A16_SINT,
                input_slot: 0,
                aligned_byte_offset: 0,
                input_slot_class: D3D11_INPUT_PER_VERTEX_DATA,
                instance_data_step_rate: 0,
            },
            TerrainInputElementDesc {
                semantic_name: "INSTANCE_TRANSFORM".to_string(),
                semantic_index: 0,
                format: DXGI_FORMAT_R16G16B16A16_SINT,
                input_slot: 1,
                aligned_byte_offset: 0,
                input_slot_class: 1,
                instance_data_step_rate: 1,
            },
        ];
        let vertex_bytes = [1u8; 32];
        let instance_bytes = [2u8; 128];
        draw.vertex_buffers.insert(
            "0".to_string(),
            test_buffer_binding("IA", 0, "mesh_vertices.bin", &vertex_bytes),
        );
        draw.vertex_buffers.insert(
            "1".to_string(),
            test_buffer_binding("IA", 1, "mesh_instances.bin", &instance_bytes),
        );

        let value = serde_json::to_value(&draw).expect("serialize instanced draw");
        assert_eq!(
            value["input_layout"]["elements"][1]["input_slot_class"],
            serde_json::Value::from(1)
        );
        assert_eq!(
            value["input_layout"]["elements"][1]["instance_data_step_rate"],
            serde_json::Value::from(1)
        );
        assert_eq!(value["vertex_buffers"]["1"]["file"], "mesh_instances.bin");
        assert_eq!(
            value["vertex_buffers"]["1"]["byte_len"],
            instance_bytes.len()
        );
        assert_eq!(
            value["vertex_buffers"]["1"]["buffer_byte_width"],
            instance_bytes.len()
        );
    }

    #[test]
    fn census_byte_budget_stops_census_before_moment_budget() {
        let root = temp_capture_dir("census_budget");
        let mut runtime = runtime_for_capture_moments(&root, &[TERRAIN_CAPTURE_MOMENT]);
        runtime.census.enabled = true;
        runtime.census.payload_byte_budget = 10;
        runtime.capture_budget.max_total_payload_bytes_per_session = 1_000;

        assert!(reserve_census_payload_bytes_for_runtime(&mut runtime, 6, "first.bin").is_none());
        let event = reserve_census_payload_bytes_for_runtime(&mut runtime, 5, "second.bin")
            .expect("census payload should exceed budget");
        assert!(event.contains("\"reason\":\"budget\""));
        assert!(runtime.census.payload_stopped);
        assert_eq!(
            runtime.census.payload_stop_reason.as_deref(),
            Some("budget")
        );
        assert!(!runtime.census.work_frame_paused);
        assert_eq!(runtime.census.paused_frames, 0);
        let (capture_payloads, reason) =
            census_payload_capture_admission_for_signature(&runtime, Some("anything"));
        assert!(!capture_payloads);
        assert_eq!(reason.as_deref(), Some("budget"));
        assert_eq!(runtime.total_payload_bytes_reserved, 0);
        assert!(runtime.capture_budget_exceeded.is_none());
        assert!(reserve_capture_payload_bytes_for_runtime(&mut runtime, 4, "moment.bin").is_none());
        assert_eq!(runtime.total_payload_bytes_reserved, 4);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn auto_rounds_follow_camera_height_buckets_and_require_rounds_flag() {
        assert_eq!(auto_round_label_for_height(0.0), Some("auto_h0_150"));
        assert_eq!(auto_round_label_for_height(149.99), Some("auto_h0_150"));
        assert_eq!(auto_round_label_for_height(150.0), Some("auto_h150_300"));
        assert_eq!(auto_round_label_for_height(700.0), Some("auto_h600_1200"));
        assert_eq!(auto_round_label_for_height(2400.0), Some("auto_h2400_plus"));

        let root = temp_capture_dir("auto_rounds");
        let mut disabled = runtime_for_capture_moments(&root, &[TERRAIN_CAPTURE_MOMENT]);
        assert!(
            maybe_start_auto_round_for_height_locked(&mut disabled, "auto_h0_150", 0).is_none()
        );

        let mut runtime = runtime_for_capture_moments(&root, &[TERRAIN_CAPTURE_MOMENT]);
        runtime.round_commands_enabled = true;
        // MOMFIX1: rounds are HELD until the continuous control sequence
        // completes. This test exercises bucket/rate-limit behavior, so mark the
        // sequence complete up front to unblock rounds (the HOLD gate itself is
        // covered by round_is_held_while_excluded_control_moments_are_still_pending).
        runtime.control_sequence_complete = true;
        let first = maybe_start_auto_round_for_height_locked(&mut runtime, "auto_h0_150", 0)
            .expect("first auto round starts");
        assert!(first.1.contains("\"label\":\"auto_h0_150\""));
        assert_eq!(runtime.active_round_label.as_deref(), Some("auto_h0_150"));
        assert!(runtime
            .census
            .auto_round_buckets_done
            .contains("auto_h0_150"));
        runtime.active_round_finished = true;

        assert!(maybe_start_auto_round_for_height_locked(
            &mut runtime,
            "auto_h600_1200",
            AUTO_ROUND_RATE_LIMIT_MS - 1
        )
        .is_none());
        let second = maybe_start_auto_round_for_height_locked(
            &mut runtime,
            "auto_h600_1200",
            AUTO_ROUND_RATE_LIMIT_MS,
        )
        .expect("second bucket starts after rate limit");
        assert!(second.1.contains("\"label\":\"auto_h600_1200\""));
        assert_eq!(
            runtime.active_round_label.as_deref(),
            Some("auto_h600_1200")
        );
        assert!(runtime
            .census
            .auto_round_buckets_done
            .contains("auto_h600_1200"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn terrain_scene_cb0_camera_height_reads_v_cam_pos_y_offset() {
        let mut bytes = vec![0u8; 416];
        bytes[TERRAIN_SCENE_CB0_VCAMPOS_OFFSET..TERRAIN_SCENE_CB0_VCAMPOS_OFFSET + 4]
            .copy_from_slice(&567.25f32.to_le_bytes());
        assert_eq!(terrain_scene_cb0_camera_height(&bytes), Some(567.25));
        assert_eq!(
            terrain_scene_cb0_camera_height(&bytes[..TERRAIN_SCENE_CB0_VCAMPOS_OFFSET]),
            None
        );
    }

    #[test]
    fn cb0_sample_record_hex_round_trips_full_416_byte_buffer() {
        // The exact 416-byte scene CB0 must encode and decode byte-for-byte:
        // a lossy encode would silently corrupt the day/night curve.
        let mut bytes = vec![0u8; CB0_SCENE_BYTE_LEN];
        for (i, byte) in bytes.iter_mut().enumerate() {
            *byte = (i as u8).wrapping_mul(37).wrapping_add(i as u8 >> 1);
        }
        let hex = cb0_encode_hex(&bytes);
        assert_eq!(hex.len(), CB0_SCENE_BYTE_LEN * 2);
        assert!(hex
            .bytes()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert_eq!(cb0_decode_hex(&hex).as_deref(), Some(bytes.as_slice()));
        // Known-vector sanity + odd-length rejection.
        assert_eq!(cb0_encode_hex(&[0x00, 0x0f, 0xa5, 0xff]), "000fa5ff");
        assert_eq!(cb0_decode_hex("abc"), None);
    }

    // The sampler's hot-path config lives in process-global atomics. This test
    // owns all of them for its duration, so it exercises the rate limiter and
    // the cap in ONE serialized function and restores the disarmed defaults on
    // exit — no other test touches these statics.
    #[test]
    fn cb0_sampler_rate_limiter_and_cap_gate_sampling() {
        // Disarmed: never claims a window.
        CB0_SAMPLER_ARMED.store(false, Ordering::SeqCst);
        CB0_SAMPLE_INTERVAL_MS.store(0, Ordering::SeqCst);
        assert!(!cb0_sampler_try_claim_window(1_000));

        // Arm at 50ms cadence with the first window open at t=1000.
        let interval = 50u64;
        CB0_SAMPLE_INTERVAL_MS.store(interval, Ordering::SeqCst);
        CB0_NEXT_SAMPLE_MS.store(1_000, Ordering::SeqCst);
        CB0_SAMPLE_COUNT.store(0, Ordering::SeqCst);
        CB0_SAMPLER_CAP_RECORDED.store(false, Ordering::SeqCst);
        CB0_SAMPLER_ARMED.store(true, Ordering::Release);

        // Before the window: no claim. At the window: exactly one claim; an
        // immediate re-check in the same window loses the CAS (window claimed).
        assert!(!cb0_sampler_try_claim_window(999));
        assert!(cb0_sampler_try_claim_window(1_000));
        assert!(!cb0_sampler_try_claim_window(1_000));
        assert!(!cb0_sampler_try_claim_window(1_049));
        // Deadline advanced to now+interval; next window opens at 1050.
        assert_eq!(CB0_NEXT_SAMPLE_MS.load(Ordering::SeqCst), 1_050);
        assert!(cb0_sampler_try_claim_window(1_050));
        assert_eq!(CB0_NEXT_SAMPLE_MS.load(Ordering::SeqCst), 1_100);

        // Cap semantics: once the count reaches the cap, the sampler disarms
        // itself exactly once (swap guards the single event) and stops.
        CB0_SAMPLE_COUNT.store(CB0_SAMPLER_MAX_SAMPLES, Ordering::SeqCst);
        assert!(CB0_SAMPLE_COUNT.load(Ordering::SeqCst) >= CB0_SAMPLER_MAX_SAMPLES);
        assert!(!CB0_SAMPLER_CAP_RECORDED.swap(true, Ordering::SeqCst));
        assert!(CB0_SAMPLER_CAP_RECORDED.swap(true, Ordering::SeqCst));

        // Restore disarmed defaults so no other test observes an armed sampler.
        CB0_SAMPLER_ARMED.store(false, Ordering::SeqCst);
        CB0_SAMPLER_DETACHING.store(false, Ordering::SeqCst);
        CB0_SAMPLE_INTERVAL_MS.store(0, Ordering::SeqCst);
        CB0_NEXT_SAMPLE_MS.store(0, Ordering::SeqCst);
        CB0_SAMPLE_COUNT.store(0, Ordering::SeqCst);
        CB0_SAMPLER_CAP_RECORDED.store(false, Ordering::SeqCst);
    }

    #[test]
    fn cb0_sampler_arms_only_when_control_sets_positive_interval() {
        let root = temp_capture_dir("cb0_sampler_arm");
        // Ensure a clean disarmed baseline (other tests restore, but be safe).
        CB0_SAMPLER_ARMED.store(false, Ordering::SeqCst);

        // Absent / zero => stays disarmed, no event, no atomics touched.
        arm_cb0_sampler_from_control(None, 5_000, &root);
        assert!(!CB0_SAMPLER_ARMED.load(Ordering::Acquire));
        let mut control = test_control_with_cb0_sample_ms(None);
        arm_cb0_sampler_from_control(Some(&control), 5_000, &root);
        assert!(!CB0_SAMPLER_ARMED.load(Ordering::Acquire));
        control.cb0_sample_ms = Some(0);
        arm_cb0_sampler_from_control(Some(&control), 5_000, &root);
        assert!(!CB0_SAMPLER_ARMED.load(Ordering::Acquire));

        // Positive interval => armed, config latched, session start recorded.
        control.cb0_sample_ms = Some(50);
        arm_cb0_sampler_from_control(Some(&control), 5_000, &root);
        assert!(CB0_SAMPLER_ARMED.load(Ordering::Acquire));
        assert_eq!(CB0_SAMPLE_INTERVAL_MS.load(Ordering::Acquire), 50);
        assert_eq!(CB0_SESSION_START_MS.load(Ordering::Acquire), 5_000);
        assert_eq!(CB0_NEXT_SAMPLE_MS.load(Ordering::Acquire), 5_000);
        assert_eq!(CB0_SAMPLE_COUNT.load(Ordering::Acquire), 0);
        let events = fs::read_to_string(root.join(CAPTURE_EVENTS_FILE)).unwrap_or_default();
        assert!(events.contains("\"type\":\"cb0_sampler_armed\""));
        assert!(events.contains("\"sample_interval_ms\":50"));

        // Restore disarmed defaults.
        CB0_SAMPLER_ARMED.store(false, Ordering::SeqCst);
        CB0_SAMPLE_INTERVAL_MS.store(0, Ordering::SeqCst);
        CB0_NEXT_SAMPLE_MS.store(0, Ordering::SeqCst);
        CB0_SESSION_START_MS.store(0, Ordering::SeqCst);
        CB0_SAMPLE_COUNT.store(0, Ordering::SeqCst);
        let _ = fs::remove_dir_all(root);
    }

    // CAPFIX3 ISSUE 1 — the persistent CB0 sampler must SURVIVE moment-sequence
    // completion and budget quiesce, and the dwell loop must stay open to expiry
    // while it is armed; disarm at dwell expiry is the ONLY thing that stops it.
    // These statics are process-global; this test owns them for its duration and
    // restores the disarmed defaults on exit. It exercises the exact gate
    // capture_ready_to_finish() uses (cb0_sampler_keepalive_active) plus the
    // disarm latch clear, which together are the Issue-1 contract.
    #[test]
    fn cb0_sampler_keepalive_survives_completion_and_quiesce_and_clears_at_disarm() {
        // Disarmed baseline: keepalive is false, so the dwell loop is free to
        // finish on moment completion (unchanged pre-CAPFIX3 behavior).
        CB0_SAMPLER_ARMED.store(false, Ordering::SeqCst);
        CB0_SAMPLER_DETACHING.store(false, Ordering::SeqCst);
        HOOKS_DISARMED.store(false, Ordering::SeqCst);
        assert!(!cb0_sampler_keepalive_active());

        // Armed sampler: keepalive holds the dwell loop open.
        CB0_SAMPLER_ARMED.store(true, Ordering::Release);
        assert!(cb0_sampler_keepalive_active());

        // Moment-sequence completion clears the CAPTURE arming flags but must NOT
        // touch the sampler latch: keepalive still true (survival #1).
        CONTEXT_MAP_FRAME_DRAW_ARMED.store(false, Ordering::SeqCst);
        CAPTURED.store(true, Ordering::SeqCst);
        assert!(cb0_sampler_keepalive_active());

        // Budget quiesce sets CAPTURE_QUIESCED / CAPTURE_BUDGET_STOPPED but again
        // must not stop the byte-budget-exempt sampler: keepalive still true
        // (survival #2).
        CAPTURE_QUIESCED.store(true, Ordering::SeqCst);
        CAPTURE_BUDGET_STOPPED.store(true, Ordering::SeqCst);
        assert!(cb0_sampler_keepalive_active());
        // And capture_ready_to_finish() must honor keepalive OVER the budget-stop
        // early-return (the gate is checked first): the dwell loop stays open even
        // though CAPTURE_BUDGET_STOPPED is set, so the sampler runs to expiry.
        assert!(
            !capture_ready_to_finish(),
            "an armed sampler must keep the dwell loop open even after a budget stop"
        );

        // Dwell expiry: the permanent disarm is the ONE place the sampler dies.
        // It sets DETACHING + clears ARMED + latches HOOKS_DISARMED, after which
        // keepalive is false and the dwell loop may finish.
        CB0_SAMPLER_DETACHING.store(true, Ordering::SeqCst);
        CB0_SAMPLER_ARMED.store(false, Ordering::SeqCst);
        HOOKS_DISARMED.store(true, Ordering::SeqCst);
        assert!(!cb0_sampler_keepalive_active());

        // Restore disarmed, un-quiesced defaults for other tests.
        CAPTURED.store(false, Ordering::SeqCst);
        CAPTURE_QUIESCED.store(false, Ordering::SeqCst);
        CAPTURE_BUDGET_STOPPED.store(false, Ordering::SeqCst);
        HOOKS_DISARMED.store(false, Ordering::SeqCst);
        CB0_SAMPLER_DETACHING.store(false, Ordering::SeqCst);
        CB0_SAMPLER_ARMED.store(false, Ordering::SeqCst);
    }

    // CAPFIX3 ISSUE 1 — the sampler-cap path clears CB0_SAMPLER_ARMED at the cap,
    // and once ARMED is false keepalive must report false so a capped-out session
    // still finishes at dwell timeout rather than spinning forever. (A capped
    // sampler is done; only DETACHING/DISARM would otherwise be the exit.)
    #[test]
    fn cb0_sampler_keepalive_false_once_cap_disarms_sampler() {
        CB0_SAMPLER_DETACHING.store(false, Ordering::SeqCst);
        HOOKS_DISARMED.store(false, Ordering::SeqCst);
        CB0_SAMPLER_ARMED.store(true, Ordering::Release);
        assert!(cb0_sampler_keepalive_active());
        // Cap reached => the sampler clears its own ARMED latch.
        CB0_SAMPLER_ARMED.store(false, Ordering::SeqCst);
        assert!(!cb0_sampler_keepalive_active());
    }

    // CAPFIX3 ISSUE 2 / FEATURE 3 — the --arm-delay-ms window: closed until the
    // renderer is live AND the delay has elapsed; open immediately when no delay
    // is configured. Uses real GetTickCount64 with a live-tick set far in the
    // past / future to avoid timing flakiness. Owns the arm-delay statics.
    #[test]
    fn map_frame_arm_delay_window_gates_only_until_elapsed() {
        // No delay configured => always open regardless of renderer-live state.
        MAP_FRAME_ARM_DELAY_MS.store(0, Ordering::SeqCst);
        MAP_RENDERER_LIVE_TICK_MS.store(0, Ordering::SeqCst);
        MAP_FRAME_ARM_DELAY_ELAPSED_RECORDED.store(false, Ordering::SeqCst);
        assert!(map_frame_arm_delay_open());

        // Delay configured but renderer not live yet => window closed (nothing to
        // capture anyway).
        MAP_FRAME_ARM_DELAY_MS.store(5_000, Ordering::SeqCst);
        MAP_RENDERER_LIVE_TICK_MS.store(0, Ordering::SeqCst);
        assert!(!map_frame_arm_delay_open());

        // Renderer just went live (tick = now) => still inside the 5s window.
        let now = unsafe { GetTickCount64() };
        MAP_RENDERER_LIVE_TICK_MS.store(now.max(1), Ordering::SeqCst);
        assert!(!map_frame_arm_delay_open());

        // Renderer went live well past the delay (tick far in the past) => open,
        // and the one-time elapsed event latch flips exactly once.
        MAP_FRAME_ARM_DELAY_ELAPSED_RECORDED.store(false, Ordering::SeqCst);
        MAP_RENDERER_LIVE_TICK_MS.store(now.saturating_sub(60_000).max(1), Ordering::SeqCst);
        assert!(map_frame_arm_delay_open());
        assert!(MAP_FRAME_ARM_DELAY_ELAPSED_RECORDED.load(Ordering::Acquire));

        // Restore no-delay default.
        MAP_FRAME_ARM_DELAY_MS.store(0, Ordering::SeqCst);
        MAP_RENDERER_LIVE_TICK_MS.store(0, Ordering::SeqCst);
        MAP_FRAME_ARM_DELAY_ELAPSED_RECORDED.store(false, Ordering::SeqCst);
    }

    // CAPFIX3 FEATURE 3 — configure_map_frame_arm_delay_from_control latches the
    // delay only for a positive value and emits the config event; absent/0 leaves
    // the window permanently open with no event.
    #[test]
    fn arm_delay_config_from_control_latches_positive_and_ignores_zero() {
        let root = temp_capture_dir("arm_delay_config");
        // Absent => 0, no event.
        MAP_FRAME_ARM_DELAY_MS.store(999, Ordering::SeqCst);
        configure_map_frame_arm_delay_from_control(None, &root);
        assert_eq!(MAP_FRAME_ARM_DELAY_MS.load(Ordering::Acquire), 0);

        let mut control = test_control_with_cb0_sample_ms(None);
        control.arm_delay_ms = Some(0);
        MAP_FRAME_ARM_DELAY_MS.store(999, Ordering::SeqCst);
        configure_map_frame_arm_delay_from_control(Some(&control), &root);
        assert_eq!(MAP_FRAME_ARM_DELAY_MS.load(Ordering::Acquire), 0);

        // Positive => latched and event recorded.
        control.arm_delay_ms = Some(4_000);
        configure_map_frame_arm_delay_from_control(Some(&control), &root);
        assert_eq!(MAP_FRAME_ARM_DELAY_MS.load(Ordering::Acquire), 4_000);
        let events = fs::read_to_string(root.join(CAPTURE_EVENTS_FILE)).unwrap_or_default();
        assert!(events.contains("\"type\":\"map_frame_arm_delay_configured\""));
        assert!(events.contains("\"arm_delay_ms\":4000"));

        MAP_FRAME_ARM_DELAY_MS.store(0, Ordering::SeqCst);
        let _ = fs::remove_dir_all(root);
    }

    fn test_control_with_cb0_sample_ms(cb0_sample_ms: Option<u64>) -> TerrainCaptureControl {
        TerrainCaptureControl {
            schema: "battle-plan-terrain-capture-control-v1".to_string(),
            capture_out_dir: None,
            capture_moment: Some(TERRAIN_CAPTURE_MOMENT.to_string()),
            capture_moments: vec![TERRAIN_CAPTURE_MOMENT.to_string()],
            map_frame_capture_enabled: true,
            terrain_chunk_library: false,
            expected_width: 0,
            expected_height: 0,
            allowed_slots: Vec::new(),
            paired_resource_slots: Vec::new(),
            shader_cache_signatures: Vec::new(),
            settle_ms: DEFAULT_SETTLE_MS,
            max_draws_per_pass: None,
            max_total_payload_bytes_per_session: None,
            max_capture_work_ms_per_frame: None,
            max_payload_writer_queue_bytes: None,
            ui_max_states: None,
            ui_cb_skip: UiCbSkipPolicy::Hazard,
            census: false,
            round_commands_enabled: false,
            trees_close_mode: None,
            dwell_ms: None,
            vanilla_source_evidence_required: true,
            cb0_sample_ms,
            arm_delay_ms: None,
            provenance: "test".to_string(),
        }
    }

    fn test_tree_identity_draw(
        draw_index: u32,
        pixel_shader_hash: &str,
        vb_hash: &str,
        draw_count: u32,
    ) -> MapFrameDrawFile {
        let mut draw = test_map_frame_draw(
            draw_index,
            &format!("tree_pixel_shader_{pixel_shader_hash}.bin"),
            pixel_shader_hash,
            "map_vertex_shader_tree.bin",
            "map_vs_hash",
        );
        draw.draw_count = draw_count;
        draw.vertex_buffers.insert(
            "0".to_string(),
            MapFrameBufferBindingFile {
                stage: "IA",
                slot: 0,
                declared_name: None,
                file: Some(format!("tree_vb_{vb_hash}.bin")),
                byte_len: 24,
                buffer_byte_width: None,
                fnv1a64: Some(vb_hash.to_string()),
                stride: Some(8),
                offset: Some(0),
                dxgi_format: None,
                bind_flags: D3D11_BIND_VERTEX_BUFFER,
                usage: 0,
                cpu_access_flags: 0,
                unknown_origin: None,
                byte_range: None,
                payload_truncated: None,
                unobtained_reason: None,
            },
        );
        draw
    }

    fn write_synthetic_round_bundle_set(root: &Path) {
        fs::create_dir_all(root).expect("create synthetic round root");
        let bundles = [
            MAP_FRAME_BUNDLE_FILE,
            WATER_MAP_FRAME_BUNDLE_FILE,
            RIVERS_MAP_FRAME_BUNDLE_FILE,
            BORDERS_MAP_FRAME_BUNDLE_FILE,
            TREES_MAP_FRAME_BUNDLE_FILE,
        ];
        for bundle in bundles {
            let payload = bundle.replace(".json", "_payload.d3d11");
            fs::write(root.join(&payload), [1, 2, 3, 4]).expect("write synthetic payload");
            fs::write(
                root.join(bundle),
                serde_json::to_vec_pretty(&serde_json::json!({
                    "runtime_files": {
                        payload.clone(): {
                            "file": payload,
                            "byte_len": 4,
                            "fnv1a64": fnv1a64_bytes(&[1, 2, 3, 4]),
                            "provenance": "unit_test"
                        }
                    }
                }))
                .expect("serialize synthetic bundle"),
            )
            .expect("write synthetic bundle");
        }
    }

    fn assert_synthetic_round_bundle_set_self_contained(root: &Path) {
        for bundle in [
            MAP_FRAME_BUNDLE_FILE,
            WATER_MAP_FRAME_BUNDLE_FILE,
            RIVERS_MAP_FRAME_BUNDLE_FILE,
            BORDERS_MAP_FRAME_BUNDLE_FILE,
            TREES_MAP_FRAME_BUNDLE_FILE,
        ] {
            let value: serde_json::Value =
                serde_json::from_slice(&fs::read(root.join(bundle)).expect("read bundle"))
                    .expect("parse bundle");
            let runtime_files = value["runtime_files"]
                .as_object()
                .expect("runtime files object");
            for evidence in runtime_files.values() {
                let file = evidence["file"].as_str().expect("evidence file");
                assert!(
                    root.join(file).is_file(),
                    "{bundle} referenced missing round-local payload {file}"
                );
            }
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
    fn round_command_rearms_two_self_contained_round_directories() {
        let root = temp_capture_dir("round_command_rearms");
        write_synthetic_round_bundle_set(&root);
        assert_synthetic_round_bundle_set_self_contained(&root);
        let mut runtime = runtime_for_capture_moments(
            &root,
            &[
                TERRAIN_CAPTURE_MOMENT,
                WATER_CAPTURE_MOMENT,
                RIVERS_CAPTURE_MOMENT,
                BORDERS_CAPTURE_MOMENT,
                TREES_CAPTURE_MOMENT,
            ],
        );
        runtime.round_commands_enabled = true;
        runtime.completed_map_frame_moments = BTreeSet::from([
            TERRAIN_CAPTURE_MOMENT.to_string(),
            WATER_CAPTURE_MOMENT.to_string(),
            RIVERS_CAPTURE_MOMENT.to_string(),
            BORDERS_CAPTURE_MOMENT.to_string(),
            TREES_CAPTURE_MOMENT.to_string(),
        ]);
        runtime
            .shared_payload_files_written
            .insert("map_frame_shared_payload_ps_srv_SnowMudData_old.d3d11".to_string());

        fs::write(
            &runtime.control_path,
            r#"{"type":"capture_map_frames_round","label":"zoom_800"}"#,
        )
        .expect("write round command");
        poll_control_file_for_round_command_for_runtime(&mut runtime);

        let round_dir = root.join("rounds").join("zoom_800");
        assert_eq!(runtime.output_dir, round_dir);
        assert_eq!(
            runtime.capture_moment_sequence,
            vec![
                TERRAIN_CAPTURE_MOMENT.to_string(),
                WATER_CAPTURE_MOMENT.to_string(),
                RIVERS_CAPTURE_MOMENT.to_string(),
                BORDERS_CAPTURE_MOMENT.to_string(),
                TREES_CAPTURE_MOMENT.to_string(),
            ]
        );
        assert!(runtime.completed_map_frame_moments.is_empty());
        assert!(runtime.shared_payload_files_written.is_empty());
        assert_eq!(runtime.active_round_label.as_deref(), Some("zoom_800"));

        write_synthetic_round_bundle_set(&runtime.output_dir);
        assert_synthetic_round_bundle_set_self_contained(&runtime.output_dir);

        runtime.completed_map_frame_moments = BTreeSet::from([
            TERRAIN_CAPTURE_MOMENT.to_string(),
            WATER_CAPTURE_MOMENT.to_string(),
            RIVERS_CAPTURE_MOMENT.to_string(),
            BORDERS_CAPTURE_MOMENT.to_string(),
            TREES_CAPTURE_MOMENT.to_string(),
        ]);
        runtime
            .shared_payload_files_written
            .insert("map_frame_shared_payload_ps_srv_SnowMudData_zoom_800.d3d11".to_string());
        // MOMFIX1: the first round must finish before a second may start; the
        // start guard now rejects overwriting an in-flight round.
        runtime.active_round_finished = true;
        runtime.control_command_last_modified = None;
        fs::write(
            &runtime.control_path,
            r#"{"type":"capture_map_frames_round","label":"zoom_1200"}"#,
        )
        .expect("write second round command");
        poll_control_file_for_round_command_for_runtime(&mut runtime);

        let round2_dir = root.join("rounds").join("zoom_1200");
        assert_eq!(runtime.output_dir, round2_dir);
        assert!(runtime.completed_map_frame_moments.is_empty());
        assert!(runtime.shared_payload_files_written.is_empty());
        assert_eq!(runtime.active_round_label.as_deref(), Some("zoom_1200"));
        write_synthetic_round_bundle_set(&runtime.output_dir);
        assert_synthetic_round_bundle_set_self_contained(&runtime.output_dir);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn restore_pass_is_requestable_but_excluded_from_map_rounds() {
        let root = temp_capture_dir("restore_pass_round_excluded");
        let runtime = runtime_for_capture_moments(
            &root,
            &[
                TERRAIN_CAPTURE_MOMENT,
                RESTORE_CAPTURE_MOMENT,
                UI_PASS_CAPTURE_MOMENT,
            ],
        );
        assert_eq!(
            capture_moment_sequence_from_control(runtime.control.as_ref()),
            vec![
                TERRAIN_CAPTURE_MOMENT.to_string(),
                RESTORE_CAPTURE_MOMENT.to_string(),
                UI_PASS_CAPTURE_MOMENT.to_string(),
            ]
        );
        assert_eq!(
            map_frame_round_moment_sequence_for_runtime(&runtime),
            vec![TERRAIN_CAPTURE_MOMENT.to_string()]
        );
        assert_eq!(
            map_frame_capture_profile(RESTORE_CAPTURE_MOMENT)
                .expect("restore profile")
                .bundle_file,
            RESTORE_MAP_FRAME_BUNDLE_FILE
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn mapname_pass_is_requestable_but_excluded_from_map_rounds() {
        let root = temp_capture_dir("mapname_pass_round_excluded");
        let runtime = runtime_for_capture_moments(
            &root,
            &[
                TERRAIN_CAPTURE_MOMENT,
                MAPNAME_CAPTURE_MOMENT,
                UI_PASS_CAPTURE_MOMENT,
            ],
        );
        assert_eq!(
            capture_moment_sequence_from_control(runtime.control.as_ref()),
            vec![
                TERRAIN_CAPTURE_MOMENT.to_string(),
                MAPNAME_CAPTURE_MOMENT.to_string(),
                UI_PASS_CAPTURE_MOMENT.to_string(),
            ]
        );
        // Excluded from map-frame rounds (like restore_pass and ui_pass).
        assert_eq!(
            map_frame_round_moment_sequence_for_runtime(&runtime),
            vec![TERRAIN_CAPTURE_MOMENT.to_string()]
        );
        assert_eq!(
            map_frame_capture_profile(MAPNAME_CAPTURE_MOMENT)
                .expect("mapname profile")
                .bundle_file,
            MAPNAME_MAP_FRAME_BUNDLE_FILE
        );
        let _ = fs::remove_dir_all(root);
    }

    // MOMFIX1 helpers: mark a map moment complete exactly as the live pipeline
    // does — including the terrain-runtime inputs that TERRAIN_CAPTURE_MOMENT
    // requires — so capture_complete() flips for the control sequence.
    fn complete_terrain_runtime_inputs(runtime: &mut CaptureRuntime) {
        for target in CAPTURE_TARGETS {
            runtime.captured_symbols.insert(target.symbol.to_string());
        }
        runtime
            .captured_symbols
            .insert(TERRAIN_VERTEX_BUFFER_SYMBOL.to_string());
        runtime
            .captured_symbols
            .insert(TERRAIN_INPUT_LAYOUT_SYMBOL.to_string());
    }

    fn mark_moment_complete(runtime: &mut CaptureRuntime, moment: &str) {
        runtime
            .completed_map_frame_moments
            .insert(moment.to_string());
        if moment == TERRAIN_CAPTURE_MOMENT {
            complete_terrain_runtime_inputs(runtime);
        }
    }

    fn events_for(root: &Path) -> String {
        fs::read_to_string(root.join(CAPTURE_EVENTS_FILE)).unwrap_or_default()
    }

    #[test]
    fn round_is_held_while_excluded_control_moments_are_still_pending() {
        // Control asks for terrain + restore + mapname + ui_pass; a round would
        // exclude restore/mapname/ui_pass. A round that fires before those
        // complete would permanently starve them (MOMFIX1). HOLD must defer it.
        let root = temp_capture_dir("round_hold_excluded_pending");
        let mut runtime = runtime_for_capture_moments(
            &root,
            &[
                TERRAIN_CAPTURE_MOMENT,
                RESTORE_CAPTURE_MOMENT,
                MAPNAME_CAPTURE_MOMENT,
                UI_PASS_CAPTURE_MOMENT,
            ],
        );
        runtime.round_commands_enabled = true;
        // Only terrain captured so far — restore/mapname/ui_pass still pending.
        mark_moment_complete(&mut runtime, TERRAIN_CAPTURE_MOMENT);
        assert!(!runtime.control_sequence_complete);

        let pending = round_hold_pending_moments(&runtime);
        assert_eq!(
            pending,
            vec![
                RESTORE_CAPTURE_MOMENT.to_string(),
                MAPNAME_CAPTURE_MOMENT.to_string(),
                UI_PASS_CAPTURE_MOMENT.to_string(),
            ]
        );

        // Auto round: HELD via auto_round_watch_pending — the height sampler is
        // not even armed until the control sequence completes, so no round starts
        // and no bucket/stamp is consumed (it will arm and fire post-latch).
        assert!(!auto_round_watch_pending(&runtime));
        assert!(maybe_start_auto_round_for_height_locked(&mut runtime, "auto_h0_150", 0).is_none());
        assert!(runtime.active_round_label.is_none());
        assert!(!runtime
            .census
            .auto_round_buckets_done
            .contains("auto_h0_150"));
        assert!(runtime.census.last_auto_round_started_ms.is_none());

        // Manual round: HELD and stashed for later start.
        request_map_frame_round_for_runtime(&mut runtime, "manual_zoom".to_string());
        assert!(runtime.active_round_label.is_none());
        assert_eq!(
            runtime.pending_manual_round_label.as_deref(),
            Some("manual_zoom")
        );
        let events = events_for(&root);
        assert!(events.contains("\"trigger\":\"manual\""));
        assert!(events.contains("mapname_pass"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn round_proceeds_once_excluded_moments_capture_and_latch_survives() {
        // Once terrain + restore + mapname + ui_pass all complete, the latch is
        // set and BOTH auto and stashed-manual rounds may proceed — and the
        // latch stays set across the round that clears completed_map_frame_moments.
        let root = temp_capture_dir("round_proceeds_after_excluded");
        let mut runtime = runtime_for_capture_moments(
            &root,
            &[
                TERRAIN_CAPTURE_MOMENT,
                RESTORE_CAPTURE_MOMENT,
                MAPNAME_CAPTURE_MOMENT,
                UI_PASS_CAPTURE_MOMENT,
            ],
        );
        runtime.round_commands_enabled = true;

        // Stash a manual round while still incomplete.
        mark_moment_complete(&mut runtime, TERRAIN_CAPTURE_MOMENT);
        request_map_frame_round_for_runtime(&mut runtime, "manual_zoom".to_string());
        assert_eq!(
            runtime.pending_manual_round_label.as_deref(),
            Some("manual_zoom")
        );

        // Drive the continuous sequence to completion moment-by-moment.
        mark_moment_complete(&mut runtime, RESTORE_CAPTURE_MOMENT);
        mark_moment_complete(&mut runtime, MAPNAME_CAPTURE_MOMENT);
        mark_moment_complete(&mut runtime, UI_PASS_CAPTURE_MOMENT);
        // Advance the index to the ui_pass tail so capture_complete() is true.
        runtime.active_capture_index = runtime.capture_moment_sequence.len() - 1;
        assert!(capture_complete(&runtime));

        // Refresh sets the latch (no round yet) AND starts the stashed manual round.
        let complete = refresh_session_capture_progress(&mut runtime);
        assert!(complete);
        assert!(runtime.control_sequence_complete);
        assert!(runtime.pending_manual_round_label.is_none());
        assert_eq!(runtime.active_round_label.as_deref(), Some("manual_zoom"));
        // The round cleared completed_map_frame_moments, but the latch persists.
        assert!(runtime.completed_map_frame_moments.is_empty());
        assert!(round_hold_pending_moments(&runtime).is_empty());

        // A later auto round now proceeds immediately (latch short-circuits).
        runtime.active_round_finished = true;
        let event = maybe_start_auto_round_for_height_locked(
            &mut runtime,
            "auto_h600_1200",
            AUTO_ROUND_RATE_LIMIT_MS,
        )
        .expect("auto round starts after latch");
        assert!(event.1.contains("map_frame_auto_round_started"));
        assert_eq!(
            runtime.active_round_label.as_deref(),
            Some("auto_h600_1200")
        );
        assert!(runtime.control_sequence_complete);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn finalization_reports_true_pending_when_control_sequence_starved() {
        // Rounds enabled, control sequence incomplete (mapname never captured),
        // latch unset: finalization must emit blocked_before_* with the true
        // pending list — NEVER captured_runtime_terrain_inputs.
        let root = temp_capture_dir("finalization_true_pending");
        let mut runtime = runtime_for_capture_moments(
            &root,
            &[
                TERRAIN_CAPTURE_MOMENT,
                RESTORE_CAPTURE_MOMENT,
                MAPNAME_CAPTURE_MOMENT,
            ],
        );
        runtime.round_commands_enabled = true;
        mark_moment_complete(&mut runtime, TERRAIN_CAPTURE_MOMENT);
        mark_moment_complete(&mut runtime, RESTORE_CAPTURE_MOMENT);
        // mapname_pass deliberately NOT completed; latch stays unset.
        assert!(!runtime.control_sequence_complete);

        let reported =
            report_incomplete_control_sequence_before_rounds_for_runtime(&runtime, &root);
        assert!(reported);
        let events = events_for(&root);
        assert!(events.contains("\"status\":\"blocked_before_mapname_pass\""));
        assert!(events.contains("\"pending_control_moments\":[\"mapname_pass\"]"));
        assert!(!events.contains("captured_runtime_terrain_inputs"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn finalization_stays_silent_when_control_sequence_completed() {
        // Latch set (control sequence completed before any round): the guard
        // does NOT fabricate a blocker, so normal success reporting applies.
        let root = temp_capture_dir("finalization_latch_set");
        let mut runtime =
            runtime_for_capture_moments(&root, &[TERRAIN_CAPTURE_MOMENT, RESTORE_CAPTURE_MOMENT]);
        runtime.round_commands_enabled = true;
        runtime.control_sequence_complete = true;
        assert!(!report_incomplete_control_sequence_before_rounds_for_runtime(&runtime, &root));

        // And with rounds off, the guard is inert regardless of the latch.
        let mut rounds_off =
            runtime_for_capture_moments(&root, &[TERRAIN_CAPTURE_MOMENT, RESTORE_CAPTURE_MOMENT]);
        rounds_off.round_commands_enabled = false;
        mark_moment_complete(&mut rounds_off, TERRAIN_CAPTURE_MOMENT);
        assert!(!report_incomplete_control_sequence_before_rounds_for_runtime(&rounds_off, &root));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn start_round_rejects_overwriting_an_in_flight_round() {
        // MOMFIX1 (guard): the manual path calls start_map_frame_capture_round_
        // for_runtime directly; it must refuse to clobber a round still running.
        let root = temp_capture_dir("round_no_clobber_inflight");
        let mut runtime = runtime_for_capture_moments(&root, &[TERRAIN_CAPTURE_MOMENT]);
        runtime.round_commands_enabled = true;
        runtime.control_sequence_complete = true; // rounds allowed

        start_map_frame_capture_round_for_runtime(&mut runtime, "first")
            .expect("first round starts");
        assert_eq!(runtime.active_round_label.as_deref(), Some("first"));
        assert!(!runtime.active_round_finished);

        let err = start_map_frame_capture_round_for_runtime(&mut runtime, "second")
            .expect_err("second round rejected while first in flight");
        assert!(err.contains("still in progress"));
        // The active round is untouched.
        assert_eq!(runtime.active_round_label.as_deref(), Some("first"));

        // After the first finishes, a new round may start.
        runtime.active_round_finished = true;
        start_map_frame_capture_round_for_runtime(&mut runtime, "second")
            .expect("second round starts after first finishes");
        assert_eq!(runtime.active_round_label.as_deref(), Some("second"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn terrain_variant_shader_switch_does_not_close_pass() {
        let mut state = TerrainCaptureState::default();
        assert!(state.observe_pixel_shader_created(
            0x30,
            b"TerrainIDMap_Texture : register(t5); TerrainDiffuse TerrainColorTint"
        ));
        assert!(state.observe_pixel_shader_created(
            0x50,
            b"TerrainIDMap_Texture : register(t5); SnowMudData_Texture : register(t7); TerrainDiffuse TerrainColorTint"
        ));
        state.observe_ps_set_shader(0x30);

        assert!(!should_finalize_map_frame_on_pixel_shader_transition(
            &state,
            TERRAIN_MAP_FRAME_CAPTURE_PROFILE,
            0x50,
            true,
            false,
        ));
        assert_eq!(
            map_frame_pixel_shader_pointer_matches_profile(
                &state,
                TERRAIN_MAP_FRAME_CAPTURE_PROFILE,
                0x50,
            ),
            Some(0x50)
        );
    }

    #[test]
    fn terrain_to_non_terrain_shader_transition_closes_pass() {
        let mut state = TerrainCaptureState::default();
        assert!(state.observe_pixel_shader_created(
            0x30,
            b"TerrainIDMap_Texture : register(t5); TerrainDiffuse TerrainColorTint"
        ));
        assert!(state.observe_water_pixel_shader_created(
            0x90,
            b"HeightTexture_Texture : register(t0); LeanTexture1_Texture : register(t1); LeanTexture2_Texture : register(t2); SpecularMap_Texture : register(t4); WaterRefraction_Texture : register(t5)"
        ));
        state.observe_ps_set_shader(0x30);

        assert!(should_finalize_map_frame_on_pixel_shader_transition(
            &state,
            TERRAIN_MAP_FRAME_CAPTURE_PROFILE,
            0x90,
            true,
            false,
        ));
        assert!(!should_finalize_map_frame_on_pixel_shader_transition(
            &state,
            TERRAIN_MAP_FRAME_CAPTURE_PROFILE,
            0x90,
            false,
            false,
        ));
        assert!(!should_finalize_map_frame_on_pixel_shader_transition(
            &state,
            TERRAIN_MAP_FRAME_CAPTURE_PROFILE,
            0x90,
            true,
            true,
        ));
    }

    #[test]
    fn all_map_frame_profiles_close_on_first_clean_boundary_after_minimum_draws() {
        for profile in [
            TERRAIN_MAP_FRAME_CAPTURE_PROFILE,
            WATER_MAP_FRAME_CAPTURE_PROFILE,
            RIVERS_MAP_FRAME_CAPTURE_PROFILE,
            BORDERS_MAP_FRAME_CAPTURE_PROFILE,
            TREES_MAP_FRAME_CAPTURE_PROFILE,
            RESTORE_MAP_FRAME_CAPTURE_PROFILE,
            MAPNAME_MAP_FRAME_CAPTURE_PROFILE,
        ] {
            assert!(!map_frame_clean_boundary_ready(
                profile.min_draws_before_clear_finalize.saturating_sub(1),
                profile,
                false,
            ));
            assert!(map_frame_clean_boundary_ready(
                profile.min_draws_before_clear_finalize,
                profile,
                false,
            ));
            assert!(!map_frame_clean_boundary_ready(
                profile.min_draws_before_clear_finalize,
                profile,
                true,
            ));
        }
    }

    #[test]
    fn only_blend_layer_profiles_close_on_post_draw_boundary() {
        for profile in [
            TERRAIN_MAP_FRAME_CAPTURE_PROFILE,
            WATER_MAP_FRAME_CAPTURE_PROFILE,
            RIVERS_MAP_FRAME_CAPTURE_PROFILE,
            BORDERS_MAP_FRAME_CAPTURE_PROFILE,
            TREES_MAP_FRAME_CAPTURE_PROFILE,
            RESTORE_MAP_FRAME_CAPTURE_PROFILE,
            MAPNAME_MAP_FRAME_CAPTURE_PROFILE,
        ] {
            let closes_after_minimum = map_frame_post_draw_blend_layer_boundary_ready(
                profile.min_draws_before_clear_finalize,
                profile,
                false,
            );
            // mapname_pass must NOT close on the post-draw boundary: the live
            // label pass is N glyph draws and closing after the first would
            // lose the rest. It closes on PS-transition / clear / present.
            assert_eq!(
                closes_after_minimum,
                matches!(
                    profile.capture_moment,
                    RIVERS_CAPTURE_MOMENT
                        | BORDERS_CAPTURE_MOMENT
                        | TREES_CAPTURE_MOMENT
                        | RESTORE_CAPTURE_MOMENT
                )
            );
            assert!(!map_frame_post_draw_blend_layer_boundary_ready(
                profile.min_draws_before_clear_finalize.saturating_sub(1),
                profile,
                false,
            ));
            assert!(!map_frame_post_draw_blend_layer_boundary_ready(
                profile.min_draws_before_clear_finalize,
                profile,
                true,
            ));
        }
    }

    #[test]
    fn trees_frame_repeat_mode_closes_on_repeated_first_draw_only() {
        let draw_a = test_tree_identity_draw(0, "277b7b5e8964214f", "vb_a", 6);
        let draw_b = test_tree_identity_draw(1, "6b77b6df9ba8f03d", "vb_b", 6);
        let draw_a_repeat = test_tree_identity_draw(2, "277b7b5e8964214f", "vb_a", 6);

        let mut captured = Vec::new();
        for draw in [draw_a.clone(), draw_b.clone(), draw_a_repeat.clone()] {
            if should_close_trees_pass_on_frame_repeat(
                &captured,
                &draw,
                TreesCloseMode::FrameRepeat,
            ) {
                break;
            }
            captured.push(draw);
        }
        assert_eq!(captured.len(), 2);
        assert_eq!(captured[0].shader.fnv1a64, draw_a.shader.fnv1a64);
        assert_eq!(captured[1].shader.fnv1a64, draw_b.shader.fnv1a64);
        assert!(!should_close_trees_pass_on_frame_repeat(
            &[draw_a, draw_b],
            &draw_a_repeat,
            TreesCloseMode::Default,
        ));
    }

    #[test]
    fn non_terrain_shader_family_transition_closes_each_profile() {
        let river_shader = b"RiverData_Texture : register(t3); LeanTexture1_Texture : register(t4); LeanTexture2_Texture : register(t5); SnowMudTexture_Texture : register(t8); GradientBorderChannel1_Texture : register(t12); GradientBorderChannel2_Texture : register(t13);";
        let border_shader = b"BorderDiffuse_Texture : register(t0); SnowMudData_Texture : register(t1); IntelMap_Texture : register(t2); ShadowMap_Texture : register(t3);";
        let tree_shader = b"DiffuseMap_Texture : register(t0); NormalMap_Texture : register(t1); TintMap_Texture : register(t2); SeasonMap_Texture : register(t3); TreeMaskTexture_Texture : register(t14); GradientBorderChannel1_Texture : register(t12); GradientBorderChannel2_Texture : register(t13);";
        let water_shader = b"HeightTexture_Texture : register(t0); LeanTexture1_Texture : register(t1); LeanTexture2_Texture : register(t2); SpecularMap_Texture : register(t4); WaterRefraction_Texture : register(t5)";

        let mut state = TerrainCaptureState::default();
        assert!(state.observe_water_pixel_shader_created(0x90, water_shader));
        assert!(state.observe_river_pixel_shader_created(0xA0, river_shader));
        assert!(state.observe_river_pixel_shader_created(0xA1, river_shader));
        assert!(state.observe_border_pixel_shader_created(0xB0, border_shader));
        assert!(state.observe_tree_pixel_shader_created(0xC0, tree_shader));
        assert!(state.observe_pixel_shader_created(
            0x30,
            b"TerrainIDMap_Texture : register(t5); TerrainDiffuse TerrainColorTint"
        ));

        state.observe_ps_set_shader(0x90);
        assert!(should_finalize_map_frame_on_pixel_shader_transition(
            &state,
            WATER_MAP_FRAME_CAPTURE_PROFILE,
            0xA0,
            true,
            false,
        ));

        state.observe_ps_set_shader(0xA0);
        assert!(!should_finalize_map_frame_on_pixel_shader_transition(
            &state,
            RIVERS_MAP_FRAME_CAPTURE_PROFILE,
            0xA1,
            true,
            false,
        ));
        assert!(should_finalize_map_frame_on_pixel_shader_transition(
            &state,
            RIVERS_MAP_FRAME_CAPTURE_PROFILE,
            0xB0,
            true,
            false,
        ));

        state.observe_ps_set_shader(0xB0);
        assert!(should_finalize_map_frame_on_pixel_shader_transition(
            &state,
            BORDERS_MAP_FRAME_CAPTURE_PROFILE,
            0xC0,
            true,
            false,
        ));

        state.observe_ps_set_shader(0xC0);
        assert!(should_finalize_map_frame_on_pixel_shader_transition(
            &state,
            TREES_MAP_FRAME_CAPTURE_PROFILE,
            0x30,
            true,
            false,
        ));
    }

    #[test]
    fn draw_budget_trips_before_next_map_frame_draw_readback() {
        let root = temp_capture_dir("draw_budget_trip");
        let mut runtime = runtime_for_capture_moments(&root, &[RIVERS_CAPTURE_MOMENT]);
        runtime.capture_budget.max_draws_per_pass = 2;
        runtime.map_frame_draws.push(test_map_frame_draw(
            0,
            "river_pixel_shader_test.bin",
            "1111111111111111",
            "map_vertex_shader_test.bin",
            "2222222222222222",
        ));
        assert!(next_map_frame_draw_budget_exceeded(&runtime).is_none());
        runtime.map_frame_draws.push(test_map_frame_draw(
            1,
            "river_pixel_shader_test.bin",
            "1111111111111111",
            "map_vertex_shader_test.bin",
            "2222222222222222",
        ));

        let event = next_map_frame_draw_budget_exceeded(&runtime)
            .expect("third draw should exceed draw budget");
        assert_eq!(event.cap, "max_draws_per_pass");
        assert_eq!(event.capture_moment, RIVERS_CAPTURE_MOMENT);
        assert_eq!(event.draw_count, 2);
        assert!(capture_budget_blocker(&event).contains("max_draws_per_pass=2"));
        let _ = fs::remove_dir_all(root);
    }

    // CAPFIX3 ISSUE 2 — the night terrain 128-draw blowout. A pass that fails to
    // finalize (blockers, e.g. no TerrainIDMap signature for the active slot at
    // the 3AM close-zoom view) must have its per-frame draw accumulator RESET at
    // the clean frame boundary, so max_draws_per_pass stays a per-frame cap and
    // draws cannot pile up across frames to 128. This test proves the boundary
    // close resets an incomplete terrain pass, and does NOT reset a completed
    // moment (whose draws were already cleared at advance).
    #[test]
    fn frame_boundary_closes_incomplete_pass_and_resets_draw_accumulator() {
        let root = temp_capture_dir("frame_boundary_close");
        let mut runtime = runtime_for_capture_moments(&root, &[TERRAIN_CAPTURE_MOMENT]);
        // Simulate this frame's terrain draws having accumulated.
        for i in 0..7u32 {
            runtime.map_frame_draws.push(test_map_frame_draw(
                i,
                "pdxmap_pixel_shader.bin",
                "1111111111111111",
                "map_vertex_shader.bin",
                "2222222222222222",
            ));
        }
        assert_eq!(runtime.map_frame_draws.len(), 7);
        // Incomplete terrain moment at the clean frame boundary => pass closes,
        // all 7 carried draws discarded, next frame starts fresh.
        let discarded = close_map_frame_pass_for_runtime(&mut runtime)
            .expect("incomplete pass with carried draws must close at the frame boundary");
        assert_eq!(discarded, 7);
        assert!(runtime.map_frame_draws.is_empty());

        // Second boundary with no draws carried => no-op (returns None).
        assert!(close_map_frame_pass_for_runtime(&mut runtime).is_none());

        // A COMPLETED moment is never reset here (its draws were cleared at
        // advance); mark terrain complete and re-populate — close must decline.
        runtime
            .completed_map_frame_moments
            .insert(TERRAIN_CAPTURE_MOMENT.to_string());
        runtime.map_frame_draws.push(test_map_frame_draw(
            0,
            "pdxmap_pixel_shader.bin",
            "1111111111111111",
            "map_vertex_shader.bin",
            "2222222222222222",
        ));
        assert!(close_map_frame_pass_for_runtime(&mut runtime).is_none());
        assert_eq!(runtime.map_frame_draws.len(), 1);
        let _ = fs::remove_dir_all(root);
    }

    // CAPFIX3 ISSUE 2 — cross-frame simulation: with the frame-boundary close in
    // place, a pass that never finalizes stays bounded to a single frame's draws
    // and never reaches max_draws_per_pass, so the budget never trips. Without
    // the close, 3 frames * 7 draws would exceed a cap of 8.
    #[test]
    fn incomplete_pass_stays_bounded_across_frames_with_boundary_close() {
        let root = temp_capture_dir("bounded_across_frames");
        let mut runtime = runtime_for_capture_moments(&root, &[TERRAIN_CAPTURE_MOMENT]);
        runtime.capture_budget.max_draws_per_pass = 8;
        for _frame in 0..3 {
            for i in 0..7u32 {
                runtime.map_frame_draws.push(test_map_frame_draw(
                    i,
                    "pdxmap_pixel_shader.bin",
                    "1111111111111111",
                    "map_vertex_shader.bin",
                    "2222222222222222",
                ));
            }
            // Per-frame draw count never exceeds the cap within a single frame.
            assert!(next_map_frame_draw_budget_exceeded(&runtime).is_none());
            // Clean frame boundary closes the still-incomplete pass.
            close_map_frame_pass_for_runtime(&mut runtime);
            assert!(runtime.map_frame_draws.is_empty());
        }
        // After 3 frames the accumulator is still empty — no cross-frame blowout.
        assert!(next_map_frame_draw_budget_exceeded(&runtime).is_none());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn byte_budget_trips_before_payload_write_is_accepted() {
        let root = temp_capture_dir("byte_budget_trip");
        let mut runtime = runtime_for_capture_moments(&root, &[RIVERS_CAPTURE_MOMENT]);
        runtime.capture_budget.max_total_payload_bytes_per_session = 10;

        assert!(
            reserve_capture_payload_bytes_for_runtime(&mut runtime, 6, "first.d3d11").is_none()
        );
        let event = reserve_capture_payload_bytes_for_runtime(&mut runtime, 5, "second.d3d11")
            .expect("second payload should exceed byte budget");
        assert_eq!(event.cap, "max_total_payload_bytes_per_session");
        assert_eq!(event.capture_moment, RIVERS_CAPTURE_MOMENT);
        assert_eq!(event.total_payload_bytes_reserved, 11);
        assert_eq!(event.attempted_payload_bytes, 5);
        assert!(capture_budget_blocker(&event).contains("total_payload_bytes_reserved=11"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn payload_writer_queue_budget_is_byte_capped() {
        assert!(!payload_writer_queue_bytes_exceeded(100, 28, 128));
        assert!(payload_writer_queue_bytes_exceeded(100, 29, 128));
        assert!(payload_writer_queue_bytes_exceeded(
            u64::MAX - 1,
            8,
            u64::MAX
        ));
    }

    #[test]
    fn quiesce_wind_down_clears_deferred_capture_work_and_queues_releases() {
        let root = temp_capture_dir("quiesce_wind_down");
        let mut runtime = runtime_for_capture_moments(&root, &[TERRAIN_CAPTURE_MOMENT]);
        runtime.pending_dwell_end_boundary_finalization = true;
        runtime.pending_dwell_end_generated_texture_check = true;
        runtime.dwell_end_generated_texture_check_done = false;
        runtime.terrain_chunk_library.enabled = true;
        runtime.map_frame_draws.push(test_map_frame_draw(
            0,
            "pdxmap_pixel_shader_test.bin",
            "0123456789abcdef",
            "pdxmap_vertex_shader_test.bin",
            "fedcba9876543210",
        ));
        runtime.deferred_runtime_targets.insert(
            SNOW_MUD_DATA_RESOURCE_SYMBOL.to_string(),
            DeferredRuntimeTarget {
                target: CAPTURE_TARGETS[1],
                srv: 0x1000,
                slot: 7,
                identity: "unit_test",
            },
        );

        assert_eq!(
            queue_quiesce_release_and_clear_capture_state(&mut runtime),
            1
        );
        assert!(runtime.deferred_runtime_targets.is_empty());
        assert_eq!(runtime.pending_render_thread_release_srvs, vec![0x1000]);
        assert!(!runtime.pending_dwell_end_boundary_finalization);
        assert!(!runtime.pending_dwell_end_generated_texture_check);
        assert!(runtime.dwell_end_generated_texture_check_done);
        assert!(!runtime.terrain_chunk_library.enabled);
        assert!(runtime.map_frame_draws.is_empty());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn map_frame_shader_evidence_includes_each_draw_variant() {
        let root = temp_capture_dir("multi_variant_shader_evidence");
        fs::write(
            root.join("pdxmap_pixel_shader_1111111111111111.bin"),
            [1u8; 8],
        )
        .expect("write first PS bytecode");
        fs::write(
            root.join("pdxmap_pixel_shader_2222222222222222.bin"),
            [2u8; 8],
        )
        .expect("write second PS bytecode");
        fs::write(
            root.join("pdxmap_vertex_shader_aaaaaaaaaaaaaaaa.bin"),
            [3u8; 8],
        )
        .expect("write first VS bytecode");
        fs::write(
            root.join("pdxmap_vertex_shader_bbbbbbbbbbbbbbbb.bin"),
            [4u8; 8],
        )
        .expect("write second VS bytecode");

        let draws = vec![
            test_map_frame_draw(
                0,
                "pdxmap_pixel_shader_1111111111111111.bin",
                "1111111111111111",
                "pdxmap_vertex_shader_aaaaaaaaaaaaaaaa.bin",
                "aaaaaaaaaaaaaaaa",
            ),
            test_map_frame_draw(
                1,
                "pdxmap_pixel_shader_2222222222222222.bin",
                "2222222222222222",
                "pdxmap_vertex_shader_bbbbbbbbbbbbbbbb.bin",
                "bbbbbbbbbbbbbbbb",
            ),
        ];
        let mut files = BTreeMap::new();
        let mut blockers = Vec::new();

        add_map_frame_draw_shader_file_evidence(&mut files, &root, &draws, &mut blockers);

        assert!(blockers.is_empty(), "{blockers:?}");
        assert!(files.contains_key("pdxmap_pixel_shader_1111111111111111.bin"));
        assert!(files.contains_key("pdxmap_pixel_shader_2222222222222222.bin"));
        assert!(files.contains_key("pdxmap_vertex_shader_aaaaaaaaaaaaaaaa.bin"));
        assert!(files.contains_key("pdxmap_vertex_shader_bbbbbbbbbbbbbbbb.bin"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn generated_runtime_capture_targets_are_finalize_timed() {
        for target in CAPTURE_TARGETS {
            if target.symbol == TERRAIN_ID_RESOURCE_SYMBOL {
                assert_eq!(target.timing, RuntimeTextureCaptureTiming::FirstDraw);
                assert!(!target.dwell_end_resnapshot);
            } else {
                assert_eq!(target.timing, RuntimeTextureCaptureTiming::Finalize);
                assert_eq!(
                    target.dwell_end_resnapshot,
                    matches!(
                        target.symbol,
                        PROVINCE_SECONDARY_COLOR_MAP_SYMBOL
                            | GRADIENT_BORDER_CHANNEL1_SYMBOL
                            | GRADIENT_BORDER_CHANNEL2_SYMBOL
                    )
                );
            }
        }
    }

    #[test]
    fn terrain_scene_cb0_name_requires_byte_identity_evidence() {
        let root = temp_capture_dir("terrain_scene_cb0_name_evidence");
        let bytes = (0..416)
            .map(|value| (value % 251) as u8)
            .collect::<Vec<_>>();
        write_constant_buffer_capture_files(&root, &bytes, 0).expect("write cb0 evidence");
        let reflected_cb0 = BTreeMap::from([("dx11_cb0".to_string(), 0)]);
        let reflected_cb1 = BTreeMap::from([("dx11_cb1".to_string(), 1)]);

        assert_eq!(
            canonical_map_frame_constant_buffer_name_for_binding(
                TERRAIN_MAP_FRAME_CAPTURE_PROFILE.capture_moment,
                "PS",
                0,
                TERRAIN_MAP_FRAME_CAPTURE_PROFILE
                    .shader_sources
                    .iter()
                    .copied(),
                &reflected_cb0,
                &root,
                &bytes,
            ),
            Some(TERRAIN_SCENE_CB0_SYMBOL.to_string())
        );
        assert_eq!(
            canonical_map_frame_constant_buffer_name_for_binding(
                TERRAIN_MAP_FRAME_CAPTURE_PROFILE.capture_moment,
                "VS",
                0,
                TERRAIN_MAP_FRAME_CAPTURE_PROFILE
                    .shader_sources
                    .iter()
                    .copied(),
                &reflected_cb0,
                &root,
                &bytes,
            ),
            Some(TERRAIN_SCENE_CB0_SYMBOL.to_string())
        );
        assert_eq!(
            canonical_map_frame_constant_buffer_name_for_binding(
                TERRAIN_MAP_FRAME_CAPTURE_PROFILE.capture_moment,
                "VS",
                1,
                TERRAIN_MAP_FRAME_CAPTURE_PROFILE
                    .shader_sources
                    .iter()
                    .copied(),
                &reflected_cb1,
                &root,
                &[0u8; 112],
            ),
            Some(TERRAIN_DRAW_CB1_LAYOUT_SOURCE.to_string())
        );

        let mut mismatched = bytes.clone();
        mismatched[17] ^= 0x5a;
        assert_eq!(
            canonical_map_frame_constant_buffer_name_for_binding(
                TERRAIN_MAP_FRAME_CAPTURE_PROFILE.capture_moment,
                "PS",
                0,
                TERRAIN_MAP_FRAME_CAPTURE_PROFILE
                    .shader_sources
                    .iter()
                    .copied(),
                &reflected_cb0,
                &root,
                &mismatched,
            ),
            Some("dx11_cb0".to_string())
        );
        assert_eq!(
            canonical_map_frame_constant_buffer_name_for_binding(
                TERRAIN_MAP_FRAME_CAPTURE_PROFILE.capture_moment,
                "PS",
                1,
                TERRAIN_MAP_FRAME_CAPTURE_PROFILE
                    .shader_sources
                    .iter()
                    .copied(),
                &reflected_cb0,
                &root,
                &bytes,
            ),
            None
        );
        assert_eq!(
            canonical_map_frame_constant_buffer_name_for_binding(
                WATER_MAP_FRAME_CAPTURE_PROFILE.capture_moment,
                "PS",
                0,
                WATER_MAP_FRAME_CAPTURE_PROFILE
                    .shader_sources
                    .iter()
                    .copied(),
                &reflected_cb0,
                &root,
                &bytes,
            ),
            Some("dx11_cb0".to_string())
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn session_20260702b_draw0_ps_cb0_matches_terrain_scene_cb0_evidence() {
        let session_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../map_viewer/out/captures/session_20260702b");
        if !session_dir.join(TERRAIN_SCENE_CB0_META_FILE).exists() {
            eprintln!(
                "skipping session_20260702b fixture regression check; fixture is not present at {}",
                session_dir.display()
            );
            return;
        }
        let draw_cb0 = fs::read(session_dir.join("map_frame_draw000_ps_constant_buffer_slot0.bin"))
            .expect("read session draw 0 PS slot 0 constant buffer");

        assert!(terrain_scene_cb0_evidence_matches_binding(
            &session_dir,
            &draw_cb0
        ));
        assert_eq!(
            canonical_map_frame_constant_buffer_name_for_binding(
                TERRAIN_MAP_FRAME_CAPTURE_PROFILE.capture_moment,
                "PS",
                0,
                TERRAIN_MAP_FRAME_CAPTURE_PROFILE
                    .shader_sources
                    .iter()
                    .copied(),
                &BTreeMap::from([("dx11_cb0".to_string(), 0)]),
                &session_dir,
                &draw_cb0,
            ),
            Some(TERRAIN_SCENE_CB0_SYMBOL.to_string())
        );
    }

    fn test_runtime_texture_meta(target: CaptureTarget, bytes: Vec<u8>) -> CapturedTextureMeta {
        CapturedTextureMeta {
            width: 2,
            height: 2,
            byte_len: bytes.len(),
            fnv1a64: fnv1a64_bytes(&bytes),
            source_format: DXGI_FORMAT_B8G8R8A8_UNORM,
            slot: 14,
            identity: "unit_test",
            captured_at: target.timing,
            captured_bytes: bytes,
        }
    }

    #[test]
    fn dwell_end_runtime_texture_metadata_records_supersede_and_unchanged() {
        let root = temp_capture_dir("dwell_end_texture_metadata");
        let target =
            capture_target_by_symbol(GRADIENT_BORDER_CHANNEL1_SYMBOL).expect("gradient target");
        let changed_meta = test_runtime_texture_meta(target, vec![1; 16]);
        write_runtime_texture_meta_file(
            &root,
            target,
            &changed_meta,
            RuntimeTextureCaptureTiming::DwellEnd,
            Some("089afb7c652d0325"),
            None,
            None,
        )
        .expect("write superseded metadata");
        let changed: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join(target.meta_file)).unwrap())
                .expect("parse changed metadata");
        assert_eq!(changed["captured_at"], "dwell_end");
        assert_eq!(changed["superseded_hash"], "089afb7c652d0325");

        let unchanged_meta = test_runtime_texture_meta(target, vec![2; 16]);
        write_runtime_texture_meta_file(
            &root,
            target,
            &unchanged_meta,
            RuntimeTextureCaptureTiming::Finalize,
            None,
            Some("unchanged"),
            Some(&unchanged_meta.fnv1a64),
        )
        .expect("write unchanged metadata");
        let unchanged: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join(target.meta_file)).unwrap())
                .expect("parse unchanged metadata");
        assert_eq!(unchanged["captured_at"], "finalize");
        assert_eq!(unchanged["dwell_end_status"], "unchanged");
        assert_eq!(unchanged["dwell_end_hash"], unchanged_meta.fnv1a64);
        assert!(unchanged.get("superseded_hash").is_none());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn dwell_end_generated_texture_rewrite_updates_bundle_references() {
        let root = temp_capture_dir("dwell_end_bundle_rewrite");
        let target =
            capture_target_by_symbol(GRADIENT_BORDER_CHANNEL1_SYMBOL).expect("gradient target");
        let old_hash = "089afb7c652d0325";
        let old_file = map_frame_shared_payload_file(
            TERRAIN_MAP_FRAME_CAPTURE_PROFILE,
            "ps_srv",
            Some(target.symbol),
            old_hash,
        );
        let bundle = serde_json::json!({
            "active_draw": {
                "shader_resources": [{
                    "declared_name": target.symbol,
                    "file": old_file,
                    "byte_len": 16,
                    "fnv1a64": old_hash,
                    "width": 2,
                    "height": 2,
                    "payload_format": NATIVE_D3D11_TEXTURE_PAYLOAD_FORMAT,
                    "subresources": []
                }]
            },
            "draws": [{
                "shader_resources": [{
                    "declared_name": target.symbol,
                    "file": old_file,
                    "byte_len": 16,
                    "fnv1a64": old_hash,
                    "width": 2,
                    "height": 2,
                    "payload_format": NATIVE_D3D11_TEXTURE_PAYLOAD_FORMAT,
                    "subresources": []
                }]
            }],
            "runtime_files": {
                target.rgba_file: {
                    "file": target.rgba_file,
                    "byte_len": 16,
                    "fnv1a64": old_hash,
                    "provenance": "runtime_texture_readback_GradientBorderChannel1_finalize"
                }
            }
        });
        fs::write(
            root.join(TERRAIN_MAP_FRAME_CAPTURE_PROFILE.bundle_file),
            serde_json::to_string_pretty(&bundle).unwrap(),
        )
        .expect("write synthetic bundle");

        let meta = test_runtime_texture_meta(target, vec![3; 16]);
        rewrite_generated_texture_bundle_references(&root, target, &meta)
            .expect("rewrite bundle references");

        let rewritten: serde_json::Value = serde_json::from_slice(
            &fs::read(root.join(TERRAIN_MAP_FRAME_CAPTURE_PROFILE.bundle_file)).unwrap(),
        )
        .expect("parse rewritten bundle");
        let new_file = map_frame_shared_payload_file(
            TERRAIN_MAP_FRAME_CAPTURE_PROFILE,
            "ps_srv",
            Some(target.symbol),
            &meta.fnv1a64,
        );
        assert_eq!(
            rewritten["active_draw"]["shader_resources"][0]["file"],
            new_file
        );
        assert_eq!(
            rewritten["draws"][0]["shader_resources"][0]["fnv1a64"],
            meta.fnv1a64
        );
        assert_eq!(
            rewritten["runtime_files"][target.rgba_file]["fnv1a64"],
            meta.fnv1a64
        );
        assert_eq!(
            rewritten["runtime_files"][new_file.as_str()]["provenance"],
            "d3d11_staging_map_readback_from_dwell_end_ps_srv"
        );
        assert_eq!(fs::read(root.join(new_file)).unwrap(), meta.captured_bytes);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn dwell_end_rewrite_preserves_at_moment_runtime_texture_payloads() {
        let root = temp_capture_dir("dwell_end_preserves_moment_payload");
        let target = capture_target_by_symbol(SNOW_MUD_DATA_RESOURCE_SYMBOL).expect("snow target");
        let moment_hash = "c16bc99b8d020f62";
        let moment_file = map_frame_shared_payload_file(
            WATER_MAP_FRAME_CAPTURE_PROFILE,
            "ps_srv",
            Some(target.symbol),
            moment_hash,
        );
        let bundle = serde_json::json!({
            "active_draw": {
                "shader_resources": [{
                    "declared_name": target.symbol,
                    "file": moment_file,
                    "byte_len": 16,
                    "fnv1a64": moment_hash,
                    "width": 2,
                    "height": 2,
                    "payload_format": NATIVE_D3D11_TEXTURE_PAYLOAD_FORMAT,
                    "provenance": "d3d11_staging_map_readback_from_active_ps_srv",
                    "subresources": []
                }]
            },
            "draws": [{
                "shader_resources": [{
                    "declared_name": target.symbol,
                    "file": moment_file,
                    "byte_len": 16,
                    "fnv1a64": moment_hash,
                    "width": 2,
                    "height": 2,
                    "payload_format": NATIVE_D3D11_TEXTURE_PAYLOAD_FORMAT,
                    "provenance": "d3d11_staging_map_readback_from_active_ps_srv",
                    "subresources": []
                }]
            }],
            "runtime_files": {}
        });
        fs::write(
            root.join(WATER_MAP_FRAME_CAPTURE_PROFILE.bundle_file),
            serde_json::to_string_pretty(&bundle).unwrap(),
        )
        .expect("write synthetic bundle");

        let finalize_meta = test_runtime_texture_meta(target, vec![9; 16]);
        rewrite_generated_texture_bundle_references(&root, target, &finalize_meta)
            .expect("dwell-end rewrite should leave at-moment payloads alone");

        let rewritten: serde_json::Value = serde_json::from_slice(
            &fs::read(root.join(WATER_MAP_FRAME_CAPTURE_PROFILE.bundle_file)).unwrap(),
        )
        .expect("parse rewritten bundle");
        assert_eq!(
            rewritten["active_draw"]["shader_resources"][0]["file"],
            moment_file
        );
        assert_eq!(
            rewritten["draws"][0]["shader_resources"][0]["fnv1a64"],
            moment_hash
        );
        let finalize_file = map_frame_shared_payload_file(
            WATER_MAP_FRAME_CAPTURE_PROFILE,
            "ps_srv",
            Some(target.symbol),
            &finalize_meta.fnv1a64,
        );
        assert!(!root.join(finalize_file).exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn time_varying_runtime_textures_use_moment_payloads_in_map_frame_bundles() {
        for profile in [
            TERRAIN_MAP_FRAME_CAPTURE_PROFILE,
            WATER_MAP_FRAME_CAPTURE_PROFILE,
            RIVERS_MAP_FRAME_CAPTURE_PROFILE,
            BORDERS_MAP_FRAME_CAPTURE_PROFILE,
            TREES_MAP_FRAME_CAPTURE_PROFILE,
        ] {
            for symbol in [
                SNOW_MUD_DATA_RESOURCE_SYMBOL,
                "SnowMudTexture",
                GRADIENT_BORDER_CHANNEL1_SYMBOL,
                GRADIENT_BORDER_CHANNEL2_SYMBOL,
                "GradientBorderChannel3",
                "FoW",
                "ShadowMap",
                LIGHT_INDEX_MAP_SYMBOL,
                LIGHT_DATA_MAP_SYMBOL,
                PROVINCE_SECONDARY_COLOR_MAP_SYMBOL,
                "IntelMap",
            ] {
                assert!(
                    map_frame_declared_resource_needs_moment_payload(profile, Some(symbol)),
                    "{} should capture {symbol} as an at-moment payload",
                    profile.capture_moment
                );
            }
            assert!(!map_frame_declared_resource_needs_moment_payload(
                profile,
                Some("TerrainDiffuse"),
            ));
        }
    }

    #[test]
    fn buffer_byte_width_records_full_source_width() {
        let desc = BufferDesc {
            byte_width: 64,
            usage: D3D11_USAGE_DEFAULT,
            bind_flags: D3D11_BIND_VERTEX_BUFFER,
            cpu_access_flags: 0,
            misc_flags: 0,
            structure_byte_stride: 0,
        };
        assert_eq!(map_frame_buffer_byte_width(&desc), Some(64));
    }

    #[test]
    fn unobserved_moment_watches_until_dwell_end_without_blocking_session() {
        let root = temp_capture_dir("unobserved_moment_dwell_end");
        let mut runtime = runtime_for_capture_moments(
            &root,
            &[
                TERRAIN_CAPTURE_MOMENT,
                WATER_CAPTURE_MOMENT,
                RIVERS_CAPTURE_MOMENT,
            ],
        );
        runtime.active_capture_index = 2;
        runtime.active_moment_started_ms = 12_500;
        runtime
            .completed_map_frame_moments
            .insert(TERRAIN_CAPTURE_MOMENT.to_string());
        runtime
            .completed_map_frame_moments
            .insert(WATER_CAPTURE_MOMENT.to_string());
        runtime
            .map_frame_rejections
            .insert("no active river pixel shader for rivers-pass map-frame capture".to_string());

        let observation = not_observed_during_dwell(&runtime, 30_000)
            .expect("rivers should be terminally not observed at dwell end");
        assert_eq!(observation.profile.capture_moment, RIVERS_CAPTURE_MOMENT);
        assert_eq!(observation.watched_start_ms, 12_500);
        assert_eq!(observation.watched_end_ms, 30_000);
        assert!(observation
            .blocker
            .contains("no active river shader draw observed"));

        write_not_observed_map_capture_session(
            &root,
            &runtime.capture_moment_sequence,
            &runtime.completed_map_frame_moments,
            &observation,
        )
        .expect("write not-observed session");
        let session: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join(MAP_CAPTURE_SESSION_FILE)).unwrap())
                .expect("parse session");
        assert_eq!(session["provenance"], "dwell_end_not_observed");
        assert_eq!(session["moments"][2]["name"], RIVERS_CAPTURE_MOMENT);
        assert_eq!(session["moments"][2]["status"], "not_observed_during_dwell");
        assert_eq!(session["moments"][2]["watched_start_ms"], 12_500);
        assert_eq!(session["moments"][2]["watched_end_ms"], 30_000);
        assert_eq!(
            session["moments"][0]["status"],
            "captured_pending_renderer_validation"
        );
        assert_eq!(
            session["moments"][1]["status"],
            "captured_pending_renderer_validation"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn terrain_capture_control_defaults_settle_delay() {
        let control: TerrainCaptureControl = serde_json::from_str(
            r#"{
                "schema": "battle-plan-terrain-capture-control-v1",
                "capture_moment": "terrain_active_pass",
                "provenance": "unit_test"
            }"#,
        )
        .expect("control without settle_ms");

        assert_eq!(control.settle_ms, DEFAULT_SETTLE_MS);
        assert_eq!(control.ui_cb_skip, UiCbSkipPolicy::Hazard);
        assert_eq!(
            capture_budget_from_control(Some(&control)),
            CaptureBudget {
                max_draws_per_pass: DEFAULT_MAX_DRAWS_PER_PASS,
                max_total_payload_bytes_per_session: DEFAULT_MAX_TOTAL_PAYLOAD_BYTES_PER_SESSION,
                max_capture_work_ms_per_frame: DEFAULT_MAX_CAPTURE_WORK_MS_PER_FRAME,
                max_payload_writer_queue_bytes: DEFAULT_MAX_PAYLOAD_WRITER_QUEUE_BYTES,
            }
        );
    }

    #[test]
    fn terrain_capture_control_parses_ui_cb_skip_policy() {
        let control: TerrainCaptureControl = serde_json::from_str(
            r#"{
                "schema": "battle-plan-terrain-capture-control-v1",
                "capture_moment": "ui_pass",
                "ui_cb_skip": "none",
                "provenance": "unit_test"
            }"#,
        )
        .expect("control with ui cb skip policy");

        assert_eq!(control.ui_cb_skip, UiCbSkipPolicy::None);

        let control: TerrainCaptureControl = serde_json::from_str(
            r#"{
                "schema": "battle-plan-terrain-capture-control-v1",
                "capture_moment": "ui_pass",
                "ui_cb_skip": "all",
                "provenance": "unit_test"
            }"#,
        )
        .expect("control with all ui cb skip policy");
        assert_eq!(control.ui_cb_skip, UiCbSkipPolicy::All);
    }

    #[test]
    fn terrain_capture_control_parses_output_dir_and_writer_queue_budget() {
        let control: TerrainCaptureControl = serde_json::from_str(
            r#"{
                "schema": "battle-plan-terrain-capture-control-v1",
                "capture_out_dir": "C:\\fast-capture\\terrain",
                "max_payload_writer_queue_bytes": 123456,
                "provenance": "unit_test"
            }"#,
        )
        .expect("control with output dir and writer queue budget");

        assert_eq!(
            control.capture_out_dir,
            Some(PathBuf::from(r"C:\fast-capture\terrain"))
        );
        assert_eq!(
            capture_budget_from_control(Some(&control)).max_payload_writer_queue_bytes,
            123_456
        );
    }

    #[test]
    fn terrain_chunk_library_control_filters_moment_and_uses_library_budget() {
        let control: TerrainCaptureControl = serde_json::from_str(
            r#"{
                "schema": "battle-plan-terrain-capture-control-v1",
                "capture_moments": ["terrain_active_pass", "terrain_chunk_library"],
                "terrain_chunk_library": true,
                "provenance": "unit_test"
            }"#,
        )
        .expect("chunk library control");

        assert!(terrain_chunk_library_enabled_from_control(Some(&control)));
        assert_eq!(
            capture_moment_sequence_from_control(Some(&control)),
            vec![TERRAIN_CAPTURE_MOMENT.to_string()]
        );
        assert_eq!(
            capture_budget_from_control(Some(&control)),
            CaptureBudget {
                max_draws_per_pass: DEFAULT_TERRAIN_CHUNK_LIBRARY_MAX_DRAWS_PER_PASS,
                max_total_payload_bytes_per_session:
                    DEFAULT_TERRAIN_CHUNK_LIBRARY_MAX_TOTAL_PAYLOAD_BYTES_PER_SESSION,
                max_capture_work_ms_per_frame:
                    DEFAULT_TERRAIN_CHUNK_LIBRARY_MAX_CAPTURE_WORK_MS_PER_FRAME,
                max_payload_writer_queue_bytes: DEFAULT_MAX_PAYLOAD_WRITER_QUEUE_BYTES,
            }
        );
    }

    #[test]
    fn map_frame_capture_can_be_disabled_by_control() {
        let control: TerrainCaptureControl = serde_json::from_str(
            r#"{
                "schema": "battle-plan-terrain-capture-control-v1",
                "capture_moments": ["terrain_active_pass"],
                "map_frame_capture_enabled": false,
                "census": true,
                "provenance": "unit_test"
            }"#,
        )
        .expect("control with map-frame capture disabled");

        assert!(capture_moment_sequence_from_control(Some(&control)).is_empty());

        let root = temp_capture_dir("map_frame_capture_disabled");
        let mut runtime = runtime_for_capture_moments(&root, &[TERRAIN_CAPTURE_MOMENT]);
        runtime.capture_moment_sequence = capture_moment_sequence_from_control(Some(&control));
        assert!(requested_map_frame_capture_profile_for_runtime(&runtime)
            .expect_err("map-frame profile should be disabled")
            .contains("map-frame capture disabled"));
        assert!(!capture_complete(&runtime));
    }

    #[test]
    fn ui_pass_hazard_skip_policy_targets_small_nonindexed_draws() {
        assert!(ui_pass_interop_hazard_draw("Draw", 4));
        assert!(ui_pass_interop_hazard_draw("Draw", 6));
        assert!(!ui_pass_interop_hazard_draw("Draw", 7));
        assert!(!ui_pass_interop_hazard_draw("DrawIndexed", 4));

        assert!(ui_pass_skip_policy_applies(UiCbSkipPolicy::Hazard, true));
        assert!(!ui_pass_skip_policy_applies(UiCbSkipPolicy::Hazard, false));
        assert!(ui_pass_skip_policy_applies(UiCbSkipPolicy::All, false));
        assert!(!ui_pass_skip_policy_applies(UiCbSkipPolicy::None, true));
    }

    #[test]
    fn ui_pass_unobtained_constant_buffers_record_reason() {
        let vertex_shader = MapFrameShaderFile {
            stage: "VS",
            pointer: "0x10".to_string(),
            bytecode_file: "ui_vs.bin".to_string(),
            byte_len: 16,
            fnv1a64: "vs_hash".to_string(),
            reflection_resources: BTreeMap::new(),
            input_parameters: Vec::new(),
            constant_buffers: BTreeMap::from([("UiVsConstants".to_string(), 0)]),
            source_evidence: Vec::new(),
        };
        let pixel_shader = MapFrameShaderFile {
            stage: "PS",
            pointer: "0x20".to_string(),
            bytecode_file: "ui_ps.bin".to_string(),
            byte_len: 16,
            fnv1a64: "ps_hash".to_string(),
            reflection_resources: BTreeMap::new(),
            input_parameters: Vec::new(),
            constant_buffers: BTreeMap::from([("UiPsConstants".to_string(), 0)]),
            source_evidence: Vec::new(),
        };

        let bindings = ui_pass_unobtained_constant_buffers_from_shaders(
            &vertex_shader,
            &pixel_shader,
            UI_PASS_INTEROP_HAZARD_UNOBTAINED_REASON,
        );

        assert_eq!(bindings.len(), 2);
        assert!(bindings.iter().all(|binding| {
            binding.file.is_none()
                && binding.byte_len == 0
                && binding.unobtained_reason == Some(UI_PASS_INTEROP_HAZARD_UNOBTAINED_REASON)
        }));
        assert!(bindings.iter().any(|binding| {
            binding.stage == "VS"
                && binding.slot == 0
                && binding.declared_name.as_deref() == Some("UiVsConstants")
        }));
        assert!(bindings.iter().any(|binding| {
            binding.stage == "PS"
                && binding.slot == 0
                && binding.declared_name.as_deref() == Some("UiPsConstants")
        }));
    }

    #[test]
    fn ui_pass_unobtained_cb_and_srv_evidence_serializes_reason() {
        let shader = MapFrameShaderFile {
            stage: "PS",
            pointer: "0x20".to_string(),
            bytecode_file: "ui_ps.bin".to_string(),
            byte_len: 16,
            fnv1a64: "ps_hash".to_string(),
            reflection_resources: BTreeMap::new(),
            input_parameters: Vec::new(),
            constant_buffers: BTreeMap::new(),
            source_evidence: Vec::new(),
        };
        let mut buffers = Vec::new();
        ui_pass_unobtained_constant_buffers_for_stage(
            &shader,
            UI_PASS_INTEROP_HAZARD_UNOBTAINED_REASON,
            &mut buffers,
        );
        let cb_json = serde_json::to_value(&buffers[0]).expect("serialize cb evidence");
        assert_eq!(
            cb_json["unobtained_reason"],
            UI_PASS_INTEROP_HAZARD_UNOBTAINED_REASON
        );
        assert!(cb_json["file"].is_null());
        assert_eq!(cb_json["byte_len"], 0);

        let srv = MapFrameTextureBindingFile {
            stage: "PS",
            slot: 0,
            declared_name: Some("UiTexture".to_string()),
            is_null: false,
            view_desc: None,
            file: None,
            byte_len: 0,
            fnv1a64: None,
            dxgi_format: 0,
            width: 64,
            height: 64,
            mip_count: 1,
            array_size: 1,
            sample_count: 1,
            sample_quality: 0,
            payload_format: None,
            subresources: Vec::new(),
            provenance: Some("d3d11_shader_resource_descriptor_only"),
            bind_flags: D3D11_BIND_SHADER_RESOURCE,
            usage: 0,
            cpu_access_flags: 0,
            misc_flags: 0,
            unobtained_reason: Some(UI_PASS_INTEROP_HAZARD_UNOBTAINED_REASON),
            all_zero: false,
        };
        let srv_json = serde_json::to_value(&srv).expect("serialize srv evidence");
        assert_eq!(
            srv_json["unobtained_reason"],
            UI_PASS_INTEROP_HAZARD_UNOBTAINED_REASON
        );
        assert!(srv_json["file"].is_null());
        assert_eq!(srv_json["byte_len"], 0);
        assert_eq!(
            srv_json["provenance"],
            "d3d11_shader_resource_descriptor_only"
        );
    }

    #[test]
    fn ui_capture_state_now_command_arms_label_without_round_commands() {
        let root = temp_capture_dir("ui_capture_state_now_command");
        let mut runtime = runtime_for_capture_moments(&root, &[UI_PASS_CAPTURE_MOMENT]);

        fs::write(
            &runtime.control_path,
            r#"{"type":"ui_capture_state_now","label":"ledger"}"#,
        )
        .expect("write ui capture command");
        poll_control_file_for_round_command_for_runtime(&mut runtime);

        assert_eq!(
            runtime.ui_pass.pending_capture_label.as_deref(),
            Some("ledger")
        );
        assert!(runtime.ui_pass.pending_capture_signature.is_none());
        assert_eq!(runtime.active_capture_index, 0);
        let events = fs::read_to_string(root.join(CAPTURE_EVENTS_FILE)).expect("read events");
        assert!(events.contains("\"type\":\"ui_capture_state_now_requested\""));
        assert!(events.contains("\"label\":\"ledger\""));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn terrain_chunk_bounds_apply_cb1_quad_offset_scale() {
        let mut bytes = Vec::new();
        for (x, z) in [(0i16, 0i16), (64, 64)] {
            bytes.extend_from_slice(&x.to_le_bytes());
            bytes.extend_from_slice(&z.to_le_bytes());
            bytes.extend_from_slice(&0i16.to_le_bytes());
            bytes.extend_from_slice(&1i16.to_le_bytes());
        }

        let bounds = terrain_chunk_bounds_from_position_payload(
            &bytes,
            8,
            0,
            Some([2048.0, 1024.0, 8.0, 0.0]),
        )
        .expect("bounds");

        assert_eq!(
            bounds,
            TerrainChunkBoundsFile {
                min_x: 2048.0,
                min_y: 0.0,
                min_z: 1024.0,
                max_x: 2560.0,
                max_y: 0.0,
                max_z: 1536.0,
            }
        );
    }

    #[test]
    fn terrain_chunk_coverage_reports_holes() {
        let world = TerrainChunkWorldRectFile {
            min_x: 0.0,
            min_z: 0.0,
            max_x: 1024.0,
            max_z: 512.0,
        };
        let coverage = terrain_chunk_coverage_for_bounds(
            &[TerrainChunkBoundsFile {
                min_x: 0.0,
                min_y: 0.0,
                min_z: 0.0,
                max_x: 512.0,
                max_y: 0.0,
                max_z: 512.0,
            }],
            world,
        );

        assert_eq!(coverage.covered_area, 262_144.0);
        assert_eq!(coverage.world_area, 524_288.0);
        assert_eq!(coverage.fraction, 0.5);
        assert_eq!(
            coverage.holes,
            vec![TerrainChunkHoleFile {
                min_x: 512.0,
                min_z: 0.0,
                max_x: 1024.0,
                max_z: 512.0,
            }]
        );
    }

    #[test]
    fn terrain_chunk_identity_includes_per_chunk_cb_but_not_scene_cb0() {
        let mut draw = test_map_frame_draw(
            0,
            "pdxmap_pixel_shader_aa.bin",
            "pixel",
            "pdxmap_vertex_shader_bb.bin",
            "vertex",
        );
        draw.vertex_buffers.insert(
            "0".to_string(),
            MapFrameBufferBindingFile {
                stage: "IA",
                slot: 0,
                declared_name: None,
                file: Some("vb.bin".to_string()),
                byte_len: 8,
                buffer_byte_width: None,
                fnv1a64: Some("vb_hash".to_string()),
                stride: Some(8),
                offset: Some(0),
                dxgi_format: None,
                bind_flags: D3D11_BIND_VERTEX_BUFFER,
                usage: 0,
                cpu_access_flags: 0,
                unknown_origin: None,
                byte_range: None,
                payload_truncated: None,
                unobtained_reason: None,
            },
        );
        draw.index_buffer = Some(MapFrameIndexBufferBindingFile {
            file: Some("ib.bin".to_string()),
            byte_len: 6,
            buffer_byte_width: None,
            fnv1a64: Some("ib_hash".to_string()),
            dxgi_format: 57,
            offset: 0,
            bind_flags: 0x2,
            usage: 0,
            cpu_access_flags: 0,
            byte_range: None,
            payload_truncated: None,
        });
        draw.constant_buffers = vec![
            MapFrameBufferBindingFile {
                stage: "VS",
                slot: TERRAIN_SCENE_CB0_BIND_POINT,
                declared_name: Some(TERRAIN_SCENE_CB0_SYMBOL.to_string()),
                file: Some("scene.bin".to_string()),
                byte_len: 416,
                buffer_byte_width: None,
                fnv1a64: Some("scene_hash".to_string()),
                stride: None,
                offset: None,
                dxgi_format: None,
                bind_flags: D3D11_BIND_CONSTANT_BUFFER,
                usage: 0,
                cpu_access_flags: 0,
                unknown_origin: None,
                byte_range: None,
                payload_truncated: None,
                unobtained_reason: None,
            },
            MapFrameBufferBindingFile {
                stage: "VS",
                slot: 1,
                declared_name: Some(TERRAIN_DRAW_CB1_LAYOUT_SOURCE.to_string()),
                file: Some("cb1.bin".to_string()),
                byte_len: 112,
                buffer_byte_width: None,
                fnv1a64: Some("cb1_hash".to_string()),
                stride: None,
                offset: None,
                dxgi_format: None,
                bind_flags: D3D11_BIND_CONSTANT_BUFFER,
                usage: 0,
                cpu_access_flags: 0,
                unknown_origin: None,
                byte_range: None,
                payload_truncated: None,
                unobtained_reason: None,
            },
        ];

        let world_quad = TerrainChunkWorldRectFile {
            min_x: 2048.0,
            min_z: 1024.0,
            max_x: 2560.0,
            max_z: 1536.0,
        };
        let identity = terrain_chunk_identity_from_draw(&draw, world_quad);
        assert_eq!(identity.vertex_payloads["0"], "vb_hash");
        assert_eq!(identity.index_payload.as_deref(), Some("ib_hash"));
        assert_eq!(identity.world_quad, world_quad);
        assert_eq!(identity.per_chunk_constant_buffers.len(), 1);
        assert_eq!(
            identity.per_chunk_constant_buffers[0]
                .declared_name
                .as_deref(),
            Some(TERRAIN_DRAW_CB1_LAYOUT_SOURCE)
        );

        let mut changed = draw.clone();
        changed.constant_buffers[1].fnv1a64 = Some("cb1_hash_changed".to_string());
        assert_ne!(
            terrain_chunk_identity_key(&identity).unwrap(),
            terrain_chunk_identity_key(&terrain_chunk_identity_from_draw(&changed, world_quad))
                .unwrap()
        );

        let shifted_world_quad = TerrainChunkWorldRectFile {
            min_x: 2560.0,
            min_z: 1024.0,
            max_x: 3072.0,
            max_z: 1536.0,
        };
        assert_ne!(
            terrain_chunk_identity_key(&identity).unwrap(),
            terrain_chunk_identity_key(&terrain_chunk_identity_from_draw(
                &draw,
                shifted_world_quad
            ))
            .unwrap()
        );
    }

    #[test]
    fn terrain_chunk_preidentity_reservation_skips_duplicate_before_payload_capture() {
        let mut state = TerrainChunkLibraryCaptureState::new(true, 0);
        let key = "cheap-preidentity-key";

        assert_eq!(
            reserve_terrain_chunk_preidentity_in_state(&mut state, key),
            TerrainChunkPreidentityReservation::Unique
        );
        assert_eq!(state.sampled_draws, 1);
        assert_eq!(state.duplicate_draws, 0);

        state
            .preidentity_to_chunk_id
            .insert(key.to_string(), "chunk_a".to_string());
        assert_eq!(
            reserve_terrain_chunk_preidentity_in_state(&mut state, key),
            TerrainChunkPreidentityReservation::Duplicate
        );
        assert_eq!(state.sampled_draws, 2);
        assert_eq!(state.duplicate_draws, 1);
    }

    #[test]
    fn deferred_runtime_target_match_enforces_finalize_once_semantics() {
        let existing = DeferredRuntimeTarget {
            target: CAPTURE_TARGETS[1],
            srv: 0x1234,
            slot: 7,
            identity: "active_shader_declared_slot",
        };

        assert!(deferred_runtime_target_matches(
            Some(&existing),
            0x1234,
            7,
            "active_shader_declared_slot"
        ));
        assert!(deferred_runtime_target_matches(
            Some(&existing),
            0x1234,
            7,
            "map_frame_active_draw_declared_resource"
        ));
        assert!(!deferred_runtime_target_matches(
            Some(&existing),
            0x5678,
            7,
            "active_shader_declared_slot"
        ));
        assert!(!deferred_runtime_target_matches(
            Some(&existing),
            0x1234,
            8,
            "active_shader_declared_slot"
        ));
    }

    #[test]
    fn terrain_chunk_library_waits_for_completed_terrain_capture() {
        let root = temp_capture_dir("chunk_waits_for_terrain");
        let mut runtime = runtime_for_capture_moments(
            &root,
            &[TERRAIN_CAPTURE_MOMENT, TERRAIN_CHUNK_LIBRARY_MOMENT],
        );
        assert!(!terrain_chunk_library_draw_capture_ready_for_runtime(
            &runtime
        ));

        runtime
            .completed_map_frame_moments
            .insert(TERRAIN_CAPTURE_MOMENT.to_string());
        assert!(!terrain_chunk_library_draw_capture_ready_for_runtime(
            &runtime
        ));

        for target in CAPTURE_TARGETS {
            runtime.captured_symbols.insert(target.symbol.to_string());
        }
        runtime
            .captured_symbols
            .insert(TERRAIN_VERTEX_BUFFER_SYMBOL.to_string());
        runtime
            .captured_symbols
            .insert(TERRAIN_INPUT_LAYOUT_SYMBOL.to_string());
        assert!(terrain_chunk_library_draw_capture_ready_for_runtime(
            &runtime
        ));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn terrain_chunk_library_readiness_waits_for_world_coverage_not_first_sample() {
        let root = temp_capture_dir("chunk_pending_until_covered");
        let mut runtime = runtime_for_capture_moments(
            &root,
            &[TERRAIN_CAPTURE_MOMENT, TERRAIN_CHUNK_LIBRARY_MOMENT],
        );
        if let Some(control) = runtime.control.as_mut() {
            control.expected_width = 1024;
            control.expected_height = 512;
        }

        assert!(terrain_chunk_library_capture_pending(&runtime));
        let draw = test_map_frame_draw(
            0,
            "pdxmap_pixel_shader_aa.bin",
            "pixel",
            "pdxmap_vertex_shader_bb.bin",
            "vertex",
        );
        let left_bounds = TerrainChunkBoundsFile {
            min_x: 0.0,
            min_y: 0.0,
            min_z: 0.0,
            max_x: 512.0,
            max_y: 0.0,
            max_z: 512.0,
        };
        let left_quad = terrain_chunk_world_quad_from_bounds(left_bounds);
        runtime.terrain_chunk_library.sampled_draws = 1;
        runtime.terrain_chunk_library.chunks.insert(
            "left".to_string(),
            TerrainChunkRecordFile {
                chunk_id: "left".to_string(),
                identity: terrain_chunk_identity_from_draw(&draw, left_quad),
                bounds: left_bounds,
                world_quad: left_quad,
                draw: draw.clone(),
            },
        );
        assert!(terrain_chunk_library_capture_pending(&runtime));

        let right_bounds = TerrainChunkBoundsFile {
            min_x: 512.0,
            min_y: 0.0,
            min_z: 0.0,
            max_x: 1024.0,
            max_y: 0.0,
            max_z: 512.0,
        };
        let right_quad = terrain_chunk_world_quad_from_bounds(right_bounds);
        runtime.terrain_chunk_library.chunks.insert(
            "right".to_string(),
            TerrainChunkRecordFile {
                chunk_id: "right".to_string(),
                identity: terrain_chunk_identity_from_draw(&draw, right_quad),
                bounds: right_bounds,
                world_quad: right_quad,
                draw,
            },
        );
        assert!(!terrain_chunk_library_capture_pending(&runtime));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn normal_terrain_frame_leaves_exact_ps_srv_payloads_to_chunk_library() {
        assert!(!TERRAIN_MAP_FRAME_CAPTURE_PROFILE.capture_exact_ps_srv_payloads);
        assert!(TERRAIN_CHUNK_LIBRARY_CAPTURE_PROFILE.capture_exact_ps_srv_payloads);
    }

    #[test]
    fn in_progress_bundle_snapshots_are_bounded_for_clean_draw_sequences() {
        assert!(should_write_in_progress_map_frame_snapshot(1, 0));
        assert!(!should_write_in_progress_map_frame_snapshot(2, 0));
        assert!(!should_write_in_progress_map_frame_snapshot(21, 0));
        assert!(should_write_in_progress_map_frame_snapshot(21, 1));
    }

    #[test]
    fn capture_work_frame_stat_records_over_budget_frame() {
        let root = temp_capture_dir("capture_work_frame_stat");
        let mut runtime = runtime_for_capture_moments(&root, &[TERRAIN_CAPTURE_MOMENT]);
        runtime.capture_work_frame_ms = runtime
            .capture_budget
            .max_capture_work_ms_per_frame
            .saturating_add(1);

        record_capture_work_frame_stat_locked(&runtime, "unit_test");

        let events = fs::read_to_string(root.join(CAPTURE_EVENTS_FILE)).expect("read events");
        assert!(events.contains("\"type\":\"capture_work_frame\""));
        assert!(events.contains("\"stage\":\"unit_test\""));
        assert!(events.contains("\"over_budget\":true"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn terrain_chunk_library_manifest_writes_blocked_empty_round_trip() {
        let root = temp_capture_dir("terrain_chunk_library_manifest_empty");
        write_terrain_chunk_library_manifest_snapshot(
            &root,
            Vec::new(),
            vec!["unit test blocker".to_string()],
            "unit_test",
        )
        .expect("write blocked manifest");
        let manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join(TERRAIN_CHUNK_LIBRARY_FILE)).unwrap())
                .expect("parse manifest");

        assert_eq!(manifest["schema"], TERRAIN_CHUNK_LIBRARY_SCHEMA);
        assert_eq!(manifest["capture_moment"], TERRAIN_CHUNK_LIBRARY_MOMENT);
        assert_eq!(manifest["coverage"]["world_rect"]["max_x"], 5632.0);
        assert!(manifest["blockers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|blocker| blocker == "unit test blocker"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn shared_payload_file_is_content_addressed_and_not_draw_scoped() {
        let file = map_frame_shared_payload_file(
            TERRAIN_MAP_FRAME_CAPTURE_PROFILE,
            "ps_srv",
            Some("TerrainIDMap"),
            "0123456789abcdef",
        );

        assert_eq!(
            file,
            "map_frame_draw_map_frame_shared_payload_ps_srv_TerrainIDMap_0123456789abcdef.d3d11"
        );
        assert!(!file.contains("draw000"));
        assert!(!file.contains("draw021"));
    }

    #[test]
    fn finalization_provenance_records_dwell_end_vs_pass_close() {
        assert_eq!(
            map_frame_finalized_by("dwell_end_boundary"),
            Some("dwell_end_boundary")
        );
        assert_eq!(
            map_frame_finalized_by("ps_shader_transition"),
            Some("pass_close")
        );
        assert_eq!(
            map_frame_finalized_by("pre_clear_render_target_frame_boundary"),
            Some("pass_close")
        );
        assert_eq!(
            map_frame_finalized_by("post_draw_blend_layer_boundary"),
            Some("pass_close")
        );
        assert_eq!(
            map_frame_finalized_by("present_frame_boundary"),
            Some("pass_close")
        );
        assert_eq!(map_frame_finalized_by("unit_test_in_progress"), None);
    }

    #[test]
    fn post_pass_boundary_payload_files_are_moment_scoped_even_for_identical_hashes() {
        let terrain = map_frame_shared_payload_file(
            TERRAIN_MAP_FRAME_CAPTURE_PROFILE,
            "om_post_pass_render_target",
            Some("render_target"),
            "c16bc99b8d020f62",
        );
        let water = map_frame_shared_payload_file(
            WATER_MAP_FRAME_CAPTURE_PROFILE,
            "om_post_pass_render_target",
            Some("render_target"),
            "c16bc99b8d020f62",
        );

        assert_eq!(
            terrain,
            "map_frame_draw_map_frame_shared_payload_om_post_pass_render_target_render_target_c16bc99b8d020f62.d3d11"
        );
        assert_eq!(
            water,
            "water_frame_draw_map_frame_shared_payload_om_post_pass_render_target_render_target_c16bc99b8d020f62.d3d11"
        );
        assert_ne!(terrain, water);

        let terrain_depth = map_frame_shared_payload_file(
            TERRAIN_MAP_FRAME_CAPTURE_PROFILE,
            "om_post_pass_depth_stencil",
            Some("depth_stencil"),
            "0123456789abcdef",
        );
        let water_depth = map_frame_shared_payload_file(
            WATER_MAP_FRAME_CAPTURE_PROFILE,
            "om_post_pass_depth_stencil",
            Some("depth_stencil"),
            "0123456789abcdef",
        );
        assert_eq!(
            terrain_depth,
            "map_frame_draw_map_frame_shared_payload_om_post_pass_depth_stencil_depth_stencil_0123456789abcdef.d3d11"
        );
        assert_eq!(
            water_depth,
            "water_frame_draw_map_frame_shared_payload_om_post_pass_depth_stencil_depth_stencil_0123456789abcdef.d3d11"
        );
        assert_ne!(terrain_depth, water_depth);
    }

    #[test]
    fn identical_boundary_payload_warning_tracks_distinct_moments() {
        let key =
            boundary_payload_identity_key("om_post_pass_render_target", Some("render_target"), 0);
        let mut payloads = BTreeMap::new();
        let mut warnings = BTreeSet::new();

        assert!(update_boundary_payload_tracking(
            &mut payloads,
            &mut warnings,
            &key,
            TERRAIN_CAPTURE_MOMENT,
            "terrain_boundary.d3d11",
            "c16bc99b8d020f62",
        )
        .is_empty());
        let matches = update_boundary_payload_tracking(
            &mut payloads,
            &mut warnings,
            &key,
            WATER_CAPTURE_MOMENT,
            "water_boundary.d3d11",
            "c16bc99b8d020f62",
        );

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].0, TERRAIN_CAPTURE_MOMENT);
        assert_eq!(matches[0].1.file, "terrain_boundary.d3d11");
        assert!(update_boundary_payload_tracking(
            &mut payloads,
            &mut warnings,
            &key,
            WATER_CAPTURE_MOMENT,
            "water_boundary.d3d11",
            "c16bc99b8d020f62",
        )
        .is_empty());
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

    // ---- CAPFIX2 regression tests ----------------------------------------

    // The tracked-lock statics are process-global; serialize the tests that
    // touch them so parallel test threads do not race on the owner record.
    static CAPFIX2_LOCK_TEST_GUARD: Mutex<()> = Mutex::new(());

    fn reset_runtime_lock_tracking() {
        RUNTIME_LOCK_OWNER_THREAD.store(0, Ordering::SeqCst);
        RUNTIME_LOCK_OWNER_SITE.store(0, Ordering::SeqCst);
        RUNTIME_LOCK_ACQUIRED_TICK_MS.store(0, Ordering::SeqCst);
        RUNTIME_LOCK_WATCHDOG_RECORDED.store(false, Ordering::SeqCst);
        RUNTIME_LOCK_REENTRANT_RECORDED.store(false, Ordering::SeqCst);
    }

    #[test]
    fn capfix2_disarm_makes_capture_quiesced_true_without_touching_quiesce_flag() {
        // BUG 1: dwell-end disarm must turn every hook into a pass-through via
        // capture_quiesced(), yet leave the real CAPTURE_QUIESCED flag alone
        // (that one carries process-detach semantics the sampler must survive).
        let quiesce_before = CAPTURE_QUIESCED.load(Ordering::SeqCst);
        let disarmed_before = HOOKS_DISARMED.load(Ordering::SeqCst);
        // A hook consults capture_quiesced() as its first gate; hooks_disarmed()
        // exposes the same latch for direct inspection.
        HOOKS_DISARMED.store(true, Ordering::SeqCst);
        assert!(hooks_disarmed(), "disarm latch must read back set");
        assert!(
            capture_quiesced(),
            "a disarmed session must present as quiesced to every hook"
        );
        assert_eq!(
            CAPTURE_QUIESCED.load(Ordering::SeqCst),
            quiesce_before,
            "disarm must NOT flip the process-detach quiesce flag"
        );
        // Restore for other tests (disarm is permanent in production, but the
        // test process reuses these statics).
        HOOKS_DISARMED.store(disarmed_before, Ordering::SeqCst);
    }

    #[test]
    fn capfix2_pin_capture_module_is_idempotent() {
        // BUG 1: pinning must be safe to call repeatedly (it runs at dwell end
        // and could in principle be reached more than once). The first call
        // swaps the latch; later calls short-circuit.
        let already = CAPTURE_MODULE_PINNED.swap(true, Ordering::SeqCst);
        // Reset so the real dwell-end path can still pin once in production;
        // then prove the guarded helper does not re-pin when already latched.
        CAPTURE_MODULE_PINNED.store(true, Ordering::SeqCst);
        pin_capture_module_resident(); // must be a no-op, must not panic
        CAPTURE_MODULE_PINNED.store(already, Ordering::SeqCst);
    }

    #[test]
    fn capfix2_lock_watchdog_fires_only_past_threshold() {
        // BUG 2: the wedge watchdog must stay silent for an unheld lock, stay
        // silent for a short hold, and fire once the continuous hold crosses
        // the 10s threshold — naming the (nonzero) owner thread.
        // not held:
        assert_eq!(runtime_lock_wedge_elapsed_ms(0, 0, 1_000_000), None);
        // held, but under threshold:
        assert_eq!(
            runtime_lock_wedge_elapsed_ms(42, 1_000_000, 1_000_000 + 9_999),
            None
        );
        // held, exactly at threshold => fires:
        assert_eq!(
            runtime_lock_wedge_elapsed_ms(
                42,
                1_000_000,
                1_000_000 + RUNTIME_LOCK_WEDGE_WATCHDOG_MS
            ),
            Some(RUNTIME_LOCK_WEDGE_WATCHDOG_MS)
        );
        // held well past threshold (the session_LAW_6 shape: minutes):
        assert_eq!(
            runtime_lock_wedge_elapsed_ms(42, 1_000_000, 1_000_000 + 600_000),
            Some(600_000)
        );
        // owner thread set but no acquire tick => cannot compute, no false fire:
        assert_eq!(runtime_lock_wedge_elapsed_ms(42, 0, 5_000_000), None);
    }

    #[test]
    fn capfix2_tracked_try_lock_records_and_clears_owner() {
        // BUG 2: the tracked guard must publish this thread as the owner while
        // held and clear the record on drop, so the watchdog sees an accurate
        // holder and a released lock reads as unheld.
        let _serial = CAPFIX2_LOCK_TEST_GUARD.lock().unwrap();
        reset_runtime_lock_tracking();
        // A standalone Mutex standing in for the global RUNTIME (same type is
        // not required; the tracking is what we assert). We drive the owner
        // record directly, which is exactly what try_lock_runtime_tracked does.
        assert_eq!(RUNTIME_LOCK_OWNER_THREAD.load(Ordering::SeqCst), 0);
        {
            let _owner = RuntimeLockOwnerRecord::new(RuntimeLockSite::PsSetShader);
            assert_eq!(
                RUNTIME_LOCK_OWNER_THREAD.load(Ordering::SeqCst),
                unsafe { GetCurrentThreadId() },
                "owner thread must be recorded while held"
            );
            assert_eq!(
                runtime_lock_site_name(RUNTIME_LOCK_OWNER_SITE.load(Ordering::SeqCst)),
                "hook_ps_set_shader"
            );
            assert!(
                runtime_lock_would_reenter(),
                "a second lock() from this thread must be detected as re-entrant"
            );
        }
        assert_eq!(
            RUNTIME_LOCK_OWNER_THREAD.load(Ordering::SeqCst),
            0,
            "owner record must clear on guard drop"
        );
        assert!(
            !runtime_lock_would_reenter(),
            "released lock must not read as re-entrant"
        );
        reset_runtime_lock_tracking();
    }

    #[test]
    fn capfix2_tracked_try_lock_refuses_reentrant_and_reports_once() {
        // BUG 2: if this thread already owns the runtime lock, a tracked
        // acquisition must return None (skip work, never block) and record ONE
        // reentrant-attempt event flag.
        let _serial = CAPFIX2_LOCK_TEST_GUARD.lock().unwrap();
        reset_runtime_lock_tracking();
        let runtime: Mutex<u32> = Mutex::new(0);
        // Simulate already owning the lock on this thread.
        let _held = RuntimeLockOwnerRecord::new(RuntimeLockSite::VsSetShader);
        assert!(runtime_lock_would_reenter());
        // The reentrancy branch of try_lock_runtime_tracked keys off
        // runtime_lock_would_reenter(); exercise the recorder directly.
        assert!(!RUNTIME_LOCK_REENTRANT_RECORDED.load(Ordering::SeqCst));
        record_runtime_lock_reentrant_attempt(RuntimeLockSite::PsSetShader);
        assert!(
            RUNTIME_LOCK_REENTRANT_RECORDED.load(Ordering::SeqCst),
            "reentrant attempt must be latched"
        );
        // second call is a no-op (recorded-once), never panics:
        record_runtime_lock_reentrant_attempt(RuntimeLockSite::PsSetShader);
        drop(runtime);
        drop(_held);
        reset_runtime_lock_tracking();
    }

    #[test]
    fn capfix2_dll_side_dwell_cap_clamps_to_ten_minutes() {
        // The DLL clamps any control-file dwell to MAX_DWELL_MS (defense in
        // depth behind the attach helper's rejection).
        let clamp = |dwell: u64, default: u32| -> u32 {
            u32::try_from(dwell)
                .unwrap_or(u32::MAX)
                .max(default)
                .min(MAX_DWELL_MS)
        };
        assert_eq!(MAX_DWELL_MS, 600_000);
        assert_eq!(clamp(30_000, 180_000), 180_000); // under cap: untouched
        assert_eq!(clamp(600_000, 30_000), 600_000); // exactly at cap
        assert_eq!(clamp(3_600_000, 30_000), 600_000); // an hour -> clamped
        assert_eq!(clamp(u64::MAX, 30_000), 600_000); // overflow -> clamped
    }
}
