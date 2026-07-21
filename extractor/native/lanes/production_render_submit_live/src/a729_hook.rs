//! Signature-gated hooks for the pinned a729 particle submit graph.

mod a25_gui_bridge;
mod g2_capture;
mod o05_building_capture;
pub use o05_building_capture::{
    take_scope_for_selected_present as take_o05_scope_for_selected_present,
    RawO05RuntimeRecordObservation, RawO05RuntimeScopeObservation,
};

use crate::{
    canonicalize_packet, fnv1a32, DefinitionIdentity, FinalDrawCall, FinalDrawFamily,
    RawBlendStateCreation, RawBufferSnapshot, RawCaptureLaunchEpoch, RawD3dDrawState,
    RawDepthStencilStateCreation, RawDrawCall, RawDrawFamily, RawDrawIdentity, RawEntitySubmission,
    RawA729CameraCarrier, RawA729CameraEventBatch, RawA729CameraInputEvent,
    RawA729CameraMapPick, RawA729CameraOwnerKind, RawIndexBufferObject, RawInputElement,
    RawInputLayoutCreation, RawMapShaderLookup,
    RawOutputViewKind, RawParticlePacket, RawParticleSubmit, RawPayloadSnapshot, RawPayloadSource,
    RawProcessEpoch, RawRasterizerStateCreation, RawRegisteredRenderTargetKind,
    RawRenderCatalogCapture, RawRenderOrderEvent, RawRenderSubmitCapture, RawRenderTargetBlendDesc,
    RawRenderTargetRegistrationObservation, RawRenderTargetViewSnapshot, RawResourceViewDesc,
    RawSamplerStateCreation, RawShaderBytecodeReflectionObservation, RawShaderCompilerInvocation,
    RawShaderCreation, RawShaderDefine, RawShaderResourceViewSnapshot, RawShaderStage, RawSkinning,
    RawStageSlotObject, RawStaticD3dDrawState, RawStaticDrawCall, RawStaticStreamOutputBuffer,
    RawStencilFaceDesc, RawTexture2dDesc, RawTextureObject, RawTextureResourceSnapshot,
    RawTextureSubresourceSnapshot, RawVertexBufferObject, RawWeatherEntityPair,
    RawOccupationSecondaryTextureObservation, RawSnowMudTextureObservation, RawWeatherEntityRef,
    RawWeatherFactoryAttempt,
    RawWeatherObservation, RawWeatherUpdateOwner, RawWeatherVisibilityObservation,
    RenderOrderBoundary, WeatherEntityRole, WeatherEntityVisibility,
    ENGINE_SHADER_EFFECT_LOOKUP_RVA, ENTITY_DESTROY_RVA, ENTITY_FACTORY_RVA, ENTITY_FINAL_DRAW_RVA,
    ENTITY_RENDER_ROOT_RVA, GLOBAL_RENDER_ENVELOPE_RVA, GPU_INSTANCE_STRIDE,
    HIGH_RESOLUTION_SEED_RVA, INDEXED_INSTANCED_FINAL_DRAW_RVA, INSTANCED_FINAL_DRAW_RVA,
    LUT_BLEND_EFFECT_REGISTRATION_ROOT_RVA, MAP_EFFECT_REGISTRATION_ROOT_RVA,
    MAP_NAME_EFFECT_REGISTRATION_ROOT_RVA, MAP_SCENE_RENDER_ROOT_RVA, PACKET_BYTES,
    PARTICLE_BUCKET_SUBMIT_RVA, PARTICLE_FINAL_DRAW_RVA, PARTICLE_OBJECT_CTOR_RVA,
    PARTICLE_PACKET_BUILDER_RVA, PARTICLE_RENDER_SUBMIT_RVA, PARTICLE_RUNTIME_CREATE_RVA,
    PARTICLE_RUNTIME_DESTROY_RVA, POST_PROCESS_RENDER_ROOT_RVA, PRESENT_RENDER_ROOT_RVA,
    RAILROAD_EFFECT_REGISTRATION_ROOT_RVA, RENDER_TARGET_CLEAR_RVA,
    RESTORE_BLOOM_EFFECT_REGISTRATION_ROOT_RVA, SHADER_PROGRAM_BINDER_RVA,
    CAMERA_MAP_PICK_RVA, CAMERA_VERTICAL_EVENT_PRODUCER_RVA, CAMERA_WHEEL_UPDATE_RVA,
    WEATHER_EDITOR_REFRESH_OWNER_RVA, WEATHER_ENTITY_ACTIVE_SETTER_RVA,
    WEATHER_ENTITY_ALPHA_SETTER_RVA, WEATHER_ENTITY_PLAYBACK_SETTER_RVA,
    WEATHER_NORMAL_UPDATE_OWNER_RVA, WEATHER_VISUAL_PRODUCER_RVA,
};
use core::cell::RefCell;
use core::ffi::c_void;
use minhook::{MinHook, MH_STATUS};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::ffi::CStr;
use std::fmt::Write as _;
use std::mem;
use std::ptr;
use std::slice;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::Duration;
use winapi::um::d3d11::{
    ID3D11Device, ID3D11DeviceContext, D3D11_BLEND_DESC, D3D11_BUFFER_DESC,
    D3D11_DEPTH_STENCIL_DESC, D3D11_DEPTH_STENCIL_VIEW_DESC, D3D11_RASTERIZER_DESC,
    D3D11_RENDER_TARGET_VIEW_DESC, D3D11_SAMPLER_DESC, D3D11_SHADER_RESOURCE_VIEW_DESC,
    D3D11_SUBRESOURCE_DATA, D3D11_TEXTURE1D_DESC, D3D11_TEXTURE2D_DESC, D3D11_TEXTURE3D_DESC,
};
use windows_sys::Win32::Foundation::FILETIME;
use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows_sys::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentProcessId, GetProcessTimes,
};

const MAX_SUBMITS: usize = 4096;
const MAX_PACKETS_PER_SUBMIT: usize = 262_144;
const MAX_ENTITY_DRAWS_PER_SUBMIT: usize = 262_144;
const MAX_LIVE_PARTICLES_PER_PACKET: u32 = 1_048_576;
const MAX_WEATHER_PAIRS: u32 = 65_536;
const MAX_ENTITY_LIST_ROWS: u32 = 1_048_576;
const MAX_MAP_SHADER_LOOKUPS: usize = 65_536;
const MAX_RENDER_ORDER_EVENTS: usize = 1_048_576;
const MAX_D3D_SHADER_RESOURCE_SLOTS: usize = 128;
const MAX_D3D_SAMPLER_SLOTS: usize = 16;
const MAX_D3D_CONSTANT_BUFFER_SLOTS: usize = 14;
const MAX_D3D_VERTEX_BUFFER_SLOTS: usize = 32;
const MAX_D3D_RENDER_TARGET_SLOTS: usize = 8;
const MAX_D3D_STREAM_OUTPUT_SLOTS: usize = 4;
const MAX_D3D_VIEWPORTS_AND_SCISSORS: usize = 16;
const MAX_D3D_CLASS_INSTANCES: usize = 256;
const MAX_D3D_CREATION_ROWS: usize = 262_144;
const MAX_VFS_OPEN_RECEIPTS: usize = 1_048_576;
const MAX_OCCUPATION_SECONDARY_RESOURCES: usize = 16;
const MAX_D3D_SHADER_BYTECODE_BYTES: usize = 64 * 1024 * 1024;
const MAX_D3D_INPUT_ELEMENTS: u32 = 32;
pub const BOOTSTRAP_HOOK_COUNT: usize = 2;
const HARDWARE_D3D_HOOK_COUNT: usize = 19;
const ENGINE_HOOK_COUNT: usize = 47;
pub const STAGED_HOOK_COUNT: usize = HARDWARE_D3D_HOOK_COUNT + ENGINE_HOOK_COUNT;
pub const READY_HOOK_COUNT: usize = BOOTSTRAP_HOOK_COUNT + STAGED_HOOK_COUNT;
pub const STAGED_HOOK_GROUP_COM: u64 = 0x01;
pub const STAGED_HOOK_GROUP_VFS_CONTENT: u64 = 0x02;
pub const STAGED_HOOK_GROUP_RENDER_TARGETS: u64 = 0x04;
pub const STAGED_HOOK_GROUP_PARTICLE: u64 = 0x08;
pub const STAGED_HOOK_GROUP_ENTITY: u64 = 0x10;
pub const STAGED_HOOK_GROUP_SHADER_EFFECTS: u64 = 0x20;
pub const STAGED_HOOK_GROUP_WEATHER_SEED: u64 = 0x40;
pub const STAGED_HOOK_GROUP_SCENE_BOUNDARIES: u64 = 0x80;
pub const STAGED_HOOK_GROUP_SNOW_MUD: u64 = 0x100;
pub const STAGED_HOOK_GROUP_CAMERA_EVENTS: u64 = 0x200;
pub const PRODUCTION_STAGED_HOOK_GROUP_MASK: u64 = 0x3ff;
pub const DIAGNOSTIC_STAGE_MASK_ENV: &str = "BATTLE_PLAN_XN15L_DIAGNOSTIC_STAGE_MASK";
const A729_EXECUTABLE_SHA256: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";
pub const RUNTIME_DLC_MANAGER_GETTER_RVA: usize = 0x0206_0F10;
pub const RUNTIME_ACTIVE_DLC_ENUMERATOR_RVA: usize = 0x0206_3660;
pub const RUNTIME_DLC_STRING_VECTOR_TEARDOWN_RVA: usize = 0x0012_07D0;
const RUNTIME_DLC_VECTOR_ALLOCATOR_RVA: usize = 0x0270_1980;
const MAX_RUNTIME_DLC_NAMES: u32 = 256;
const MAX_RUNTIME_DLC_NAME_BYTES: usize = 4_096;

// Crash-dump transcript `cdb_d3d_import_thunk.txt` proves that taking these
// imported functions as Rust function pointers selects collector-local `ff 25`
// import stubs, not the exports invoked through the game's import table. Resolve
// the loaded system-module exports explicitly and fail closed on the observed
// export prologues instead.
const D3D11_CREATE_DEVICE_EXPORT_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x57,
];
const D3D_COMPILE_EXPORT_SENTINEL: [u8; 7] = [0x4c, 0x8b, 0xdc, 0x48, 0x83, 0xec, 0x78];

const ENTITY_RENDER_LISTS_REGULAR_RVA: usize = 0x0344_FDD0;
const ENTITY_RENDER_LISTS_ALTERNATE_RVA: usize = 0x0344_FE60;
const ENTITY_SHADER_FALLBACK_RVA: usize = 0x035C_1970;
const VFS_OPEN_CONSTRUCTOR_RVA: usize = 0x024C_A2F0;
const ENTITY_NAMED_FACTORY_RVA: usize = 0x0227_8710;
const ENTITY_DEFINITION_LOOKUP_RVA: usize = 0x0227_E5F0;
const WINDOW_BACKBUFFER_RTV_PRODUCER_RVA: usize = 0x020F_DDD0;
const OPAQUE_TEXTURE_TARGET_PRODUCER_RVA: usize = 0x020F_DA10;
const TEXTURE_CACHE_LOAD_ROOT_RVA: usize = 0x023F_17E0;
const SNOW_MUD_PRODUCER_RVA: usize = portable_render_stream::SNOW_MUD_PRODUCER_FUNCTION_RVA as usize;
const SNOW_MUD_UPLOAD_RVA: usize = portable_render_stream::SNOW_MUD_UPLOAD_TARGET_RVA as usize;

const ENVELOPE_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x57,
];
const OBJECT_CTOR_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x57, 0x48, 0x83, 0xec, 0x20, 0x48,
];
const BUCKET_SENTINEL: [u8; 16] = [
    0x40, 0x55, 0x41, 0x56, 0x41, 0x57, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0x41, 0x58, 0x4c, 0x8d,
];
const CREATE_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x6c, 0x24, 0x18, 0x56, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0xf2, 0x48, 0x8b, 0xe9,
];
const DESTROY_SENTINEL: [u8; 16] = [
    0x48, 0x85, 0xc9, 0x0f, 0x84, 0xf4, 0x00, 0x00, 0x00, 0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89,
];
const BUILDER_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x4c, 0x89, 0x4c, 0x24, 0x20, 0x55,
];
const ENTITY_FACTORY_SENTINEL: [u8; 16] = [
    0x40, 0x55, 0x48, 0x8d, 0xac, 0x24, 0xd0, 0xfd, 0xff, 0xff, 0x48, 0x81, 0xec, 0x30, 0x03, 0x00,
];
const ENTITY_NAMED_FACTORY_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x40, 0x41, 0x8b, 0xd9, 0x41, 0x8b, 0xf8,
];
const ENTITY_DEFINITION_LOOKUP_SENTINEL: [u8; 16] = [
    0x48, 0x8b, 0xc4, 0x48, 0x89, 0x58, 0x08, 0x48, 0x89, 0x68, 0x10, 0x48, 0x89, 0x70, 0x18, 0x48,
];
const VFS_OPEN_CONSTRUCTOR_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24, 0x20, 0x48, 0x89, 0x54, 0x24, 0x10, 0x48,
];
const WINDOW_BACKBUFFER_RTV_PRODUCER_SENTINEL: [u8; 16] = [
    0x48, 0x8b, 0xc4, 0x55, 0x48, 0x8d, 0x68, 0xa1, 0x48, 0x81, 0xec, 0x00, 0x01, 0x00, 0x00, 0x48,
];
const OPAQUE_TEXTURE_TARGET_PRODUCER_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24, 0x20, 0x55, 0x41, 0x56, 0x41, 0x57, 0x48,
];
const TEXTURE_CACHE_LOAD_ROOT_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x44, 0x88, 0x4c, 0x24, 0x20, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41,
];
const ENTITY_DESTROY_SENTINEL: [u8; 16] = [
    0x48, 0x85, 0xc9, 0x74, 0x1f, 0x53, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0xd9, 0xe8, 0x3e, 0x41,
];
const ENTITY_RENDER_SENTINEL: [u8; 16] = [
    0x48, 0x8b, 0xc4, 0x48, 0x89, 0x48, 0x08, 0x55, 0x57, 0x41, 0x57, 0x48, 0x8d, 0xa8, 0x48, 0xfd,
];
const WEATHER_SENTINEL: [u8; 16] = [
    0x48, 0x8b, 0xc4, 0x48, 0x89, 0x50, 0x10, 0x55, 0x53, 0x57, 0x41, 0x54, 0x41, 0x56, 0x41, 0x57,
];
const WEATHER_NORMAL_UPDATE_OWNER_SENTINEL: [u8; 16] = [
    0x40, 0x57, 0x41, 0x56, 0x41, 0x57, 0x48, 0x83, 0xec, 0x50, 0x48, 0x89, 0x5c, 0x24, 0x78, 0x4d,
];
const WEATHER_EDITOR_REFRESH_OWNER_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x20, 0x89, 0x54, 0x24, 0x10, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55,
];
const WEATHER_ACTIVE_SETTER_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x57, 0x48, 0x83, 0xec, 0x20, 0x48,
];
const WEATHER_ALPHA_SETTER_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x57,
];
const WEATHER_PLAYBACK_SETTER_SENTINEL: [u8; 16] = WEATHER_ALPHA_SETTER_SENTINEL;
const SEED_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0xf9, 0xe8, 0x9e, 0xc7,
];
const PARTICLE_RENDER_SUBMIT_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x20, 0x4c, 0x89, 0x44, 0x24, 0x18, 0x48, 0x89, 0x54, 0x24, 0x10, 0x48,
];
const PARTICLE_FINAL_DRAW_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x20, 0x41, 0x8b, 0xf8, 0x8b, 0xda, 0x85,
];
const ENTITY_FINAL_DRAW_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x20, 0x41, 0x8b, 0xf9, 0x8b, 0xda, 0x85,
];
const SHADER_PROGRAM_BINDER_SENTINEL: [u8; 16] = [
    0x48, 0x85, 0xd2, 0x74, 0x79, 0x56, 0x48, 0x83, 0xec, 0x20, 0x80, 0xb9, 0x98, 0x02, 0x00, 0x00,
];
const ENGINE_SHADER_EFFECT_LOOKUP_SENTINEL: [u8; 16] = [
    0x48, 0x8b, 0xc4, 0x48, 0x89, 0x58, 0x20, 0x4c, 0x89, 0x40, 0x18, 0x48, 0x89, 0x50, 0x10, 0x55,
];
const MAP_EFFECT_REGISTRATION_ROOT_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x4c, 0x24, 0x08, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41,
];
const RAILROAD_EFFECT_REGISTRATION_ROOT_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x48, 0x89, 0x7c, 0x24, 0x18, 0x4c,
];
const MAP_NAME_EFFECT_REGISTRATION_ROOT_SENTINEL: [u8; 16] = [
    0x48, 0x8b, 0xc4, 0x48, 0x89, 0x58, 0x18, 0x48, 0x89, 0x48, 0x08, 0x55, 0x56, 0x57, 0x41, 0x54,
];
const RESTORE_BLOOM_EFFECT_REGISTRATION_ROOT_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x20, 0x48, 0x89, 0x54, 0x24, 0x10, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41,
];
const LUT_BLEND_EFFECT_REGISTRATION_ROOT_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x48, 0x89, 0x7c, 0x24, 0x18, 0x55,
];
const MAP_SCENE_RENDER_ROOT_SENTINEL: [u8; 16] = [
    0x48, 0x8b, 0xc4, 0x44, 0x88, 0x48, 0x20, 0x48, 0x89, 0x50, 0x10, 0x48, 0x89, 0x48, 0x08, 0x55,
];
const POST_PROCESS_RENDER_ROOT_SENTINEL: [u8; 16] = [
    0x4c, 0x89, 0x4c, 0x24, 0x20, 0x4c, 0x89, 0x44, 0x24, 0x18, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54,
];
const PRESENT_RENDER_ROOT_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x57, 0x48, 0x83, 0xec, 0x30, 0x48,
];
const SNOW_MUD_PRODUCER_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57,
];
const SNOW_MUD_UPLOAD_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x57, 0x48, 0x83, 0xec, 0x60, 0x48,
];
const CAMERA_WHEEL_UPDATE_SENTINEL: [u8; 16] = [
    0x48, 0x8b, 0xc4, 0x4c, 0x89, 0x48, 0x20, 0x55, 0x53, 0x56, 0x48, 0x8d, 0x68, 0x98, 0x48, 0x81,
];
const CAMERA_VERTICAL_EVENT_PRODUCER_SENTINEL: [u8; 16] = [
    0x48, 0x8b, 0xc4, 0x88, 0x50, 0x10, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56,
];
const CAMERA_MAP_PICK_SENTINEL: [u8; 16] = [
    0x4c, 0x8b, 0xdc, 0x45, 0x88, 0x4b, 0x20, 0x49, 0x89, 0x53, 0x10, 0x49, 0x89, 0x4b, 0x08, 0x55,
];
const INSTANCED_FINAL_DRAW_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x30, 0x41, 0x8b, 0xf8, 0x8b, 0xda, 0x85,
];
const RENDER_TARGET_CLEAR_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x57, 0x48, 0x83, 0xec, 0x40, 0x49,
];
const DEFAULT_CLEAR_COLOR_RGBA_F32_BITS: [u32; 4] = [0, 0, 0, 0x3f80_0000];
const DEFAULT_CLEAR_DEPTH_F32_BITS: u32 = 0x3f80_0000;

type EnvelopeFn = unsafe extern "system" fn(usize, usize, u32);
type ObjectCtorFn = unsafe extern "system" fn(usize, usize) -> usize;
type BucketFn = unsafe extern "system" fn(usize, i32);
type CreateFn = unsafe extern "system" fn(usize, usize) -> usize;
type DestroyFn = unsafe extern "system" fn(usize);
type BuilderFn = unsafe extern "system" fn(usize, usize, *mut u32, usize, *mut u32, u32);
type ParticleRenderSubmitFn = unsafe extern "system" fn(usize, usize, usize, usize, f32);
type CameraWheelUpdateFn =
    unsafe extern "system" fn(usize, usize, usize, usize, f32, u8, u8);
type CameraVerticalEventProducerFn = unsafe extern "system" fn(usize, u8);
type CameraMapPickFn = unsafe extern "system" fn(usize, usize, usize, u8) -> usize;
type ParticleFinalDrawFn = unsafe extern "system" fn(usize, i32, u32);
type EntityFactoryFn = unsafe extern "system" fn(usize, u32, u32, u32) -> usize;
type EntityNamedFactoryFn = unsafe extern "system" fn(usize, usize, u32, u32, u32) -> usize;
type EntityDefinitionLookupFn = unsafe extern "system" fn(usize, usize) -> usize;
type VfsOpenConstructorFn = unsafe extern "system" fn(usize, usize, usize, u32) -> usize;
type WindowBackbufferRtvProducerFn = unsafe extern "system" fn(usize) -> i32;
type OpaqueTextureTargetProducerFn = unsafe extern "system" fn(usize, usize) -> u64;
type TextureCacheLoadRootFn = unsafe extern "system" fn(usize, usize, usize, u8) -> u32;
type EntityDestroyFn = unsafe extern "system" fn(usize);
type EntityRenderFn = unsafe extern "system" fn(usize, usize, usize, i32, u8, u8);
type EntityFinalDrawFn = unsafe extern "system" fn(usize, i32, usize, u32);
type InstancedFinalDrawFn = unsafe extern "system" fn(usize, i32, u32);
type RenderTargetClearFn = unsafe extern "system" fn(usize, u64, *const u32);
type ShaderProgramBinderFn = unsafe extern "system" fn(usize, usize);
type EngineShaderEffectLookupFn =
    unsafe extern "system" fn(usize, usize, usize, usize, usize, usize, usize) -> i32;
type MapEffectRegistrationRootFn = unsafe extern "system" fn(usize, usize) -> usize;
type RailroadEffectRegistrationRootFn = unsafe extern "system" fn(usize);
type MapNameEffectRegistrationRootFn = unsafe extern "system" fn(usize, usize) -> usize;
type RestoreBloomEffectRegistrationRootFn = unsafe extern "system" fn(usize, usize);
type LutBlendEffectRegistrationRootFn = unsafe extern "system" fn(usize, usize) -> usize;
type WeatherFn = unsafe extern "system" fn(usize, usize, usize);
type WeatherNormalUpdateOwnerFn = unsafe extern "system" fn(usize, usize, usize, f32, u8);
type WeatherEditorRefreshOwnerFn = unsafe extern "system" fn(usize, u32);
type WeatherActiveSetterFn = unsafe extern "system" fn(usize, u8);
type WeatherAlphaSetterFn = unsafe extern "system" fn(usize, f32);
type WeatherPlaybackSetterFn = unsafe extern "system" fn(usize, f64);
type SeedFn = unsafe extern "system" fn(*mut u32);
type MapSceneRenderFn = unsafe extern "system" fn(usize, usize, i32, u8);
type PostProcessRenderFn = unsafe extern "system" fn(
    usize,
    usize,
    usize,
    usize,
    usize,
    usize,
    usize,
    usize,
    usize,
    usize,
    usize,
    usize,
    usize,
);
type PresentRenderFn = unsafe extern "system" fn(usize);
type SnowMudProducerFn = unsafe extern "system" fn(usize, *mut usize, u8);
type SnowMudUploadFn = unsafe extern "system" fn(usize, *const u8, u32, u32, u32, usize);
type SoSetTargetsFn = unsafe extern "system" fn(usize, u32, *const usize, *const u32);

static ORIGINAL_ENVELOPE: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_OBJECT_CTOR: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_BUCKET: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_CREATE: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_DESTROY: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_BUILDER: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_PARTICLE_RENDER_SUBMIT: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_PARTICLE_FINAL_DRAW: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_ENTITY_FACTORY: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_ENTITY_NAMED_FACTORY: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_ENTITY_DEFINITION_LOOKUP: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_VFS_OPEN_CONSTRUCTOR: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_WINDOW_BACKBUFFER_RTV_PRODUCER: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_OPAQUE_TEXTURE_TARGET_PRODUCER: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_TEXTURE_CACHE_LOAD_ROOT: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_ENTITY_DESTROY: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_ENTITY_RENDER: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_ENTITY_FINAL_DRAW: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_INDEXED_INSTANCED_FINAL_DRAW: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_INSTANCED_FINAL_DRAW: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_RENDER_TARGET_CLEAR: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_SHADER_PROGRAM_BINDER: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_ENGINE_SHADER_EFFECT_LOOKUP: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_MAP_EFFECT_REGISTRATION_ROOT: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_RAILROAD_EFFECT_REGISTRATION_ROOT: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_MAP_NAME_EFFECT_REGISTRATION_ROOT: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_RESTORE_BLOOM_EFFECT_REGISTRATION_ROOT: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_LUT_BLEND_EFFECT_REGISTRATION_ROOT: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_WEATHER: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_WEATHER_NORMAL_UPDATE_OWNER: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_WEATHER_EDITOR_REFRESH_OWNER: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_WEATHER_ACTIVE_SETTER: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_WEATHER_ALPHA_SETTER: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_WEATHER_PLAYBACK_SETTER: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_SEED: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_MAP_SCENE_RENDER_ROOT: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_POST_PROCESS_RENDER_ROOT: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_PRESENT_RENDER_ROOT: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_SNOW_MUD_PRODUCER: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_SNOW_MUD_UPLOAD: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_CAMERA_WHEEL_UPDATE: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_CAMERA_VERTICAL_EVENT_PRODUCER: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_CAMERA_MAP_PICK: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_D3D_CREATE_INPUT_LAYOUT: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_D3D_CREATE_VERTEX_SHADER: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_D3D_CREATE_PIXEL_SHADER: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_D3D_CREATE_GEOMETRY_SHADER: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_D3D_CREATE_HULL_SHADER: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_D3D_CREATE_DOMAIN_SHADER: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_D3D_CREATE_COMPUTE_SHADER: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_D3D_CREATE_BLEND_STATE: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_D3D_CREATE_DEPTH_STENCIL_STATE: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_D3D_CREATE_RASTERIZER_STATE: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_D3D_CREATE_SAMPLER_STATE: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_D3D_CREATE_BUFFER: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_D3D_CREATE_TEXTURE1D: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_D3D_CREATE_TEXTURE2D: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_D3D_CREATE_TEXTURE3D: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_D3D_CREATE_SRV: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_D3D_CREATE_RTV: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_D3D_CREATE_DSV: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_D3D11_CREATE_DEVICE: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_D3D_COMPILE: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_D3D_SO_SET_TARGETS: AtomicUsize = AtomicUsize::new(0);
static COM_DETOUR_TEARDOWN: AtomicBool = AtomicBool::new(false);
static ACTIVE_COM_DETOURS: AtomicUsize = AtomicUsize::new(0);
static COM_DETOUR_QUIESCENCE: OnceLock<(Mutex<()>, Condvar)> = OnceLock::new();
static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);
static INSTALLED: AtomicBool = AtomicBool::new(false);
static NEXT_EFFECT_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_SUBMIT_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_ENTITY_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_WEATHER_OBSERVATION_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_WEATHER_UPDATE_INDEX: AtomicU64 = AtomicU64::new(1);
static NEXT_MAP_SHADER_LOOKUP_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static NEXT_RENDER_ORDER_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static NEXT_RENDER_FRAME_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static NEXT_PRESENT_GLOBAL_ORDER: AtomicU32 = AtomicU32::new(0);
static NEXT_SNOW_MUD_PRODUCER_ORDINAL: AtomicU64 = AtomicU64::new(1);
static NEXT_OCCUPATION_SECONDARY_UPLOAD_ORDER: AtomicU32 = AtomicU32::new(1);
static NEXT_CATALOG_OBSERVATION_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static PENDING_STAGE_FIRST_CREATION: AtomicU64 = AtomicU64::new(0);
static PRESENT_GAME_STATE_ROOT_SLOT: AtomicUsize = AtomicUsize::new(0);
static STATE: OnceLock<Mutex<HookState>> = OnceLock::new();
static RENDER_SUBMIT_PUBLICATION: OnceLock<Condvar> = OnceLock::new();
static BOOTSTRAP_HOOK_TARGETS: OnceLock<Mutex<Vec<usize>>> = OnceLock::new();
static STAGED_HOOK_TARGETS: OnceLock<Mutex<Vec<usize>>> = OnceLock::new();
static HOOK_STAGE: OnceLock<Mutex<HookStage>> = OnceLock::new();
static EARLY_CAPTURE_EPOCH: OnceLock<Mutex<EarlyCaptureEpochState>> = OnceLock::new();
static RUNTIME_DLC_ROSTER: OnceLock<Mutex<Option<RuntimeDlcRosterObservation>>> = OnceLock::new();
static HOOK_LIFECYCLE_LOCK: Mutex<()> = Mutex::new(());

struct ActiveComDetour;

impl ActiveComDetour {
    fn enter() -> Option<Self> {
        if COM_DETOUR_TEARDOWN.load(Ordering::Acquire) {
            return None;
        }
        ACTIVE_COM_DETOURS.fetch_add(1, Ordering::AcqRel);
        if COM_DETOUR_TEARDOWN.load(Ordering::Acquire) {
            release_active_com_detour();
            return None;
        }
        Some(Self)
    }
}

impl Drop for ActiveComDetour {
    fn drop(&mut self) {
        release_active_com_detour();
    }
}

fn release_active_com_detour() {
    let prior = ACTIVE_COM_DETOURS.fetch_sub(1, Ordering::AcqRel);
    debug_assert_ne!(prior, 0, "COM detour active count underflow");
    if prior == 1 {
        COM_DETOUR_QUIESCENCE
            .get_or_init(|| (Mutex::new(()), Condvar::new()))
            .1
            .notify_all();
    }
}

fn begin_com_detour_teardown() {
    COM_DETOUR_TEARDOWN.store(true, Ordering::Release);
}

fn wait_for_com_detour_quiescence() -> Result<(), String> {
    let (lock, wake) = COM_DETOUR_QUIESCENCE.get_or_init(|| (Mutex::new(()), Condvar::new()));
    let mut guard = lock
        .lock()
        .map_err(|_| "COM detour quiescence state poisoned")?;
    while ACTIVE_COM_DETOURS.load(Ordering::Acquire) != 0 {
        guard = wake
            .wait(guard)
            .map_err(|_| "COM detour quiescence state poisoned")?;
    }
    Ok(())
}

fn finish_com_detour_teardown() {
    COM_DETOUR_TEARDOWN.store(false, Ordering::Release);
}

fn is_com_original(original: &'static AtomicUsize) -> bool {
    com_originals()
        .into_iter()
        .any(|candidate| ptr::eq(candidate, original))
}

fn com_originals() -> [&'static AtomicUsize; HARDWARE_D3D_HOOK_COUNT] {
    [
        &ORIGINAL_D3D_CREATE_INPUT_LAYOUT,
        &ORIGINAL_D3D_CREATE_VERTEX_SHADER,
        &ORIGINAL_D3D_CREATE_PIXEL_SHADER,
        &ORIGINAL_D3D_CREATE_GEOMETRY_SHADER,
        &ORIGINAL_D3D_CREATE_HULL_SHADER,
        &ORIGINAL_D3D_CREATE_DOMAIN_SHADER,
        &ORIGINAL_D3D_CREATE_COMPUTE_SHADER,
        &ORIGINAL_D3D_CREATE_BLEND_STATE,
        &ORIGINAL_D3D_CREATE_DEPTH_STENCIL_STATE,
        &ORIGINAL_D3D_CREATE_RASTERIZER_STATE,
        &ORIGINAL_D3D_CREATE_SAMPLER_STATE,
        &ORIGINAL_D3D_CREATE_BUFFER,
        &ORIGINAL_D3D_CREATE_TEXTURE1D,
        &ORIGINAL_D3D_CREATE_TEXTURE2D,
        &ORIGINAL_D3D_CREATE_TEXTURE3D,
        &ORIGINAL_D3D_CREATE_SRV,
        &ORIGINAL_D3D_CREATE_RTV,
        &ORIGINAL_D3D_CREATE_DSV,
        &ORIGINAL_D3D_SO_SET_TARGETS,
    ]
}

thread_local! {
    static CURRENT_SUBMIT: RefCell<Option<PendingSubmit>> = const { RefCell::new(None) };
    static CURRENT_PARTICLE_DRAW_RANGE: RefCell<Option<DrawRange>> = const { RefCell::new(None) };
    static CURRENT_ENTITY_DRAW_RANGE: RefCell<Option<DrawRange>> = const { RefCell::new(None) };
    static CURRENT_RENDER_PROGRAM: RefCell<Option<CurrentRenderProgram>> = const { RefCell::new(None) };
    /// Last program bound per native render context. Unlike the fresh runtime
    /// join above, this cache is retained across consecutive static draws
    /// because the engine binder legitimately suppresses redundant D3D state.
    static BOUND_RENDER_PROGRAMS: RefCell<HashMap<usize, usize>> = RefCell::new(HashMap::new());
    static CURRENT_MAP_SCENE_PHASE: RefCell<bool> = const { RefCell::new(false) };
    static CURRENT_POST_PROCESS_PHASE: RefCell<bool> = const { RefCell::new(false) };
    static CURRENT_MAP_CONTENT_REGISTRATION_ROOT: RefCell<Option<u32>> = const { RefCell::new(None) };
    static TEXTURE_LOAD_STACK: RefCell<Vec<PendingTextureLoad>> = const { RefCell::new(Vec::new()) };
    static ENTITY_FACTORY_STACK: RefCell<Vec<PendingEntityFactory>> = const { RefCell::new(Vec::new()) };
    static CURRENT_WEATHER: RefCell<Option<PendingWeather>> = const { RefCell::new(None) };
    static CURRENT_WEATHER_OWNER: RefCell<Option<WeatherUpdateOwner>> = const { RefCell::new(None) };
    static CURRENT_SNOW_MUD_PRODUCER: RefCell<Option<PendingSnowMudProducer>> = const { RefCell::new(None) };
    static CURRENT_CAMERA_EVENT_PRODUCER: RefCell<Option<PendingCameraEventProducer>> = const { RefCell::new(None) };
}

#[derive(Clone, Copy)]
struct PendingSnowMudProducer {
    resource_before: usize,
    upload_expected: bool,
    upload_count: u8,
    rejected: bool,
}

struct PendingCameraEventProducer {
    camera_root: usize,
    batch: RawA729CameraEventBatch,
    event_nodes: Vec<usize>,
    rejected: bool,
}

#[derive(Clone, Debug)]
struct PendingTextureLoad {
    entry_sequence: u64,
    logical_path: String,
    nested_open_sequences: Vec<u64>,
    nested_texture_creations: Vec<(u64, usize)>,
    nested_srv_creations: Vec<(u64, usize, usize)>,
}

#[derive(Clone, Copy)]
struct CurrentRenderProgram {
    render_context_ptr: usize,
    shader_program_ptr: usize,
}

#[derive(Clone)]
struct RuntimeIdentity {
    stable_id: u64,
    definition: DefinitionIdentity,
    owner: usize,
    render_list_class: u32,
    render_subbucket: u16,
    scale_f32_bits: u32,
    parent_entity_instance_id: Option<u64>,
}

#[derive(Clone)]
struct EntityIdentity {
    stable_id: u64,
    definition_ptr: usize,
    registration_token: Option<String>,
    ordered_effect_instance_ids: Vec<u64>,
}

struct PendingEntityFactory {
    stable_id: u64,
    definition_ptr: usize,
    ordered_effect_instance_ids: Vec<u64>,
}

struct PendingWeather {
    producer: usize,
    update_index: u64,
    owner: RawWeatherUpdateOwner,
    strategic_region_id: u32,
    previous_code: u8,
    view_distance_f32_bits: u32,
    selection_seed_words: Option<[u32; 2]>,
    factory_attempts: Vec<RawWeatherFactoryAttempt>,
    setter_writes: HashMap<usize, PendingWeatherSetterWrites>,
    rejected: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct WeatherUpdateOwner {
    update_index: u64,
    owner: RawWeatherUpdateOwner,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct PendingWeatherSetterWrites {
    active_count: u8,
    active_value: bool,
    alpha_count: u8,
    alpha_f32_bits: u32,
    playback_count: u8,
    playback_f32_bits: u32,
}

struct PendingSubmit {
    submit_index: u64,
    camera_carrier_ordinal: u8,
    camera_carrier: Option<RawA729CameraCarrier>,
    next_builder_ordinal: u32,
    next_global_draw_order: u32,
    next_entity_pass_ordinal: u32,
    packets: Vec<RawParticlePacket>,
    entity_submissions: Vec<RawEntitySubmission>,
    draw_states: Vec<RawD3dDrawState>,
    buffer_snapshots: Vec<RawBufferSnapshot>,
    texture_resource_snapshots: Vec<RawTextureResourceSnapshot>,
    shader_resource_view_snapshots: Vec<RawShaderResourceViewSnapshot>,
    render_target_view_snapshots: Vec<RawRenderTargetViewSnapshot>,
    rejected: bool,
}

struct CapturedRuntimeDraw {
    state: RawD3dDrawState,
    buffer_snapshots: Vec<RawBufferSnapshot>,
    texture_resource_snapshots: Vec<RawTextureResourceSnapshot>,
    shader_resource_view_snapshots: Vec<RawShaderResourceViewSnapshot>,
    render_target_view_snapshots: Vec<RawRenderTargetViewSnapshot>,
}

/// Identity-free D3D state shared by runtime and static capture. Semantic
/// producer identity is attached only after this observation succeeds.
struct NeutralD3dDrawState {
    final_draw_sequence: u64,
    render_context_ptr: usize,
    device_context_ptr: usize,
    shader_program_ptr: usize,
    vertex_shader_ptr: usize,
    pixel_shader_ptr: usize,
    geometry_shader_ptr: Option<usize>,
    hull_shader_ptr: Option<usize>,
    domain_shader_ptr: Option<usize>,
    compute_shader_ptr: Option<usize>,
    dynamic_shader_linkage_present: bool,
    input_layout_ptr: Option<usize>,
    blend_state_ptr: Option<usize>,
    blend_factor_rgba_f32_bits: [u32; 4],
    sample_mask_u32: u32,
    depth_stencil_state_ptr: Option<usize>,
    stencil_reference_u32: u32,
    rasterizer_state_ptr: Option<usize>,
    primitive_topology_u32: u32,
    ordered_texture_objects: Vec<RawTextureObject>,
    ordered_sampler_objects: Vec<RawStageSlotObject>,
    ordered_constant_buffer_objects: Vec<RawStageSlotObject>,
    ordered_vertex_buffer_objects: Vec<RawVertexBufferObject>,
    index_buffer: Option<RawIndexBufferObject>,
    ordered_stream_output_buffer_ptrs: Vec<usize>,
    ordered_color_target_view_ptrs: Vec<usize>,
    depth_stencil_view_ptr: Option<usize>,
    ordered_viewports_f32_bits: Vec<[u32; 6]>,
    ordered_scissor_rects_i32: Vec<[i32; 4]>,
    draw_call: RawDrawCall,
}

struct CapturedD3dCore {
    state: NeutralD3dDrawState,
    buffer_snapshots: Vec<RawBufferSnapshot>,
    texture_resource_snapshots: Vec<RawTextureResourceSnapshot>,
    shader_resource_view_snapshots: Vec<RawShaderResourceViewSnapshot>,
    render_target_view_snapshots: Vec<RawRenderTargetViewSnapshot>,
}

#[derive(Clone, Copy)]
struct PendingRenderFrame {
    frame_sequence: u64,
    submit_index: u64,
}

#[derive(Clone, Copy)]
struct PresentationQpcReceipt {
    global_order: u32,
    ticks: u64,
    frequency_hz: u64,
    process_start_nonce: u64,
    hook_epoch_nonce: u64,
    game_hour_raw: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawMapShaderProgramJoin {
    pub lookup: RawMapShaderLookup,
    /// Exact successful VFS open nested before this lookup returned. The
    /// collector joins this sequence to its retained open receipt; no path
    /// search or filename fallback is permitted later.
    pub source_open_observation_sequence: u64,
    /// Transient address of `shader_cache + 0x20 + program_index * 0x38`.
    /// This is discarded after exact correlation and never serialized.
    pub shader_program_ptr: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawStaticD3dCapture {
    pub state: RawStaticD3dDrawState,
    pub buffer_snapshots: Vec<RawBufferSnapshot>,
    pub texture_resource_snapshots: Vec<RawTextureResourceSnapshot>,
    pub shader_resource_view_snapshots: Vec<RawShaderResourceViewSnapshot>,
    pub render_target_view_snapshots: Vec<RawRenderTargetViewSnapshot>,
}

/// Exact successful a729 archive-file open. Pointer fields are extractor-local
/// joins only; the collector hydrates payload SHA/count from the authenticated
/// effective-VFS winner and H4CT admission before authority construction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawVfsOpenReceipt {
    pub observation_sequence: u64,
    pub ordered_logical_path_tokens: Vec<String>,
    pub archive_object_ptr: usize,
    pub stream_handle: usize,
    pub data_ptr: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawTextureVfsResourceJoin {
    pub root_entry_sequence: u64,
    pub root_exit_sequence: u64,
    pub source_open_observation_sequence: u64,
    pub resource_creation_sequence: u64,
    pub shader_resource_view_creation_sequence: u64,
    pub logical_path: String,
    pub runtime_texture_index_u32: u32,
    pub resource_ptr: usize,
    pub shader_resource_view_ptr: usize,
}

#[derive(Clone, Copy)]
struct DrawRange {
    start: usize,
    end: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ParticleTypeSubsystemTables {
    type_definition_ptr: usize,
    program_indices_ptr: usize,
    atlas_rows_ptr: usize,
    subsystem_count: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ParticleSubsystemRow {
    shader_program_index_u32: u32,
    texture_runtime_index_u32: u32,
    atlas_half_pixel_width_f32_bits: u32,
    atlas_half_pixel_height_f32_bits: u32,
    atlas_columns_f32_bits: u32,
    atlas_rows_f32_bits: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct EngineEffectIdentity {
    program_index_u32: u32,
    effect_token: String,
}

#[derive(Default)]
struct HookState {
    runtimes: HashMap<usize, RuntimeIdentity>,
    entities: HashMap<usize, EntityIdentity>,
    /// Authenticated case-sensitive named-registry lookup result. Weather
    /// later creates through the definition-backed factory directly, so this
    /// process-local receipt is the only admitted pointer-to-token bridge.
    entity_definition_tokens: HashMap<usize, String>,
    completed: VecDeque<RawParticleSubmit>,
    completed_catalogs: VecDeque<RawRenderCatalogCapture>,
    shader_creation_observations: Vec<ObservedShaderCreation>,
    compiler_outputs: Vec<ObservedCompilerOutput>,
    input_layout_creations: Vec<RawInputLayoutCreation>,
    blend_state_creations: Vec<RawBlendStateCreation>,
    depth_stencil_state_creations: Vec<RawDepthStencilStateCreation>,
    rasterizer_state_creations: Vec<RawRasterizerStateCreation>,
    sampler_state_creations: Vec<RawSamplerStateCreation>,
    created_d3d_objects: HashMap<usize, CreatedD3dObject>,
    weather_changes: VecDeque<RawWeatherObservation>,
    /// A compatibility drain may request the complete transition history.
    /// Track bounded-queue loss so that API fails instead of silently
    /// returning a truncated history; selected-Present snapshots use the
    /// independent per-region maps below.
    weather_change_overflows: u64,
    weather_visibility_updates: VecDeque<RawWeatherVisibilityObservation>,
    latest_weather_observation_by_region: BTreeMap<u32, RawWeatherObservation>,
    latest_weather_visibility_by_region: BTreeMap<u32, RawWeatherVisibilityObservation>,
    /// Exact region-record count from the authenticated CGameState weather
    /// manager. A selected gameplay Present is incomplete until every region
    /// has supplied its first normal-update membership row.
    expected_weather_region_count: Option<u32>,
    /// Last airborne code observed at the authenticated visual producer for
    /// every strategic region reached since hook installation. This is a
    /// discriminator only: it never substitutes for the ordered membership,
    /// lifecycle, visibility, or submitted-packet carrier.
    weather_runtime_codes: HashMap<u32, u8>,
    map_shader_lookups: VecDeque<RawMapShaderLookup>,
    map_shader_program_joins: VecDeque<RawMapShaderProgramJoin>,
    /// Exact successful engine effect token keyed by the process-local
    /// shader-program entry returned by the same 0x1423f4ec0 lookup. This is
    /// extractor-local and is joined to a particle only at its final draw.
    engine_effect_tokens_by_program: HashMap<usize, EngineEffectIdentity>,
    latest_vfs_open_sequence_by_path: HashMap<String, u64>,
    texture_vfs_resource_joins: HashMap<usize, RawTextureVfsResourceJoin>,
    texture_cache_associations: HashMap<(String, u32), usize>,
    registered_render_targets: HashMap<usize, RawRenderTargetRegistrationObservation>,
    render_target_use_observations: VecDeque<RawRenderTargetRegistrationObservation>,
    swapchain_backbuffer_view_ptr: Option<usize>,
    swapchain_recreate_generation: u64,
    render_order_events: VecDeque<RawRenderOrderEvent>,
    clear_output_view_snapshots: VecDeque<RawRenderTargetViewSnapshot>,
    stream_output_bindings: HashMap<usize, Vec<RawStaticStreamOutputBuffer>>,
    static_d3d_captures: VecDeque<RawStaticD3dCapture>,
    vfs_open_receipts: VecDeque<RawVfsOpenReceipt>,
    pending_render_frame: Option<PendingRenderFrame>,
    completed_render_frames: VecDeque<RawRenderFrameCapture>,
    latest_snow_mud_observation: Option<Arc<RawSnowMudTextureObservation>>,
    /// Latest exact nontransparent 2816x1024 BGRA upload per process-local
    /// resource. These remain candidates until draw-time bytes and reflected
    /// ProvinceSecondaryColorMap consumers independently identify the resource.
    latest_occupation_secondary_by_resource:
        BTreeMap<usize, Arc<RawOccupationSecondaryTextureObservation>>,
    pending_camera_event_batches: HashMap<usize, VecDeque<RawA729CameraEventBatch>>,
    runtime_publication_generation: u64,
    /// Exact collector cutoff captured while this state lock was held. It is
    /// retained only while a cutoff-eligible Present is waiting for its
    /// runtime/catalog publication mate.
    latched_render_hour: Option<SealedRenderHour>,
    sealed_render_hour: Option<SealedRenderHour>,
    sealed_purge_receipt_count: u128,
    sealed_purge_receipt_count_overflowed: bool,
    sealed_purge_rolling_sha256: [u8; 32],
    /// Compact proofs for completed same-hour Presents retired online before
    /// the collector seals that hour. Only the latest complete joined
    /// candidate remains in the full frame/runtime/catalog queues.
    online_candidate_proofs: BTreeMap<i32, OnlineCandidateProof>,
    last_presentation_qpc_ticks: Option<u64>,
    rejected_observations: u64,
    evicted_submits: u64,
    duplicate_runtime_ids: u64,
    nested_envelopes: u64,
    duplicate_entity_ids: u64,
    nested_weather_calls: u64,
    rejected_map_shader_lookups: u64,
    map_shader_lookup_overflows: u64,
    render_order_event_overflows: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SealedRenderHour {
    game_hour_raw: i32,
    cutoff_qpc_ticks: u64,
    cutoff_qpc_frequency_hz: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct OnlineCandidateProof {
    purged_candidate_count: u32,
    candidate_rolling_sha256: [u8; 32],
    purged_receipt_count: u32,
    consumed_rolling_sha256: [u8; 32],
    game_hour_raw: Option<i32>,
    presentation_qpc_frequency_hz: Option<u64>,
    process_start_nonce: Option<u64>,
    hook_epoch_nonce: Option<u64>,
    last_purged_order: Option<(u64, u32, u64)>,
}

#[derive(Clone)]
struct ObservedShaderCreation {
    creation_sequence: u64,
    shader_ptr: usize,
    stage: RawShaderStage,
    bytecode: Vec<u8>,
    compiler: Option<ObservedCompilerOutput>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ObservedCompilerOutput {
    /// Exact `pSourceName` supplied to D3DCompile. It remains an observed
    /// token until the effective-VFS authority independently admits the
    /// matching payload; it is never treated as a vanilla default path.
    source_name: String,
    source_payload_sha256: String,
    source_payload_bytes: u64,
    compiler_identity: String,
    target_profile: String,
    compiler_flags_u64: u64,
    entry_token: String,
    ordered_defines: Vec<(String, String)>,
    bytecode: Vec<u8>,
}

#[repr(C)]
struct D3dShaderMacro {
    name: *const i8,
    definition: *const i8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CreatedD3dObjectKind {
    Buffer,
    Texture1D,
    Texture2D,
    Texture3D,
    ShaderResourceView,
    RenderTargetView,
    DepthStencilView,
}

#[derive(Clone, Copy)]
struct CreatedD3dObject {
    creation_sequence: u64,
    kind: CreatedD3dObjectKind,
}

#[repr(C)]
struct D3dInputElementDesc {
    semantic_name: *const i8,
    semantic_index: u32,
    format: u32,
    input_slot: u32,
    aligned_byte_offset: u32,
    input_slot_class: u32,
    instance_data_step_rate: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct EarlyCaptureEpochState {
    process: Option<RawProcessEpoch>,
    observer_installed_qpc: u64,
    qpc_frequency_hz: u64,
    hook_ready_sequence: u64,
    d3d_device_created_qpc: u64,
    game_device_count: u64,
    first_creation_sequence: u64,
    hook_epoch_nonce: u64,
    hook_set_receipt_sha256: String,
    capture_receipt_sha256: Option<String>,
    excluded_probe_device_ptr: usize,
    staged_hook_generation: u64,
    staged_hook_count: u32,
    staged_hook_set_sha256: String,
    diagnostic_stage_probe: bool,
    staged_hook_group_mask: u64,
    game_device_ptr: usize,
    immediate_context_ptr: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EarlyCaptureReadiness {
    pub process: RawProcessEpoch,
    pub observer_installed_qpc: u64,
    pub qpc_frequency_hz: u64,
    pub hook_ready_sequence: u64,
    pub d3d_device_created_qpc: Option<u64>,
    pub game_device_count: u64,
    pub first_creation_sequence: Option<u64>,
    pub hook_epoch_nonce: u64,
    pub hook_set_receipt_sha256: String,
    pub creation_hook_count: u32,
    pub excluded_probe_device_ptr: usize,
    pub staged_hook_generation: u64,
    pub staged_hook_count: u32,
    pub staged_hook_set_sha256: String,
    pub diagnostic_stage_probe: bool,
    pub staged_hook_group_mask: u64,
    pub game_device_ptr: usize,
    pub immediate_context_ptr: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EarlyCaptureLivenessSample {
    pub readiness: EarlyCaptureReadiness,
    pub sampled_qpc: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeDlcRosterObservation {
    pub process_id: u32,
    pub process_start_nonce: u64,
    pub hook_epoch_nonce: u64,
    pub staged_hook_generation: u64,
    pub game_device_ptr: usize,
    pub manager_getter_rva: usize,
    pub active_dlc_enumerator_rva: usize,
    pub string_vector_teardown_rva: usize,
    pub raw_names: Vec<String>,
    pub roster_count: u32,
    pub raw_name_roster_sha256: String,
    pub sample_count: u32,
    pub double_stable: bool,
}

#[repr(C)]
struct RuntimeDlcStringVector {
    data: usize,
    capacity: u32,
    count: u32,
    allocator: usize,
    allocator_aux: usize,
}

type HookSpec = (usize, *mut c_void, &'static AtomicUsize);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum HookStage {
    #[default]
    Bootstrap,
    Installing,
    Ready,
    Failed,
}

impl HookStage {
    fn begin_staging(&mut self) -> Result<(), &'static str> {
        match *self {
            Self::Bootstrap => {
                *self = Self::Installing;
                Ok(())
            }
            Self::Installing => Err("recursive game-device hook staging"),
            Self::Ready => Err("game-device hook staging already completed"),
            Self::Failed => Err("game-device hook staging previously failed"),
        }
    }

    fn finish_staging(&mut self, succeeded: bool) {
        debug_assert_eq!(*self, Self::Installing);
        *self = if succeeded { Self::Ready } else { Self::Failed };
    }

    fn admits_first_creation(self) -> bool {
        self == Self::Ready
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct StagedHookSelection {
    diagnostic_stage_probe: bool,
    group_mask: u64,
}

fn parse_diagnostic_stage_mask(raw: Option<&str>) -> Result<StagedHookSelection, String> {
    let Some(raw) = raw else {
        return Ok(StagedHookSelection {
            diagnostic_stage_probe: false,
            group_mask: PRODUCTION_STAGED_HOOK_GROUP_MASK,
        });
    };
    let trimmed = raw.trim();
    let digits = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
        .unwrap_or(trimmed);
    let group_mask = u64::from_str_radix(digits, 16)
        .map_err(|_| "diagnostic staged-hook mask is not hexadecimal")?;
    if group_mask & !PRODUCTION_STAGED_HOOK_GROUP_MASK != 0
        || group_mask & STAGED_HOOK_GROUP_COM == 0
        || group_mask == PRODUCTION_STAGED_HOOK_GROUP_MASK
    {
        return Err(
            "diagnostic staged-hook mask must be a strict production subset containing COM".into(),
        );
    }
    Ok(StagedHookSelection {
        diagnostic_stage_probe: true,
        group_mask,
    })
}

fn staged_hook_selection() -> Result<StagedHookSelection, String> {
    match std::env::var(DIAGNOSTIC_STAGE_MASK_ENV) {
        Ok(raw) => parse_diagnostic_stage_mask(Some(&raw)),
        Err(std::env::VarError::NotPresent) => parse_diagnostic_stage_mask(None),
        Err(std::env::VarError::NotUnicode(_)) => {
            Err("diagnostic staged-hook mask is not Unicode".into())
        }
    }
}

fn bootstrap_hook_specs() -> Result<[HookSpec; BOOTSTRAP_HOOK_COUNT], String> {
    let d3d11_create_device = resolve_loaded_export(
        "d3d11.dll",
        b"D3D11CreateDevice\0",
        &D3D11_CREATE_DEVICE_EXPORT_SENTINEL,
        "d3d11!D3D11CreateDevice",
    )?;
    let d3d_compile = resolve_loaded_export(
        "d3dcompiler_47.dll",
        b"D3DCompile\0",
        &D3D_COMPILE_EXPORT_SENTINEL,
        "d3dcompiler_47!D3DCompile",
    )?;
    Ok([
        (
            d3d11_create_device,
            hook_d3d11_create_device as *mut c_void,
            &ORIGINAL_D3D11_CREATE_DEVICE,
        ),
        (
            d3d_compile,
            hook_d3d_compile as *mut c_void,
            &ORIGINAL_D3D_COMPILE,
        ),
    ])
}

fn resolve_loaded_export(
    module_name: &str,
    export_name_nul: &[u8],
    expected_sentinel: &[u8],
    label: &str,
) -> Result<usize, String> {
    if export_name_nul.last() != Some(&0)
        || export_name_nul[..export_name_nul.len() - 1].contains(&0)
    {
        return Err(format!(
            "{label} export name is not one terminal-NUL string"
        ));
    }
    let module_name_wide = module_name
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let module = unsafe { GetModuleHandleW(module_name_wide.as_ptr()) };
    if module.is_null() {
        return Err(format!("{label} module {module_name} is not loaded"));
    }
    let module_base = module as usize;
    let image_size = loaded_pe_image_size(module_base)
        .ok_or_else(|| format!("{label} loaded module has no valid PE image bounds"))?;
    let export = unsafe { GetProcAddress(module, export_name_nul.as_ptr()) }
        .map(|function| function as usize)
        .unwrap_or(0);
    if export == 0 {
        return Err(format!("{label} export is absent from loaded module"));
    }
    if !export_target_within_image(module_base, image_size, export, expected_sentinel.len()) {
        return Err(format!(
            "{label} export {export:#x} is outside loaded module image {module_base:#x}+{image_size:#x}"
        ));
    }
    let mut actual = vec![0u8; expected_sentinel.len().max(7)];
    read_exact(export, &mut actual)
        .ok_or_else(|| format!("could not read {label} export sentinel"))?;
    validate_export_sentinel_bytes(&actual, expected_sentinel, label)?;
    Ok(export)
}

fn loaded_pe_image_size(module_base: usize) -> Option<usize> {
    if read_u16(module_base)? != 0x5a4d {
        return None;
    }
    let nt_offset = read_u32(module_base.checked_add(0x3c)?)? as usize;
    if nt_offset < 0x40 || nt_offset > 16 * 1024 * 1024 {
        return None;
    }
    let nt_headers = module_base.checked_add(nt_offset)?;
    if read_u32(nt_headers)? != 0x0000_4550 {
        return None;
    }
    let optional_header = nt_headers.checked_add(24)?;
    if read_u16(optional_header)? != 0x20b {
        return None;
    }
    let image_size = read_u32(optional_header.checked_add(56)?)? as usize;
    (image_size >= 0x1000 && image_size <= 1024 * 1024 * 1024).then_some(image_size)
}

fn export_target_within_image(
    module_base: usize,
    image_size: usize,
    export: usize,
    sentinel_len: usize,
) -> bool {
    if module_base == 0 || image_size == 0 || export < module_base || sentinel_len == 0 {
        return false;
    }
    module_base
        .checked_add(image_size)
        .zip(export.checked_add(sentinel_len))
        .is_some_and(|(image_end, sentinel_end)| sentinel_end <= image_end)
}

fn is_x64_import_thunk(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0xff, 0x25]) || bytes.starts_with(&[0x48, 0xff, 0x25])
}

fn validate_export_sentinel_bytes(
    actual: &[u8],
    expected: &[u8],
    label: &str,
) -> Result<(), String> {
    if is_x64_import_thunk(actual) {
        return Err(format!("{label} resolved to an x64 import thunk"));
    }
    if actual.get(..expected.len()) != Some(expected) {
        return Err(format!("{label} export sentinel mismatch"));
    }
    Ok(())
}

fn actual_hardware_d3d_hook_specs(device: usize, context: usize) -> Result<Vec<HookSpec>, String> {
    if device == 0 || context == 0 {
        return Err("successful game D3D creation returned a null device or context".into());
    }
    let device_method = |slot, label| {
        com_method(device, slot)
            .ok_or_else(|| format!("actual hardware {label} vtable slot is null"))
    };
    let context_method = |slot, label| {
        com_method(context, slot)
            .ok_or_else(|| format!("actual hardware {label} vtable slot is null"))
    };
    let specs = vec![
        (
            device_method(11, "CreateInputLayout")?,
            hook_d3d_create_input_layout as *mut c_void,
            &ORIGINAL_D3D_CREATE_INPUT_LAYOUT,
        ),
        (
            device_method(12, "CreateVertexShader")?,
            hook_d3d_create_vertex_shader as *mut c_void,
            &ORIGINAL_D3D_CREATE_VERTEX_SHADER,
        ),
        (
            device_method(15, "CreatePixelShader")?,
            hook_d3d_create_pixel_shader as *mut c_void,
            &ORIGINAL_D3D_CREATE_PIXEL_SHADER,
        ),
        (
            device_method(13, "CreateGeometryShader")?,
            hook_d3d_create_geometry_shader as *mut c_void,
            &ORIGINAL_D3D_CREATE_GEOMETRY_SHADER,
        ),
        (
            device_method(16, "CreateHullShader")?,
            hook_d3d_create_hull_shader as *mut c_void,
            &ORIGINAL_D3D_CREATE_HULL_SHADER,
        ),
        (
            device_method(17, "CreateDomainShader")?,
            hook_d3d_create_domain_shader as *mut c_void,
            &ORIGINAL_D3D_CREATE_DOMAIN_SHADER,
        ),
        (
            device_method(18, "CreateComputeShader")?,
            hook_d3d_create_compute_shader as *mut c_void,
            &ORIGINAL_D3D_CREATE_COMPUTE_SHADER,
        ),
        (
            device_method(20, "CreateBlendState")?,
            hook_d3d_create_blend_state as *mut c_void,
            &ORIGINAL_D3D_CREATE_BLEND_STATE,
        ),
        (
            device_method(21, "CreateDepthStencilState")?,
            hook_d3d_create_depth_stencil_state as *mut c_void,
            &ORIGINAL_D3D_CREATE_DEPTH_STENCIL_STATE,
        ),
        (
            device_method(22, "CreateRasterizerState")?,
            hook_d3d_create_rasterizer_state as *mut c_void,
            &ORIGINAL_D3D_CREATE_RASTERIZER_STATE,
        ),
        (
            device_method(23, "CreateSamplerState")?,
            hook_d3d_create_sampler_state as *mut c_void,
            &ORIGINAL_D3D_CREATE_SAMPLER_STATE,
        ),
        (
            device_method(3, "CreateBuffer")?,
            hook_d3d_create_buffer as *mut c_void,
            &ORIGINAL_D3D_CREATE_BUFFER,
        ),
        (
            device_method(4, "CreateTexture1D")?,
            hook_d3d_create_texture1d as *mut c_void,
            &ORIGINAL_D3D_CREATE_TEXTURE1D,
        ),
        (
            device_method(5, "CreateTexture2D")?,
            hook_d3d_create_texture2d as *mut c_void,
            &ORIGINAL_D3D_CREATE_TEXTURE2D,
        ),
        (
            device_method(6, "CreateTexture3D")?,
            hook_d3d_create_texture3d as *mut c_void,
            &ORIGINAL_D3D_CREATE_TEXTURE3D,
        ),
        (
            device_method(7, "CreateShaderResourceView")?,
            hook_d3d_create_srv as *mut c_void,
            &ORIGINAL_D3D_CREATE_SRV,
        ),
        (
            device_method(9, "CreateRenderTargetView")?,
            hook_d3d_create_rtv as *mut c_void,
            &ORIGINAL_D3D_CREATE_RTV,
        ),
        (
            device_method(10, "CreateDepthStencilView")?,
            hook_d3d_create_dsv as *mut c_void,
            &ORIGINAL_D3D_CREATE_DSV,
        ),
        (
            context_method(37, "SOSetTargets")?,
            hook_d3d_so_set_targets as *mut c_void,
            &ORIGINAL_D3D_SO_SET_TARGETS,
        ),
    ];
    debug_assert_eq!(specs.len(), HARDWARE_D3D_HOOK_COUNT);
    Ok(specs)
}

fn rollback_created_hooks(specs: &[HookSpec], created: usize) -> Result<(), String> {
    let mut failures = Vec::new();
    let includes_com = specs
        .iter()
        .take(created)
        .any(|(_, _, original)| is_com_original(original));
    let includes_a25_gui_bridge = specs
        .iter()
        .take(created)
        .any(|(_, _, original)| a25_gui_bridge::owns_original_slot(original));
    if includes_com {
        begin_com_detour_teardown();
    }
    if includes_a25_gui_bridge {
        a25_gui_bridge::begin_teardown();
    }
    let mut disable_failed = false;
    for (target, _, _) in specs.iter().take(created).rev() {
        match unsafe { MinHook::disable_hook(*target as *mut c_void) } {
            Ok(()) | Err(MH_STATUS::MH_ERROR_DISABLED) | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => {}
            Err(status) => {
                disable_failed = true;
                failures.push(format!("disable {target:#x}: {status}"));
            }
        }
    }
    let mut quiescence_failed = false;
    if includes_com {
        if let Err(error) = wait_for_com_detour_quiescence() {
            quiescence_failed = true;
            failures.push(error);
        }
    }
    if includes_a25_gui_bridge {
        if let Err(error) = a25_gui_bridge::wait_for_quiescence() {
            quiescence_failed = true;
            failures.push(error);
        }
    }
    if disable_failed || quiescence_failed {
        return Err(format!("hook rollback failed: {}", failures.join(", ")));
    }
    let mut removal_failed = false;
    for (target, _, _) in specs.iter().take(created).rev() {
        match unsafe { MinHook::remove_hook(*target as *mut c_void) } {
            Ok(()) | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => {}
            Err(status) => {
                removal_failed = true;
                failures.push(format!("remove {target:#x}: {status}"));
            }
        }
    }
    if !removal_failed && !quiescence_failed {
        clear_created_originals(specs, created);
        if includes_com {
            finish_com_detour_teardown();
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!("hook rollback failed: {}", failures.join(", ")))
    }
}

fn clear_created_originals(specs: &[HookSpec], created: usize) {
    for (_, _, original) in specs.iter().take(created) {
        original.store(0, Ordering::Release);
    }
}

fn create_and_enable_hooks(specs: &[HookSpec], label: &str) -> Result<Vec<usize>, String> {
    let mut created = 0usize;
    for (target, detour, original) in specs {
        match unsafe { MinHook::create_hook(*target as *mut c_void, *detour) } {
            Ok(value) => {
                original.store(value as usize, Ordering::Release);
                created += 1;
            }
            Err(status) => {
                let rollback = rollback_created_hooks(specs, created)
                    .err()
                    .map(|error| format!("; {error}"))
                    .unwrap_or_default();
                return Err(format!(
                    "{label} hook {target:#x} creation failed: {status}{rollback}"
                ));
            }
        }
    }
    for (index, (target, _, _)) in specs.iter().enumerate() {
        if let Err(status) = unsafe { MinHook::enable_hook(*target as *mut c_void) } {
            let rollback = rollback_created_hooks(specs, created)
                .err()
                .map(|error| format!("; {error}"))
                .unwrap_or_default();
            return Err(format!(
                "{label} hook {target:#x} enable failed at staged index {index}: {status}{rollback}"
            ));
        }
    }
    Ok(specs.iter().map(|(target, _, _)| *target).collect())
}

fn stage_hooks_after_game_device(
    device: usize,
    context: usize,
    creation_qpc: u64,
) -> Result<(), String> {
    let mut stage = HOOK_STAGE
        .get_or_init(|| Mutex::new(HookStage::Bootstrap))
        .lock()
        .map_err(|_| "hook stage state poisoned")?;
    if *stage == HookStage::Ready {
        return Err("a second successful game-device creation was rejected".into());
    }
    stage.begin_staging().map_err(String::from)?;

    let result = (|| {
        if !INSTALLED.load(Ordering::Acquire) {
            return Err("bootstrap hooks are not installed at game-device staging".into());
        }
        let base = MODULE_BASE.load(Ordering::Acquire);
        if base == 0 {
            return Err("module base is absent at game-device staging".into());
        }
        let addresses = addresses(base);
        if !engine_originals_are_clear(addresses) {
            return Err("an engine hook was patched before successful game-device creation".into());
        }
        let mut staged_targets = STAGED_HOOK_TARGETS
            .get_or_init(|| Mutex::new(Vec::new()))
            .lock()
            .map_err(|_| "staged hook target state poisoned")?;
        if !staged_targets.is_empty() {
            return Err("staged hook target list was non-empty before game-device staging".into());
        }

        let selection = staged_hook_selection()?;
        let mut all_specs = actual_hardware_d3d_hook_specs(device, context)?;
        all_specs.extend(engine_hook_specs(addresses));
        let all_labels = staged_hook_identity_labels();
        let all_groups = staged_hook_group_bits();
        let selected: Vec<_> = all_specs
            .into_iter()
            .zip(all_labels)
            .zip(all_groups)
            .filter(|(_, group)| selection.group_mask & group != 0)
            .collect();
        let specs: Vec<_> = selected.iter().map(|((spec, _), _)| *spec).collect();
        let labels: Vec<_> = selected.iter().map(|((_, label), _)| *label).collect();
        if staged_hook_count_for_group_mask(selection.group_mask) != Some(specs.len() as u32) {
            return Err("selected staged-hook inventory differs from its group mask".into());
        }
        let targets = create_and_enable_hooks(&specs, "post-device staged")?;

        let epoch_result: Result<(), String> = EARLY_CAPTURE_EPOCH
            .get()
            .ok_or("early capture epoch is absent at game-device staging")?
            .lock()
            .map_err(|_| "early capture epoch state poisoned".to_owned())
            .and_then(|mut epoch| {
                if creation_qpc == 0
                    || epoch.observer_installed_qpc == 0
                    || creation_qpc < epoch.observer_installed_qpc
                {
                    return Err("game-device creation QPC predates bootstrap readiness".into());
                }
                let generation = 1;
                let receipt = staged_hook_set_receipt_sha256(
                    epoch
                        .process
                        .as_ref()
                        .ok_or("early capture process epoch is absent during staging")?
                        .process_id,
                    epoch
                        .process
                        .as_ref()
                        .ok_or("early capture process epoch is absent during staging")?
                        .process_start_nonce,
                    epoch.hook_epoch_nonce,
                    &epoch.hook_set_receipt_sha256,
                    device,
                    context,
                    generation,
                    selection.diagnostic_stage_probe,
                    selection.group_mask,
                    &labels,
                    &targets,
                )?;
                epoch.game_device_count = epoch.game_device_count.saturating_add(1);
                if epoch.d3d_device_created_qpc == 0 {
                    epoch.d3d_device_created_qpc = creation_qpc;
                }
                epoch.staged_hook_generation = generation;
                epoch.staged_hook_count = targets.len() as u32;
                epoch.staged_hook_set_sha256 = receipt;
                epoch.diagnostic_stage_probe = selection.diagnostic_stage_probe;
                epoch.staged_hook_group_mask = selection.group_mask;
                epoch.game_device_ptr = device;
                epoch.immediate_context_ptr = context;
                Ok(())
            });
        if let Err(error) = epoch_result {
            let rollback = rollback_created_hooks(&specs, specs.len())
                .err()
                .map(|rollback| format!("; {rollback}"))
                .unwrap_or_default();
            return Err(format!("{error}{rollback}"));
        }
        *staged_targets = targets;
        Ok(())
    })();

    match result {
        Ok(()) => {
            stage.finish_staging(true);
            drop(stage);
            let pending = PENDING_STAGE_FIRST_CREATION.swap(0, Ordering::AcqRel);
            if pending != 0 {
                mark_first_creation_sequence(pending);
            }
            Ok(())
        }
        Err(error) => {
            stage.finish_staging(false);
            PENDING_STAGE_FIRST_CREATION.store(0, Ordering::Release);
            Err(error)
        }
    }
}

#[allow(clippy::too_many_arguments)]
unsafe extern "system" fn hook_d3d11_create_device(
    adapter: *mut c_void,
    driver_type: i32,
    software: *mut c_void,
    flags: u32,
    feature_levels: *const u32,
    feature_level_count: u32,
    sdk_version: u32,
    device_out: *mut *mut ID3D11Device,
    actual_feature_level: *mut u32,
    context_out: *mut *mut ID3D11DeviceContext,
) -> i32 {
    type CreateDeviceFn = unsafe extern "system" fn(
        *mut c_void,
        i32,
        *mut c_void,
        u32,
        *const u32,
        u32,
        u32,
        *mut *mut ID3D11Device,
        *mut u32,
        *mut *mut ID3D11DeviceContext,
    ) -> i32;
    let original_ptr = ORIGINAL_D3D11_CREATE_DEVICE.load(Ordering::Acquire);
    if original_ptr == 0 {
        return 0x8000_4005u32 as i32;
    }
    let original: CreateDeviceFn = unsafe { mem::transmute(original_ptr) };
    let hr = unsafe {
        original(
            adapter,
            driver_type,
            software,
            flags,
            feature_levels,
            feature_level_count,
            sdk_version,
            device_out,
            actual_feature_level,
            context_out,
        )
    };
    if hr >= 0 && !device_out.is_null() && !context_out.is_null() {
        let device = unsafe { *device_out } as usize;
        let context = unsafe { *context_out } as usize;
        let qpc = query_performance_counter().unwrap_or(0);
        if device == 0
            || context == 0
            || stage_hooks_after_game_device(device, context, qpc).is_err()
        {
            reject();
            unsafe {
                release_com(context);
                release_com(device);
                *context_out = ptr::null_mut();
                *device_out = ptr::null_mut();
            }
            return 0x8000_4005u32 as i32;
        }
    } else if hr >= 0 {
        reject();
        return 0x8000_4005u32 as i32;
    }
    hr
}

unsafe extern "system" fn hook_d3d_so_set_targets(
    device_context: usize,
    buffer_count: u32,
    buffers: *const usize,
    offsets: *const u32,
) {
    let Some(_active_detour) = ActiveComDetour::enter() else {
        reject();
        return;
    };
    let original_ptr = ORIGINAL_D3D_SO_SET_TARGETS.load(Ordering::Acquire);
    if original_ptr == 0 {
        reject();
        return;
    }
    let original: SoSetTargetsFn = unsafe { mem::transmute(original_ptr) };
    unsafe { original(device_context, buffer_count, buffers, offsets) };
    if device_context == 0 || buffer_count as usize > MAX_D3D_STREAM_OUTPUT_SLOTS {
        reject();
        return;
    }
    let mut observed = Vec::new();
    if buffer_count != 0 {
        if buffers.is_null() || offsets.is_null() {
            reject();
            return;
        }
        for slot in 0..buffer_count as usize {
            let Some(buffer_ptr) = read_usize(buffers as usize + slot * mem::size_of::<usize>())
            else {
                reject();
                return;
            };
            let Some(byte_offset) = read_u32(offsets as usize + slot * mem::size_of::<u32>())
            else {
                reject();
                return;
            };
            if buffer_ptr != 0 {
                observed.push(RawStaticStreamOutputBuffer {
                    slot: slot as u32,
                    buffer_ptr,
                    byte_offset,
                });
            }
        }
    }
    if let Some(mut state) = STATE.get().and_then(|value| value.lock().ok()) {
        state
            .stream_output_bindings
            .insert(device_context, observed);
    }
}

#[allow(clippy::too_many_arguments)]
unsafe extern "system" fn hook_d3d_compile(
    source_data: *const c_void,
    source_bytes: usize,
    source_name: *const i8,
    defines: *const D3dShaderMacro,
    include: *mut c_void,
    entry_point: *const i8,
    target: *const i8,
    flags1: u32,
    flags2: u32,
    code_out: *mut usize,
    errors_out: *mut usize,
) -> i32 {
    type CompileFn = unsafe extern "system" fn(
        *const c_void,
        usize,
        *const i8,
        *const D3dShaderMacro,
        *mut c_void,
        *const i8,
        *const i8,
        u32,
        u32,
        *mut usize,
        *mut usize,
    ) -> i32;
    let original_ptr = ORIGINAL_D3D_COMPILE.load(Ordering::Acquire);
    if original_ptr == 0 {
        return 0x8000_4005u32 as i32;
    }
    let original: CompileFn = unsafe { mem::transmute(original_ptr) };
    let hr = unsafe {
        original(
            source_data,
            source_bytes,
            source_name,
            defines,
            include,
            entry_point,
            target,
            flags1,
            flags2,
            code_out,
            errors_out,
        )
    };
    if hr < 0 || code_out.is_null() || entry_point.is_null() || target.is_null() {
        return hr;
    }
    let blob = unsafe { *code_out };
    if blob == 0 {
        reject();
        return hr;
    }
    type GetBufferPointerFn = unsafe extern "system" fn(usize) -> *mut c_void;
    type GetBufferSizeFn = unsafe extern "system" fn(usize) -> usize;
    let (Some(pointer_method), Some(size_method)) = (com_method(blob, 3), com_method(blob, 4))
    else {
        reject();
        return hr;
    };
    let get_pointer: GetBufferPointerFn = unsafe { mem::transmute(pointer_method) };
    let get_size: GetBufferSizeFn = unsafe { mem::transmute(size_method) };
    let bytecode_ptr = unsafe { get_pointer(blob) };
    let bytecode_len = unsafe { get_size(blob) };
    if bytecode_ptr.is_null() || bytecode_len == 0 || bytecode_len > MAX_D3D_SHADER_BYTECODE_BYTES {
        reject();
        return hr;
    }
    let Ok(entry_token) = unsafe { CStr::from_ptr(entry_point) }.to_str() else {
        reject();
        return hr;
    };
    let Ok(target_profile) = unsafe { CStr::from_ptr(target) }.to_str() else {
        reject();
        return hr;
    };
    if source_data.is_null() || source_bytes == 0 || source_bytes > MAX_D3D_SHADER_BYTECODE_BYTES {
        reject();
        return hr;
    }
    let source_name = if source_name.is_null() {
        String::new()
    } else {
        match unsafe { CStr::from_ptr(source_name) }.to_str() {
            Ok(value) if value.len() <= 512 && !value.chars().any(char::is_control) => {
                value.to_owned()
            }
            _ => {
                reject();
                return hr;
            }
        }
    };
    let source_payload = unsafe { slice::from_raw_parts(source_data.cast::<u8>(), source_bytes) };
    let source_payload_sha256 = format!("{:x}", Sha256::digest(source_payload));
    let mut ordered_defines = Vec::new();
    if !defines.is_null() {
        for index in 0..256usize {
            let value = unsafe { &*defines.add(index) };
            if value.name.is_null() {
                break;
            }
            let (Ok(name), value_text) = (
                unsafe { CStr::from_ptr(value.name) }.to_str(),
                if value.definition.is_null() {
                    Ok("")
                } else {
                    unsafe { CStr::from_ptr(value.definition) }.to_str()
                },
            ) else {
                reject();
                return hr;
            };
            let Ok(value_text) = value_text else {
                reject();
                return hr;
            };
            ordered_defines.push((name.to_owned(), value_text.to_owned()));
            if index == 255 {
                reject();
                return hr;
            }
        }
    }
    let mut compiler_code = [0u8; 32];
    if read_exact(original_ptr, &mut compiler_code).is_none() {
        reject();
        return hr;
    }
    let compiler_identity = format!(
        "d3dcompile-code-sha256:{}",
        format!("{:x}", Sha256::digest(compiler_code))
    );
    let bytecode =
        unsafe { slice::from_raw_parts(bytecode_ptr.cast::<u8>(), bytecode_len) }.to_vec();
    with_creation_state(|state| {
        state.compiler_outputs.push(ObservedCompilerOutput {
            source_name,
            source_payload_sha256,
            source_payload_bytes: source_bytes as u64,
            compiler_identity,
            target_profile: target_profile.to_owned(),
            compiler_flags_u64: u64::from(flags1) | (u64::from(flags2) << 32),
            entry_token: entry_token.to_owned(),
            ordered_defines,
            bytecode,
        });
    });
    hr
}

unsafe extern "system" fn hook_d3d_create_vertex_shader(
    device: usize,
    bytecode: *const c_void,
    bytecode_len: usize,
    class_linkage: usize,
    shader_out: *mut usize,
) -> i32 {
    hook_d3d_create_shader(
        RawShaderStage::Vertex,
        ORIGINAL_D3D_CREATE_VERTEX_SHADER.load(Ordering::Acquire),
        device,
        bytecode,
        bytecode_len,
        class_linkage,
        shader_out,
    )
}

unsafe extern "system" fn hook_d3d_create_pixel_shader(
    device: usize,
    bytecode: *const c_void,
    bytecode_len: usize,
    class_linkage: usize,
    shader_out: *mut usize,
) -> i32 {
    hook_d3d_create_shader(
        RawShaderStage::Pixel,
        ORIGINAL_D3D_CREATE_PIXEL_SHADER.load(Ordering::Acquire),
        device,
        bytecode,
        bytecode_len,
        class_linkage,
        shader_out,
    )
}

unsafe extern "system" fn hook_d3d_create_geometry_shader(
    device: usize,
    bytecode: *const c_void,
    bytecode_len: usize,
    class_linkage: usize,
    shader_out: *mut usize,
) -> i32 {
    hook_d3d_create_shader(
        RawShaderStage::Geometry,
        ORIGINAL_D3D_CREATE_GEOMETRY_SHADER.load(Ordering::Acquire),
        device,
        bytecode,
        bytecode_len,
        class_linkage,
        shader_out,
    )
}

unsafe extern "system" fn hook_d3d_create_hull_shader(
    device: usize,
    bytecode: *const c_void,
    bytecode_len: usize,
    class_linkage: usize,
    shader_out: *mut usize,
) -> i32 {
    hook_d3d_create_shader(
        RawShaderStage::Hull,
        ORIGINAL_D3D_CREATE_HULL_SHADER.load(Ordering::Acquire),
        device,
        bytecode,
        bytecode_len,
        class_linkage,
        shader_out,
    )
}

unsafe extern "system" fn hook_d3d_create_domain_shader(
    device: usize,
    bytecode: *const c_void,
    bytecode_len: usize,
    class_linkage: usize,
    shader_out: *mut usize,
) -> i32 {
    hook_d3d_create_shader(
        RawShaderStage::Domain,
        ORIGINAL_D3D_CREATE_DOMAIN_SHADER.load(Ordering::Acquire),
        device,
        bytecode,
        bytecode_len,
        class_linkage,
        shader_out,
    )
}

unsafe extern "system" fn hook_d3d_create_compute_shader(
    device: usize,
    bytecode: *const c_void,
    bytecode_len: usize,
    class_linkage: usize,
    shader_out: *mut usize,
) -> i32 {
    hook_d3d_create_shader(
        RawShaderStage::Compute,
        ORIGINAL_D3D_CREATE_COMPUTE_SHADER.load(Ordering::Acquire),
        device,
        bytecode,
        bytecode_len,
        class_linkage,
        shader_out,
    )
}

#[allow(clippy::too_many_arguments)]
unsafe fn hook_d3d_create_shader(
    stage: RawShaderStage,
    original_ptr: usize,
    device: usize,
    bytecode: *const c_void,
    bytecode_len: usize,
    class_linkage: usize,
    shader_out: *mut usize,
) -> i32 {
    type CreateShaderFn =
        unsafe extern "system" fn(usize, *const c_void, usize, usize, *mut usize) -> i32;
    let Some(_active_detour) = ActiveComDetour::enter() else {
        reject();
        return 0x8000_4005u32 as i32;
    };
    if original_ptr == 0 {
        reject();
        return 0x8000_4005u32 as i32;
    }
    let original: CreateShaderFn = unsafe { mem::transmute(original_ptr) };
    let hr = unsafe { original(device, bytecode, bytecode_len, class_linkage, shader_out) };
    if hr < 0
        || bytecode.is_null()
        || shader_out.is_null()
        || bytecode_len == 0
        || bytecode_len > MAX_D3D_SHADER_BYTECODE_BYTES
    {
        return hr;
    }
    let shader_ptr = unsafe { *shader_out };
    let Some(sequence) = next_catalog_observation_sequence() else {
        reject();
        return hr;
    };
    mark_first_creation_sequence(sequence);
    if shader_ptr == 0 {
        reject();
        return hr;
    }
    let bytes = unsafe { slice::from_raw_parts(bytecode.cast::<u8>(), bytecode_len) }.to_vec();
    if let Some(mut state) = STATE.get().and_then(|value| value.lock().ok()) {
        if state.shader_creation_observations.len() >= MAX_D3D_CREATION_ROWS {
            state.rejected_observations += 1;
        } else {
            let mut matches = state
                .compiler_outputs
                .iter()
                .filter(|value| value.bytecode == bytes)
                .cloned();
            let compiler = matches.next();
            if compiler
                .as_ref()
                .is_some_and(|first| matches.any(|value| value != *first))
            {
                state.rejected_observations += 1;
                return hr;
            }
            state
                .shader_creation_observations
                .push(ObservedShaderCreation {
                    creation_sequence: sequence,
                    shader_ptr,
                    stage,
                    bytecode: bytes,
                    compiler,
                });
        }
    }
    hr
}

unsafe extern "system" fn hook_d3d_create_input_layout(
    device: usize,
    elements: *const D3dInputElementDesc,
    element_count: u32,
    shader_signature: *const c_void,
    shader_signature_bytes: usize,
    layout_out: *mut usize,
) -> i32 {
    type CreateInputLayoutFn = unsafe extern "system" fn(
        usize,
        *const D3dInputElementDesc,
        u32,
        *const c_void,
        usize,
        *mut usize,
    ) -> i32;
    let Some(_active_detour) = ActiveComDetour::enter() else {
        reject();
        return 0x8000_4005u32 as i32;
    };
    let original_ptr = ORIGINAL_D3D_CREATE_INPUT_LAYOUT.load(Ordering::Acquire);
    if original_ptr == 0 {
        reject();
        return 0x8000_4005u32 as i32;
    }
    let original: CreateInputLayoutFn = unsafe { mem::transmute(original_ptr) };
    let hr = unsafe {
        original(
            device,
            elements,
            element_count,
            shader_signature,
            shader_signature_bytes,
            layout_out,
        )
    };
    if hr < 0
        || elements.is_null()
        || layout_out.is_null()
        || element_count == 0
        || element_count > MAX_D3D_INPUT_ELEMENTS
    {
        return hr;
    }
    let input_layout_ptr = unsafe { *layout_out };
    let Some(creation_sequence) = next_catalog_observation_sequence() else {
        reject();
        return hr;
    };
    mark_first_creation_sequence(creation_sequence);
    if input_layout_ptr == 0 {
        reject();
        return hr;
    }
    let raw = unsafe { slice::from_raw_parts(elements, element_count as usize) };
    let mut ordered_elements = Vec::with_capacity(raw.len());
    for element in raw {
        if element.semantic_name.is_null() {
            reject();
            return hr;
        }
        let Ok(semantic_token) = unsafe { CStr::from_ptr(element.semantic_name) }.to_str() else {
            reject();
            return hr;
        };
        if semantic_token.is_empty() {
            reject();
            return hr;
        }
        ordered_elements.push(RawInputElement {
            semantic_token: semantic_token.to_owned(),
            semantic_index: element.semantic_index,
            dxgi_format_u32: element.format,
            input_slot: element.input_slot,
            aligned_byte_offset: element.aligned_byte_offset,
            input_slot_class_u32: element.input_slot_class,
            instance_step_rate: element.instance_data_step_rate,
        });
    }
    if let Some(mut state) = STATE.get().and_then(|value| value.lock().ok()) {
        if state.input_layout_creations.len() >= MAX_D3D_CREATION_ROWS {
            state.rejected_observations += 1;
        } else {
            state.input_layout_creations.push(RawInputLayoutCreation {
                creation_sequence,
                input_layout_ptr,
                ordered_elements,
            });
        }
    }
    hr
}

unsafe extern "system" fn hook_d3d_create_blend_state(
    device: usize,
    desc: *const D3D11_BLEND_DESC,
    state_out: *mut usize,
) -> i32 {
    type CreateFn = unsafe extern "system" fn(usize, *const D3D11_BLEND_DESC, *mut usize) -> i32;
    let Some(_active_detour) = ActiveComDetour::enter() else {
        reject();
        return 0x8000_4005u32 as i32;
    };
    let original_ptr = ORIGINAL_D3D_CREATE_BLEND_STATE.load(Ordering::Acquire);
    if original_ptr == 0 {
        reject();
        return 0x8000_4005u32 as i32;
    }
    let original: CreateFn = unsafe { mem::transmute(original_ptr) };
    let hr = unsafe { original(device, desc, state_out) };
    if hr < 0 || desc.is_null() || state_out.is_null() {
        return hr;
    }
    let state_ptr = unsafe { *state_out };
    let Some(creation_sequence) = next_catalog_observation_sequence() else {
        reject();
        return hr;
    };
    mark_first_creation_sequence(creation_sequence);
    if state_ptr == 0 {
        reject();
        return hr;
    }
    let desc = unsafe { &*desc };
    let ordered_targets = desc
        .RenderTarget
        .iter()
        .map(|target| RawRenderTargetBlendDesc {
            blend_enable_u32: target.BlendEnable as u32,
            source_color_u32: target.SrcBlend as u32,
            destination_color_u32: target.DestBlend as u32,
            color_operation_u32: target.BlendOp as u32,
            source_alpha_u32: target.SrcBlendAlpha as u32,
            destination_alpha_u32: target.DestBlendAlpha as u32,
            alpha_operation_u32: target.BlendOpAlpha as u32,
            write_mask_rgba_u8: target.RenderTargetWriteMask,
        })
        .collect();
    with_creation_state(|state| {
        state.blend_state_creations.push(RawBlendStateCreation {
            creation_sequence,
            blend_state_ptr: state_ptr,
            alpha_to_coverage_enable_u32: desc.AlphaToCoverageEnable as u32,
            independent_blend_enable_u32: desc.IndependentBlendEnable as u32,
            ordered_targets,
        });
    });
    hr
}

fn raw_stencil_face(value: &winapi::um::d3d11::D3D11_DEPTH_STENCILOP_DESC) -> RawStencilFaceDesc {
    RawStencilFaceDesc {
        fail_operation_u32: value.StencilFailOp as u32,
        depth_fail_operation_u32: value.StencilDepthFailOp as u32,
        pass_operation_u32: value.StencilPassOp as u32,
        comparison_u32: value.StencilFunc as u32,
    }
}

unsafe extern "system" fn hook_d3d_create_depth_stencil_state(
    device: usize,
    desc: *const D3D11_DEPTH_STENCIL_DESC,
    state_out: *mut usize,
) -> i32 {
    type CreateFn =
        unsafe extern "system" fn(usize, *const D3D11_DEPTH_STENCIL_DESC, *mut usize) -> i32;
    let Some(_active_detour) = ActiveComDetour::enter() else {
        reject();
        return 0x8000_4005u32 as i32;
    };
    let original_ptr = ORIGINAL_D3D_CREATE_DEPTH_STENCIL_STATE.load(Ordering::Acquire);
    if original_ptr == 0 {
        reject();
        return 0x8000_4005u32 as i32;
    }
    let original: CreateFn = unsafe { mem::transmute(original_ptr) };
    let hr = unsafe { original(device, desc, state_out) };
    if hr < 0 || desc.is_null() || state_out.is_null() {
        return hr;
    }
    let state_ptr = unsafe { *state_out };
    let Some(creation_sequence) = next_catalog_observation_sequence() else {
        reject();
        return hr;
    };
    mark_first_creation_sequence(creation_sequence);
    if state_ptr == 0 {
        reject();
        return hr;
    }
    let desc = unsafe { &*desc };
    with_creation_state(|state| {
        state
            .depth_stencil_state_creations
            .push(RawDepthStencilStateCreation {
                creation_sequence,
                depth_stencil_state_ptr: state_ptr,
                depth_enable_u32: desc.DepthEnable as u32,
                depth_write_mask_u32: desc.DepthWriteMask as u32,
                depth_comparison_u32: desc.DepthFunc as u32,
                stencil_enable_u32: desc.StencilEnable as u32,
                stencil_read_mask_u8: desc.StencilReadMask,
                stencil_write_mask_u8: desc.StencilWriteMask,
                front_face: raw_stencil_face(&desc.FrontFace),
                back_face: raw_stencil_face(&desc.BackFace),
            });
    });
    hr
}

unsafe extern "system" fn hook_d3d_create_rasterizer_state(
    device: usize,
    desc: *const D3D11_RASTERIZER_DESC,
    state_out: *mut usize,
) -> i32 {
    type CreateFn =
        unsafe extern "system" fn(usize, *const D3D11_RASTERIZER_DESC, *mut usize) -> i32;
    let Some(_active_detour) = ActiveComDetour::enter() else {
        reject();
        return 0x8000_4005u32 as i32;
    };
    let original_ptr = ORIGINAL_D3D_CREATE_RASTERIZER_STATE.load(Ordering::Acquire);
    if original_ptr == 0 {
        reject();
        return 0x8000_4005u32 as i32;
    }
    let original: CreateFn = unsafe { mem::transmute(original_ptr) };
    let hr = unsafe { original(device, desc, state_out) };
    if hr < 0 || desc.is_null() || state_out.is_null() {
        return hr;
    }
    let state_ptr = unsafe { *state_out };
    let Some(creation_sequence) = next_catalog_observation_sequence() else {
        reject();
        return hr;
    };
    mark_first_creation_sequence(creation_sequence);
    if state_ptr == 0 {
        reject();
        return hr;
    }
    let desc = unsafe { &*desc };
    with_creation_state(|state| {
        state
            .rasterizer_state_creations
            .push(RawRasterizerStateCreation {
                creation_sequence,
                rasterizer_state_ptr: state_ptr,
                fill_mode_u32: desc.FillMode as u32,
                cull_mode_u32: desc.CullMode as u32,
                front_counter_clockwise_u32: desc.FrontCounterClockwise as u32,
                depth_bias_i32: desc.DepthBias,
                depth_bias_clamp_f32_bits: desc.DepthBiasClamp.to_bits(),
                slope_scaled_depth_bias_f32_bits: desc.SlopeScaledDepthBias.to_bits(),
                depth_clip_enable_u32: desc.DepthClipEnable as u32,
                scissor_enable_u32: desc.ScissorEnable as u32,
                multisample_enable_u32: desc.MultisampleEnable as u32,
                antialiased_line_enable_u32: desc.AntialiasedLineEnable as u32,
            });
    });
    hr
}

unsafe extern "system" fn hook_d3d_create_sampler_state(
    device: usize,
    desc: *const D3D11_SAMPLER_DESC,
    state_out: *mut usize,
) -> i32 {
    type CreateFn = unsafe extern "system" fn(usize, *const D3D11_SAMPLER_DESC, *mut usize) -> i32;
    let Some(_active_detour) = ActiveComDetour::enter() else {
        reject();
        return 0x8000_4005u32 as i32;
    };
    let original_ptr = ORIGINAL_D3D_CREATE_SAMPLER_STATE.load(Ordering::Acquire);
    if original_ptr == 0 {
        reject();
        return 0x8000_4005u32 as i32;
    }
    let original: CreateFn = unsafe { mem::transmute(original_ptr) };
    let hr = unsafe { original(device, desc, state_out) };
    if hr < 0 || desc.is_null() || state_out.is_null() {
        return hr;
    }
    let state_ptr = unsafe { *state_out };
    let Some(creation_sequence) = next_catalog_observation_sequence() else {
        reject();
        return hr;
    };
    mark_first_creation_sequence(creation_sequence);
    if state_ptr == 0 {
        reject();
        return hr;
    }
    let desc = unsafe { &*desc };
    with_creation_state(|state| {
        state.sampler_state_creations.push(RawSamplerStateCreation {
            creation_sequence,
            sampler_state_ptr: state_ptr,
            filter_u32: desc.Filter as u32,
            address_u_u32: desc.AddressU as u32,
            address_v_u32: desc.AddressV as u32,
            address_w_u32: desc.AddressW as u32,
            mip_lod_bias_f32_bits: desc.MipLODBias.to_bits(),
            max_anisotropy: desc.MaxAnisotropy,
            comparison_u32: desc.ComparisonFunc as u32,
            border_color_rgba_f32_bits: desc.BorderColor.map(f32::to_bits),
            min_lod_f32_bits: desc.MinLOD.to_bits(),
            max_lod_f32_bits: desc.MaxLOD.to_bits(),
        });
    });
    hr
}

fn with_creation_state(record: impl FnOnce(&mut HookState)) {
    let Some(mut state) = STATE.get().and_then(|value| value.lock().ok()) else {
        return;
    };
    let total = state.shader_creation_observations.len()
        + state.input_layout_creations.len()
        + state.blend_state_creations.len()
        + state.depth_stencil_state_creations.len()
        + state.rasterizer_state_creations.len()
        + state.sampler_state_creations.len()
        + state.created_d3d_objects.len();
    if total >= MAX_D3D_CREATION_ROWS {
        state.rejected_observations += 1;
    } else {
        record(&mut state);
    }
}

fn remember_created_object(pointer: usize, kind: CreatedD3dObjectKind) -> Option<u64> {
    let Some(creation_sequence) = next_catalog_observation_sequence() else {
        reject();
        return None;
    };
    mark_first_creation_sequence(creation_sequence);
    if pointer == 0 {
        reject();
        return None;
    }
    let mut state = STATE.get()?.lock().ok()?;
    let total = state.shader_creation_observations.len()
        + state.input_layout_creations.len()
        + state.blend_state_creations.len()
        + state.depth_stencil_state_creations.len()
        + state.rasterizer_state_creations.len()
        + state.sampler_state_creations.len()
        + state.created_d3d_objects.len();
    if total >= MAX_D3D_CREATION_ROWS {
        state.rejected_observations += 1;
        return None;
    }
    state.created_d3d_objects.insert(
        pointer,
        CreatedD3dObject {
            creation_sequence,
            kind,
        },
    );
    Some(creation_sequence)
}

unsafe extern "system" fn hook_d3d_create_buffer(
    device: usize,
    desc: *const D3D11_BUFFER_DESC,
    initial_data: *const D3D11_SUBRESOURCE_DATA,
    output: *mut usize,
) -> i32 {
    type CreateFn = unsafe extern "system" fn(
        usize,
        *const D3D11_BUFFER_DESC,
        *const D3D11_SUBRESOURCE_DATA,
        *mut usize,
    ) -> i32;
    let Some(_active_detour) = ActiveComDetour::enter() else {
        reject();
        return 0x8000_4005u32 as i32;
    };
    let original_ptr = ORIGINAL_D3D_CREATE_BUFFER.load(Ordering::Acquire);
    if original_ptr == 0 {
        reject();
        return 0x8000_4005u32 as i32;
    }
    let original: CreateFn = unsafe { mem::transmute(original_ptr) };
    let hr = unsafe { original(device, desc, initial_data, output) };
    if hr >= 0 && !output.is_null() {
        let _ = remember_created_object(unsafe { *output }, CreatedD3dObjectKind::Buffer);
    }
    hr
}

macro_rules! create_texture_hook {
    ($name:ident, $original:ident, $desc:ty, $kind:expr) => {
        unsafe extern "system" fn $name(
            device: usize,
            desc: *const $desc,
            initial_data: *const D3D11_SUBRESOURCE_DATA,
            output: *mut usize,
        ) -> i32 {
            type CreateFn = unsafe extern "system" fn(
                usize,
                *const $desc,
                *const D3D11_SUBRESOURCE_DATA,
                *mut usize,
            ) -> i32;
            let Some(_active_detour) = ActiveComDetour::enter() else {
                reject();
                return 0x8000_4005u32 as i32;
            };
            let original_ptr = $original.load(Ordering::Acquire);
            if original_ptr == 0 {
                reject();
                return 0x8000_4005u32 as i32;
            }
            let original: CreateFn = unsafe { mem::transmute(original_ptr) };
            let hr = unsafe { original(device, desc, initial_data, output) };
            if hr >= 0 && !output.is_null() {
                let _ = remember_created_object(unsafe { *output }, $kind);
            }
            hr
        }
    };
}

create_texture_hook!(
    hook_d3d_create_texture1d,
    ORIGINAL_D3D_CREATE_TEXTURE1D,
    D3D11_TEXTURE1D_DESC,
    CreatedD3dObjectKind::Texture1D
);
unsafe extern "system" fn hook_d3d_create_texture2d(
    device: usize,
    desc: *const D3D11_TEXTURE2D_DESC,
    initial_data: *const D3D11_SUBRESOURCE_DATA,
    output: *mut usize,
) -> i32 {
    type CreateFn = unsafe extern "system" fn(
        usize,
        *const D3D11_TEXTURE2D_DESC,
        *const D3D11_SUBRESOURCE_DATA,
        *mut usize,
    ) -> i32;
    let Some(_active_detour) = ActiveComDetour::enter() else {
        reject();
        return 0x8000_4005u32 as i32;
    };
    let original_ptr = ORIGINAL_D3D_CREATE_TEXTURE2D.load(Ordering::Acquire);
    if original_ptr == 0 {
        reject();
        return 0x8000_4005u32 as i32;
    }
    let original: CreateFn = unsafe { mem::transmute(original_ptr) };
    let hr = unsafe { original(device, desc, initial_data, output) };
    if hr >= 0 && !output.is_null() {
        let resource_ptr = unsafe { *output };
        if let Some(sequence) =
            remember_created_object(resource_ptr, CreatedD3dObjectKind::Texture2D)
        {
            TEXTURE_LOAD_STACK.with(|stack| {
                if let Some(active) = stack.borrow_mut().last_mut() {
                    active
                        .nested_texture_creations
                        .push((sequence, resource_ptr));
                }
            });
        }
    }
    hr
}
create_texture_hook!(
    hook_d3d_create_texture3d,
    ORIGINAL_D3D_CREATE_TEXTURE3D,
    D3D11_TEXTURE3D_DESC,
    CreatedD3dObjectKind::Texture3D
);

macro_rules! create_view_hook {
    ($name:ident, $original:ident, $desc:ty, $kind:expr) => {
        unsafe extern "system" fn $name(
            device: usize,
            resource: usize,
            desc: *const $desc,
            output: *mut usize,
        ) -> i32 {
            type CreateFn =
                unsafe extern "system" fn(usize, usize, *const $desc, *mut usize) -> i32;
            let Some(_active_detour) = ActiveComDetour::enter() else {
                reject();
                return 0x8000_4005u32 as i32;
            };
            let original_ptr = $original.load(Ordering::Acquire);
            if original_ptr == 0 {
                reject();
                return 0x8000_4005u32 as i32;
            }
            let original: CreateFn = unsafe { mem::transmute(original_ptr) };
            let hr = unsafe { original(device, resource, desc, output) };
            if hr >= 0 && !output.is_null() {
                let _ = remember_created_object(unsafe { *output }, $kind);
            }
            hr
        }
    };
}

unsafe extern "system" fn hook_d3d_create_srv(
    device: usize,
    resource: usize,
    desc: *const D3D11_SHADER_RESOURCE_VIEW_DESC,
    output: *mut usize,
) -> i32 {
    type CreateFn = unsafe extern "system" fn(
        usize,
        usize,
        *const D3D11_SHADER_RESOURCE_VIEW_DESC,
        *mut usize,
    ) -> i32;
    let Some(_active_detour) = ActiveComDetour::enter() else {
        reject();
        return 0x8000_4005u32 as i32;
    };
    let original_ptr = ORIGINAL_D3D_CREATE_SRV.load(Ordering::Acquire);
    if original_ptr == 0 {
        reject();
        return 0x8000_4005u32 as i32;
    }
    let original: CreateFn = unsafe { mem::transmute(original_ptr) };
    let hr = unsafe { original(device, resource, desc, output) };
    if hr >= 0 && !output.is_null() {
        let srv_ptr = unsafe { *output };
        if let Some(sequence) =
            remember_created_object(srv_ptr, CreatedD3dObjectKind::ShaderResourceView)
        {
            TEXTURE_LOAD_STACK.with(|stack| {
                if let Some(active) = stack.borrow_mut().last_mut() {
                    active
                        .nested_srv_creations
                        .push((sequence, srv_ptr, resource));
                }
            });
        }
    }
    hr
}
create_view_hook!(
    hook_d3d_create_rtv,
    ORIGINAL_D3D_CREATE_RTV,
    D3D11_RENDER_TARGET_VIEW_DESC,
    CreatedD3dObjectKind::RenderTargetView
);
create_view_hook!(
    hook_d3d_create_dsv,
    ORIGINAL_D3D_CREATE_DSV,
    D3D11_DEPTH_STENCIL_VIEW_DESC,
    CreatedD3dObjectKind::DepthStencilView
);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HookDiagnostics {
    pub live_runtime_count: u64,
    pub queued_submit_count: u64,
    pub rejected_observation_count: u64,
    pub evicted_submit_count: u64,
    pub duplicate_runtime_count: u64,
    pub nested_envelope_count: u64,
    pub live_entity_count: u64,
    pub queued_weather_change_count: u64,
    pub weather_change_overflow_count: u64,
    pub observed_weather_region_count: u64,
    pub active_airborne_weather_region_count: u64,
    pub duplicate_entity_count: u64,
    pub nested_weather_count: u64,
    pub queued_map_shader_lookup_count: u64,
    pub rejected_map_shader_lookup_count: u64,
    pub map_shader_lookup_overflow_count: u64,
    pub queued_render_order_event_count: u64,
    pub queued_clear_output_snapshot_count: u64,
    pub queued_static_d3d_capture_count: u64,
    pub render_order_event_overflow_count: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawRenderOrderCapture {
    pub events: Vec<RawRenderOrderEvent>,
    pub static_d3d_captures: Vec<RawStaticD3dCapture>,
    /// Exact immutable/static authority closure selected atomically from the
    /// same hook state as `events` and `static_d3d_captures`. It deliberately
    /// carries no process/launch/frame envelope; the collector supplies that
    /// once when it pairs this fragment with the matching runtime submit.
    pub static_catalog_fragment: RawStaticCatalogFragment,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawStaticCatalogFragment {
    pub map_shader_program_joins: Vec<RawMapShaderProgramJoin>,
    pub render_target_registrations: Vec<RawRenderTargetRegistrationObservation>,
    pub texture_vfs_resource_joins: Vec<RawTextureVfsResourceJoin>,
    pub shader_creations: Vec<RawShaderCreation>,
    pub shader_reflections: Vec<RawShaderBytecodeReflectionObservation>,
    pub input_layout_creations: Vec<RawInputLayoutCreation>,
    pub blend_state_creations: Vec<RawBlendStateCreation>,
    pub depth_stencil_state_creations: Vec<RawDepthStencilStateCreation>,
    pub rasterizer_state_creations: Vec<RawRasterizerStateCreation>,
    pub sampler_state_creations: Vec<RawSamplerStateCreation>,
    pub buffer_snapshots: Vec<RawBufferSnapshot>,
    pub texture_resource_snapshots: Vec<RawTextureResourceSnapshot>,
    pub shader_resource_view_snapshots: Vec<RawShaderResourceViewSnapshot>,
    pub render_target_view_snapshots: Vec<RawRenderTargetViewSnapshot>,
    pub clear_output_view_snapshots: Vec<RawRenderTargetViewSnapshot>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawRenderFrameCapture {
    pub frame_sequence: u64,
    pub submit_index: u64,
    /// Exact completed-Present ordinal captured by the hook at PresentEnd.
    /// It is epoch-global and is never derived from `frame_sequence`.
    pub global_order: u32,
    pub presentation_qpc_ticks: u64,
    pub presentation_qpc_frequency_hz: u64,
    pub process_start_nonce: u64,
    pub hook_epoch_nonce: u64,
    /// Atomic weather discriminator captured at this Present boundary. These
    /// counts prove when an empty portable weather carrier would be false;
    /// they do not replace the weather rows themselves.
    pub expected_weather_region_count: u64,
    pub observed_weather_region_count: u64,
    pub active_airborne_weather_region_count: u64,
    pub queued_weather_change_count: u64,
    /// Latest complete normal-update membership/checkpoint row per observed
    /// region at this exact PresentEnd, ordered by strategic-region id.
    pub weather_membership_updates: Vec<RawWeatherObservation>,
    /// Latest complete normal-update visibility row per active region at this
    /// exact PresentEnd, ordered by strategic-region id.
    pub weather_visibility_updates: Vec<RawWeatherVisibilityObservation>,
    pub latest_completed_weather_update_index: u64,
    /// Pointer-free semantic digest of both vectors. It is folded into every
    /// candidate receipt so selection cannot detach weather from Present.
    pub weather_snapshot_sha256: String,
    /// Canonical engine hour read at this exact PresentEnd from the validated
    /// a729 CGameState root slot. It is never assigned at collector drain time.
    pub game_hour_raw: i32,
    /// Latest exact SnowMudData pre-upload observation ordered no later than
    /// this Present. Shared byte storage avoids copying the 2.75 MiB texture
    /// when the engine legitimately reuses an unchanged producer result.
    pub snow_mud_texture: Option<Arc<RawSnowMudTextureObservation>>,
    /// Candidate upload rows ordered by process-local resource identity. The
    /// collector may promote exactly one only after complete draw-time byte
    /// equality and reflected terrain-t6/water-t3 joins.
    pub occupation_secondary_textures: Vec<Arc<RawOccupationSecondaryTextureObservation>>,
    pub events: Vec<RawRenderOrderEvent>,
    pub static_d3d_captures: Vec<RawStaticD3dCapture>,
    pub static_catalog_fragment: RawStaticCatalogFragment,
}

/// One completed Present frame joined to the exact runtime submit/catalog
/// published by the enclosing render envelope. Production drains this unit
/// atomically by `submit_index`; queue position and publication timing are
/// never used as authority.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawJoinedRenderFrameCapture {
    pub frame: RawRenderFrameCapture,
    pub runtime: RawRenderSubmitCapture,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RawJoinedRenderFrameDrain {
    /// No completed frame and no runtime/catalog row is waiting in hook state.
    EmptyNoCapture,
    /// At least one side is retained but no exact submit-id join is ready.
    /// Waiting on `publication_generation` is race-free: envelope publication
    /// increments it under the same hook-state lock before waking collectors.
    PendingIncomplete {
        publication_generation: u64,
        completed_frame_count: usize,
        runtime_submit_count: usize,
    },
    Ready(Vec<RawJoinedRenderFrameCapture>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawSelectedRenderFrameReceipt {
    pub target_game_hour_raw: i32,
    pub cutoff_qpc_ticks: u64,
    pub cutoff_qpc_frequency_hz: u64,
    pub candidate_count: u32,
    pub candidate_set_sha256: String,
    pub selected_submit_index: u64,
    pub selected_frame_sequence: u64,
    pub selected_global_order: u32,
    pub selected_presentation_qpc_ticks: u64,
    pub discarded_candidate_count: u32,
    pub consumed_unselected_sha256: String,
    /// Pointer-free hash of the complete scalar selection receipt.
    pub selection_receipt_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawConsumedUnselectedRenderFrameReceipt {
    pub submit_index: u64,
    pub frame_sequence: u64,
    pub global_order: u32,
    pub presentation_qpc_ticks: u64,
    pub live_draw_count: usize,
    pub ordered_live_draw_sequences: Vec<u64>,
    pub ordered_static_capture_sequences: Vec<u64>,
}

/// Fixed-size audit telemetry for late rows retired after an hour was sealed.
/// The digest is a canonical rolling hash over every exact pointer-free purge
/// receipt since the previous drain; no per-Present row is retained.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SealedRenderPurgeTelemetry {
    pub receipt_count: u128,
    pub receipt_count_overflowed: bool,
    pub rolling_receipt_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawSelectedRenderFrameCapture {
    pub receipt: RawSelectedRenderFrameReceipt,
    pub selected: RawJoinedRenderFrameCapture,
    /// Extractor-local exact identities retired before payload admission. Only
    /// their count/hash is portable.
    pub consumed_unselected: Vec<RawConsumedUnselectedRenderFrameReceipt>,
    /// Ready stale/after-cutoff rows purged while sealing this hour. They are
    /// not display candidates and do not enter `candidate_count`.
    pub sealed_purges: Vec<RawConsumedUnselectedRenderFrameReceipt>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RawSelectedRenderFrameDrain {
    EmptyNoCandidate {
        sealed_game_hour_raw: i32,
        sealed_purges: Vec<RawConsumedUnselectedRenderFrameReceipt>,
    },
    PendingIncomplete {
        publication_generation: u64,
        target_frame_count: usize,
        target_ready_count: usize,
    },
    Ready(RawSelectedRenderFrameCapture),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderSubmitPublicationWait {
    PublicationAdvanced { publication_generation: u64 },
    TimedOut { publication_generation: u64 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PresentGameHourSourceReceipt {
    /// Address of the authenticated a729 static CGameState pointer slot.
    pub game_state_root_slot_address: usize,
    pub game_hour_field_offset: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstalledAddresses {
    pub vfs_open_constructor: usize,
    pub window_backbuffer_rtv_producer: usize,
    pub opaque_texture_target_producer: usize,
    pub texture_cache_load_root: usize,
    pub global_envelope: usize,
    pub object_ctor: usize,
    pub bucket_submit: usize,
    pub runtime_create: usize,
    pub runtime_destroy: usize,
    pub packet_builder: usize,
    pub particle_render_submit: usize,
    pub particle_final_draw: usize,
    pub entity_factory: usize,
    pub entity_named_factory: usize,
    pub entity_definition_lookup: usize,
    pub entity_destroy: usize,
    pub entity_render_root: usize,
    pub entity_final_draw: usize,
    pub indexed_instanced_final_draw: usize,
    pub instanced_final_draw: usize,
    pub render_target_clear: usize,
    pub shader_program_binder: usize,
    pub engine_shader_effect_lookup: usize,
    pub map_effect_registration_root: usize,
    pub railroad_effect_registration_root: usize,
    pub map_name_effect_registration_root: usize,
    pub restore_bloom_effect_registration_root: usize,
    pub lut_blend_effect_registration_root: usize,
    pub weather_normal_update_owner: usize,
    pub weather_editor_refresh_owner: usize,
    pub weather_visual_producer: usize,
    pub weather_entity_active_setter: usize,
    pub weather_entity_alpha_setter: usize,
    pub weather_entity_playback_setter: usize,
    pub high_resolution_seed: usize,
    pub map_scene_render_root: usize,
    pub post_process_render_root: usize,
    pub present_render_root: usize,
    pub snow_mud_producer: usize,
    pub snow_mud_upload: usize,
    pub camera_wheel_update: usize,
    pub camera_vertical_event_producer: usize,
    pub camera_map_pick: usize,
    pub o05_update_provinces: usize,
    pub o05_building_constructor: usize,
    pub o05_building_vector_update: usize,
    pub o05_entity_transform_setter: usize,
}

/// Binds the authenticated a729 global CGameState pointer slot before render
/// hooks are installed. The pointee may legitimately still be null while the
/// primary thread is suspended; every accepted Present later requires a
/// stable, non-null pointee and canonical current-hour read.
pub fn bind_present_game_hour_root_slot(
    root_slot_address: usize,
) -> Result<PresentGameHourSourceReceipt, String> {
    if INSTALLED.load(Ordering::Acquire) {
        return Err("Present game-hour root slot must be bound before hook installation".into());
    }
    let base = unsafe { GetModuleHandleW(ptr::null()) } as usize;
    let expected = base
        .checked_add(compact_contract::build_profile::CG_STATE_DATE_OFFSET)
        .ok_or("Present game-hour root slot address overflowed")?;
    if base == 0 || root_slot_address != expected {
        return Err("Present game-hour root slot is not the authenticated a729 slot".into());
    }
    // Null is valid before engine initialization, but the slot itself must be
    // readable before readiness is published.
    read_usize(root_slot_address).ok_or("Present game-hour CGameState root slot is unreadable")?;
    match PRESENT_GAME_STATE_ROOT_SLOT.compare_exchange(
        0,
        root_slot_address,
        Ordering::AcqRel,
        Ordering::Acquire,
    ) {
        Ok(_) => {}
        Err(existing) if existing == root_slot_address => {}
        Err(_) => return Err("Present game-hour root slot was rebound".into()),
    }
    Ok(PresentGameHourSourceReceipt {
        game_state_root_slot_address: root_slot_address,
        game_hour_field_offset: compact_contract::build_profile::CGAMESTATE_CURRENT_HOUR,
    })
}

fn engine_hook_specs(a: InstalledAddresses) -> [HookSpec; ENGINE_HOOK_COUNT] {
    let detours = [
        hook_vfs_open_constructor as *mut c_void,
        hook_window_backbuffer_rtv_producer as *mut c_void,
        hook_opaque_texture_target_producer as *mut c_void,
        hook_texture_cache_load_root as *mut c_void,
        hook_envelope as *mut c_void,
        hook_object_ctor as *mut c_void,
        hook_bucket as *mut c_void,
        hook_create as *mut c_void,
        hook_destroy as *mut c_void,
        hook_builder as *mut c_void,
        hook_particle_render_submit as *mut c_void,
        hook_particle_final_draw as *mut c_void,
        hook_entity_factory as *mut c_void,
        hook_entity_named_factory as *mut c_void,
        hook_entity_definition_lookup as *mut c_void,
        hook_entity_destroy as *mut c_void,
        hook_entity_render as *mut c_void,
        hook_entity_final_draw as *mut c_void,
        hook_indexed_instanced_final_draw as *mut c_void,
        hook_instanced_final_draw as *mut c_void,
        hook_render_target_clear as *mut c_void,
        hook_shader_program_binder as *mut c_void,
        hook_engine_shader_effect_lookup as *mut c_void,
        hook_map_effect_registration_root as *mut c_void,
        hook_railroad_effect_registration_root as *mut c_void,
        hook_map_name_effect_registration_root as *mut c_void,
        hook_restore_bloom_effect_registration_root as *mut c_void,
        hook_lut_blend_effect_registration_root as *mut c_void,
        hook_weather_normal_update_owner as *mut c_void,
        hook_weather_editor_refresh_owner as *mut c_void,
        hook_weather as *mut c_void,
        hook_weather_active_setter as *mut c_void,
        hook_weather_alpha_setter as *mut c_void,
        hook_weather_playback_setter as *mut c_void,
        hook_seed as *mut c_void,
        hook_map_scene_render_root as *mut c_void,
        hook_post_process_render_root as *mut c_void,
        hook_present_render_root as *mut c_void,
        hook_snow_mud_producer as *mut c_void,
        hook_snow_mud_upload as *mut c_void,
        hook_camera_wheel_update as *mut c_void,
        hook_camera_vertical_event_producer as *mut c_void,
        hook_camera_map_pick as *mut c_void,
        o05_building_capture::hook_update_provinces as *mut c_void,
        o05_building_capture::hook_building_constructor as *mut c_void,
        o05_building_capture::hook_building_vector_update as *mut c_void,
        o05_building_capture::hook_entity_transform_setter as *mut c_void,
    ];
    let originals = [
        &ORIGINAL_VFS_OPEN_CONSTRUCTOR,
        &ORIGINAL_WINDOW_BACKBUFFER_RTV_PRODUCER,
        &ORIGINAL_OPAQUE_TEXTURE_TARGET_PRODUCER,
        &ORIGINAL_TEXTURE_CACHE_LOAD_ROOT,
        &ORIGINAL_ENVELOPE,
        &ORIGINAL_OBJECT_CTOR,
        &ORIGINAL_BUCKET,
        &ORIGINAL_CREATE,
        &ORIGINAL_DESTROY,
        &ORIGINAL_BUILDER,
        &ORIGINAL_PARTICLE_RENDER_SUBMIT,
        &ORIGINAL_PARTICLE_FINAL_DRAW,
        &ORIGINAL_ENTITY_FACTORY,
        &ORIGINAL_ENTITY_NAMED_FACTORY,
        &ORIGINAL_ENTITY_DEFINITION_LOOKUP,
        &ORIGINAL_ENTITY_DESTROY,
        &ORIGINAL_ENTITY_RENDER,
        &ORIGINAL_ENTITY_FINAL_DRAW,
        &ORIGINAL_INDEXED_INSTANCED_FINAL_DRAW,
        &ORIGINAL_INSTANCED_FINAL_DRAW,
        &ORIGINAL_RENDER_TARGET_CLEAR,
        &ORIGINAL_SHADER_PROGRAM_BINDER,
        &ORIGINAL_ENGINE_SHADER_EFFECT_LOOKUP,
        &ORIGINAL_MAP_EFFECT_REGISTRATION_ROOT,
        &ORIGINAL_RAILROAD_EFFECT_REGISTRATION_ROOT,
        &ORIGINAL_MAP_NAME_EFFECT_REGISTRATION_ROOT,
        &ORIGINAL_RESTORE_BLOOM_EFFECT_REGISTRATION_ROOT,
        &ORIGINAL_LUT_BLEND_EFFECT_REGISTRATION_ROOT,
        &ORIGINAL_WEATHER_NORMAL_UPDATE_OWNER,
        &ORIGINAL_WEATHER_EDITOR_REFRESH_OWNER,
        &ORIGINAL_WEATHER,
        &ORIGINAL_WEATHER_ACTIVE_SETTER,
        &ORIGINAL_WEATHER_ALPHA_SETTER,
        &ORIGINAL_WEATHER_PLAYBACK_SETTER,
        &ORIGINAL_SEED,
        &ORIGINAL_MAP_SCENE_RENDER_ROOT,
        &ORIGINAL_POST_PROCESS_RENDER_ROOT,
        &ORIGINAL_PRESENT_RENDER_ROOT,
        &ORIGINAL_SNOW_MUD_PRODUCER,
        &ORIGINAL_SNOW_MUD_UPLOAD,
        &ORIGINAL_CAMERA_WHEEL_UPDATE,
        &ORIGINAL_CAMERA_VERTICAL_EVENT_PRODUCER,
        &ORIGINAL_CAMERA_MAP_PICK,
        &o05_building_capture::ORIGINAL_UPDATE_PROVINCES,
        &o05_building_capture::ORIGINAL_BUILDING_CONSTRUCTOR,
        &o05_building_capture::ORIGINAL_BUILDING_VECTOR_UPDATE,
        &o05_building_capture::ORIGINAL_ENTITY_TRANSFORM_SETTER,
    ];
    let targets = targets(a);
    std::array::from_fn(|index| (targets[index], detours[index], originals[index]))
}

fn engine_originals_are_clear(a: InstalledAddresses) -> bool {
    engine_hook_specs(a)
        .iter()
        .all(|(_, _, original)| original.load(Ordering::Acquire) == 0)
}

fn staged_hook_identity_labels() -> [&'static str; STAGED_HOOK_COUNT] {
    [
        "d3d.device.CreateInputLayout",
        "d3d.device.CreateVertexShader",
        "d3d.device.CreatePixelShader",
        "d3d.device.CreateGeometryShader",
        "d3d.device.CreateHullShader",
        "d3d.device.CreateDomainShader",
        "d3d.device.CreateComputeShader",
        "d3d.device.CreateBlendState",
        "d3d.device.CreateDepthStencilState",
        "d3d.device.CreateRasterizerState",
        "d3d.device.CreateSamplerState",
        "d3d.device.CreateBuffer",
        "d3d.device.CreateTexture1D",
        "d3d.device.CreateTexture2D",
        "d3d.device.CreateTexture3D",
        "d3d.device.CreateShaderResourceView",
        "d3d.device.CreateRenderTargetView",
        "d3d.device.CreateDepthStencilView",
        "d3d.context.SOSetTargets",
        "engine.VfsOpenConstructor",
        "engine.WindowBackbufferRtvProducer",
        "engine.OpaqueTextureTargetProducer",
        "engine.TextureCacheLoadRoot",
        "engine.GlobalRenderEnvelope",
        "engine.ParticleObjectConstructor",
        "engine.ParticleBucketSubmit",
        "engine.ParticleRuntimeCreate",
        "engine.ParticleRuntimeDestroy",
        "engine.ParticlePacketBuilder",
        "engine.ParticleRenderSubmit",
        "engine.ParticleFinalDraw",
        "engine.EntityFactory",
        "engine.EntityNamedFactory",
        "engine.EntityDefinitionLookup",
        "engine.EntityDestroy",
        "engine.EntityRenderRoot",
        "engine.EntityFinalDraw",
        "engine.IndexedInstancedFinalDraw",
        "engine.InstancedFinalDraw",
        "engine.RenderTargetClear",
        "engine.ShaderProgramBinder",
        "engine.ShaderEffectLookup",
        "engine.MapEffectRegistrationRoot",
        "engine.RailroadEffectRegistrationRoot",
        "engine.MapNameEffectRegistrationRoot",
        "engine.RestoreBloomEffectRegistrationRoot",
        "engine.LutBlendEffectRegistrationRoot",
        "engine.WeatherNormalUpdateOwner",
        "engine.WeatherEditorRefreshOwner",
        "engine.WeatherVisualProducer",
        "engine.WeatherEntityActiveSetter",
        "engine.WeatherEntityAlphaSetter",
        "engine.WeatherEntityPlaybackSetter",
        "engine.HighResolutionSeed",
        "engine.MapSceneRenderRoot",
        "engine.PostProcessRenderRoot",
        "engine.PresentRenderRoot",
        "engine.SnowMudProducer",
        "engine.SnowMudUpload",
        "engine.CameraWheelUpdate",
        "engine.CameraVerticalEventProducer",
        "engine.CameraMapPick",
        "engine.O05UpdateProvinces",
        "engine.O05BuildingConstructor",
        "engine.O05BuildingVectorUpdate",
        "engine.O05EntityTransformSetter",
    ]
}

fn staged_hook_group_bits() -> [u64; STAGED_HOOK_COUNT] {
    [
        STAGED_HOOK_GROUP_COM,
        STAGED_HOOK_GROUP_COM,
        STAGED_HOOK_GROUP_COM,
        STAGED_HOOK_GROUP_COM,
        STAGED_HOOK_GROUP_COM,
        STAGED_HOOK_GROUP_COM,
        STAGED_HOOK_GROUP_COM,
        STAGED_HOOK_GROUP_COM,
        STAGED_HOOK_GROUP_COM,
        STAGED_HOOK_GROUP_COM,
        STAGED_HOOK_GROUP_COM,
        STAGED_HOOK_GROUP_COM,
        STAGED_HOOK_GROUP_COM,
        STAGED_HOOK_GROUP_COM,
        STAGED_HOOK_GROUP_COM,
        STAGED_HOOK_GROUP_COM,
        STAGED_HOOK_GROUP_COM,
        STAGED_HOOK_GROUP_COM,
        STAGED_HOOK_GROUP_COM,
        STAGED_HOOK_GROUP_VFS_CONTENT,
        STAGED_HOOK_GROUP_RENDER_TARGETS,
        STAGED_HOOK_GROUP_RENDER_TARGETS,
        STAGED_HOOK_GROUP_VFS_CONTENT,
        STAGED_HOOK_GROUP_PARTICLE,
        STAGED_HOOK_GROUP_PARTICLE,
        STAGED_HOOK_GROUP_PARTICLE,
        STAGED_HOOK_GROUP_PARTICLE,
        STAGED_HOOK_GROUP_PARTICLE,
        STAGED_HOOK_GROUP_PARTICLE,
        STAGED_HOOK_GROUP_PARTICLE,
        STAGED_HOOK_GROUP_PARTICLE,
        STAGED_HOOK_GROUP_ENTITY,
        STAGED_HOOK_GROUP_ENTITY,
        STAGED_HOOK_GROUP_ENTITY,
        STAGED_HOOK_GROUP_ENTITY,
        STAGED_HOOK_GROUP_ENTITY,
        STAGED_HOOK_GROUP_ENTITY,
        STAGED_HOOK_GROUP_ENTITY,
        STAGED_HOOK_GROUP_ENTITY,
        STAGED_HOOK_GROUP_RENDER_TARGETS,
        STAGED_HOOK_GROUP_SHADER_EFFECTS,
        STAGED_HOOK_GROUP_SHADER_EFFECTS,
        STAGED_HOOK_GROUP_SHADER_EFFECTS,
        STAGED_HOOK_GROUP_SHADER_EFFECTS,
        STAGED_HOOK_GROUP_SHADER_EFFECTS,
        STAGED_HOOK_GROUP_SHADER_EFFECTS,
        STAGED_HOOK_GROUP_SHADER_EFFECTS,
        STAGED_HOOK_GROUP_WEATHER_SEED,
        STAGED_HOOK_GROUP_WEATHER_SEED,
        STAGED_HOOK_GROUP_WEATHER_SEED,
        STAGED_HOOK_GROUP_WEATHER_SEED,
        STAGED_HOOK_GROUP_WEATHER_SEED,
        STAGED_HOOK_GROUP_WEATHER_SEED,
        STAGED_HOOK_GROUP_WEATHER_SEED,
        STAGED_HOOK_GROUP_SCENE_BOUNDARIES,
        STAGED_HOOK_GROUP_SCENE_BOUNDARIES,
        STAGED_HOOK_GROUP_SCENE_BOUNDARIES,
        STAGED_HOOK_GROUP_SNOW_MUD,
        STAGED_HOOK_GROUP_SNOW_MUD,
        STAGED_HOOK_GROUP_CAMERA_EVENTS,
        STAGED_HOOK_GROUP_CAMERA_EVENTS,
        STAGED_HOOK_GROUP_CAMERA_EVENTS,
        STAGED_HOOK_GROUP_ENTITY,
        STAGED_HOOK_GROUP_ENTITY,
        STAGED_HOOK_GROUP_ENTITY,
        STAGED_HOOK_GROUP_ENTITY,
    ]
}

pub fn staged_hook_count_for_group_mask(group_mask: u64) -> Option<u32> {
    if group_mask & !PRODUCTION_STAGED_HOOK_GROUP_MASK != 0
        || group_mask & STAGED_HOOK_GROUP_COM == 0
    {
        return None;
    }
    Some(
        staged_hook_group_bits()
            .into_iter()
            .filter(|group| group_mask & group != 0)
            .count() as u32,
    )
}

fn staged_hook_set_receipt_sha256(
    process_id: u32,
    process_start_nonce: u64,
    hook_epoch_nonce: u64,
    bootstrap_hook_set_sha256: &str,
    game_device_ptr: usize,
    immediate_context_ptr: usize,
    generation: u64,
    diagnostic_stage_probe: bool,
    group_mask: u64,
    labels: &[&str],
    targets: &[usize],
) -> Result<String, String> {
    if process_id == 0
        || process_start_nonce == 0
        || hook_epoch_nonce == 0
        || bootstrap_hook_set_sha256.len() != 64
        || game_device_ptr == 0
        || immediate_context_ptr == 0
        || generation == 0
        || staged_hook_count_for_group_mask(group_mask) != Some(targets.len() as u32)
        || labels.len() != targets.len()
        || diagnostic_stage_probe == (group_mask == PRODUCTION_STAGED_HOOK_GROUP_MASK)
        || targets.iter().any(|target| *target == 0)
        || targets.iter().copied().collect::<BTreeSet<_>>().len() != targets.len()
    {
        return Err("staged hook-set receipt inputs are incomplete or duplicated".into());
    }
    let mut hasher = Sha256::new();
    hasher.update(b"hoi4-a729-staged-hook-set-v4\0");
    hasher.update(process_id.to_le_bytes());
    hasher.update(process_start_nonce.to_le_bytes());
    hasher.update(hook_epoch_nonce.to_le_bytes());
    hasher.update(bootstrap_hook_set_sha256.as_bytes());
    hasher.update((game_device_ptr as u64).to_le_bytes());
    hasher.update((immediate_context_ptr as u64).to_le_bytes());
    hasher.update(generation.to_le_bytes());
    hasher.update([u8::from(diagnostic_stage_probe)]);
    hasher.update(group_mask.to_le_bytes());
    hasher.update((targets.len() as u32).to_le_bytes());
    for (ordinal, (label, target)) in labels.iter().zip(targets).enumerate() {
        let label_len =
            u16::try_from(label.len()).map_err(|_| "staged hook identity is too long")?;
        hasher.update((ordinal as u32).to_le_bytes());
        hasher.update(label_len.to_le_bytes());
        hasher.update(label.as_bytes());
        hasher.update((*target as u64).to_le_bytes());
    }
    Ok(format!("{:x}", hasher.finalize()))
}

pub fn install() -> Result<InstalledAddresses, String> {
    let _lifecycle = HOOK_LIFECYCLE_LOCK
        .lock()
        .map_err(|_| "render-hook lifecycle mutex poisoned".to_string())?;
    if INSTALLED.load(Ordering::Acquire) {
        return Ok(addresses(MODULE_BASE.load(Ordering::Acquire)));
    }
    let base = unsafe { GetModuleHandleW(ptr::null()) } as usize;
    if base == 0 {
        return Err("GetModuleHandleW(NULL) returned null".into());
    }
    let expected_hour_root = base
        .checked_add(compact_contract::build_profile::CG_STATE_DATE_OFFSET)
        .ok_or("Present game-hour root slot address overflowed")?;
    if PRESENT_GAME_STATE_ROOT_SLOT.load(Ordering::Acquire) != expected_hour_root {
        return Err("authenticated Present game-hour root slot is not bound".into());
    }
    let a = addresses(base);
    for (address, sentinel, label) in [
        (
            a.vfs_open_constructor,
            &VFS_OPEN_CONSTRUCTOR_SENTINEL,
            "VFS archive-open constructor",
        ),
        (
            a.window_backbuffer_rtv_producer,
            &WINDOW_BACKBUFFER_RTV_PRODUCER_SENTINEL,
            "window backbuffer RTV producer",
        ),
        (
            a.opaque_texture_target_producer,
            &OPAQUE_TEXTURE_TARGET_PRODUCER_SENTINEL,
            "opaque texture target producer",
        ),
        (
            a.texture_cache_load_root,
            &TEXTURE_CACHE_LOAD_ROOT_SENTINEL,
            "texture cache load root",
        ),
        (
            a.global_envelope,
            &ENVELOPE_SENTINEL,
            "global render envelope",
        ),
        (
            a.object_ctor,
            &OBJECT_CTOR_SENTINEL,
            "particle object constructor",
        ),
        (a.bucket_submit, &BUCKET_SENTINEL, "particle bucket submit"),
        (
            a.runtime_create,
            &CREATE_SENTINEL,
            "particle runtime create",
        ),
        (
            a.runtime_destroy,
            &DESTROY_SENTINEL,
            "particle runtime destroy",
        ),
        (
            a.packet_builder,
            &BUILDER_SENTINEL,
            "particle packet builder",
        ),
        (
            a.particle_render_submit,
            &PARTICLE_RENDER_SUBMIT_SENTINEL,
            "particle render submit",
        ),
        (
            a.particle_final_draw,
            &PARTICLE_FINAL_DRAW_SENTINEL,
            "particle final draw",
        ),
        (a.entity_factory, &ENTITY_FACTORY_SENTINEL, "entity factory"),
        (
            a.entity_named_factory,
            &ENTITY_NAMED_FACTORY_SENTINEL,
            "named entity factory",
        ),
        (
            a.entity_definition_lookup,
            &ENTITY_DEFINITION_LOOKUP_SENTINEL,
            "named entity definition lookup",
        ),
        (a.entity_destroy, &ENTITY_DESTROY_SENTINEL, "entity destroy"),
        (
            a.entity_render_root,
            &ENTITY_RENDER_SENTINEL,
            "entity render root",
        ),
        (
            a.entity_final_draw,
            &ENTITY_FINAL_DRAW_SENTINEL,
            "entity final draw",
        ),
        (
            a.indexed_instanced_final_draw,
            &INSTANCED_FINAL_DRAW_SENTINEL,
            "indexed instanced final draw",
        ),
        (
            a.instanced_final_draw,
            &INSTANCED_FINAL_DRAW_SENTINEL,
            "instanced final draw",
        ),
        (
            a.render_target_clear,
            &RENDER_TARGET_CLEAR_SENTINEL,
            "render-target clear",
        ),
        (
            a.shader_program_binder,
            &SHADER_PROGRAM_BINDER_SENTINEL,
            "shader program binder",
        ),
        (
            a.engine_shader_effect_lookup,
            &ENGINE_SHADER_EFFECT_LOOKUP_SENTINEL,
            "engine shader/effect lookup",
        ),
        (
            a.map_effect_registration_root,
            &MAP_EFFECT_REGISTRATION_ROOT_SENTINEL,
            "map-effect registration root",
        ),
        (
            a.railroad_effect_registration_root,
            &RAILROAD_EFFECT_REGISTRATION_ROOT_SENTINEL,
            "railroad-effect registration root",
        ),
        (
            a.map_name_effect_registration_root,
            &MAP_NAME_EFFECT_REGISTRATION_ROOT_SENTINEL,
            "map-name-effect registration root",
        ),
        (
            a.restore_bloom_effect_registration_root,
            &RESTORE_BLOOM_EFFECT_REGISTRATION_ROOT_SENTINEL,
            "restore/bloom-effect registration root",
        ),
        (
            a.lut_blend_effect_registration_root,
            &LUT_BLEND_EFFECT_REGISTRATION_ROOT_SENTINEL,
            "LUT-blend-effect registration root",
        ),
        (
            a.weather_normal_update_owner,
            &WEATHER_NORMAL_UPDATE_OWNER_SENTINEL,
            "weather normal-update owner",
        ),
        (
            a.weather_editor_refresh_owner,
            &WEATHER_EDITOR_REFRESH_OWNER_SENTINEL,
            "weather editor-refresh owner",
        ),
        (
            a.weather_visual_producer,
            &WEATHER_SENTINEL,
            "weather visual producer",
        ),
        (
            a.weather_entity_active_setter,
            &WEATHER_ACTIVE_SETTER_SENTINEL,
            "weather entity-active setter",
        ),
        (
            a.weather_entity_alpha_setter,
            &WEATHER_ALPHA_SETTER_SENTINEL,
            "weather entity-alpha setter",
        ),
        (
            a.weather_entity_playback_setter,
            &WEATHER_PLAYBACK_SETTER_SENTINEL,
            "weather entity-playback setter",
        ),
        (
            a.high_resolution_seed,
            &SEED_SENTINEL,
            "high-resolution seed",
        ),
        (
            a.map_scene_render_root,
            &MAP_SCENE_RENDER_ROOT_SENTINEL,
            "map scene render root",
        ),
        (
            a.post_process_render_root,
            &POST_PROCESS_RENDER_ROOT_SENTINEL,
            "post-process render root",
        ),
        (
            a.present_render_root,
            &PRESENT_RENDER_ROOT_SENTINEL,
            "present render root",
        ),
        (
            a.snow_mud_producer,
            &SNOW_MUD_PRODUCER_SENTINEL,
            "SnowMudData CPU producer",
        ),
        (
            a.snow_mud_upload,
            &SNOW_MUD_UPLOAD_SENTINEL,
            "SnowMudData upload helper",
        ),
        (
            a.camera_wheel_update,
            &CAMERA_WHEEL_UPDATE_SENTINEL,
            "camera wheel update producer",
        ),
        (
            a.camera_vertical_event_producer,
            &CAMERA_VERTICAL_EVENT_PRODUCER_SENTINEL,
            "camera vertical event producer",
        ),
        (
            a.camera_map_pick,
            &CAMERA_MAP_PICK_SENTINEL,
            "camera screen-to-map picker",
        ),
    ] {
        verify_sentinel(address, sentinel, label)?;
    }
    o05_building_capture::verify_sentinels(base)?;
    STATE.get_or_init(|| Mutex::new(HookState::default()));
    let _deferred_engine_specs = [
        (
            a.vfs_open_constructor,
            hook_vfs_open_constructor as *mut c_void,
            &ORIGINAL_VFS_OPEN_CONSTRUCTOR,
        ),
        (
            a.window_backbuffer_rtv_producer,
            hook_window_backbuffer_rtv_producer as *mut c_void,
            &ORIGINAL_WINDOW_BACKBUFFER_RTV_PRODUCER,
        ),
        (
            a.opaque_texture_target_producer,
            hook_opaque_texture_target_producer as *mut c_void,
            &ORIGINAL_OPAQUE_TEXTURE_TARGET_PRODUCER,
        ),
        (
            a.texture_cache_load_root,
            hook_texture_cache_load_root as *mut c_void,
            &ORIGINAL_TEXTURE_CACHE_LOAD_ROOT,
        ),
        (
            a.global_envelope,
            hook_envelope as *mut c_void,
            &ORIGINAL_ENVELOPE,
        ),
        (
            a.object_ctor,
            hook_object_ctor as *mut c_void,
            &ORIGINAL_OBJECT_CTOR,
        ),
        (
            a.bucket_submit,
            hook_bucket as *mut c_void,
            &ORIGINAL_BUCKET,
        ),
        (
            a.runtime_create,
            hook_create as *mut c_void,
            &ORIGINAL_CREATE,
        ),
        (
            a.runtime_destroy,
            hook_destroy as *mut c_void,
            &ORIGINAL_DESTROY,
        ),
        (
            a.packet_builder,
            hook_builder as *mut c_void,
            &ORIGINAL_BUILDER,
        ),
        (
            a.particle_render_submit,
            hook_particle_render_submit as *mut c_void,
            &ORIGINAL_PARTICLE_RENDER_SUBMIT,
        ),
        (
            a.particle_final_draw,
            hook_particle_final_draw as *mut c_void,
            &ORIGINAL_PARTICLE_FINAL_DRAW,
        ),
        (
            a.entity_factory,
            hook_entity_factory as *mut c_void,
            &ORIGINAL_ENTITY_FACTORY,
        ),
        (
            a.entity_named_factory,
            hook_entity_named_factory as *mut c_void,
            &ORIGINAL_ENTITY_NAMED_FACTORY,
        ),
        (
            a.entity_definition_lookup,
            hook_entity_definition_lookup as *mut c_void,
            &ORIGINAL_ENTITY_DEFINITION_LOOKUP,
        ),
        (
            a.entity_destroy,
            hook_entity_destroy as *mut c_void,
            &ORIGINAL_ENTITY_DESTROY,
        ),
        (
            a.entity_render_root,
            hook_entity_render as *mut c_void,
            &ORIGINAL_ENTITY_RENDER,
        ),
        (
            a.entity_final_draw,
            hook_entity_final_draw as *mut c_void,
            &ORIGINAL_ENTITY_FINAL_DRAW,
        ),
        (
            a.indexed_instanced_final_draw,
            hook_indexed_instanced_final_draw as *mut c_void,
            &ORIGINAL_INDEXED_INSTANCED_FINAL_DRAW,
        ),
        (
            a.instanced_final_draw,
            hook_instanced_final_draw as *mut c_void,
            &ORIGINAL_INSTANCED_FINAL_DRAW,
        ),
        (
            a.render_target_clear,
            hook_render_target_clear as *mut c_void,
            &ORIGINAL_RENDER_TARGET_CLEAR,
        ),
        (
            a.shader_program_binder,
            hook_shader_program_binder as *mut c_void,
            &ORIGINAL_SHADER_PROGRAM_BINDER,
        ),
        (
            a.engine_shader_effect_lookup,
            hook_engine_shader_effect_lookup as *mut c_void,
            &ORIGINAL_ENGINE_SHADER_EFFECT_LOOKUP,
        ),
        (
            a.map_effect_registration_root,
            hook_map_effect_registration_root as *mut c_void,
            &ORIGINAL_MAP_EFFECT_REGISTRATION_ROOT,
        ),
        (
            a.railroad_effect_registration_root,
            hook_railroad_effect_registration_root as *mut c_void,
            &ORIGINAL_RAILROAD_EFFECT_REGISTRATION_ROOT,
        ),
        (
            a.map_name_effect_registration_root,
            hook_map_name_effect_registration_root as *mut c_void,
            &ORIGINAL_MAP_NAME_EFFECT_REGISTRATION_ROOT,
        ),
        (
            a.restore_bloom_effect_registration_root,
            hook_restore_bloom_effect_registration_root as *mut c_void,
            &ORIGINAL_RESTORE_BLOOM_EFFECT_REGISTRATION_ROOT,
        ),
        (
            a.lut_blend_effect_registration_root,
            hook_lut_blend_effect_registration_root as *mut c_void,
            &ORIGINAL_LUT_BLEND_EFFECT_REGISTRATION_ROOT,
        ),
        (
            a.weather_normal_update_owner,
            hook_weather_normal_update_owner as *mut c_void,
            &ORIGINAL_WEATHER_NORMAL_UPDATE_OWNER,
        ),
        (
            a.weather_editor_refresh_owner,
            hook_weather_editor_refresh_owner as *mut c_void,
            &ORIGINAL_WEATHER_EDITOR_REFRESH_OWNER,
        ),
        (
            a.weather_visual_producer,
            hook_weather as *mut c_void,
            &ORIGINAL_WEATHER,
        ),
        (
            a.weather_entity_active_setter,
            hook_weather_active_setter as *mut c_void,
            &ORIGINAL_WEATHER_ACTIVE_SETTER,
        ),
        (
            a.weather_entity_alpha_setter,
            hook_weather_alpha_setter as *mut c_void,
            &ORIGINAL_WEATHER_ALPHA_SETTER,
        ),
        (
            a.weather_entity_playback_setter,
            hook_weather_playback_setter as *mut c_void,
            &ORIGINAL_WEATHER_PLAYBACK_SETTER,
        ),
        (
            a.high_resolution_seed,
            hook_seed as *mut c_void,
            &ORIGINAL_SEED,
        ),
        (
            a.map_scene_render_root,
            hook_map_scene_render_root as *mut c_void,
            &ORIGINAL_MAP_SCENE_RENDER_ROOT,
        ),
        (
            a.post_process_render_root,
            hook_post_process_render_root as *mut c_void,
            &ORIGINAL_POST_PROCESS_RENDER_ROOT,
        ),
        (
            a.present_render_root,
            hook_present_render_root as *mut c_void,
            &ORIGINAL_PRESENT_RENDER_ROOT,
        ),
        (
            a.snow_mud_producer,
            hook_snow_mud_producer as *mut c_void,
            &ORIGINAL_SNOW_MUD_PRODUCER,
        ),
        (
            a.snow_mud_upload,
            hook_snow_mud_upload as *mut c_void,
            &ORIGINAL_SNOW_MUD_UPLOAD,
        ),
        (
            a.camera_wheel_update,
            hook_camera_wheel_update as *mut c_void,
            &ORIGINAL_CAMERA_WHEEL_UPDATE,
        ),
        (
            a.camera_vertical_event_producer,
            hook_camera_vertical_event_producer as *mut c_void,
            &ORIGINAL_CAMERA_VERTICAL_EVENT_PRODUCER,
        ),
        (
            a.camera_map_pick,
            hook_camera_map_pick as *mut c_void,
            &ORIGINAL_CAMERA_MAP_PICK,
        ),
        (
            a.o05_update_provinces,
            o05_building_capture::hook_update_provinces as *mut c_void,
            &o05_building_capture::ORIGINAL_UPDATE_PROVINCES,
        ),
        (
            a.o05_building_constructor,
            o05_building_capture::hook_building_constructor as *mut c_void,
            &o05_building_capture::ORIGINAL_BUILDING_CONSTRUCTOR,
        ),
        (
            a.o05_building_vector_update,
            o05_building_capture::hook_building_vector_update as *mut c_void,
            &o05_building_capture::ORIGINAL_BUILDING_VECTOR_UPDATE,
        ),
        (
            a.o05_entity_transform_setter,
            o05_building_capture::hook_entity_transform_setter as *mut c_void,
            &o05_building_capture::ORIGINAL_ENTITY_TRANSFORM_SETTER,
        ),
    ];
    if !engine_originals_are_clear(a) {
        return Err("engine hook originals were populated before bootstrap install".into());
    }
    let bootstrap_specs = bootstrap_hook_specs()?;
    let bootstrap_targets = create_and_enable_hooks(&bootstrap_specs, "bootstrap export")?;
    if let Err(error) = initialize_early_capture_epoch(&bootstrap_targets) {
        let rollback = rollback_created_hooks(&bootstrap_specs, bootstrap_specs.len())
            .err()
            .map(|rollback| format!("; {rollback}"))
            .unwrap_or_default();
        return Err(format!("{error}{rollback}"));
    }
    *BOOTSTRAP_HOOK_TARGETS
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .map_err(|_| "bootstrap hook target state poisoned")? = bootstrap_targets;
    STAGED_HOOK_TARGETS
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .map_err(|_| "staged hook target state poisoned")?
        .clear();
    *HOOK_STAGE
        .get_or_init(|| Mutex::new(HookStage::Bootstrap))
        .lock()
        .map_err(|_| "hook stage state poisoned")? = HookStage::Bootstrap;
    MODULE_BASE.store(base, Ordering::Release);
    INSTALLED.store(true, Ordering::Release);
    if let Err(error) = a25_gui_bridge::install(base) {
        let cleanup = uninstall_locked()
            .err()
            .map(|cleanup| format!("; render-hook rollback failed: {cleanup}"))
            .unwrap_or_default();
        return Err(format!("A25 evidence-only GUI bridge install failed: {error}{cleanup}"));
    }
    Ok(a)
}

pub fn uninstall() -> Result<(), String> {
    let _lifecycle = HOOK_LIFECYCLE_LOCK
        .lock()
        .map_err(|_| "render-hook lifecycle mutex poisoned".to_string())?;
    uninstall_locked()
}

fn uninstall_locked() -> Result<(), String> {
    if !INSTALLED.load(Ordering::Acquire) {
        crate::teardown_gui_paint_instrumentation()?;
        return Ok(());
    }
    let staged_targets = STAGED_HOOK_TARGETS
        .get()
        .and_then(|value| value.lock().ok().map(|value| value.clone()))
        .unwrap_or_default();
    let bootstrap_targets = BOOTSTRAP_HOOK_TARGETS
        .get()
        .and_then(|value| value.lock().ok().map(|value| value.clone()))
        .unwrap_or_default();
    let all_targets: Vec<_> = staged_targets
        .iter()
        .copied()
        .chain(bootstrap_targets.iter().copied())
        .collect();
    let includes_com = !staged_targets.is_empty();
    if includes_com {
        begin_com_detour_teardown();
    }
    let mut failures = Vec::new();
    if let Err(error) = a25_gui_bridge::uninstall() {
        return Err(format!(
            "A25 evidence-only GUI bridge teardown failed: {error}"
        ));
    }
    let mut disable_failed = false;
    for target in all_targets.iter().copied() {
        match unsafe { MinHook::disable_hook(target as *mut c_void) } {
            Ok(()) | Err(MH_STATUS::MH_ERROR_DISABLED) | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => {}
            Err(status) => {
                disable_failed = true;
                failures.push(format!("{target:#x} hook disable failed: {status}"));
            }
        }
    }
    if disable_failed {
        return Err(failures.join(", "));
    }
    if includes_com {
        if let Err(error) = wait_for_com_detour_quiescence() {
            let mut message = error;
            if let Err(teardown_error) = crate::teardown_gui_paint_instrumentation() {
                message.push_str(&format!(
                    "; GUI paint instrumentation teardown failed: {teardown_error}"
                ));
            }
            return Err(message);
        }
    }
    let mut removal_failed = false;
    for target in all_targets.iter().copied() {
        match unsafe { MinHook::remove_hook(target as *mut c_void) } {
            Ok(()) | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => {}
            Err(status) => {
                removal_failed = true;
                failures.push(format!("{target:#x} hook removal failed: {status}"));
            }
        }
    }
    if removal_failed {
        if let Err(error) = crate::teardown_gui_paint_instrumentation() {
            failures.push(format!(
                "GUI paint instrumentation teardown failed: {error}"
            ));
        }
        return Err(failures.join(", "));
    }
    if let Err(error) = crate::teardown_gui_paint_instrumentation() {
        failures.push(format!(
            "GUI paint instrumentation teardown failed: {error}"
        ));
    }
    INSTALLED.store(false, Ordering::Release);
    notify_render_submit_state_change();
    MODULE_BASE.store(0, Ordering::Release);
    PRESENT_GAME_STATE_ROOT_SLOT.store(0, Ordering::Release);
    NEXT_MAP_SHADER_LOOKUP_SEQUENCE.store(1, Ordering::Release);
    NEXT_RENDER_ORDER_SEQUENCE.store(1, Ordering::Release);
    NEXT_RENDER_FRAME_SEQUENCE.store(1, Ordering::Release);
    NEXT_PRESENT_GLOBAL_ORDER.store(0, Ordering::Release);
    NEXT_SNOW_MUD_PRODUCER_ORDINAL.store(1, Ordering::Release);
    NEXT_OCCUPATION_SECONDARY_UPLOAD_ORDER.store(1, Ordering::Release);
    NEXT_CATALOG_OBSERVATION_SEQUENCE.store(1, Ordering::Release);
    NEXT_WEATHER_UPDATE_INDEX.store(1, Ordering::Release);
    for value in [
        &ORIGINAL_VFS_OPEN_CONSTRUCTOR,
        &ORIGINAL_WINDOW_BACKBUFFER_RTV_PRODUCER,
        &ORIGINAL_OPAQUE_TEXTURE_TARGET_PRODUCER,
        &ORIGINAL_TEXTURE_CACHE_LOAD_ROOT,
        &ORIGINAL_ENVELOPE,
        &ORIGINAL_OBJECT_CTOR,
        &ORIGINAL_BUCKET,
        &ORIGINAL_CREATE,
        &ORIGINAL_DESTROY,
        &ORIGINAL_BUILDER,
        &ORIGINAL_PARTICLE_RENDER_SUBMIT,
        &ORIGINAL_PARTICLE_FINAL_DRAW,
        &ORIGINAL_ENTITY_FACTORY,
        &ORIGINAL_ENTITY_NAMED_FACTORY,
        &ORIGINAL_ENTITY_DEFINITION_LOOKUP,
        &ORIGINAL_ENTITY_DESTROY,
        &ORIGINAL_ENTITY_RENDER,
        &ORIGINAL_ENTITY_FINAL_DRAW,
        &ORIGINAL_INDEXED_INSTANCED_FINAL_DRAW,
        &ORIGINAL_INSTANCED_FINAL_DRAW,
        &ORIGINAL_RENDER_TARGET_CLEAR,
        &ORIGINAL_SHADER_PROGRAM_BINDER,
        &ORIGINAL_ENGINE_SHADER_EFFECT_LOOKUP,
        &ORIGINAL_MAP_EFFECT_REGISTRATION_ROOT,
        &ORIGINAL_RAILROAD_EFFECT_REGISTRATION_ROOT,
        &ORIGINAL_MAP_NAME_EFFECT_REGISTRATION_ROOT,
        &ORIGINAL_RESTORE_BLOOM_EFFECT_REGISTRATION_ROOT,
        &ORIGINAL_LUT_BLEND_EFFECT_REGISTRATION_ROOT,
        &ORIGINAL_WEATHER_NORMAL_UPDATE_OWNER,
        &ORIGINAL_WEATHER_EDITOR_REFRESH_OWNER,
        &ORIGINAL_WEATHER,
        &ORIGINAL_WEATHER_ACTIVE_SETTER,
        &ORIGINAL_WEATHER_ALPHA_SETTER,
        &ORIGINAL_WEATHER_PLAYBACK_SETTER,
        &ORIGINAL_SEED,
        &ORIGINAL_MAP_SCENE_RENDER_ROOT,
        &ORIGINAL_POST_PROCESS_RENDER_ROOT,
        &ORIGINAL_PRESENT_RENDER_ROOT,
        &ORIGINAL_SNOW_MUD_PRODUCER,
        &ORIGINAL_SNOW_MUD_UPLOAD,
        &ORIGINAL_CAMERA_WHEEL_UPDATE,
        &ORIGINAL_CAMERA_VERTICAL_EVENT_PRODUCER,
        &ORIGINAL_CAMERA_MAP_PICK,
        &o05_building_capture::ORIGINAL_UPDATE_PROVINCES,
        &o05_building_capture::ORIGINAL_BUILDING_CONSTRUCTOR,
        &o05_building_capture::ORIGINAL_BUILDING_VECTOR_UPDATE,
        &o05_building_capture::ORIGINAL_ENTITY_TRANSFORM_SETTER,
        &ORIGINAL_D3D_CREATE_INPUT_LAYOUT,
        &ORIGINAL_D3D_CREATE_VERTEX_SHADER,
        &ORIGINAL_D3D_CREATE_PIXEL_SHADER,
        &ORIGINAL_D3D_CREATE_GEOMETRY_SHADER,
        &ORIGINAL_D3D_CREATE_HULL_SHADER,
        &ORIGINAL_D3D_CREATE_DOMAIN_SHADER,
        &ORIGINAL_D3D_CREATE_COMPUTE_SHADER,
        &ORIGINAL_D3D_CREATE_BLEND_STATE,
        &ORIGINAL_D3D_CREATE_DEPTH_STENCIL_STATE,
        &ORIGINAL_D3D_CREATE_RASTERIZER_STATE,
        &ORIGINAL_D3D_CREATE_SAMPLER_STATE,
        &ORIGINAL_D3D_CREATE_BUFFER,
        &ORIGINAL_D3D_CREATE_TEXTURE1D,
        &ORIGINAL_D3D_CREATE_TEXTURE2D,
        &ORIGINAL_D3D_CREATE_TEXTURE3D,
        &ORIGINAL_D3D_CREATE_SRV,
        &ORIGINAL_D3D_CREATE_RTV,
        &ORIGINAL_D3D_CREATE_DSV,
        &ORIGINAL_D3D_SO_SET_TARGETS,
        &ORIGINAL_D3D11_CREATE_DEVICE,
        &ORIGINAL_D3D_COMPILE,
    ] {
        value.store(0, Ordering::Release);
    }
    if includes_com {
        finish_com_detour_teardown();
    }
    if let Some(state) = STATE.get() {
        *state.lock().map_err(|_| "render hook state poisoned")? = HookState::default();
    }
    o05_building_capture::reset();
    if let Some(targets) = STAGED_HOOK_TARGETS.get() {
        targets
            .lock()
            .map_err(|_| "staged hook target state poisoned")?
            .clear();
    }
    if let Some(targets) = BOOTSTRAP_HOOK_TARGETS.get() {
        targets
            .lock()
            .map_err(|_| "bootstrap hook target state poisoned")?
            .clear();
    }
    if let Some(stage) = HOOK_STAGE.get() {
        *stage.lock().map_err(|_| "hook stage state poisoned")? = HookStage::Bootstrap;
    }
    PENDING_STAGE_FIRST_CREATION.store(0, Ordering::Release);
    if let Some(epoch) = EARLY_CAPTURE_EPOCH.get() {
        *epoch
            .lock()
            .map_err(|_| "early capture epoch state poisoned")? = EarlyCaptureEpochState::default();
    }
    if let Some(roster) = RUNTIME_DLC_ROSTER.get() {
        *roster
            .lock()
            .map_err(|_| "runtime DLC roster state poisoned")? = None;
    }
    CURRENT_SUBMIT.with(|value| *value.borrow_mut() = None);
    CURRENT_PARTICLE_DRAW_RANGE.with(|value| *value.borrow_mut() = None);
    CURRENT_ENTITY_DRAW_RANGE.with(|value| *value.borrow_mut() = None);
    CURRENT_RENDER_PROGRAM.with(|value| *value.borrow_mut() = None);
    BOUND_RENDER_PROGRAMS.with(|value| value.borrow_mut().clear());
    CURRENT_MAP_SCENE_PHASE.with(|value| *value.borrow_mut() = false);
    CURRENT_POST_PROCESS_PHASE.with(|value| *value.borrow_mut() = false);
    CURRENT_MAP_CONTENT_REGISTRATION_ROOT.with(|value| *value.borrow_mut() = None);
    TEXTURE_LOAD_STACK.with(|value| value.borrow_mut().clear());
    ENTITY_FACTORY_STACK.with(|value| value.borrow_mut().clear());
    CURRENT_WEATHER.with(|value| *value.borrow_mut() = None);
    CURRENT_WEATHER_OWNER.with(|value| *value.borrow_mut() = None);
    CURRENT_SNOW_MUD_PRODUCER.with(|value| *value.borrow_mut() = None);
    CURRENT_CAMERA_EVENT_PRODUCER.with(|value| *value.borrow_mut() = None);
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join(", "))
    }
}

/// Drains only a complete, authenticated queue. Any observed omission or
/// eviction rejects the drain rather than returning a deceptively empty frame.
#[cfg(test)]
pub fn drain_complete_submits() -> Result<Vec<RawParticleSubmit>, String> {
    if !INSTALLED.load(Ordering::Acquire) {
        return Err("a729 render hooks are not installed".into());
    }
    let mut state = STATE
        .get()
        .ok_or("render hook state is unavailable")?
        .lock()
        .map_err(|_| "render hook state poisoned")?;
    if completeness_violated(&state) {
        return Err(completeness_message(&state));
    }
    let output: Vec<_> = state.completed.drain(..).collect();
    // This compatibility drain intentionally consumes the paired raw catalogs
    // too. Normal production must use `drain_complete_submit_captures` so a
    // submit can never be promoted without its immutable descriptor evidence.
    state.completed_catalogs.clear();
    Ok(output)
}

/// Atomically drains each runtime submit with the exact immutable D3D capture
/// observed at its authenticated final draws. A split/misaligned queue fails
/// closed instead of pairing unrelated process-local observations.
pub fn drain_complete_submit_captures() -> Result<Vec<RawRenderSubmitCapture>, String> {
    if !INSTALLED.load(Ordering::Acquire) {
        return Err("a729 render hooks are not installed".into());
    }
    let mut state = STATE
        .get()
        .ok_or("render hook state is unavailable")?
        .lock()
        .map_err(|_| "render hook state poisoned")?;
    if completeness_violated(&state) {
        return Err(completeness_message(&state));
    }
    if state.completed.len() != state.completed_catalogs.len() {
        return Err("runtime submit/catalog queues are misaligned".into());
    }
    let submits: Vec<_> = state.completed.drain(..).collect();
    let catalogs: Vec<_> = state.completed_catalogs.drain(..).collect();
    submits
        .into_iter()
        .zip(catalogs)
        .map(|(submit, catalog)| {
            let shader_reflections = build_live_shader_reflections(&catalog)?;
            Ok(RawRenderSubmitCapture {
                submit,
                catalog,
                shader_reflections,
            })
        })
        .collect()
}

/// Drains exact successful archive opens accumulated since the previous
/// collector poll. No opened payload bytes are retained in hook state.
pub fn drain_vfs_open_receipts() -> Result<Vec<RawVfsOpenReceipt>, String> {
    if !INSTALLED.load(Ordering::Acquire) {
        return Err("a729 render hooks are not installed".into());
    }
    let mut state = STATE
        .get()
        .ok_or("render hook state is unavailable")?
        .lock()
        .map_err(|_| "render hook state poisoned")?;
    if completeness_violated(&state) {
        return Err(completeness_message(&state));
    }
    Ok(state.vfs_open_receipts.drain(..).collect())
}

/// Atomically drains only render frames whose exact runtime submit/catalog
/// mate has also been published. A Present can complete before the enclosing
/// render envelope publishes its runtime capture; that ordinary publication
/// gap returns no row and retains the unmatched observations for the next
/// drain.
pub fn drain_complete_render_frame_submit_captures() -> Result<RawJoinedRenderFrameDrain, String> {
    if !INSTALLED.load(Ordering::Acquire) {
        return Err("a729 render hooks are not installed".into());
    }
    let mut state = STATE
        .get()
        .ok_or("render hook state is unavailable")?
        .lock()
        .map_err(|_| "render hook state poisoned")?;
    drain_complete_render_frame_submit_captures_locked(&mut state)
}

fn drain_complete_render_frame_submit_captures_locked(
    state: &mut HookState,
) -> Result<RawJoinedRenderFrameDrain, String> {
    if completeness_violated(state) {
        return Err(completeness_message(state));
    }
    if state.completed.len() != state.completed_catalogs.len() {
        return Err("runtime submit/catalog queues are misaligned".into());
    }
    let mut runtime_by_submit = BTreeMap::new();
    for (submit, catalog) in state.completed.iter().zip(&state.completed_catalogs) {
        if submit.submit_index != catalog.submit_index {
            return Err(format!(
                "runtime submit/catalog id mismatch: submit {} catalog {}",
                submit.submit_index, catalog.submit_index
            ));
        }
        if runtime_by_submit
            .insert(submit.submit_index, (submit, catalog))
            .is_some()
        {
            return Err(format!(
                "duplicate runtime submit/catalog id {}",
                submit.submit_index
            ));
        }
    }

    let mut frame_ids = BTreeSet::new();
    for frame in &state.completed_render_frames {
        if !frame_ids.insert(frame.submit_index) {
            return Err(format!(
                "duplicate completed render frame submit id {}",
                frame.submit_index
            ));
        }
    }

    // Build the complete output before mutating any queue. Reflection failure
    // therefore leaves every source observation available for diagnosis.
    let mut output = Vec::new();
    let mut drained_ids = BTreeSet::new();
    for frame in &state.completed_render_frames {
        let Some((submit, catalog)) = runtime_by_submit.get(&frame.submit_index) else {
            continue;
        };
        let shader_reflections = build_live_shader_reflections(catalog)?;
        output.push(RawJoinedRenderFrameCapture {
            frame: frame.clone(),
            runtime: RawRenderSubmitCapture {
                submit: (*submit).clone(),
                catalog: (*catalog).clone(),
                shader_reflections,
            },
        });
        drained_ids.insert(frame.submit_index);
    }

    if drained_ids.is_empty() {
        if state.completed_render_frames.is_empty()
            && state.completed.is_empty()
            && state.completed_catalogs.is_empty()
        {
            return Ok(RawJoinedRenderFrameDrain::EmptyNoCapture);
        }
        return Ok(RawJoinedRenderFrameDrain::PendingIncomplete {
            publication_generation: state.runtime_publication_generation,
            completed_frame_count: state.completed_render_frames.len(),
            runtime_submit_count: state.completed.len(),
        });
    }
    state
        .completed_render_frames
        .retain(|frame| !drained_ids.contains(&frame.submit_index));
    state
        .completed
        .retain(|submit| !drained_ids.contains(&submit.submit_index));
    state
        .completed_catalogs
        .retain(|catalog| !drained_ids.contains(&catalog.submit_index));
    Ok(RawJoinedRenderFrameDrain::Ready(output))
}

/// Atomically latches the current QPC cutoff and selects exactly the last
/// completed Present for one authenticated compact hour. Querying QPC while
/// holding `HookState` is part of the law: a post-cutoff Present cannot enter
/// online compaction between cutoff capture and selection. If an eligible
/// Present is still waiting for its runtime/catalog mate, the same cutoff is
/// retained across retries and later publications are classified against it.
pub fn latch_selected_render_frame_submit_capture(
    target_game_hour_raw: i32,
) -> Result<RawSelectedRenderFrameDrain, String> {
    if !INSTALLED.load(Ordering::Acquire) {
        return Err("a729 render hooks are not installed".into());
    }
    let mut state = STATE
        .get()
        .ok_or("render hook state is unavailable")?
        .lock()
        .map_err(|_| "render hook state poisoned")?;
    let cutoff = match state.latched_render_hour {
        Some(latched) => {
            if latched.game_hour_raw != target_game_hour_raw {
                return Err("another render hour already owns the atomic cutoff latch".into());
            }
            latched
        }
        None => {
            let cutoff = SealedRenderHour {
                game_hour_raw: target_game_hour_raw,
                cutoff_qpc_ticks: query_performance_counter()
                    .ok_or("QueryPerformanceCounter failed at render-hour latch")?,
                cutoff_qpc_frequency_hz: query_performance_frequency()
                    .ok_or("QueryPerformanceFrequency failed at render-hour latch")?,
            };
            state.latched_render_hour = Some(cutoff);
            cutoff
        }
    };
    latch_selected_render_frame_submit_capture_locked(&mut state, cutoff)
}

fn latch_selected_render_frame_submit_capture_locked(
    state: &mut HookState,
    cutoff: SealedRenderHour,
) -> Result<RawSelectedRenderFrameDrain, String> {
    match state.latched_render_hour {
        Some(existing) if existing != cutoff => {
            return Err("render-hour cutoff latch changed across selection retries".into())
        }
        Some(_) => {}
        None => state.latched_render_hour = Some(cutoff),
    }
    let result = drain_selected_render_frame_submit_capture_locked(
        state,
        cutoff.game_hour_raw,
        cutoff.cutoff_qpc_ticks,
        cutoff.cutoff_qpc_frequency_hz,
    )?;
    if matches!(
        result,
        RawSelectedRenderFrameDrain::Ready(_)
            | RawSelectedRenderFrameDrain::EmptyNoCandidate { .. }
    ) {
        if state.sealed_render_hour != Some(cutoff) {
            return Err("selected render hour did not seal its atomic cutoff".into());
        }
        state.latched_render_hour = None;
    }
    Ok(result)
}

/// Drains fixed-size audit telemetry for rows that published only after their
/// compact hour had already been sealed. Exact identities were validated and
/// retired atomically at publication, then folded into this rolling digest;
/// no per-Present queue can accumulate while the game is paused.
pub fn drain_completed_sealed_render_purge_receipts() -> Result<SealedRenderPurgeTelemetry, String>
{
    if !INSTALLED.load(Ordering::Acquire) {
        return Err("a729 render hooks are not installed".into());
    }
    let mut state = STATE
        .get()
        .ok_or("render hook state is unavailable")?
        .lock()
        .map_err(|_| "render hook state poisoned")?;
    if completeness_violated(&state) {
        return Err(completeness_message(&state));
    }
    let telemetry = SealedRenderPurgeTelemetry {
        receipt_count: state.sealed_purge_receipt_count,
        receipt_count_overflowed: state.sealed_purge_receipt_count_overflowed,
        rolling_receipt_sha256: sha256_bytes_hex(&state.sealed_purge_rolling_sha256),
    };
    state.sealed_purge_receipt_count = 0;
    state.sealed_purge_receipt_count_overflowed = false;
    state.sealed_purge_rolling_sha256 = [0; 32];
    Ok(telemetry)
}

fn drain_selected_render_frame_submit_capture_locked(
    state: &mut HookState,
    target_game_hour_raw: i32,
    cutoff_qpc_ticks: u64,
    cutoff_qpc_frequency_hz: u64,
) -> Result<RawSelectedRenderFrameDrain, String> {
    if completeness_violated(state) {
        return Err(completeness_message(state));
    }
    compact_contract::calendar::hour_index_from_raw(target_game_hour_raw)
        .ok_or("selected render-frame target hour is noncanonical")?;
    if cutoff_qpc_ticks == 0 || cutoff_qpc_frequency_hz == 0 {
        return Err("selected render-frame QPC cutoff is null".into());
    }
    if state
        .sealed_render_hour
        .is_some_and(|sealed| target_game_hour_raw <= sealed.game_hour_raw)
    {
        return Err("selected render-frame target hour is already sealed or reordered".into());
    }
    if state.completed.len() != state.completed_catalogs.len() {
        return Err("runtime submit/catalog queues are misaligned".into());
    }
    let online_proof = state
        .online_candidate_proofs
        .get(&target_game_hour_raw)
        .cloned();

    let mut runtime_by_submit = BTreeMap::new();
    for (submit, catalog) in state.completed.iter().zip(&state.completed_catalogs) {
        if submit.submit_index != catalog.submit_index {
            return Err(format!(
                "runtime submit/catalog id mismatch: submit {} catalog {}",
                submit.submit_index, catalog.submit_index
            ));
        }
        if runtime_by_submit
            .insert(submit.submit_index, (submit, catalog))
            .is_some()
        {
            return Err(format!(
                "duplicate runtime submit/catalog id {}",
                submit.submit_index
            ));
        }
    }

    let mut frame_ids = BTreeSet::new();
    let mut target_frame_count = 0usize;
    let mut target_missing_runtime_count = 0usize;
    for frame in &state.completed_render_frames {
        if !frame_ids.insert(frame.submit_index) {
            return Err(format!(
                "duplicate completed render frame submit id {}",
                frame.submit_index
            ));
        }
        if frame.game_hour_raw == target_game_hour_raw
            && frame.presentation_qpc_ticks <= cutoff_qpc_ticks
        {
            target_frame_count += 1;
            if frame.presentation_qpc_frequency_hz != cutoff_qpc_frequency_hz {
                return Err("selected render-frame QPC frequency differs from cutoff epoch".into());
            }
            if !runtime_by_submit.contains_key(&frame.submit_index) {
                target_missing_runtime_count += 1;
            }
        }
    }
    if target_missing_runtime_count != 0 {
        return Ok(RawSelectedRenderFrameDrain::PendingIncomplete {
            publication_generation: state.runtime_publication_generation,
            target_frame_count,
            target_ready_count: target_frame_count.saturating_sub(target_missing_runtime_count),
        });
    }

    // Build and validate every same-hour cutoff candidate before consuming an
    // earlier display Present. This prevents a bad discarded candidate from
    // being hidden behind a later valid row.
    let mut candidates = Vec::new();
    for frame in &state.completed_render_frames {
        if frame.game_hour_raw != target_game_hour_raw
            || frame.presentation_qpc_ticks > cutoff_qpc_ticks
        {
            continue;
        }
        let (submit, catalog) = runtime_by_submit
            .get(&frame.submit_index)
            .ok_or("selected render-frame candidate lost its runtime join")?;
        candidates.push(RawJoinedRenderFrameCapture {
            frame: frame.clone(),
            runtime: RawRenderSubmitCapture {
                submit: (*submit).clone(),
                catalog: (*catalog).clone(),
                shader_reflections: build_live_shader_reflections(catalog)?,
            },
        });
    }
    let candidate_ids = candidates
        .iter()
        .map(|row| row.frame.submit_index)
        .collect::<BTreeSet<_>>();
    let sealed_purges = state
        .completed_render_frames
        .iter()
        .filter(|frame| {
            frame.game_hour_raw <= target_game_hour_raw
                && !candidate_ids.contains(&frame.submit_index)
                && runtime_by_submit.contains_key(&frame.submit_index)
        })
        .map(|frame| {
            let (submit, catalog) = runtime_by_submit
                .get(&frame.submit_index)
                .expect("ready sealed purge was checked above");
            consumed_unselected_receipt(frame, submit, catalog)
        })
        .collect::<Result<Vec<_>, _>>()?;

    if candidates.is_empty() {
        if online_proof
            .as_ref()
            .is_some_and(|proof| proof.purged_candidate_count != 0)
        {
            return Err(
                "online current-hour proof has no retained cutoff-eligible candidate".into(),
            );
        }
        discard_unselected_render_authority(&sealed_purges)?;
        let purged_ids = sealed_purges
            .iter()
            .map(|row| row.submit_index)
            .collect::<BTreeSet<_>>();
        remove_render_join_ids(state, &purged_ids);
        state.sealed_render_hour = Some(SealedRenderHour {
            game_hour_raw: target_game_hour_raw,
            cutoff_qpc_ticks,
            cutoff_qpc_frequency_hz,
        });
        state.online_candidate_proofs.remove(&target_game_hour_raw);
        return Ok(RawSelectedRenderFrameDrain::EmptyNoCandidate {
            sealed_game_hour_raw: target_game_hour_raw,
            sealed_purges,
        });
    }
    candidates.sort_by_key(|row| {
        (
            row.frame.presentation_qpc_ticks,
            row.frame.global_order,
            row.frame.frame_sequence,
        )
    });
    let (candidate_count, candidate_set_sha256) =
        finalized_online_candidate_set_sha256(online_proof.as_ref(), &candidates)?;
    let selected_index = candidates
        .iter()
        .enumerate()
        .max_by_key(|(_, row)| {
            (
                row.frame.presentation_qpc_ticks,
                row.frame.global_order,
                row.frame.frame_sequence,
            )
        })
        .map(|(index, _)| index)
        .ok_or("selected render-frame candidate set became empty")?;
    let selected = candidates[selected_index].clone();
    validate_selected_candidate_epoch(&candidates, &selected)?;
    for purge in &sealed_purges {
        let frame = state
            .completed_render_frames
            .iter()
            .find(|frame| frame.submit_index == purge.submit_index)
            .ok_or("sealed purge receipt lost its frame before epoch validation")?;
        let (_, catalog) = runtime_by_submit
            .get(&purge.submit_index)
            .ok_or("sealed purge receipt lost its catalog before epoch validation")?;
        validate_discard_frame_epoch(frame, catalog, &selected)?;
    }
    let consumed_unselected = candidates
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != selected_index)
        .map(|(_, row)| {
            consumed_unselected_receipt(&row.frame, &row.runtime.submit, &row.runtime.catalog)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let (discarded_candidate_count, consumed_unselected_sha256) =
        finalized_online_consumed_sha256(online_proof.as_ref(), &consumed_unselected)?;
    if candidate_count.checked_sub(1) != Some(discarded_candidate_count) {
        return Err("selected render-frame candidate/discard cardinality differs".into());
    }
    let selection_receipt_sha256 = selected_render_frame_receipt_sha256(
        target_game_hour_raw,
        cutoff_qpc_ticks,
        cutoff_qpc_frequency_hz,
        candidate_count,
        &candidate_set_sha256,
        &selected.frame,
        discarded_candidate_count,
        &consumed_unselected_sha256,
    )?;
    let receipt = RawSelectedRenderFrameReceipt {
        target_game_hour_raw,
        cutoff_qpc_ticks,
        cutoff_qpc_frequency_hz,
        candidate_count,
        candidate_set_sha256,
        selected_submit_index: selected.frame.submit_index,
        selected_frame_sequence: selected.frame.frame_sequence,
        selected_global_order: selected.frame.global_order,
        selected_presentation_qpc_ticks: selected.frame.presentation_qpc_ticks,
        discarded_candidate_count,
        consumed_unselected_sha256,
        selection_receipt_sha256,
    };

    let mut all_discarded = consumed_unselected.clone();
    all_discarded.extend(sealed_purges.iter().cloned());
    discard_unselected_render_authority(&all_discarded)?;
    let consumed_ids = candidates
        .iter()
        .map(|row| row.frame.submit_index)
        .chain(sealed_purges.iter().map(|row| row.submit_index))
        .collect::<BTreeSet<_>>();
    remove_render_join_ids(state, &consumed_ids);
    state.sealed_render_hour = Some(SealedRenderHour {
        game_hour_raw: target_game_hour_raw,
        cutoff_qpc_ticks,
        cutoff_qpc_frequency_hz,
    });
    state.online_candidate_proofs.remove(&target_game_hour_raw);
    Ok(RawSelectedRenderFrameDrain::Ready(
        RawSelectedRenderFrameCapture {
            receipt,
            selected,
            consumed_unselected,
            sealed_purges,
        },
    ))
}

fn consumed_unselected_receipt(
    frame: &RawRenderFrameCapture,
    submit: &RawParticleSubmit,
    catalog: &RawRenderCatalogCapture,
) -> Result<RawConsumedUnselectedRenderFrameReceipt, String> {
    if frame.submit_index != submit.submit_index || frame.submit_index != catalog.submit_index {
        return Err("unselected render-frame receipt has a mismatched exact submit join".into());
    }
    let ordered_live_draw_sequences = catalog
        .draw_states
        .iter()
        .map(|draw| draw.final_draw_sequence)
        .collect::<Vec<_>>();
    if ordered_live_draw_sequences
        .windows(2)
        .any(|pair| pair[0] == 0 || pair[0] >= pair[1])
        || ordered_live_draw_sequences
            .last()
            .is_some_and(|last| *last == 0)
        || catalog
            .draw_states
            .iter()
            .any(|draw| draw.submit_index != frame.submit_index)
    {
        return Err(
            "unselected render-frame live draw ids are null, reordered, or retargeted".into(),
        );
    }
    let ordered_static_capture_sequences = frame
        .static_d3d_captures
        .iter()
        .map(|capture| capture.state.capture_sequence)
        .collect::<Vec<_>>();
    if ordered_static_capture_sequences
        .windows(2)
        .any(|pair| pair[0] == 0 || pair[0] >= pair[1])
        || ordered_static_capture_sequences
            .last()
            .is_some_and(|last| *last == 0)
    {
        return Err("unselected render-frame static capture ids are null or reordered".into());
    }
    Ok(RawConsumedUnselectedRenderFrameReceipt {
        submit_index: frame.submit_index,
        frame_sequence: frame.frame_sequence,
        global_order: frame.global_order,
        presentation_qpc_ticks: frame.presentation_qpc_ticks,
        live_draw_count: catalog.draw_states.len(),
        ordered_live_draw_sequences,
        ordered_static_capture_sequences,
    })
}

fn discard_unselected_render_authority(
    receipts: &[RawConsumedUnselectedRenderFrameReceipt],
) -> Result<(), String> {
    let groups = receipts
        .iter()
        .map(|receipt| crate::UnselectedAuthorityGroup {
            submit_index: receipt.submit_index,
            ordered_live_observation_sequences: receipt.ordered_live_draw_sequences.clone(),
            frame_sequence: receipt.frame_sequence,
            ordered_static_capture_sequences: receipt.ordered_static_capture_sequences.clone(),
        })
        .collect::<Vec<_>>();
    crate::discard_unselected_authority_groups(&groups)
}

fn remove_render_join_ids(state: &mut HookState, ids: &BTreeSet<u64>) {
    state
        .completed_render_frames
        .retain(|frame| !ids.contains(&frame.submit_index));
    state
        .completed
        .retain(|submit| !ids.contains(&submit.submit_index));
    state
        .completed_catalogs
        .retain(|catalog| !ids.contains(&catalog.submit_index));
}

fn selected_render_frame_receipt_sha256(
    target_game_hour_raw: i32,
    cutoff_qpc_ticks: u64,
    cutoff_qpc_frequency_hz: u64,
    candidate_count: u32,
    candidate_set_sha256: &str,
    selected: &RawRenderFrameCapture,
    discarded_candidate_count: u32,
    consumed_unselected_sha256: &str,
) -> Result<String, String> {
    let mut hasher = Sha256::new();
    hasher.update(b"hoi4.a729.selected-present.v3\0");
    hasher.update(target_game_hour_raw.to_le_bytes());
    hasher.update(cutoff_qpc_ticks.to_le_bytes());
    hasher.update(cutoff_qpc_frequency_hz.to_le_bytes());
    hasher.update(candidate_count.to_le_bytes());
    update_hash_bytes(&mut hasher, candidate_set_sha256.as_bytes())?;
    hasher.update(selected.submit_index.to_le_bytes());
    hasher.update(selected.frame_sequence.to_le_bytes());
    hasher.update(selected.global_order.to_le_bytes());
    hasher.update(selected.presentation_qpc_ticks.to_le_bytes());
    hasher.update(selected.presentation_qpc_frequency_hz.to_le_bytes());
    hasher.update(selected.process_start_nonce.to_le_bytes());
    hasher.update(selected.hook_epoch_nonce.to_le_bytes());
    hasher.update(selected.game_hour_raw.to_le_bytes());
    hasher.update(selected.expected_weather_region_count.to_le_bytes());
    hasher.update(selected.observed_weather_region_count.to_le_bytes());
    hasher.update(selected.active_airborne_weather_region_count.to_le_bytes());
    hasher.update(selected.queued_weather_change_count.to_le_bytes());
    hasher.update(selected.latest_completed_weather_update_index.to_le_bytes());
    update_hash_bytes(&mut hasher, selected.weather_snapshot_sha256.as_bytes())?;
    hasher.update(discarded_candidate_count.to_le_bytes());
    update_hash_bytes(&mut hasher, consumed_unselected_sha256.as_bytes())?;
    Ok(format!("{:x}", hasher.finalize()))
}

/// Recompute the selected-Present receipt from the joined frame which actually
/// left the a729 hook.  Downstream diagnostic instrumentation must call this
/// before it treats the public scalar fields as a Present authority; a
/// well-shaped 64-character string is not sufficient authentication.
pub fn validate_selected_render_frame_receipt(
    selected: &RawSelectedRenderFrameCapture,
) -> Result<(), String> {
    let receipt = &selected.receipt;
    let frame = &selected.selected.frame;
    let expected_weather_update_index = frame
        .weather_membership_updates
        .iter()
        .map(|row| row.update_index)
        .chain(frame.weather_visibility_updates.iter().map(|row| row.update_index))
        .max()
        .unwrap_or(0);
    if frame.expected_weather_region_count == 0
        || frame.observed_weather_region_count != frame.expected_weather_region_count
        || frame.observed_weather_region_count
        != u64::try_from(frame.weather_membership_updates.len())
            .map_err(|_| "selected render-frame weather membership count exceeds u64")?
        || frame.active_airborne_weather_region_count
            != u64::try_from(frame.weather_visibility_updates.len())
                .map_err(|_| "selected render-frame weather visibility count exceeds u64")?
        || frame.latest_completed_weather_update_index != expected_weather_update_index
        || frame.weather_snapshot_sha256
            != weather_snapshot_sha256(
                &frame.weather_membership_updates,
                &frame.weather_visibility_updates,
            )?
    {
        return Err("selected render-frame weather snapshot digest does not recompute".into());
    }
    if receipt.candidate_count == 0
        || receipt.discarded_candidate_count.checked_add(1) != Some(receipt.candidate_count)
        || usize::try_from(receipt.discarded_candidate_count).ok()
            != Some(selected.consumed_unselected.len())
        || receipt.selected_submit_index != frame.submit_index
        || receipt.selected_frame_sequence != frame.frame_sequence
        || receipt.selected_global_order != frame.global_order
        || receipt.selected_presentation_qpc_ticks != frame.presentation_qpc_ticks
        || receipt.target_game_hour_raw != frame.game_hour_raw
        || receipt.cutoff_qpc_ticks == 0
        || receipt.cutoff_qpc_frequency_hz == 0
        || receipt.cutoff_qpc_frequency_hz != frame.presentation_qpc_frequency_hz
        || frame.presentation_qpc_ticks > receipt.cutoff_qpc_ticks
        || frame.process_start_nonce == 0
        || frame.hook_epoch_nonce == 0
    {
        return Err("selected render-frame receipt disagrees with its joined Present".into());
    }
    let recomputed = selected_render_frame_receipt_sha256(
        receipt.target_game_hour_raw,
        receipt.cutoff_qpc_ticks,
        receipt.cutoff_qpc_frequency_hz,
        receipt.candidate_count,
        &receipt.candidate_set_sha256,
        frame,
        receipt.discarded_candidate_count,
        &receipt.consumed_unselected_sha256,
    )?;
    if recomputed != receipt.selection_receipt_sha256 {
        return Err("selected render-frame receipt SHA-256 does not recompute".into());
    }
    Ok(())
}

fn hash_render_candidate_metadata(
    hasher: &mut Sha256,
    frame: &RawRenderFrameCapture,
    catalog: &RawRenderCatalogCapture,
) -> Result<(), String> {
    hasher.update(frame.submit_index.to_le_bytes());
    hasher.update(frame.frame_sequence.to_le_bytes());
    hasher.update(frame.global_order.to_le_bytes());
    hasher.update(frame.presentation_qpc_ticks.to_le_bytes());
    hasher.update(frame.presentation_qpc_frequency_hz.to_le_bytes());
    hasher.update(frame.game_hour_raw.to_le_bytes());
    hasher.update(frame.process_start_nonce.to_le_bytes());
    hasher.update(frame.hook_epoch_nonce.to_le_bytes());
    hasher.update(frame.expected_weather_region_count.to_le_bytes());
    hasher.update(frame.observed_weather_region_count.to_le_bytes());
    hasher.update(frame.active_airborne_weather_region_count.to_le_bytes());
    hasher.update(frame.latest_completed_weather_update_index.to_le_bytes());
    update_hash_bytes(hasher, frame.weather_snapshot_sha256.as_bytes())?;
    update_hash_bytes(hasher, catalog.process.executable_sha256.as_bytes())?;
    update_hash_bytes(
        hasher,
        catalog.launch_epoch.hook_set_receipt_sha256.as_bytes(),
    )?;
    update_hash_bytes(
        hasher,
        catalog.launch_epoch.capture_receipt_sha256.as_bytes(),
    )?;
    Ok(())
}

fn fold_online_candidate_proof(
    proof: &mut OnlineCandidateProof,
    frame: &RawRenderFrameCapture,
    catalog: &RawRenderCatalogCapture,
    receipt: &RawConsumedUnselectedRenderFrameReceipt,
) -> Result<(), String> {
    validate_online_candidate_proof_shape(proof)?;
    if receipt.submit_index != frame.submit_index
        || receipt.frame_sequence != frame.frame_sequence
        || receipt.global_order != frame.global_order
        || receipt.presentation_qpc_ticks != frame.presentation_qpc_ticks
    {
        return Err("online current-hour candidate receipt changed identity".into());
    }
    let order = (
        frame.presentation_qpc_ticks,
        frame.global_order,
        frame.frame_sequence,
    );
    if proof.purged_candidate_count == 0 {
        proof.game_hour_raw = Some(frame.game_hour_raw);
        proof.presentation_qpc_frequency_hz = Some(frame.presentation_qpc_frequency_hz);
        proof.process_start_nonce = Some(frame.process_start_nonce);
        proof.hook_epoch_nonce = Some(frame.hook_epoch_nonce);
    } else if proof.game_hour_raw != Some(frame.game_hour_raw)
        || proof.presentation_qpc_frequency_hz != Some(frame.presentation_qpc_frequency_hz)
        || proof.process_start_nonce != Some(frame.process_start_nonce)
        || proof.hook_epoch_nonce != Some(frame.hook_epoch_nonce)
        || proof.last_purged_order.is_some_and(|last| order <= last)
    {
        return Err("online current-hour proof candidate regressed or crossed epoch".into());
    }
    let next_candidate_count = proof
        .purged_candidate_count
        .checked_add(1)
        .ok_or("online current-hour candidate count overflowed")?;
    let mut candidate_hasher = Sha256::new();
    candidate_hasher.update(b"hoi4.a729.present-candidate-rolling.v3\0");
    candidate_hasher.update(proof.candidate_rolling_sha256);
    hash_render_candidate_metadata(&mut candidate_hasher, frame, catalog)?;
    proof
        .candidate_rolling_sha256
        .copy_from_slice(&candidate_hasher.finalize());
    proof.purged_candidate_count = next_candidate_count;

    let next_receipt_count = proof
        .purged_receipt_count
        .checked_add(1)
        .ok_or("online current-hour consumed receipt count overflowed")?;
    let mut receipt_hasher = Sha256::new();
    receipt_hasher.update(b"hoi4.a729.present-consumed-rolling.v2\0");
    receipt_hasher.update(proof.consumed_rolling_sha256);
    hash_consumed_unselected_receipt(&mut receipt_hasher, receipt)?;
    proof
        .consumed_rolling_sha256
        .copy_from_slice(&receipt_hasher.finalize());
    proof.purged_receipt_count = next_receipt_count;
    proof.last_purged_order = Some(order);
    validate_online_candidate_proof_shape(proof)?;
    Ok(())
}

fn validate_online_candidate_proof_shape(proof: &OnlineCandidateProof) -> Result<(), String> {
    if proof.purged_candidate_count != proof.purged_receipt_count {
        return Err("online current-hour candidate/receipt proof counts differ".into());
    }
    let has_rows = proof.purged_candidate_count != 0;
    if proof.game_hour_raw.is_some() != has_rows
        || proof.presentation_qpc_frequency_hz.is_some() != has_rows
        || proof.process_start_nonce.is_some() != has_rows
        || proof.hook_epoch_nonce.is_some() != has_rows
        || proof.last_purged_order.is_some() != has_rows
        || proof
            .presentation_qpc_frequency_hz
            .is_some_and(|value| value == 0)
        || proof.process_start_nonce.is_some_and(|value| value == 0)
        || proof.hook_epoch_nonce.is_some_and(|value| value == 0)
        || (proof.candidate_rolling_sha256 != [0; 32]) != has_rows
        || (proof.consumed_rolling_sha256 != [0; 32]) != has_rows
    {
        return Err("online current-hour proof epoch shape is invalid".into());
    }
    Ok(())
}

fn finalized_online_candidate_set_sha256(
    proof: Option<&OnlineCandidateProof>,
    candidates: &[RawJoinedRenderFrameCapture],
) -> Result<(u32, String), String> {
    if let Some(proof) = proof {
        validate_online_candidate_proof_shape(proof)?;
        if let Some(first) = candidates.first() {
            let frame = &first.frame;
            let order = (
                frame.presentation_qpc_ticks,
                frame.global_order,
                frame.frame_sequence,
            );
            if proof.game_hour_raw != Some(frame.game_hour_raw)
                || proof.presentation_qpc_frequency_hz != Some(frame.presentation_qpc_frequency_hz)
                || proof.process_start_nonce != Some(frame.process_start_nonce)
                || proof.hook_epoch_nonce != Some(frame.hook_epoch_nonce)
                || proof.last_purged_order.is_some_and(|last| order <= last)
            {
                return Err(
                    "retained current-hour candidate does not follow its online proof".into(),
                );
            }
        }
    }
    let prior_count = proof.map_or(0, |value| value.purged_candidate_count);
    let local_count = u32::try_from(candidates.len())
        .map_err(|_| "selected Present candidate count exceeds u32")?;
    let total_count = prior_count
        .checked_add(local_count)
        .ok_or("selected Present candidate count overflowed")?;
    let mut rolling = proof.map_or([0; 32], |value| value.candidate_rolling_sha256);
    for candidate in candidates {
        let mut hasher = Sha256::new();
        hasher.update(b"hoi4.a729.present-candidate-rolling.v3\0");
        hasher.update(rolling);
        hash_render_candidate_metadata(&mut hasher, &candidate.frame, &candidate.runtime.catalog)?;
        rolling.copy_from_slice(&hasher.finalize());
    }
    let mut final_hasher = Sha256::new();
    final_hasher.update(b"hoi4.a729.present-candidate-set.v3\0");
    final_hasher.update(total_count.to_le_bytes());
    final_hasher.update(rolling);
    Ok((total_count, format!("{:x}", final_hasher.finalize())))
}

fn finalized_online_consumed_sha256(
    proof: Option<&OnlineCandidateProof>,
    receipts: &[RawConsumedUnselectedRenderFrameReceipt],
) -> Result<(u32, String), String> {
    if let Some(proof) = proof {
        validate_online_candidate_proof_shape(proof)?;
    }
    let prior_count = proof.map_or(0, |value| value.purged_receipt_count);
    let local_count = u32::try_from(receipts.len())
        .map_err(|_| "selected Present consumed receipt count exceeds u32")?;
    let total_count = prior_count
        .checked_add(local_count)
        .ok_or("selected Present consumed receipt count overflowed")?;
    let mut rolling = proof.map_or([0; 32], |value| value.consumed_rolling_sha256);
    for receipt in receipts {
        let mut hasher = Sha256::new();
        hasher.update(b"hoi4.a729.present-consumed-rolling.v2\0");
        hasher.update(rolling);
        hash_consumed_unselected_receipt(&mut hasher, receipt)?;
        rolling.copy_from_slice(&hasher.finalize());
    }
    let mut final_hasher = Sha256::new();
    final_hasher.update(b"hoi4.a729.present-consumed-set.v2\0");
    final_hasher.update(total_count.to_le_bytes());
    final_hasher.update(rolling);
    Ok((total_count, format!("{:x}", final_hasher.finalize())))
}

fn hash_consumed_unselected_receipt(
    hasher: &mut Sha256,
    receipt: &RawConsumedUnselectedRenderFrameReceipt,
) -> Result<(), String> {
    let live_draw_count = u32::try_from(receipt.live_draw_count)
        .map_err(|_| "unselected live draw count exceeds u32")?;
    let live_sequence_count = u32::try_from(receipt.ordered_live_draw_sequences.len())
        .map_err(|_| "unselected live sequence count exceeds u32")?;
    let static_sequence_count = u32::try_from(receipt.ordered_static_capture_sequences.len())
        .map_err(|_| "unselected static sequence count exceeds u32")?;
    hasher.update(receipt.submit_index.to_le_bytes());
    hasher.update(receipt.frame_sequence.to_le_bytes());
    hasher.update(receipt.global_order.to_le_bytes());
    hasher.update(receipt.presentation_qpc_ticks.to_le_bytes());
    hasher.update(live_draw_count.to_le_bytes());
    hasher.update(live_sequence_count.to_le_bytes());
    for sequence in &receipt.ordered_live_draw_sequences {
        hasher.update(sequence.to_le_bytes());
    }
    hasher.update(static_sequence_count.to_le_bytes());
    for sequence in &receipt.ordered_static_capture_sequences {
        hasher.update(sequence.to_le_bytes());
    }
    Ok(())
}

fn update_hash_bytes(hasher: &mut Sha256, bytes: &[u8]) -> Result<(), String> {
    let len = u32::try_from(bytes.len()).map_err(|_| "selection hash field exceeds u32")?;
    hasher.update(len.to_le_bytes());
    hasher.update(bytes);
    Ok(())
}

fn sha256_bytes_hex(bytes: &[u8; 32]) -> String {
    let mut output = String::with_capacity(64);
    for byte in bytes {
        write!(&mut output, "{byte:02x}").expect("writing into String cannot fail");
    }
    output
}

fn validate_selected_candidate_epoch(
    candidates: &[RawJoinedRenderFrameCapture],
    selected: &RawJoinedRenderFrameCapture,
) -> Result<(), String> {
    let selected_catalog = &selected.runtime.catalog;
    let mut prior_qpc = 0u64;
    let mut prior_global_order = None;
    let mut prior_frame_sequence = 0u64;
    for candidate in candidates {
        let frame = &candidate.frame;
        let catalog = &candidate.runtime.catalog;
        if frame.submit_index != candidate.runtime.submit.submit_index
            || frame.submit_index != catalog.submit_index
            || frame.process_start_nonce != selected.frame.process_start_nonce
            || frame.hook_epoch_nonce != selected.frame.hook_epoch_nonce
            || frame.presentation_qpc_frequency_hz != selected.frame.presentation_qpc_frequency_hz
            || catalog.process.process_start_nonce != frame.process_start_nonce
            || catalog.launch_epoch.process_start_nonce != frame.process_start_nonce
            || catalog.launch_epoch.hook_epoch_nonce != frame.hook_epoch_nonce
            || catalog.process.executable_sha256 != selected_catalog.process.executable_sha256
            || catalog.launch_epoch.hook_set_receipt_sha256
                != selected_catalog.launch_epoch.hook_set_receipt_sha256
            || catalog.launch_epoch.capture_receipt_sha256
                != selected_catalog.launch_epoch.capture_receipt_sha256
            || frame.presentation_qpc_ticks <= prior_qpc
            || prior_global_order.is_some_and(|prior| frame.global_order <= prior)
            || frame.frame_sequence <= prior_frame_sequence
        {
            return Err(
                "selected render-frame candidates differ in join, epoch, authority, or order"
                    .into(),
            );
        }
        prior_qpc = frame.presentation_qpc_ticks;
        prior_global_order = Some(frame.global_order);
        prior_frame_sequence = frame.frame_sequence;
    }
    Ok(())
}

fn validate_discard_frame_epoch(
    frame: &RawRenderFrameCapture,
    catalog: &RawRenderCatalogCapture,
    selected: &RawJoinedRenderFrameCapture,
) -> Result<(), String> {
    let selected_catalog = &selected.runtime.catalog;
    if frame.submit_index != catalog.submit_index
        || frame.process_start_nonce != selected.frame.process_start_nonce
        || frame.hook_epoch_nonce != selected.frame.hook_epoch_nonce
        || frame.presentation_qpc_frequency_hz != selected.frame.presentation_qpc_frequency_hz
        || catalog.process.process_start_nonce != frame.process_start_nonce
        || catalog.launch_epoch.process_start_nonce != frame.process_start_nonce
        || catalog.launch_epoch.hook_epoch_nonce != frame.hook_epoch_nonce
        || catalog.process.executable_sha256 != selected_catalog.process.executable_sha256
        || catalog.launch_epoch.hook_set_receipt_sha256
            != selected_catalog.launch_epoch.hook_set_receipt_sha256
        || catalog.launch_epoch.capture_receipt_sha256
            != selected_catalog.launch_epoch.capture_receipt_sha256
    {
        return Err("sealed render-frame discard differs from selected admission epoch".into());
    }
    Ok(())
}

/// Waits without polling, but never beyond the collector's remaining operation
/// deadline, for publication to advance beyond a `PendingIncomplete` receipt.
/// Timeout preserves all unmatched rows. The hook-state lock closes the
/// check/sleep race; rejection or uninstall wakes the waiter and fails closed.
pub fn wait_for_render_submit_publication_timeout(
    publication_generation: u64,
    remaining_deadline: Duration,
) -> Result<RenderSubmitPublicationWait, String> {
    if !INSTALLED.load(Ordering::Acquire) {
        return Err("a729 render hooks are not installed".into());
    }
    let mut state = STATE
        .get()
        .ok_or("render hook state is unavailable")?
        .lock()
        .map_err(|_| "render hook state poisoned")?;
    let (state_after_wait, timeout) = RENDER_SUBMIT_PUBLICATION
        .get_or_init(Condvar::new)
        .wait_timeout_while(state, remaining_deadline, |state| {
            INSTALLED.load(Ordering::Acquire)
                && !completeness_violated(state)
                && state.runtime_publication_generation == publication_generation
        })
        .map_err(|_| "render hook state poisoned while waiting for publication")?;
    state = state_after_wait;
    if !INSTALLED.load(Ordering::Acquire) {
        return Err("a729 render hooks were uninstalled while waiting for publication".into());
    }
    if completeness_violated(&state) {
        return Err(completeness_message(&state));
    }
    if state.runtime_publication_generation != publication_generation {
        Ok(RenderSubmitPublicationWait::PublicationAdvanced {
            publication_generation: state.runtime_publication_generation,
        })
    } else if timeout.timed_out() {
        Ok(RenderSubmitPublicationWait::TimedOut {
            publication_generation,
        })
    } else {
        Err("render submit publication wait ended without a state transition".into())
    }
}

/// Bind one recorder-authenticated effective-VFS winner index to both live
/// authority registries before either can accept a draw. A failed second bind
/// rolls the first registry back to its empty installed state.
pub fn bind_render_authority_winner_index(
    binding: crate::RawEffectiveVfsWinnerIndexBinding,
) -> Result<(), String> {
    crate::bind_live_effective_vfs_winners(binding.clone())?;
    if let Err(error) = crate::bind_static_effective_vfs_winners(binding) {
        let rollback = crate::reset_live_authority_registry();
        return Err(match rollback {
            Ok(()) => format!("bind static render authority winner index: {error}"),
            Err(rollback) => format!(
                "bind static render authority winner index: {error}; live rollback failed: {rollback}"
            ),
        });
    }
    Ok(())
}

/// Accept one fully materialized d77 authority observation only when it names
/// exactly one queued hook draw. VFS/program semantics are then admitted by the
/// production registry; this hook check prevents cross-submit or cross-draw
/// retargeting before that semantic validation runs.
pub fn observe_authenticated_live_draw_authority(
    submit_index: u64,
    hook_draw: &RawD3dDrawState,
    observed: crate::RawLiveDrawAuthorityObservation,
) -> Result<(), String> {
    if hook_draw.submit_index != submit_index || !live_authority_matches_draw(&observed, hook_draw)
    {
        return Err("d77 authority observation does not exactly match its hook draw".into());
    }
    crate::observe_live_draw_authority(submit_index, observed)
}

/// Accept one fully materialized d80 authority observation only when it names
/// exactly one queued static draw and preserves the hook-observed output-view
/// order. No semantic role or content path is inferred here.
pub fn observe_authenticated_static_draw_authority(
    frame_sequence: u64,
    hook_draw: &RawStaticD3dDrawState,
    observed: crate::RawStaticAuthorityObservation,
) -> Result<(), String> {
    if frame_sequence == 0 || !static_authority_matches_draw(&observed, hook_draw) {
        return Err("d80 authority observation does not exactly match its hook draw".into());
    }
    crate::observe_static_authority(frame_sequence, observed)
}

/// Freeze the d77 semantic registry against the exact capture that will be
/// drained/promoted by the collector.
pub fn finalize_authenticated_live_submit_authority(
    capture: &RawRenderSubmitCapture,
) -> Result<(), String> {
    crate::finalize_live_submit_authority(capture)
}

/// Freeze the d80 semantic registry against the exact static subsequence in
/// the atomic render-order capture.
pub fn finalize_authenticated_static_frame_authority(
    process_start_nonce: u64,
    capture: &RawRenderFrameCapture,
) -> Result<(), String> {
    let draws = capture
        .static_d3d_captures
        .iter()
        .map(|row| row.state.clone())
        .collect::<Vec<_>>();
    crate::finalize_static_authority(capture.frame_sequence, process_start_nonce, &draws)
}

fn live_authority_matches_draw(
    observed: &crate::RawLiveDrawAuthorityObservation,
    draw: &RawD3dDrawState,
) -> bool {
    observed.observation_sequence == draw.final_draw_sequence
        && observed.identity == draw.identity
        && observed.program_registration.shader_program_ptr == draw.shader_program_ptr
        && observed.vertex_creation.shader_ptr == draw.vertex_shader_ptr
        && observed.pixel_creation.shader_ptr == draw.pixel_shader_ptr
        && observed.vertex_reflection.shader_ptr == draw.vertex_shader_ptr
        && observed.pixel_reflection.shader_ptr == draw.pixel_shader_ptr
        && observed
            .ordered_color_targets
            .iter()
            .map(|receipt| receipt.view_ptr)
            .eq(draw.ordered_color_target_view_ptrs.iter().copied())
        && observed
            .depth_target
            .as_ref()
            .map(|receipt| receipt.view_ptr)
            == draw.depth_stencil_view_ptr
}

fn static_authority_matches_draw(
    observed: &crate::RawStaticAuthorityObservation,
    draw: &RawStaticD3dDrawState,
) -> bool {
    observed.capture_sequence == draw.capture_sequence
        && observed.shader_program_ptr == draw.shader_program_ptr
        && observed.program_registration.shader_program_ptr == draw.shader_program_ptr
        && observed.vertex_creation.shader_ptr == draw.vertex_shader_ptr
        && observed.pixel_creation.shader_ptr == draw.pixel_shader_ptr
        && observed.vertex_reflection.shader_ptr == draw.vertex_shader_ptr
        && observed.pixel_reflection.shader_ptr == draw.pixel_shader_ptr
        && observed
            .ordered_color_targets
            .iter()
            .map(|receipt| receipt.view_ptr)
            .eq(draw.ordered_color_target_view_ptrs.iter().copied())
        && observed
            .depth_target
            .as_ref()
            .map(|receipt| receipt.view_ptr)
            == draw.depth_stencil_view_ptr
}

pub fn drain_complete_weather_changes() -> Result<Vec<RawWeatherObservation>, String> {
    if !INSTALLED.load(Ordering::Acquire) {
        return Err("a729 render hooks are not installed".into());
    }
    let mut state = STATE
        .get()
        .ok_or("render hook state is unavailable")?
        .lock()
        .map_err(|_| "render hook state poisoned")?;
    if completeness_violated(&state) {
        return Err(completeness_message(&state));
    }
    if state.weather_change_overflows != 0 {
        return Err(format!(
            "weather change history overflowed {} times",
            state.weather_change_overflows
        ));
    }
    Ok(state.weather_changes.drain(..).collect())
}

/// Compatibility snapshot of recognized map shader lookups. Production uses
/// the pointer-correlated joins carried atomically in each render frame.
pub fn drain_map_shader_lookups() -> Result<Vec<RawMapShaderLookup>, String> {
    if !INSTALLED.load(Ordering::Acquire) {
        return Err("a729 render hooks are not installed".into());
    }
    let state = STATE
        .get()
        .ok_or("render hook state is unavailable")?
        .lock()
        .map_err(|_| "render hook state poisoned")?;
    Ok(state.map_shader_lookups.iter().cloned().collect())
}

/// Compatibility frame-only drain. Production must use
/// `drain_complete_render_frame_submit_captures` so a Present frame cannot be
/// consumed during the publication gap before its runtime submit/catalog.
pub fn drain_render_frame_captures() -> Result<Vec<RawRenderFrameCapture>, String> {
    if !INSTALLED.load(Ordering::Acquire) {
        return Err("a729 render hooks are not installed".into());
    }
    let mut state = STATE
        .get()
        .ok_or("render hook state is unavailable")?
        .lock()
        .map_err(|_| "render hook state poisoned")?;
    if state.pending_render_frame.is_some() {
        return Err("cannot drain while a render frame is pending".into());
    }
    Ok(state.completed_render_frames.drain(..).collect())
}

/// Compatibility single-frame drain. Normal production must call
/// `drain_complete_render_frame_submit_captures` and retain its atomic join.
pub fn drain_render_order_capture() -> Result<RawRenderOrderCapture, String> {
    let frames = drain_render_frame_captures()?;
    if frames.len() != 1 {
        return Err(format!(
            "single render-order drain expected one completed frame, found {}",
            frames.len()
        ));
    }
    let frame = frames.into_iter().next().expect("length checked");
    Ok(RawRenderOrderCapture {
        events: frame.events,
        static_d3d_captures: frame.static_d3d_captures,
        static_catalog_fragment: frame.static_catalog_fragment,
    })
}

/// Compatibility event-only drain. Production static capture must use
/// `drain_render_order_capture` so clear target authority cannot be discarded.
#[cfg(test)]
pub fn drain_render_order_events() -> Result<Vec<RawRenderOrderEvent>, String> {
    Ok(drain_render_order_capture()?.events)
}

pub fn diagnostics() -> HookDiagnostics {
    let Some(state) = STATE.get().and_then(|value| value.lock().ok()) else {
        return HookDiagnostics::default();
    };
    HookDiagnostics {
        live_runtime_count: state.runtimes.len() as u64,
        queued_submit_count: state.completed.len() as u64,
        rejected_observation_count: state.rejected_observations,
        evicted_submit_count: state.evicted_submits,
        duplicate_runtime_count: state.duplicate_runtime_ids,
        nested_envelope_count: state.nested_envelopes,
        live_entity_count: state.entities.len() as u64,
        queued_weather_change_count: state.weather_changes.len() as u64,
        weather_change_overflow_count: state.weather_change_overflows,
        observed_weather_region_count: state.weather_runtime_codes.len() as u64,
        active_airborne_weather_region_count: state
            .weather_runtime_codes
            .values()
            .filter(|code| **code != 0)
            .count() as u64,
        duplicate_entity_count: state.duplicate_entity_ids,
        nested_weather_count: state.nested_weather_calls,
        queued_map_shader_lookup_count: state.map_shader_lookups.len() as u64,
        rejected_map_shader_lookup_count: state.rejected_map_shader_lookups,
        map_shader_lookup_overflow_count: state.map_shader_lookup_overflows,
        queued_render_order_event_count: state.render_order_events.len() as u64,
        queued_clear_output_snapshot_count: state.clear_output_view_snapshots.len() as u64,
        queued_static_d3d_capture_count: state.static_d3d_captures.len() as u64,
        render_order_event_overflow_count: state.render_order_event_overflows,
    }
}

unsafe extern "system" fn hook_create(manager: usize, definition_ptr: usize) -> usize {
    let original: CreateFn = unsafe { mem::transmute(ORIGINAL_CREATE.load(Ordering::Acquire)) };
    let runtime = unsafe { original(manager, definition_ptr) };
    if runtime != 0 {
        let identity = read_definition(definition_ptr);
        if let (Some(definition), Some(scale)) = (identity, read_u32(runtime + 0x48)) {
            if let Some(state) = STATE.get().and_then(|value| value.lock().ok()) {
                let mut state = state;
                let row = RuntimeIdentity {
                    stable_id: NEXT_EFFECT_ID.fetch_add(1, Ordering::AcqRel),
                    definition,
                    owner: 0,
                    render_list_class: u32::MAX,
                    render_subbucket: u16::MAX,
                    scale_f32_bits: scale,
                    parent_entity_instance_id: None,
                };
                if state.runtimes.insert(runtime, row).is_some() {
                    state.duplicate_runtime_ids += 1;
                }
            }
        } else {
            reject();
        }
    }
    runtime
}

unsafe extern "system" fn hook_destroy(runtime: usize) {
    if runtime != 0 {
        if let Some(mut state) = STATE.get().and_then(|value| value.lock().ok()) {
            if state.runtimes.remove(&runtime).is_none() {
                state.rejected_observations += 1;
            }
        }
    }
    let original: DestroyFn = unsafe { mem::transmute(ORIGINAL_DESTROY.load(Ordering::Acquire)) };
    unsafe { original(runtime) };
}

unsafe extern "system" fn hook_object_ctor(object: usize, type_ptr: usize) -> usize {
    let original: ObjectCtorFn =
        unsafe { mem::transmute(ORIGINAL_OBJECT_CTOR.load(Ordering::Acquire)) };
    let result = unsafe { original(object, type_ptr) };
    if let Some(runtime) = read_usize(object + 0x70).filter(|value| *value != 0) {
        if let Some(mut state) = STATE.get().and_then(|value| value.lock().ok()) {
            if let Some(row) = state.runtimes.get_mut(&runtime) {
                row.owner = object;
                ENTITY_FACTORY_STACK.with(|stack| {
                    if let Some(factory) = stack.borrow_mut().last_mut() {
                        row.parent_entity_instance_id = Some(factory.stable_id);
                        factory.ordered_effect_instance_ids.push(row.stable_id);
                    }
                });
            } else {
                state.rejected_observations += 1;
            }
        }
    }
    result
}

unsafe extern "system" fn hook_bucket(object: usize, render_class: i32) {
    let runtime = read_usize(object + 0x70).filter(|value| *value != 0);
    let subbucket = read_usize(object + 0x58).and_then(|type_ptr| read_u16(type_ptr + 0x38));
    if let (Some(runtime), Some(subbucket)) = (runtime, subbucket) {
        if let Some(mut state) = STATE.get().and_then(|value| value.lock().ok()) {
            if let Some(row) = state.runtimes.get_mut(&runtime) {
                row.owner = object;
                row.render_list_class = render_class as u32;
                row.render_subbucket = subbucket;
            } else {
                state.rejected_observations += 1;
            }
        }
    } else {
        reject();
    }
    let original: BucketFn = unsafe { mem::transmute(ORIGINAL_BUCKET.load(Ordering::Acquire)) };
    unsafe { original(object, render_class) };
}

fn weather_snapshot_sha256(
    membership: &[RawWeatherObservation],
    visibility: &[RawWeatherVisibilityObservation],
) -> Result<String, String> {
    fn hash_owner(hasher: &mut Sha256, owner: RawWeatherUpdateOwner) {
        hasher.update([match owner {
            RawWeatherUpdateOwner::NormalStrategicRegionUpdate => 0,
            RawWeatherUpdateOwner::EditorRefresh => 1,
        }]);
    }
    fn hash_entity(hasher: &mut Sha256, entity: &Option<RawWeatherEntityRef>) -> Result<(), String> {
        let Some(entity) = entity else {
            hasher.update([0]);
            return Ok(());
        };
        hasher.update([1]);
        hasher.update(entity.entity_instance_id.to_le_bytes());
        match &entity.registration_token {
            Some(token) => {
                hasher.update([1]);
                update_hash_bytes(hasher, token.as_bytes())?;
            }
            None => hasher.update([0]),
        }
        let count = u32::try_from(entity.ordered_effect_instance_ids.len())
            .map_err(|_| "weather effect-id count exceeds u32")?;
        hasher.update(count.to_le_bytes());
        for effect in &entity.ordered_effect_instance_ids {
            hasher.update(effect.to_le_bytes());
        }
        Ok(())
    }

    let mut hasher = Sha256::new();
    hasher.update(b"hoi4.a729.selected-present-weather-snapshot.v1\0");
    hasher.update(
        u32::try_from(membership.len())
            .map_err(|_| "weather membership snapshot exceeds u32")?
            .to_le_bytes(),
    );
    for row in membership {
        hasher.update(row.observation_index.to_le_bytes());
        hasher.update(row.sequence.to_le_bytes());
        hasher.update(row.update_index.to_le_bytes());
        hash_owner(&mut hasher, row.owner);
        hasher.update([
            u8::from(row.initial_observation),
            u8::from(row.checkpoint_observation),
        ]);
        hasher.update(row.strategic_region_id.to_le_bytes());
        hasher.update([
            u8::from(row.rain_light_active),
            u8::from(row.rain_heavy_active),
            u8::from(row.snow_active),
            u8::from(row.blizzard_active),
            u8::from(row.sandstorm_active),
            u8::from(row.arctic_water_active),
            row.previous_airborne_runtime_code,
            row.airborne_runtime_code,
        ]);
        hasher.update(row.region_value_i64_fixed_1e5.to_le_bytes());
        hasher.update(row.transition_expiry_game_hour_i32.to_le_bytes());
        match row.selection_seed_words {
            Some(words) => {
                hasher.update([1]);
                hasher.update(words[0].to_le_bytes());
                hasher.update(words[1].to_le_bytes());
            }
            None => hasher.update([0]),
        }
        hasher.update(
            u32::try_from(row.ordered_factory_attempts.len())
                .map_err(|_| "weather factory-attempt count exceeds u32")?
                .to_le_bytes(),
        );
        for attempt in &row.ordered_factory_attempts {
            hasher.update(attempt.allocated_entity_instance_id.to_le_bytes());
            hasher.update(attempt.returned_entity_instance_id.unwrap_or(0).to_le_bytes());
        }
        hasher.update(
            u32::try_from(row.ordered_live_pairs.len())
                .map_err(|_| "weather live-pair count exceeds u32")?
                .to_le_bytes(),
        );
        for pair in &row.ordered_live_pairs {
            hash_entity(&mut hasher, &pair.primary)?;
            hash_entity(&mut hasher, &pair.fading)?;
        }
    }
    hasher.update(
        u32::try_from(visibility.len())
            .map_err(|_| "weather visibility snapshot exceeds u32")?
            .to_le_bytes(),
    );
    for row in visibility {
        hasher.update(row.observation_index.to_le_bytes());
        hasher.update(row.sequence.to_le_bytes());
        hasher.update(row.update_index.to_le_bytes());
        hash_owner(&mut hasher, row.owner);
        hasher.update(row.strategic_region_id.to_le_bytes());
        hasher.update(row.view_distance_f32_bits.to_le_bytes());
        hasher.update([row.camera_branch as u8]);
        hasher.update(row.computed_fade_alpha_f32_bits.to_le_bytes());
        hasher.update(row.computed_playback_rate_f32_bits.to_le_bytes());
        hasher.update(
            u32::try_from(row.ordered_entity_visibility.len())
                .map_err(|_| "weather entity-visibility count exceeds u32")?
                .to_le_bytes(),
        );
        for entity in &row.ordered_entity_visibility {
            hasher.update(entity.entity_instance_id.to_le_bytes());
            hasher.update([match entity.role {
                WeatherEntityRole::Primary => 0,
                WeatherEntityRole::Fading => 1,
            }]);
            hasher.update([
                u8::from(entity.post_update_active),
                u8::from(entity.producer_wrote_active),
                u8::from(entity.producer_wrote_alpha),
            ]);
            hasher.update(entity.post_update_alpha_f32_bits.to_le_bytes());
            hasher.update(entity.post_update_playback_rate_f32_bits.to_le_bytes());
        }
    }
    Ok(format!("{:x}", hasher.finalize()))
}

const MAX_CAMERA_INPUT_NODES: usize = 4_096;
const MAX_PENDING_CAMERA_EVENT_BATCHES_PER_ROOT: usize = 1_024;

#[derive(Clone, Copy)]
struct AuthenticatedCameraRoot {
    owner: usize,
    owner_kind: RawA729CameraOwnerKind,
    owner_vtable_rva: u32,
    camera_root_offset: usize,
    event_list_offset: usize,
}

fn authenticate_camera_root(root: usize) -> Result<AuthenticatedCameraRoot, String> {
    let owner = read_usize(root.checked_add(0x08).ok_or("camera owner address overflow")?)
        .filter(|value| *value != 0)
        .ok_or("camera owner pointer is null or unreadable")?;
    let owner_vtable = read_usize(owner).ok_or("camera owner vtable is unreadable")?;
    let base = MODULE_BASE.load(Ordering::Acquire);
    let owner_vtable_rva = owner_vtable
        .checked_sub(base)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or("camera owner vtable is outside the a729 image")?;
    let (owner_kind, camera_root_offset, event_list_offset) = match owner_vtable_rva {
        0x0293_3710 => (RawA729CameraOwnerKind::FrontEnd, 0x610, 0x5f0),
        0x0295_2510 => (RawA729CameraOwnerKind::InGame, 0x750, 0x758),
        0x0299_3698 => (RawA729CameraOwnerKind::Third, 0x638, 0x648),
        _ => return Err("camera owner vtable is not one of the three authenticated C00 owners".into()),
    };
    if owner.checked_add(camera_root_offset) != Some(root)
        || read_usize(owner) != Some(owner_vtable)
    {
        return Err("camera owner-to-root join changed during authentication".into());
    }
    Ok(AuthenticatedCameraRoot {
        owner,
        owner_kind,
        owner_vtable_rva,
        camera_root_offset,
        event_list_offset,
    })
}

fn camera_vec3_bits(root: usize, offset: usize) -> Option<[u32; 3]> {
    Some([
        read_u32(root.checked_add(offset)?)?,
        read_u32(root.checked_add(offset + 4)?)?,
        read_u32(root.checked_add(offset + 8)?)?,
    ])
}

fn capture_camera_input_nodes(
    event_head: usize,
) -> Result<(Vec<RawA729CameraInputEvent>, Vec<usize>), String> {
    let mut rows = Vec::new();
    let mut nodes = Vec::new();
    let mut seen = BTreeSet::new();
    let mut node = event_head;
    while node != 0 {
        if rows.len() == MAX_CAMERA_INPUT_NODES || !seen.insert(node) {
            return Err("camera input list exceeded its bound or contained a cycle".into());
        }
        let mut embedded = [0u8; 16];
        read_exact(
            node.checked_add(0x48).ok_or("camera event address overflow")?,
            &mut embedded,
        )
        .ok_or("camera embedded event is unreadable")?;
        rows.push(RawA729CameraInputEvent {
            node_ordinal: rows.len() as u32,
            node_kind_i32: read_i32(
                node.checked_add(0x7c).ok_or("camera node kind address overflow")?,
            )
            .ok_or("camera node kind is unreadable")?,
            consumed_before_u8: read_u8(
                node.checked_add(0x80).ok_or("camera node consumed address overflow")?,
            )
            .ok_or("camera node consumed byte is unreadable")?,
            consumed_after_u8: 0,
            event_type_i32: i32::from_le_bytes(
                embedded[4..8].try_into().map_err(|_| "camera event type decode")?,
            ),
            signed_delta_i32: i32::from_le_bytes(
                embedded[8..12].try_into().map_err(|_| "camera event delta decode")?,
            ),
            embedded_event_16_bytes: embedded,
        });
        nodes.push(node);
        node = read_usize(
            node.checked_add(0x90).ok_or("camera input next address overflow")?,
        )
        .ok_or("camera input next pointer is unreadable")?;
    }
    Ok((rows, nodes))
}

fn begin_camera_event_producer(
    camera_root: usize,
    producer_rva: u32,
    event_head: usize,
    frame_scalar_f32_bits: Option<u32>,
    producer_flag_0_u8: u8,
    producer_flag_1_u8: u8,
) -> Result<(), String> {
    let auth = authenticate_camera_root(camera_root)?;
    if auth.owner.checked_add(0x508).and_then(read_usize).filter(|value| *value != 0).is_none()
        || auth.owner.checked_add(0x500).and_then(read_usize).filter(|value| *value != 0).is_none()
    {
        return Err("camera action/cursor producer pointers are null or unreadable".into());
    }
    let (input_events, event_nodes) = capture_camera_input_nodes(event_head)?;
    let batch = RawA729CameraEventBatch {
        producer_rva,
        owner_kind: auth.owner_kind,
        owner_vtable_rva: auth.owner_vtable_rva,
        action_source_owner_offset: 0x508,
        cursor_source_owner_offset: 0x500,
        event_list_owner_offset: auth.event_list_offset as u32,
        frame_scalar_f32_bits,
        producer_flag_0_u8,
        producer_flag_1_u8,
        input_events,
        map_picks: Vec::new(),
        current_eye_before_f32_bits: camera_vec3_bits(camera_root, 0x680)
            .ok_or("camera current eye before producer is unreadable")?,
        target_eye_before_f32_bits: camera_vec3_bits(camera_root, 0x698)
            .ok_or("camera target eye before producer is unreadable")?,
        target_look_at_before_f32_bits: camera_vec3_bits(camera_root, 0x6a4)
            .ok_or("camera target look-at before producer is unreadable")?,
        current_eye_after_f32_bits: [0; 3],
        target_eye_after_f32_bits: [0; 3],
        target_look_at_after_f32_bits: [0; 3],
    };
    CURRENT_CAMERA_EVENT_PRODUCER.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.is_some() {
            return Err("nested C03 camera producers are not authenticated".into());
        }
        *slot = Some(PendingCameraEventProducer {
            camera_root,
            batch,
            event_nodes,
            rejected: false,
        });
        Ok(())
    })
}

fn finish_camera_event_producer() -> Result<(), String> {
    let Some(mut pending) = CURRENT_CAMERA_EVENT_PRODUCER.with(|slot| slot.borrow_mut().take()) else {
        return Err("C03 camera producer exited without an active capture".into());
    };
    let auth = authenticate_camera_root(pending.camera_root)?;
    if auth.owner_kind != pending.batch.owner_kind
        || auth.owner_vtable_rva != pending.batch.owner_vtable_rva
        || auth.camera_root_offset == 0
        || pending.rejected
        || pending.event_nodes.len() != pending.batch.input_events.len()
    {
        return Err("C03 camera producer identity changed during capture".into());
    }
    for (row, node) in pending.batch.input_events.iter_mut().zip(&pending.event_nodes) {
        row.consumed_after_u8 = read_u8(
            node.checked_add(0x80).ok_or("camera consumed-after address overflow")?,
        )
        .ok_or("camera consumed-after byte is unreadable")?;
    }
    pending.batch.current_eye_after_f32_bits = camera_vec3_bits(pending.camera_root, 0x680)
        .ok_or("camera current eye after producer is unreadable")?;
    pending.batch.target_eye_after_f32_bits = camera_vec3_bits(pending.camera_root, 0x698)
        .ok_or("camera target eye after producer is unreadable")?;
    pending.batch.target_look_at_after_f32_bits = camera_vec3_bits(pending.camera_root, 0x6a4)
        .ok_or("camera target look-at after producer is unreadable")?;
    let mut state = STATE
        .get()
        .and_then(|value| value.lock().ok())
        .ok_or("render hook state is unavailable")?;
    let queue = state
        .pending_camera_event_batches
        .entry(pending.camera_root)
        .or_default();
    if queue.len() == MAX_PENDING_CAMERA_EVENT_BATCHES_PER_ROOT {
        queue.clear();
        return Err("pending C03 camera-event queue overflowed before a render envelope".into());
    }
    queue.push_back(pending.batch);
    Ok(())
}

unsafe extern "system" fn hook_camera_wheel_update(
    camera_root: usize,
    action_source: usize,
    cursor_source: usize,
    event_list_slot: usize,
    frame_scalar: f32,
    flag_0: u8,
    flag_1: u8,
) {
    let begin = authenticate_camera_root(camera_root).and_then(|auth| {
        if auth.owner.checked_add(0x508).and_then(read_usize) != Some(action_source)
            || auth.owner.checked_add(0x500).and_then(read_usize) != Some(cursor_source)
            || auth.owner.checked_add(auth.event_list_offset) != Some(event_list_slot)
        {
            return Err("C03 wheel producer arguments do not match authenticated owner offsets".into());
        }
        let event_head = read_usize(event_list_slot)
            .ok_or("C03 wheel event-list head is unreadable")?;
        begin_camera_event_producer(
            camera_root,
            CAMERA_WHEEL_UPDATE_RVA,
            event_head,
            Some(frame_scalar.to_bits()),
            flag_0,
            flag_1,
        )
    });
    if begin.is_err() {
        reject();
    }
    let original: CameraWheelUpdateFn =
        unsafe { mem::transmute(ORIGINAL_CAMERA_WHEEL_UPDATE.load(Ordering::Acquire)) };
    unsafe {
        original(
            camera_root,
            action_source,
            cursor_source,
            event_list_slot,
            frame_scalar,
            flag_0,
            flag_1,
        )
    };
    if begin.is_ok() && finish_camera_event_producer().is_err() {
        reject();
    }
}

unsafe extern "system" fn hook_camera_vertical_event_producer(owner: usize, flag: u8) {
    let begin = (|| {
        let camera_root = owner.checked_add(0x750).ok_or("vertical camera root overflow")?;
        let auth = authenticate_camera_root(camera_root)?;
        if auth.owner != owner || auth.owner_kind != RawA729CameraOwnerKind::InGame {
            return Err("vertical camera producer is not the authenticated in-game owner".into());
        }
        let event_head = read_usize(
            owner.checked_add(auth.event_list_offset).ok_or("vertical event-list address overflow")?,
        )
        .ok_or("vertical camera event-list head is unreadable")?;
        begin_camera_event_producer(
            camera_root,
            CAMERA_VERTICAL_EVENT_PRODUCER_RVA,
            event_head,
            None,
            flag,
            0,
        )
    })();
    if begin.is_err() {
        reject();
    }
    let original: CameraVerticalEventProducerFn = unsafe {
        mem::transmute(ORIGINAL_CAMERA_VERTICAL_EVENT_PRODUCER.load(Ordering::Acquire))
    };
    unsafe { original(owner, flag) };
    if begin.is_ok() && finish_camera_event_producer().is_err() {
        reject();
    }
}

unsafe extern "system" fn hook_camera_map_pick(
    source: usize,
    output_world_xyz: usize,
    input_screen_xy: usize,
    mode: u8,
) -> usize {
    let input = read_u32(input_screen_xy).zip(read_u32(input_screen_xy.saturating_add(4)));
    let original: CameraMapPickFn =
        unsafe { mem::transmute(ORIGINAL_CAMERA_MAP_PICK.load(Ordering::Acquire)) };
    let result = unsafe { original(source, output_world_xyz, input_screen_xy, mode) };
    CURRENT_CAMERA_EVENT_PRODUCER.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(pending) = slot.as_mut() else {
            return;
        };
        if pending.batch.map_picks.len() == MAX_CAMERA_INPUT_NODES {
            pending.rejected = true;
            return;
        }
        let output = camera_vec3_bits(output_world_xyz, 0);
        match (input, output, u32::try_from(pending.batch.map_picks.len())) {
            (Some((x, y)), Some(world), Ok(call_ordinal)) => {
                pending.batch.map_picks.push(RawA729CameraMapPick {
                    call_ordinal,
                    mode_u8: mode,
                    screen_xy_f32_bits: [x, y],
                    world_xyz_f32_bits: world,
                });
            }
            _ => pending.rejected = true,
        }
    });
    result
}

fn take_camera_event_batches(carrier: usize, carrier_ordinal: u8) -> Result<Vec<RawA729CameraEventBatch>, String> {
    let carrier_offset = [0x10usize, 0x230, 0x450]
        .get(usize::from(carrier_ordinal))
        .copied()
        .ok_or("camera event join carrier ordinal is outside 0..=2")?;
    let root = carrier
        .checked_sub(carrier_offset)
        .ok_or("camera event join carrier underflowed")?;
    let mut state = STATE
        .get()
        .and_then(|value| value.lock().ok())
        .ok_or("render hook state is unavailable")?;
    Ok(state
        .pending_camera_event_batches
        .remove(&root)
        .unwrap_or_default()
        .into_iter()
        .collect())
}

unsafe extern "system" fn hook_envelope(renderer: usize, camera: usize, carrier_ordinal: u32) {
    let submit_index = NEXT_SUBMIT_ID.fetch_add(1, Ordering::AcqRel);
    let mut camera_before = u8::try_from(carrier_ordinal)
        .ok()
        .filter(|ordinal| *ordinal <= 2)
        .ok_or_else(|| "a729 render envelope camera carrier ordinal is outside 0..=2".to_string())
        .and_then(|ordinal| capture_authenticated_camera_carrier(camera, ordinal).map(|row| (ordinal, row)));
    let camera_event_batches = camera_before
        .as_ref()
        .ok()
        .and_then(|(ordinal, carrier)| carrier.as_ref().map(|_| *ordinal))
        .map(|ordinal| take_camera_event_batches(camera, ordinal))
        .transpose();
    match camera_event_batches {
        Ok(Some(batches)) => {
            if let Ok((_, Some(carrier))) = &mut camera_before {
                carrier.camera_event_batches = batches;
            }
        }
        Err(error) => camera_before = Err(error),
        Ok(None) => {}
    }
    CURRENT_SUBMIT.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.is_some() {
            if let Some(mut state) = STATE.get().and_then(|value| value.lock().ok()) {
                state.nested_envelopes += 1;
            }
        }
        *slot = Some(PendingSubmit {
            submit_index,
            camera_carrier_ordinal: camera_before.as_ref().map(|row| row.0).unwrap_or(0),
            camera_carrier: camera_before.as_ref().ok().and_then(|row| row.1.clone()),
            next_builder_ordinal: 0,
            next_global_draw_order: 0,
            next_entity_pass_ordinal: 0,
            packets: Vec::new(),
            entity_submissions: Vec::new(),
            draw_states: Vec::new(),
            buffer_snapshots: Vec::new(),
            texture_resource_snapshots: Vec::new(),
            shader_resource_view_snapshots: Vec::new(),
            render_target_view_snapshots: Vec::new(),
            rejected: camera_before.is_err(),
        });
    });
    let original: EnvelopeFn = unsafe { mem::transmute(ORIGINAL_ENVELOPE.load(Ordering::Acquire)) };
    unsafe { original(renderer, camera, carrier_ordinal) };
    let completed = CURRENT_SUBMIT.with(|slot| slot.borrow_mut().take());
    if let Some(mut completed) = completed {
        let mut camera_after = capture_authenticated_camera_carrier(camera, completed.camera_carrier_ordinal);
        if let (Ok(Some(after)), Some(before)) = (&mut camera_after, &completed.camera_carrier) {
            after.camera_event_batches = before.camera_event_batches.clone();
        }
        if camera_after.as_ref().ok() != Some(&completed.camera_carrier) {
            completed.rejected = true;
        }
        let all_draws = completed
            .packets
            .len()
            .checked_add(completed.entity_submissions.len());
        if all_draws != usize::try_from(completed.next_global_draw_order).ok()
            || all_draws != Some(completed.draw_states.len())
            || completed
                .packets
                .iter()
                .any(|packet| packet.draw_order == u32::MAX)
            || completed
                .entity_submissions
                .iter()
                .any(|draw| draw.draw_order == u32::MAX)
        {
            completed.rejected = true;
        }
        if completed.rejected {
            reject();
            return;
        }
        if let Some(mut state) = STATE.get().and_then(|value| value.lock().ok()) {
            if state.completed.len() == MAX_SUBMITS {
                state.completed.pop_front();
                state.completed_catalogs.pop_front();
                state.evicted_submits += 1;
            }
            let submit = RawParticleSubmit {
                submit_index: completed.submit_index,
                camera_carrier_ordinal: completed.camera_carrier_ordinal,
                camera_carrier: completed.camera_carrier,
                packets: completed.packets,
                entity_submissions: completed.entity_submissions,
            };
            let Some(launch_epoch) = current_capture_launch_epoch() else {
                reject_locked(&mut state);
                return;
            };
            let Some(shader_creations) = materialize_shader_creations(
                &completed.draw_states,
                &state.shader_creation_observations,
            ) else {
                reject_locked(&mut state);
                return;
            };
            let Some(input_layout_creations) = select_latest_creation_rows(
                completed.draw_states.iter().filter_map(|draw| {
                    draw.input_layout_ptr
                        .map(|pointer| (pointer, draw.final_draw_sequence))
                }),
                &state.input_layout_creations,
                |row| (row.input_layout_ptr, row.creation_sequence),
            ) else {
                reject_locked(&mut state);
                return;
            };
            let Some(blend_state_creations) = select_latest_creation_rows(
                completed.draw_states.iter().filter_map(|draw| {
                    draw.blend_state_ptr
                        .map(|pointer| (pointer, draw.final_draw_sequence))
                }),
                &state.blend_state_creations,
                |row| (row.blend_state_ptr, row.creation_sequence),
            ) else {
                reject_locked(&mut state);
                return;
            };
            let Some(depth_stencil_state_creations) = select_latest_creation_rows(
                completed.draw_states.iter().filter_map(|draw| {
                    draw.depth_stencil_state_ptr
                        .map(|pointer| (pointer, draw.final_draw_sequence))
                }),
                &state.depth_stencil_state_creations,
                |row| (row.depth_stencil_state_ptr, row.creation_sequence),
            ) else {
                reject_locked(&mut state);
                return;
            };
            let Some(rasterizer_state_creations) = select_latest_creation_rows(
                completed.draw_states.iter().filter_map(|draw| {
                    draw.rasterizer_state_ptr
                        .map(|pointer| (pointer, draw.final_draw_sequence))
                }),
                &state.rasterizer_state_creations,
                |row| (row.rasterizer_state_ptr, row.creation_sequence),
            ) else {
                reject_locked(&mut state);
                return;
            };
            let Some(sampler_state_creations) = select_latest_creation_rows(
                completed.draw_states.iter().flat_map(|draw| {
                    draw.ordered_sampler_objects
                        .iter()
                        .map(move |value| (value.object_ptr, draw.final_draw_sequence))
                }),
                &state.sampler_state_creations,
                |row| (row.sampler_state_ptr, row.creation_sequence),
            ) else {
                reject_locked(&mut state);
                return;
            };
            let Some(next_publication_generation) =
                state.runtime_publication_generation.checked_add(1)
            else {
                reject_locked(&mut state);
                return;
            };
            state.completed_catalogs.push_back(RawRenderCatalogCapture {
                process: current_process_epoch(),
                launch_epoch,
                submit_index: completed.submit_index,
                frame_sequence: completed.submit_index,
                draw_states: completed.draw_states,
                draw_semantics: Vec::new(),
                program_identities: Vec::new(),
                shader_creations,
                input_layout_creations,
                blend_state_creations,
                depth_stencil_state_creations,
                rasterizer_state_creations,
                sampler_state_creations,
                buffer_snapshots: completed.buffer_snapshots,
                texture_resource_snapshots: completed.texture_resource_snapshots,
                shader_resource_view_snapshots: completed.shader_resource_view_snapshots,
                render_target_view_snapshots: completed.render_target_view_snapshots,
            });
            state.completed.push_back(submit);
            state.runtime_publication_generation = next_publication_generation;
            if purge_published_sealed_render_submit(&mut state, completed.submit_index).is_err() {
                reject_locked(&mut state);
                return;
            }
            if compact_published_current_hour_candidates(&mut state, completed.submit_index)
                .is_err()
            {
                reject_locked(&mut state);
                return;
            }
            notify_render_submit_state_change();
        }
    } else {
        reject();
    }
}

fn purge_published_sealed_render_submit(
    state: &mut HookState,
    submit_index: u64,
) -> Result<(), String> {
    let Some(frame) = state
        .completed_render_frames
        .iter()
        .find(|frame| frame.submit_index == submit_index)
    else {
        return Ok(());
    };
    let submit_ready = state
        .completed
        .iter()
        .any(|submit| submit.submit_index == submit_index);
    let catalog_ready = state
        .completed_catalogs
        .iter()
        .any(|catalog| catalog.submit_index == submit_index);
    if submit_ready != catalog_ready {
        return Err("render-frame purge sees a split runtime submit/catalog join".into());
    }
    if !submit_ready {
        return Ok(());
    }
    if let Some(latched) = state.latched_render_hour {
        if frame.game_hour_raw > latched.game_hour_raw {
            return Ok(());
        }
        if frame.presentation_qpc_frequency_hz != latched.cutoff_qpc_frequency_hz {
            return Err("latched render-frame publication crosses its QPC epoch".into());
        }
        if frame.game_hour_raw == latched.game_hour_raw
            && frame.presentation_qpc_ticks <= latched.cutoff_qpc_ticks
        {
            // This row belongs to the frozen candidate set. It must remain
            // available for the selector, or for bounded eligible-prefix
            // compaction, until the exact runtime join is complete.
            return Ok(());
        }
        return purge_completed_render_submit(state, submit_index);
    }
    let Some(sealed) = state.sealed_render_hour else {
        return Ok(());
    };
    if frame.game_hour_raw > sealed.game_hour_raw {
        return Ok(());
    }
    if frame.presentation_qpc_frequency_hz != sealed.cutoff_qpc_frequency_hz
        || (frame.game_hour_raw == sealed.game_hour_raw
            && frame.presentation_qpc_ticks <= sealed.cutoff_qpc_ticks)
    {
        return Err("sealed render-frame late publication violates cutoff epoch/order".into());
    }
    purge_completed_render_submit(state, submit_index)
}

fn purge_completed_render_submit(state: &mut HookState, submit_index: u64) -> Result<(), String> {
    let frame = state
        .completed_render_frames
        .iter()
        .find(|frame| frame.submit_index == submit_index)
        .ok_or("render-frame purge lost its frame")?;
    let submit = state
        .completed
        .iter()
        .find(|submit| submit.submit_index == submit_index)
        .ok_or("sealed render-frame purge lacks its runtime submit")?;
    let catalog = state
        .completed_catalogs
        .iter()
        .find(|catalog| catalog.submit_index == submit_index)
        .ok_or("sealed render-frame purge lacks its runtime catalog")?;
    let receipt = consumed_unselected_receipt(frame, submit, catalog)?;
    discard_unselected_render_authority(std::slice::from_ref(&receipt))?;
    remove_render_join_ids(state, &BTreeSet::from([submit_index]));
    let mut hasher = Sha256::new();
    hasher.update(b"hoi4.a729.sealed-purge-rolling.v1\0");
    hasher.update(state.sealed_purge_rolling_sha256);
    hash_consumed_unselected_receipt(&mut hasher, &receipt)?;
    state
        .sealed_purge_rolling_sha256
        .copy_from_slice(&hasher.finalize());
    match state.sealed_purge_receipt_count.checked_add(1) {
        Some(next) => state.sealed_purge_receipt_count = next,
        None => {
            state.sealed_purge_receipt_count = 0;
            state.sealed_purge_receipt_count_overflowed = true;
        }
    }
    Ok(())
}

/// Retires superseded *complete* Present/runtime/catalog joins for the hour of
/// `submit_index` as soon as publication makes that join available. The exact
/// authority IDs are discarded before the full rows are removed. A fixed-size
/// rolling proof preserves every candidate and purge receipt while only the
/// latest complete Present remains resident. Incomplete joins are untouched.
fn compact_published_current_hour_candidates(
    state: &mut HookState,
    submit_index: u64,
) -> Result<(), String> {
    let Some(game_hour_raw) = state
        .completed_render_frames
        .iter()
        .find(|frame| frame.submit_index == submit_index)
        .map(|frame| frame.game_hour_raw)
    else {
        return Ok(());
    };
    if !state
        .completed
        .iter()
        .any(|submit| submit.submit_index == submit_index)
        || !state
            .completed_catalogs
            .iter()
            .any(|catalog| catalog.submit_index == submit_index)
    {
        return Ok(());
    }
    if state
        .sealed_render_hour
        .is_some_and(|sealed| game_hour_raw <= sealed.game_hour_raw)
    {
        return Ok(());
    }
    let eligible_cutoff = state
        .latched_render_hour
        .filter(|latched| latched.game_hour_raw == game_hour_raw)
        .map(|latched| latched.cutoff_qpc_ticks);
    let mut ordered_frames = state
        .completed_render_frames
        .iter()
        .filter(|frame| frame.game_hour_raw == game_hour_raw)
        .filter(|frame| eligible_cutoff.is_none_or(|cutoff| frame.presentation_qpc_ticks <= cutoff))
        .map(|frame| {
            (
                frame.presentation_qpc_ticks,
                frame.global_order,
                frame.frame_sequence,
                frame.submit_index,
            )
        })
        .collect::<Vec<_>>();
    ordered_frames.sort_unstable();
    if ordered_frames.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err("online current-hour Present metadata is reordered or duplicated".into());
    }

    // Runtime publication can lag Present completion. Only the contiguous
    // ready prefix is eligible for folding; a later ready row after a gap may
    // not leapfrog the missing candidate into the rolling commitment.
    let mut ready_prefix = Vec::new();
    for row in ordered_frames {
        let submit_ready = state
            .completed
            .iter()
            .any(|submit| submit.submit_index == row.3);
        let catalog_ready = state
            .completed_catalogs
            .iter()
            .any(|catalog| catalog.submit_index == row.3);
        if submit_ready != catalog_ready {
            return Err("online current-hour runtime submit/catalog queues are split".into());
        }
        if !submit_ready {
            break;
        }
        ready_prefix.push(row);
    }
    if ready_prefix.len() <= 1 {
        return Ok(());
    }
    let retained = *ready_prefix
        .last()
        .ok_or("online current-hour candidate set became empty")?;
    let retained_frame = state
        .completed_render_frames
        .iter()
        .find(|frame| frame.submit_index == retained.3)
        .ok_or("online retained candidate lost its frame")?;

    if let Some(proof) = state.online_candidate_proofs.get(&game_hour_raw) {
        validate_online_candidate_proof_shape(proof)?;
        let first_order = (ready_prefix[0].0, ready_prefix[0].1, ready_prefix[0].2);
        if proof
            .last_purged_order
            .is_some_and(|last| first_order <= last)
        {
            return Err("online current-hour candidate prefix regressed behind its proof".into());
        }
    }

    let mut receipts = Vec::with_capacity(ready_prefix.len() - 1);
    let mut candidate_rows = Vec::with_capacity(ready_prefix.len() - 1);
    for (_, _, _, purge_id) in ready_prefix.iter().copied().take(ready_prefix.len() - 1) {
        let frame = state
            .completed_render_frames
            .iter()
            .find(|frame| frame.submit_index == purge_id)
            .ok_or("online candidate lost its frame")?;
        if frame.presentation_qpc_frequency_hz != retained_frame.presentation_qpc_frequency_hz
            || frame.process_start_nonce != retained_frame.process_start_nonce
            || frame.hook_epoch_nonce != retained_frame.hook_epoch_nonce
        {
            return Err("online current-hour candidates cross QPC/process/hook epoch".into());
        }
        let submit = state
            .completed
            .iter()
            .find(|submit| submit.submit_index == purge_id)
            .ok_or("online candidate lost its runtime submit")?;
        let catalog = state
            .completed_catalogs
            .iter()
            .find(|catalog| catalog.submit_index == purge_id)
            .ok_or("online candidate lost its runtime catalog")?;
        candidate_rows.push((frame.clone(), catalog.clone()));
        receipts.push(consumed_unselected_receipt(frame, submit, catalog)?);
    }

    discard_unselected_render_authority(&receipts)?;
    let proof = state
        .online_candidate_proofs
        .entry(game_hour_raw)
        .or_default();
    for ((frame, catalog), receipt) in candidate_rows.iter().zip(&receipts) {
        fold_online_candidate_proof(proof, frame, catalog, receipt)?;
    }
    let purged_ids = receipts
        .iter()
        .map(|receipt| receipt.submit_index)
        .collect::<BTreeSet<_>>();
    remove_render_join_ids(state, &purged_ids);
    Ok(())
}

unsafe extern "system" fn hook_builder(
    runtime: usize,
    normal: usize,
    normal_count: *mut u32,
    ribbon: usize,
    ribbon_count: *mut u32,
    capacity: u32,
) {
    let original: BuilderFn = unsafe { mem::transmute(ORIGINAL_BUILDER.load(Ordering::Acquire)) };
    unsafe {
        original(
            runtime,
            normal,
            normal_count,
            ribbon,
            ribbon_count,
            capacity,
        )
    };
    let Some(type_tables) = read_particle_type_subsystem_tables(runtime) else {
        reject_current();
        return;
    };
    if !CURRENT_PARTICLE_DRAW_RANGE.with(|range| range.borrow().is_some()) {
        reject_current();
        return;
    }
    let Some(identity) = STATE
        .get()
        .and_then(|value| value.lock().ok())
        .and_then(|state| state.runtimes.get(&runtime).cloned())
    else {
        reject_current();
        return;
    };
    if identity.render_list_class == u32::MAX || identity.render_subbucket == u16::MAX {
        reject_current();
        return;
    }
    let (Some(normal_count), Some(ribbon_count)) = (
        read_u32(normal_count as usize),
        read_u32(ribbon_count as usize),
    ) else {
        reject_current();
        return;
    };
    if normal_count > capacity
        || ribbon_count > capacity
        || normal_count.saturating_add(ribbon_count) > capacity
    {
        reject_current();
        return;
    }
    CURRENT_SUBMIT.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(submit) = slot.as_mut() else {
            reject();
            return;
        };
        let builder = submit.next_builder_ordinal;
        submit.next_builder_ordinal = submit.next_builder_ordinal.wrapping_add(1);
        for (base, count, ribbon_tail) in
            [(normal, normal_count, false), (ribbon, ribbon_count, true)]
        {
            for index in 0..count {
                if submit.packets.len() >= MAX_PACKETS_PER_SUBMIT {
                    submit.rejected = true;
                    return;
                }
                let Some(packet) = capture_packet(
                    base + index as usize * PACKET_BYTES,
                    &identity,
                    &type_tables,
                    builder,
                    u32::MAX,
                    ribbon_tail.then_some(index),
                ) else {
                    submit.rejected = true;
                    return;
                };
                submit.packets.push(packet);
            }
        }
    });
}

unsafe extern "system" fn hook_particle_render_submit(
    renderer: usize,
    render_context: usize,
    view_context: usize,
    camera: usize,
    render_class: f32,
) {
    CURRENT_RENDER_PROGRAM.with(|value| *value.borrow_mut() = None);
    let start = CURRENT_SUBMIT.with(|slot| slot.borrow().as_ref().map(|s| s.packets.len()));
    let nested = CURRENT_PARTICLE_DRAW_RANGE.with(|range| {
        let mut range = range.borrow_mut();
        let nested = range.is_some();
        *range = start.map(|start| DrawRange { start, end: None });
        nested
    });
    if nested || start.is_none() {
        reject_current();
    }
    let original: ParticleRenderSubmitFn =
        unsafe { mem::transmute(ORIGINAL_PARTICLE_RENDER_SUBMIT.load(Ordering::Acquire)) };
    unsafe { original(renderer, render_context, view_context, camera, render_class) };
    let range = CURRENT_PARTICLE_DRAW_RANGE.with(|range| range.borrow_mut().take());
    let complete = range.is_some_and(|range| {
        CURRENT_SUBMIT.with(|slot| {
            let slot = slot.borrow();
            let Some(submit) = slot.as_ref() else {
                return false;
            };
            submit.packets[range.start..]
                .iter()
                .all(|packet| packet.draw_order != u32::MAX)
        })
    });
    if !complete {
        reject_current();
    }
}

unsafe extern "system" fn hook_particle_final_draw(
    render_context: usize,
    vertex_count: i32,
    start_vertex: u32,
) {
    let program = CURRENT_RENDER_PROGRAM.with(|value| value.borrow_mut().take());
    let active = CURRENT_PARTICLE_DRAW_RANGE.with(|range| range.borrow().is_some());
    let observed_program = if active {
        program
    } else {
        bound_render_program(render_context)
    };
    if a25_gui_bridge::owner_run_is_active() {
        if let (Ok(vertex_count), Some(program)) = (
            u32::try_from(vertex_count),
            observed_program.filter(|program| program.render_context_ptr == render_context),
        ) {
            a25_gui_bridge::record_final_draw(
                PARTICLE_FINAL_DRAW_RVA as u32,
                RawDrawCall::Draw {
                    vertex_count,
                    start_vertex,
                },
                render_context,
                program.shader_program_ptr,
            );
        }
    }
    let assignment = CURRENT_PARTICLE_DRAW_RANGE.with(|range| {
        let Some(range) = *range.borrow() else {
            return None;
        };
        CURRENT_SUBMIT.with(|slot| {
            let mut slot = slot.borrow_mut();
            let Some(submit) = slot.as_mut() else {
                return None;
            };
            let Some(relative) = submit.packets[range.start..]
                .iter()
                .position(|packet| packet.draw_order == u32::MAX)
            else {
                return None;
            };
            let Some(program) = program.filter(|value| {
                value.render_context_ptr == render_context && value.shader_program_ptr != 0
            }) else {
                return None;
            };
            let engine_effect =
                STATE
                    .get()
                    .and_then(|value| value.lock().ok())
                    .and_then(|state| {
                        state
                            .engine_effect_tokens_by_program
                            .get(&program.shader_program_ptr)
                            .cloned()
                    })?;
            let Some(next) = submit.next_global_draw_order.checked_add(1) else {
                return None;
            };
            let packet = &mut submit.packets[range.start + relative];
            if engine_effect.program_index_u32 != packet.shader_program_index_u32 {
                return None;
            }
            packet.draw_order = submit.next_global_draw_order;
            packet.shader_program_ptr = program.shader_program_ptr;
            packet.engine_effect_token = engine_effect.effect_token;
            let identity = RawDrawIdentity::Particle {
                effect_instance_id: packet.effect_instance_id,
                registration_token: packet.definition.registration_token.clone(),
                definition_source_token: packet.definition.definition_source_token.clone(),
                builder_call_ordinal: packet.builder_call_ordinal,
                packet_ordinal: packet.packet_ordinal,
                draw_order: packet.draw_order,
                shader_program_ptr: packet.shader_program_ptr,
            };
            let assignment = (identity, submit.submit_index, packet.draw_order);
            submit.next_global_draw_order = next;
            Some(assignment)
        })
    });
    if active && assignment.is_none() {
        reject_current();
    }
    if active || inside_non_ui_render_phase() {
        let _ = record_final_draw(
            FinalDrawCall::Draw,
            if active {
                FinalDrawFamily::Particle
            } else {
                FinalDrawFamily::StaticOrPostProcess
            },
            PARTICLE_FINAL_DRAW_RVA,
            render_context,
            observed_program.map_or(0, |value| value.shader_program_ptr),
            [vertex_count as u32, start_vertex, 0, 0],
        );
        if active {
            let captured = assignment.and_then(|(identity, submit_index, draw_order)| {
                let vertex_count = u32::try_from(vertex_count).ok()?;
                capture_runtime_draw_state(
                    next_catalog_observation_sequence()?,
                    identity,
                    RawDrawFamily::Particle,
                    submit_index,
                    draw_order,
                    render_context,
                    observed_program?.shader_program_ptr,
                    RawDrawCall::Draw {
                        vertex_count,
                        start_vertex,
                    },
                )
            });
            CURRENT_SUBMIT.with(|slot| {
                let mut slot = slot.borrow_mut();
                if let (Some(submit), Some(captured)) = (slot.as_mut(), captured) {
                    let authenticated_optional_null = captured.state.geometry_shader_ptr.is_none()
                        && captured.state.hull_shader_ptr.is_none()
                        && captured.state.domain_shader_ptr.is_none()
                        && captured.state.compute_shader_ptr.is_none()
                        && !captured.state.dynamic_shader_linkage_present;
                    let Some(packet) = submit
                        .packets
                        .iter_mut()
                        .find(|packet| packet.draw_order == captured.state.draw_order)
                    else {
                        submit.rejected = true;
                        return;
                    };
                    packet.additional_shader_stages_authenticated_null =
                        authenticated_optional_null;
                    packet.stream_output_targets_authenticated_null =
                        captured.state.ordered_stream_output_buffer_ptrs.is_empty();
                    submit.buffer_snapshots.extend(captured.buffer_snapshots);
                    submit
                        .texture_resource_snapshots
                        .extend(captured.texture_resource_snapshots);
                    submit
                        .shader_resource_view_snapshots
                        .extend(captured.shader_resource_view_snapshots);
                    submit
                        .render_target_view_snapshots
                        .extend(captured.render_target_view_snapshots);
                    submit.draw_states.push(captured.state);
                } else if let Some(submit) = slot.as_mut() {
                    submit.rejected = true;
                } else {
                    reject();
                }
            });
        }
    }
    let original: ParticleFinalDrawFn =
        unsafe { mem::transmute(ORIGINAL_PARTICLE_FINAL_DRAW.load(Ordering::Acquire)) };
    unsafe { original(render_context, vertex_count, start_vertex) };
}

unsafe extern "system" fn hook_entity_factory(
    definition_ptr: usize,
    class: u32,
    subtype: u32,
    variant: u32,
) -> usize {
    let stable_id = NEXT_ENTITY_ID.fetch_add(1, Ordering::AcqRel);
    ENTITY_FACTORY_STACK.with(|stack| {
        stack.borrow_mut().push(PendingEntityFactory {
            stable_id,
            definition_ptr,
            ordered_effect_instance_ids: Vec::new(),
        });
    });
    let original: EntityFactoryFn =
        unsafe { mem::transmute(ORIGINAL_ENTITY_FACTORY.load(Ordering::Acquire)) };
    let entity = unsafe { original(definition_ptr, class, subtype, variant) };
    let pending = ENTITY_FACTORY_STACK.with(|stack| stack.borrow_mut().pop());
    let Some(pending) = pending else {
        reject();
        return entity;
    };
    if pending.stable_id != stable_id || pending.definition_ptr != definition_ptr {
        reject();
        return entity;
    }
    if entity != 0 {
        if let Some(mut state) = STATE.get().and_then(|value| value.lock().ok()) {
            let registration_token = state
                .entity_definition_tokens
                .get(&definition_ptr)
                .cloned();
            let identity = EntityIdentity {
                stable_id,
                definition_ptr,
                registration_token,
                ordered_effect_instance_ids: pending.ordered_effect_instance_ids,
            };
            if state.entities.insert(entity, identity).is_some() {
                state.duplicate_entity_ids += 1;
            }
        }
    }
    CURRENT_WEATHER.with(|slot| {
        if let Some(weather) = slot.borrow_mut().as_mut() {
            weather.factory_attempts.push(RawWeatherFactoryAttempt {
                definition_ptr,
                allocated_entity_instance_id: stable_id,
                returned_entity_instance_id: (entity != 0).then_some(stable_id),
            });
        }
    });
    entity
}

unsafe extern "system" fn hook_entity_definition_lookup(
    registry: usize,
    entity_name_ptr: usize,
) -> usize {
    let entity_name = read_c_string(entity_name_ptr, 512);
    let original: EntityDefinitionLookupFn = unsafe {
        mem::transmute(ORIGINAL_ENTITY_DEFINITION_LOOKUP.load(Ordering::Acquire))
    };
    let definition_ptr = unsafe { original(registry, entity_name_ptr) };
    if definition_ptr == 0 {
        return definition_ptr;
    }
    let Some(entity_name) = entity_name.filter(|token| !token.is_empty()) else {
        reject();
        return definition_ptr;
    };
    let Some(mut state) = STATE.get().and_then(|value| value.lock().ok()) else {
        reject();
        return definition_ptr;
    };
    if state
        .entity_definition_tokens
        .get(&definition_ptr)
        .is_some_and(|existing| existing != &entity_name)
    {
        reject_locked(&mut state);
        return definition_ptr;
    }
    state
        .entity_definition_tokens
        .insert(definition_ptr, entity_name);
    definition_ptr
}

unsafe extern "system" fn hook_entity_named_factory(
    registry: usize,
    entity_name_ptr: usize,
    class: u32,
    subtype: u32,
    variant: u32,
) -> usize {
    let entity_name = read_c_string(entity_name_ptr, 512);
    let original: EntityNamedFactoryFn =
        unsafe { mem::transmute(ORIGINAL_ENTITY_NAMED_FACTORY.load(Ordering::Acquire)) };
    let entity = unsafe { original(registry, entity_name_ptr, class, subtype, variant) };
    if entity == 0 {
        return entity;
    }
    let Some(entity_name) = entity_name.filter(|token| !token.is_empty()) else {
        reject();
        return entity;
    };
    let Some(definition_ptr) = entity.checked_add(0x40).and_then(read_usize) else {
        reject();
        return entity;
    };
    let Some(mut state) = STATE.get().and_then(|value| value.lock().ok()) else {
        reject();
        return entity;
    };
    let Some(identity) = state.entities.get_mut(&entity) else {
        reject_locked(&mut state);
        return entity;
    };
    if identity.definition_ptr != definition_ptr
        || identity
            .registration_token
            .as_ref()
            .is_some_and(|existing| existing != &entity_name)
    {
        reject_locked(&mut state);
        return entity;
    }
    identity.registration_token = Some(entity_name);
    entity
}

unsafe extern "system" fn hook_vfs_open_constructor(
    archive_object: usize,
    moved_path: usize,
    open_context: usize,
    open_flags: u32,
) -> usize {
    // The a729 constructor moves and clears this MSVC string, so the exact
    // logical path must be copied before control enters the original.
    let logical_path = read_msvc_string(moved_path, 4_096);
    let original: VfsOpenConstructorFn =
        unsafe { mem::transmute(ORIGINAL_VFS_OPEN_CONSTRUCTOR.load(Ordering::Acquire)) };
    let result = unsafe { original(archive_object, moved_path, open_context, open_flags) };
    if result == 0 {
        reject();
        return result;
    }
    let Some(logical_path) = logical_path.filter(|path| !path.is_empty()) else {
        reject();
        return result;
    };
    let Some(stream_handle) = result.checked_add(0x38).and_then(read_usize) else {
        reject();
        return result;
    };
    if stream_handle == 0 {
        reject();
        return result;
    }
    let observation_sequence = NEXT_CATALOG_OBSERVATION_SEQUENCE.fetch_add(1, Ordering::AcqRel);
    if observation_sequence == 0 {
        reject();
        return result;
    }
    TEXTURE_LOAD_STACK.with(|stack| {
        if let Some(active) = stack
            .borrow_mut()
            .iter_mut()
            .rev()
            .find(|active| active.logical_path == logical_path)
        {
            active.nested_open_sequences.push(observation_sequence);
        }
    });
    let Some(mut state) = STATE.get().and_then(|value| value.lock().ok()) else {
        reject();
        return result;
    };
    if state.vfs_open_receipts.len() == MAX_VFS_OPEN_RECEIPTS {
        reject_locked(&mut state);
        return result;
    }
    state.vfs_open_receipts.push_back(RawVfsOpenReceipt {
        observation_sequence,
        ordered_logical_path_tokens: vec![logical_path.clone()],
        archive_object_ptr: result,
        stream_handle,
        data_ptr: None,
    });
    if !state
        .latest_vfs_open_sequence_by_path
        .contains_key(&logical_path)
        && state.latest_vfs_open_sequence_by_path.len() == MAX_VFS_OPEN_RECEIPTS
    {
        reject_locked(&mut state);
        return result;
    }
    state
        .latest_vfs_open_sequence_by_path
        .insert(logical_path, observation_sequence);
    result
}

unsafe extern "system" fn hook_texture_cache_load_root(
    texture_manager: usize,
    authored_path: usize,
    options: usize,
    load_if_missing: u8,
) -> u32 {
    let original: TextureCacheLoadRootFn =
        unsafe { mem::transmute(ORIGINAL_TEXTURE_CACHE_LOAD_ROOT.load(Ordering::Acquire)) };
    let Some(logical_path) = read_msvc_string(authored_path, 4_096) else {
        reject();
        return unsafe { original(texture_manager, authored_path, options, load_if_missing) };
    };
    let Some(entry_sequence) = next_catalog_observation_sequence() else {
        reject();
        return unsafe { original(texture_manager, authored_path, options, load_if_missing) };
    };
    TEXTURE_LOAD_STACK.with(|stack| {
        stack.borrow_mut().push(PendingTextureLoad {
            entry_sequence,
            logical_path: logical_path.clone(),
            nested_open_sequences: Vec::new(),
            nested_texture_creations: Vec::new(),
            nested_srv_creations: Vec::new(),
        });
    });
    let runtime_texture_index_u32 =
        unsafe { original(texture_manager, authored_path, options, load_if_missing) };
    let Some(exit_sequence) = next_catalog_observation_sequence() else {
        reject();
        return runtime_texture_index_u32;
    };
    let pending = TEXTURE_LOAD_STACK.with(|stack| stack.borrow_mut().pop());
    let Some(pending) = pending
        .filter(|row| row.entry_sequence == entry_sequence && row.logical_path == logical_path)
    else {
        reject();
        return runtime_texture_index_u32;
    };
    let Some(mut state) = STATE.get().and_then(|value| value.lock().ok()) else {
        return runtime_texture_index_u32;
    };
    if finalize_texture_load_locked(
        &mut state,
        pending,
        exit_sequence,
        runtime_texture_index_u32,
    )
    .is_none()
    {
        reject_locked(&mut state);
    }
    runtime_texture_index_u32
}

fn finalize_texture_load_locked(
    state: &mut HookState,
    pending: PendingTextureLoad,
    exit_sequence: u64,
    runtime_texture_index_u32: u32,
) -> Option<()> {
    let entry_sequence = pending.entry_sequence;
    let logical_path = pending.logical_path.clone();
    let cache_key = (logical_path.clone(), runtime_texture_index_u32);
    if pending.nested_texture_creations.is_empty() {
        let valid_hit = state
            .texture_cache_associations
            .get(&cache_key)
            .and_then(|resource_ptr| state.texture_vfs_resource_joins.get(resource_ptr))
            .is_some_and(|join| {
                join.logical_path == logical_path
                    && join.runtime_texture_index_u32 == runtime_texture_index_u32
            });
        if !valid_hit
            || !pending.nested_open_sequences.is_empty()
            || !pending.nested_srv_creations.is_empty()
        {
            return None;
        }
        return Some(());
    }
    if pending.nested_open_sequences.len() != 1 || pending.nested_texture_creations.len() != 1 {
        return None;
    }
    let source_open_observation_sequence = pending.nested_open_sequences[0];
    let (resource_creation_sequence, resource_ptr) = pending.nested_texture_creations[0];
    let matching_srvs = pending
        .nested_srv_creations
        .iter()
        .copied()
        .filter(|(_, _, created_from)| *created_from == resource_ptr)
        .collect::<Vec<_>>();
    if matching_srvs.len() != 1
        || pending.nested_srv_creations.len() != 1
        || !(entry_sequence < source_open_observation_sequence
            && source_open_observation_sequence < resource_creation_sequence
            && resource_creation_sequence < matching_srvs[0].0
            && matching_srvs[0].0 < exit_sequence)
    {
        return None;
    }
    let (shader_resource_view_creation_sequence, shader_resource_view_ptr, _) = matching_srvs[0];
    let join = RawTextureVfsResourceJoin {
        root_entry_sequence: entry_sequence,
        root_exit_sequence: exit_sequence,
        source_open_observation_sequence,
        resource_creation_sequence,
        shader_resource_view_creation_sequence,
        logical_path: logical_path.clone(),
        runtime_texture_index_u32,
        resource_ptr,
        shader_resource_view_ptr,
    };
    if state.texture_vfs_resource_joins.len() >= MAX_D3D_CREATION_ROWS
        || state
            .texture_cache_associations
            .get(&cache_key)
            .is_some_and(|existing| *existing != resource_ptr)
        || state
            .texture_vfs_resource_joins
            .get(&resource_ptr)
            .is_some_and(|existing| existing != &join)
    {
        return None;
    }
    state
        .texture_cache_associations
        .insert(cache_key, resource_ptr);
    state.texture_vfs_resource_joins.insert(resource_ptr, join);
    Some(())
}

fn capture_registered_texture2d_target(
    view_ptr: usize,
    expected_resource_ptr: Option<usize>,
) -> Option<(usize, RawTexture2dDesc, RawResourceViewDesc)> {
    let (resource_ptr, rtv_desc) =
        capture_view_desc(view_ptr, CreatedD3dObjectKind::RenderTargetView)?;
    if expected_resource_ptr.is_some_and(|expected| expected != resource_ptr) {
        return None;
    }
    let meta = capture_texture_meta(resource_ptr)?;
    // D3D11_RESOURCE_DIMENSION_TEXTURE2D == 3.
    if meta.dimension != 3 {
        return None;
    }
    type GetDescFn = unsafe extern "system" fn(usize, *mut D3D11_TEXTURE2D_DESC);
    let get_desc: GetDescFn = unsafe { mem::transmute(com_method(resource_ptr, 10)?) };
    let mut raw = unsafe { mem::zeroed::<D3D11_TEXTURE2D_DESC>() };
    unsafe { get_desc(resource_ptr, &mut raw) };
    let resource_desc = RawTexture2dDesc {
        width: raw.Width,
        height: raw.Height,
        mip_levels: raw.MipLevels,
        array_size: raw.ArraySize,
        dxgi_format_u32: raw.Format,
        sample_count: raw.SampleDesc.Count,
        sample_quality: raw.SampleDesc.Quality,
        usage_u32: raw.Usage,
        bind_flags_u32: raw.BindFlags,
        cpu_access_flags_u32: raw.CPUAccessFlags,
        misc_flags_u32: raw.MiscFlags,
    };
    if resource_desc.width == 0
        || resource_desc.height == 0
        || resource_desc.mip_levels == 0
        || resource_desc.array_size == 0
        || resource_desc.sample_count == 0
    {
        return None;
    }
    Some((resource_ptr, resource_desc, rtv_desc))
}

fn register_render_target_locked(
    state: &mut HookState,
    producer_rva: u64,
    recreate_generation: u64,
    kind: RawRegisteredRenderTargetKind,
    view_ptr: usize,
    resource_ptr: usize,
    resource_desc: RawTexture2dDesc,
    rtv_desc: RawResourceViewDesc,
) -> Option<()> {
    let registration_sequence = next_catalog_observation_sequence()?;
    state.registered_render_targets.insert(
        view_ptr,
        RawRenderTargetRegistrationObservation {
            registration_sequence,
            // Replaced by the exact draw/clear sequence when this transient
            // registry entry is resolved through the OM binding.
            capture_sequence: registration_sequence,
            producer_rva,
            recreate_generation,
            attachment_ordinal: 0,
            kind,
            view_ptr,
            resource_ptr,
            resource_desc,
            rtv_desc,
        },
    );
    Some(())
}

unsafe extern "system" fn hook_window_backbuffer_rtv_producer(renderer: usize) -> i32 {
    let original: WindowBackbufferRtvProducerFn =
        unsafe { mem::transmute(ORIGINAL_WINDOW_BACKBUFFER_RTV_PRODUCER.load(Ordering::Acquire)) };
    let prior_view = read_usize(renderer.checked_add(0x268).unwrap_or(0))
        .and_then(|backend| read_usize(backend.checked_add(0x40)?));
    let result = unsafe { original(renderer) };
    if result < 0 {
        return result;
    }
    let read_post = || {
        let backend = read_usize(renderer.checked_add(0x268)?)?;
        Some((
            backend,
            read_usize(backend.checked_add(0x30)?)?,
            read_usize(backend.checked_add(0x20)?)?,
            read_usize(backend.checked_add(0x40)?)?,
            read_u32(backend.checked_add(0x25c)?)?,
        ))
    };
    let Some(post_before) = read_post() else {
        reject();
        return result;
    };
    let Some(post_after) = read_post() else {
        reject();
        return result;
    };
    if post_before != post_after {
        reject();
        return result;
    }
    let (backend, swapchain, device, view_ptr, ready) = post_before;
    if backend == 0 || swapchain == 0 || device == 0 || view_ptr == 0 || ready != 1 {
        reject();
        return result;
    }
    let Some((resource_ptr, resource_desc, rtv_desc)) =
        capture_registered_texture2d_target(view_ptr, None)
    else {
        reject();
        return result;
    };
    let Some(mut state) = STATE.get().and_then(|value| value.lock().ok()) else {
        return result;
    };
    let Some(recreate_generation) = state.swapchain_recreate_generation.checked_add(1) else {
        reject_locked(&mut state);
        return result;
    };
    if let Some(old) = prior_view.or(state.swapchain_backbuffer_view_ptr) {
        state.registered_render_targets.remove(&old);
    }
    if register_render_target_locked(
        &mut state,
        WINDOW_BACKBUFFER_RTV_PRODUCER_RVA as u64,
        recreate_generation,
        RawRegisteredRenderTargetKind::SwapchainBackBuffer { buffer_index: 0 },
        view_ptr,
        resource_ptr,
        resource_desc,
        rtv_desc,
    )
    .is_none()
    {
        reject_locked(&mut state);
        return result;
    }
    state.swapchain_recreate_generation = recreate_generation;
    state.swapchain_backbuffer_view_ptr = Some(view_ptr);
    result
}

unsafe extern "system" fn hook_opaque_texture_target_producer(
    renderer: usize,
    source_desc: usize,
) -> u64 {
    let original: OpaqueTextureTargetProducerFn =
        unsafe { mem::transmute(ORIGINAL_OPAQUE_TEXTURE_TARGET_PRODUCER.load(Ordering::Acquire)) };
    let read_source = || {
        Some((
            read_u32(source_desc.checked_add(0x10)?)?,
            read_u32(source_desc.checked_add(0x14)?)?,
            read_u32(source_desc.checked_add(0x08)?)?,
            read_u32(source_desc.checked_add(0x0c)?)?,
        ))
    };
    let before = read_source().filter(|value| Some(*value) == read_source());
    let result = unsafe { original(renderer, source_desc) };
    if result as u32 != 0 {
        return result;
    }
    let Some((source_width, source_height, _source_pixel_format, source_flags_u32)) = before else {
        reject();
        return result;
    };
    if source_flags_u32 & 2 == 0 {
        return result;
    }
    let Some((texture_ptr, view_ptr)) = (|| {
        let wrapper = read_usize(source_desc.checked_add(0x70)?)?;
        Some((
            read_usize(wrapper)?,
            read_usize(wrapper.checked_add(0x10)?)?,
        ))
    })() else {
        reject();
        return result;
    };
    if texture_ptr == 0 || view_ptr == 0 {
        reject();
        return result;
    }
    let Some((resource_ptr, resource_desc, rtv_desc)) =
        capture_registered_texture2d_target(view_ptr, Some(texture_ptr))
    else {
        reject();
        return result;
    };
    if resource_desc.width != source_width || resource_desc.height != source_height {
        reject();
        return result;
    }
    let Some(mut state) = STATE.get().and_then(|value| value.lock().ok()) else {
        return result;
    };
    if register_render_target_locked(
        &mut state,
        OPAQUE_TEXTURE_TARGET_PRODUCER_RVA as u64,
        0,
        RawRegisteredRenderTargetKind::OpaqueRenderTarget { source_flags_u32 },
        view_ptr,
        resource_ptr,
        resource_desc,
        rtv_desc,
    )
    .is_none()
    {
        reject_locked(&mut state);
    }
    result
}

unsafe extern "system" fn hook_entity_destroy(entity: usize) {
    if entity != 0 {
        o05_building_capture::observe_entity_destroy(entity);
        if let Some(mut state) = STATE.get().and_then(|value| value.lock().ok()) {
            state.entities.remove(&entity);
        }
    }
    let original: EntityDestroyFn =
        unsafe { mem::transmute(ORIGINAL_ENTITY_DESTROY.load(Ordering::Acquire)) };
    unsafe { original(entity) };
}

unsafe extern "system" fn hook_entity_render(
    renderer: usize,
    render_context: usize,
    view_context: usize,
    pass_selector: i32,
    bind_renderer_target: u8,
    alternate_list: u8,
) {
    CURRENT_RENDER_PROGRAM.with(|value| *value.borrow_mut() = None);
    let start = CURRENT_SUBMIT.with(|slot| {
        slot.borrow()
            .as_ref()
            .map(|submit| submit.entity_submissions.len())
    });
    let captured = capture_entity_render_root(
        renderer,
        render_context,
        pass_selector,
        bind_renderer_target,
        alternate_list,
    );
    if !captured {
        reject_current();
    }
    let end = CURRENT_SUBMIT.with(|slot| {
        slot.borrow()
            .as_ref()
            .map(|submit| submit.entity_submissions.len())
    });
    let nested = CURRENT_ENTITY_DRAW_RANGE.with(|range| {
        let mut range = range.borrow_mut();
        let nested = range.is_some();
        *range = start.zip(end).map(|(start, end)| DrawRange {
            start,
            end: Some(end),
        });
        nested
    });
    if nested || start.is_none() || end.is_none() {
        reject_current();
    }
    let original: EntityRenderFn =
        unsafe { mem::transmute(ORIGINAL_ENTITY_RENDER.load(Ordering::Acquire)) };
    unsafe {
        original(
            renderer,
            render_context,
            view_context,
            pass_selector,
            bind_renderer_target,
            alternate_list,
        )
    };
    let range = CURRENT_ENTITY_DRAW_RANGE.with(|range| range.borrow_mut().take());
    let complete = range.is_some_and(|range| {
        CURRENT_SUBMIT.with(|slot| {
            let slot = slot.borrow();
            let Some(submit) = slot.as_ref() else {
                return false;
            };
            let Some(end) = range.end else {
                return false;
            };
            submit.entity_submissions[range.start..end]
                .iter()
                .all(|draw| draw.draw_order != u32::MAX)
        })
    });
    if !complete {
        reject_current();
    }
}

unsafe extern "system" fn hook_entity_final_draw(
    render_context: usize,
    index_count: i32,
    start_index: usize,
    base_vertex: u32,
) {
    let program = CURRENT_RENDER_PROGRAM.with(|value| value.borrow_mut().take());
    let active = CURRENT_ENTITY_DRAW_RANGE.with(|range| range.borrow().is_some());
    let observed_program = if active {
        program
    } else {
        bound_render_program(render_context)
    };
    if a25_gui_bridge::owner_run_is_active() {
        if let (Ok(index_count), Ok(start_index), Some(program)) = (
            u32::try_from(index_count),
            u32::try_from(start_index),
            observed_program.filter(|program| program.render_context_ptr == render_context),
        ) {
            a25_gui_bridge::record_final_draw(
                ENTITY_FINAL_DRAW_RVA as u32,
                RawDrawCall::DrawIndexed {
                    index_count,
                    start_index,
                    base_vertex: base_vertex as i32,
                },
                render_context,
                program.shader_program_ptr,
            );
        }
    }
    let assignment = CURRENT_ENTITY_DRAW_RANGE.with(|range| {
        let Some(range) = *range.borrow() else {
            return None;
        };
        let Some(end) = range.end else {
            return None;
        };
        CURRENT_SUBMIT.with(|slot| {
            let mut slot = slot.borrow_mut();
            let Some(submit) = slot.as_mut() else {
                return None;
            };
            let Some(relative) = submit.entity_submissions[range.start..end]
                .iter()
                .position(|draw| draw.draw_order == u32::MAX)
            else {
                return None;
            };
            let Some(program) = program.filter(|value| {
                value.render_context_ptr == render_context && value.shader_program_ptr != 0
            }) else {
                return None;
            };
            let draw = &mut submit.entity_submissions[range.start + relative];
            if draw.shader_program_ptr != program.shader_program_ptr {
                return None;
            }
            let Some(next) = submit.next_global_draw_order.checked_add(1) else {
                return None;
            };
            draw.draw_order = submit.next_global_draw_order;
            let identity = RawDrawIdentity::Entity {
                submit_index: draw.submit_index,
                pass_ordinal: draw.pass_ordinal,
                entity_instance_id: draw.entity_instance_id,
                draw_order: draw.draw_order,
                entity_object_ptr: draw.entity_object_ptr,
                entity_definition_ptr: draw.entity_definition_ptr,
                renderer_ptr: draw.renderer_ptr,
                render_context_ptr: draw.render_context_ptr,
                mesh_draw_ptr: draw.mesh_draw_ptr,
                material_binding_ptr: draw.material_binding_ptr,
                shader_program_ptr: draw.shader_program_ptr,
            };
            let assignment = (identity, submit.submit_index, draw.draw_order);
            submit.next_global_draw_order = next;
            Some(assignment)
        })
    });
    if active && assignment.is_none() {
        reject_current();
    }
    if active || inside_non_ui_render_phase() {
        let _ = record_final_draw(
            FinalDrawCall::DrawIndexed,
            if active {
                FinalDrawFamily::Entity
            } else {
                FinalDrawFamily::StaticOrPostProcess
            },
            ENTITY_FINAL_DRAW_RVA,
            render_context,
            observed_program.map_or(0, |value| value.shader_program_ptr),
            [index_count as u32, start_index as u32, base_vertex, 0],
        );
        if active {
            let captured = assignment.and_then(|(identity, submit_index, draw_order)| {
                let index_count = u32::try_from(index_count).ok()?;
                let start_index = u32::try_from(start_index).ok()?;
                capture_runtime_draw_state(
                    next_catalog_observation_sequence()?,
                    identity,
                    RawDrawFamily::Entity,
                    submit_index,
                    draw_order,
                    render_context,
                    observed_program?.shader_program_ptr,
                    RawDrawCall::DrawIndexed {
                        index_count,
                        start_index,
                        base_vertex: base_vertex as i32,
                    },
                )
            });
            CURRENT_SUBMIT.with(|slot| {
                let mut slot = slot.borrow_mut();
                if let (Some(submit), Some(captured)) = (slot.as_mut(), captured) {
                    let authenticated_optional_null = captured.state.geometry_shader_ptr.is_none()
                        && captured.state.hull_shader_ptr.is_none()
                        && captured.state.domain_shader_ptr.is_none()
                        && captured.state.compute_shader_ptr.is_none()
                        && !captured.state.dynamic_shader_linkage_present;
                    let Some(draw) = submit
                        .entity_submissions
                        .iter_mut()
                        .find(|draw| draw.draw_order == captured.state.draw_order)
                    else {
                        submit.rejected = true;
                        return;
                    };
                    draw.additional_shader_stages_authenticated_null = authenticated_optional_null;
                    draw.stream_output_targets_authenticated_null =
                        captured.state.ordered_stream_output_buffer_ptrs.is_empty();
                    submit.buffer_snapshots.extend(captured.buffer_snapshots);
                    submit
                        .texture_resource_snapshots
                        .extend(captured.texture_resource_snapshots);
                    submit
                        .shader_resource_view_snapshots
                        .extend(captured.shader_resource_view_snapshots);
                    submit
                        .render_target_view_snapshots
                        .extend(captured.render_target_view_snapshots);
                    submit.draw_states.push(captured.state);
                } else if let Some(submit) = slot.as_mut() {
                    submit.rejected = true;
                } else {
                    reject();
                }
            });
        }
    }
    let original: EntityFinalDrawFn =
        unsafe { mem::transmute(ORIGINAL_ENTITY_FINAL_DRAW.load(Ordering::Acquire)) };
    unsafe { original(render_context, index_count, start_index, base_vertex) };
}

unsafe extern "system" fn hook_shader_program_binder(render_context: usize, shader_program: usize) {
    let original: ShaderProgramBinderFn =
        unsafe { mem::transmute(ORIGINAL_SHADER_PROGRAM_BINDER.load(Ordering::Acquire)) };
    unsafe { original(render_context, shader_program) };
    let inside_authenticated_draw = CURRENT_PARTICLE_DRAW_RANGE
        .with(|value| value.borrow().is_some())
        || CURRENT_ENTITY_DRAW_RANGE.with(|value| value.borrow().is_some());
    if (inside_authenticated_draw
        || inside_non_ui_render_phase()
        || a25_gui_bridge::owner_run_is_active())
        && render_context != 0
        && shader_program != 0
    {
        CURRENT_RENDER_PROGRAM.with(|value| {
            *value.borrow_mut() = Some(CurrentRenderProgram {
                render_context_ptr: render_context,
                shader_program_ptr: shader_program,
            });
        });
        BOUND_RENDER_PROGRAMS.with(|value| {
            value.borrow_mut().insert(render_context, shader_program);
        });
    }
}

unsafe extern "system" fn hook_indexed_instanced_final_draw(
    render_context: usize,
    index_count_per_instance: i32,
    instance_count: u32,
) {
    let program = bound_render_program(render_context);
    if a25_gui_bridge::owner_run_is_active() {
        if let (Ok(index_count_per_instance), Some(program)) = (
            u32::try_from(index_count_per_instance),
            program.filter(|program| program.render_context_ptr == render_context),
        ) {
            a25_gui_bridge::record_final_draw(
                INDEXED_INSTANCED_FINAL_DRAW_RVA as u32,
                RawDrawCall::DrawIndexedInstanced {
                    index_count_per_instance,
                    instance_count,
                    start_index: 0,
                    base_vertex: 0,
                    start_instance: 0,
                },
                render_context,
                program.shader_program_ptr,
            );
        }
    }
    if inside_non_ui_render_phase() {
        let shader_program_ptr = program.map_or(0, |value| value.shader_program_ptr);
        let sequence = record_final_draw(
            FinalDrawCall::DrawIndexedInstanced,
            FinalDrawFamily::StaticOrPostProcess,
            INDEXED_INSTANCED_FINAL_DRAW_RVA,
            render_context,
            shader_program_ptr,
            [index_count_per_instance as u32, instance_count, 0, 0],
        );
        record_static_d3d_capture(
            sequence,
            render_context,
            shader_program_ptr,
            RawStaticDrawCall::DrawIndexedInstanced {
                index_count_per_instance: index_count_per_instance as u32,
                instance_count,
                start_index: 0,
                base_vertex: 0,
                start_instance: 0,
            },
        );
    }
    let original: InstancedFinalDrawFn =
        unsafe { mem::transmute(ORIGINAL_INDEXED_INSTANCED_FINAL_DRAW.load(Ordering::Acquire)) };
    unsafe { original(render_context, index_count_per_instance, instance_count) };
}

unsafe extern "system" fn hook_instanced_final_draw(
    render_context: usize,
    vertex_count_per_instance: i32,
    instance_count: u32,
) {
    let program = bound_render_program(render_context);
    if a25_gui_bridge::owner_run_is_active() {
        if let (Ok(vertex_count_per_instance), Some(program)) = (
            u32::try_from(vertex_count_per_instance),
            program.filter(|program| program.render_context_ptr == render_context),
        ) {
            a25_gui_bridge::record_final_draw(
                INSTANCED_FINAL_DRAW_RVA as u32,
                RawDrawCall::DrawInstanced {
                    vertex_count_per_instance,
                    instance_count,
                    start_vertex: 0,
                    start_instance: 0,
                },
                render_context,
                program.shader_program_ptr,
            );
        }
    }
    if inside_non_ui_render_phase() {
        let shader_program_ptr = program.map_or(0, |value| value.shader_program_ptr);
        let sequence = record_final_draw(
            FinalDrawCall::DrawInstanced,
            FinalDrawFamily::StaticOrPostProcess,
            INSTANCED_FINAL_DRAW_RVA,
            render_context,
            shader_program_ptr,
            [vertex_count_per_instance as u32, instance_count, 0, 0],
        );
        record_static_d3d_capture(
            sequence,
            render_context,
            shader_program_ptr,
            RawStaticDrawCall::DrawInstanced {
                vertex_count_per_instance: vertex_count_per_instance as u32,
                instance_count,
                start_vertex: 0,
                start_instance: 0,
            },
        );
    }
    let original: InstancedFinalDrawFn =
        unsafe { mem::transmute(ORIGINAL_INSTANCED_FINAL_DRAW.load(Ordering::Acquire)) };
    unsafe { original(render_context, vertex_count_per_instance, instance_count) };
}

unsafe extern "system" fn hook_render_target_clear(
    render_target_object: usize,
    clear_flags: u64,
    rgba_f32_bits: *const u32,
) {
    if inside_non_ui_render_phase() {
        if clear_flags & 1 != 0 {
            let color = if rgba_f32_bits.is_null() {
                Some(DEFAULT_CLEAR_COLOR_RGBA_F32_BITS)
            } else {
                read_u32x4(rgba_f32_bits as usize)
            };
            match (read_usize(render_target_object.saturating_add(0x10)), color) {
                (Some(render_target_view_ptr), Some(rgba_f32_bits))
                    if render_target_view_ptr != 0 =>
                {
                    record_clear_output_event(
                        render_target_view_ptr,
                        RawOutputViewKind::Color,
                        CreatedD3dObjectKind::RenderTargetView,
                        |sequence, capture_sequence| RawRenderOrderEvent::ClearColor {
                            sequence,
                            capture_sequence,
                            producer_rva: RENDER_TARGET_CLEAR_RVA,
                            render_target_object_ptr: render_target_object,
                            render_target_view_ptr,
                            rgba_f32_bits,
                        },
                    );
                }
                _ => reject_render_order_event(),
            }
        }
        if clear_flags & 6 != 0 {
            let clear_depth = clear_flags & 2 != 0;
            let clear_stencil = clear_flags & 4 != 0;
            match read_usize(render_target_object.saturating_add(0x18)) {
                Some(depth_stencil_view_ptr) if depth_stencil_view_ptr != 0 => {
                    record_clear_output_event(
                        depth_stencil_view_ptr,
                        RawOutputViewKind::DepthStencil,
                        CreatedD3dObjectKind::DepthStencilView,
                        |sequence, capture_sequence| RawRenderOrderEvent::ClearDepthStencil {
                            sequence,
                            capture_sequence,
                            producer_rva: RENDER_TARGET_CLEAR_RVA,
                            render_target_object_ptr: render_target_object,
                            depth_stencil_view_ptr,
                            clear_depth,
                            clear_stencil,
                            depth_f32_bits: DEFAULT_CLEAR_DEPTH_F32_BITS,
                            stencil_u8: 0,
                        },
                    );
                }
                _ => reject_render_order_event(),
            }
        }
    }
    let original: RenderTargetClearFn =
        unsafe { mem::transmute(ORIGINAL_RENDER_TARGET_CLEAR.load(Ordering::Acquire)) };
    unsafe { original(render_target_object, clear_flags, rgba_f32_bits) };
}

unsafe extern "system" fn hook_map_scene_render_root(
    graphical_map: usize,
    render_context: usize,
    selector: i32,
    alternate: u8,
) {
    let submit_index =
        CURRENT_SUBMIT.with(|value| value.borrow().as_ref().map(|submit| submit.submit_index));
    if submit_index
        .and_then(|submit_index| begin_render_frame(submit_index).ok())
        .is_none()
    {
        reject_render_order_event();
    }
    CURRENT_RENDER_PROGRAM.with(|value| *value.borrow_mut() = None);
    BOUND_RENDER_PROGRAMS.with(|value| value.borrow_mut().clear());
    CURRENT_MAP_SCENE_PHASE.with(|value| *value.borrow_mut() = true);
    record_boundary(
        RenderOrderBoundary::MapSceneBegin,
        MAP_SCENE_RENDER_ROOT_RVA,
    );
    let original: MapSceneRenderFn =
        unsafe { mem::transmute(ORIGINAL_MAP_SCENE_RENDER_ROOT.load(Ordering::Acquire)) };
    unsafe { original(graphical_map, render_context, selector, alternate) };
    record_boundary(RenderOrderBoundary::MapSceneEnd, MAP_SCENE_RENDER_ROOT_RVA);
    CURRENT_MAP_SCENE_PHASE.with(|value| *value.borrow_mut() = false);
    CURRENT_RENDER_PROGRAM.with(|value| *value.borrow_mut() = None);
    BOUND_RENDER_PROGRAMS.with(|value| value.borrow_mut().clear());
}

unsafe extern "system" fn hook_post_process_render_root(
    a1: usize,
    a2: usize,
    a3: usize,
    a4: usize,
    a5: usize,
    a6: usize,
    a7: usize,
    a8: usize,
    a9: usize,
    a10: usize,
    a11: usize,
    a12: usize,
    a13: usize,
) {
    CURRENT_RENDER_PROGRAM.with(|value| *value.borrow_mut() = None);
    BOUND_RENDER_PROGRAMS.with(|value| value.borrow_mut().clear());
    CURRENT_POST_PROCESS_PHASE.with(|value| *value.borrow_mut() = true);
    record_boundary(
        RenderOrderBoundary::PostProcessBegin,
        POST_PROCESS_RENDER_ROOT_RVA,
    );
    let original: PostProcessRenderFn =
        unsafe { mem::transmute(ORIGINAL_POST_PROCESS_RENDER_ROOT.load(Ordering::Acquire)) };
    unsafe { original(a1, a2, a3, a4, a5, a6, a7, a8, a9, a10, a11, a12, a13) };
    record_boundary(
        RenderOrderBoundary::PostProcessEnd,
        POST_PROCESS_RENDER_ROOT_RVA,
    );
    g2_capture::capture_after_post_process();
    CURRENT_POST_PROCESS_PHASE.with(|value| *value.borrow_mut() = false);
    CURRENT_RENDER_PROGRAM.with(|value| *value.borrow_mut() = None);
    BOUND_RENDER_PROGRAMS.with(|value| value.borrow_mut().clear());
}

unsafe extern "system" fn hook_present_render_root(renderer: usize) {
    let original: PresentRenderFn =
        unsafe { mem::transmute(ORIGINAL_PRESENT_RENDER_ROOT.load(Ordering::Acquire)) };
    // Loading screens and menus legitimately Present without a map-scene
    // frame. They are outside the replay-map envelope and must not manufacture
    // boundaries or poison capture completeness.
    if !has_pending_render_frame() {
        unsafe { original(renderer) };
        return;
    }
    record_boundary(RenderOrderBoundary::PresentBegin, PRESENT_RENDER_ROOT_RVA);
    unsafe { original(renderer) };
    record_boundary(RenderOrderBoundary::PresentEnd, PRESENT_RENDER_ROOT_RVA);
    match complete_render_frame() {
        Ok(receipt) => {
            if a25_gui_bridge::seal_present(
                receipt.global_order,
                receipt.ticks,
                receipt.frequency_hz,
                receipt.process_start_nonce,
                receipt.hook_epoch_nonce,
                receipt.game_hour_raw,
            )
            .is_err()
            {
                reject_render_order_event();
            }
        }
        Err(_) => {
            a25_gui_bridge::discard_candidate();
            reject_render_order_event();
        }
    }
}

unsafe extern "system" fn hook_snow_mud_producer(
    producer: usize,
    texture_resource_slot: *mut usize,
    force_update: u8,
) {
    let original: SnowMudProducerFn =
        unsafe { mem::transmute(ORIGINAL_SNOW_MUD_PRODUCER.load(Ordering::Acquire)) };
    let slot_address = texture_resource_slot as usize;
    let before = (slot_address != 0)
        .then(|| (read_usize(slot_address), read_usize(slot_address)))
        .and_then(|(first, second)| (first == second).then_some(first))
        .flatten();
    let nested = CURRENT_SNOW_MUD_PRODUCER.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.is_some() {
            true
        } else {
            *slot = Some(PendingSnowMudProducer {
                resource_before: before.unwrap_or(0),
                upload_expected: before.is_some_and(|resource| resource != 0),
                upload_count: 0,
                rejected: before.is_none(),
            });
            false
        }
    });
    if nested {
        reject_snow_mud_capture();
        unsafe { original(producer, texture_resource_slot, force_update) };
        return;
    }
    unsafe { original(producer, texture_resource_slot, force_update) };
    let after = (slot_address != 0)
        .then(|| (read_usize(slot_address), read_usize(slot_address)))
        .and_then(|(first, second)| (first == second).then_some(first))
        .flatten();
    let completed = CURRENT_SNOW_MUD_PRODUCER.with(|slot| slot.borrow_mut().take());
    let admitted = completed.filter(|pending| {
        !pending.rejected
            && pending.upload_count == u8::from(pending.upload_expected)
            && after.is_some_and(|resource_after| {
                if pending.upload_expected {
                    resource_after == pending.resource_before
                } else {
                    resource_after != 0
                }
            })
    });
    match admitted {
        Some(pending) if !pending.upload_expected => {
            if let Some(mut state) = STATE.get().and_then(|value| value.lock().ok()) {
                state.latest_snow_mud_observation = None;
            }
        }
        Some(_) => {}
        None => reject_snow_mud_capture(),
    }
}

unsafe extern "system" fn hook_snow_mud_upload(
    upload_context: usize,
    cpu_bytes: *const u8,
    width: u32,
    height: u32,
    bytes_per_texel: u32,
    resource: usize,
) {
    let original: SnowMudUploadFn =
        unsafe { mem::transmute(ORIGINAL_SNOW_MUD_UPLOAD.load(Ordering::Acquire)) };
    let active = CURRENT_SNOW_MUD_PRODUCER.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(pending) = slot.as_mut() else {
            return false;
        };
        pending.upload_count = pending.upload_count.saturating_add(1);
        if pending.upload_count != 1 || !pending.upload_expected {
            pending.rejected = true;
            return false;
        }
        true
    });
    if active {
        match capture_snow_mud_preupload(
            cpu_bytes,
            width,
            height,
            bytes_per_texel,
            resource,
        ) {
            Ok(observation) => {
                if let Some(mut state) = STATE.get().and_then(|value| value.lock().ok()) {
                    state.latest_snow_mud_observation = Some(Arc::new(observation));
                } else {
                    mark_current_snow_mud_rejected();
                }
            }
            Err(_) => {
                mark_current_snow_mud_rejected();
                reject_snow_mud_capture();
            }
        }
    } else if CURRENT_SNOW_MUD_PRODUCER.with(|slot| slot.borrow().is_some()) {
        reject_snow_mud_capture();
    }
    if width == crate::OCCUPATION_SECONDARY_WIDTH
        && height == crate::OCCUPATION_SECONDARY_HEIGHT
        && bytes_per_texel == crate::OCCUPATION_SECONDARY_BYTES_PER_TEXEL
    {
        match capture_occupation_secondary_preupload(
            cpu_bytes,
            width,
            height,
            bytes_per_texel,
            resource,
        ) {
            Ok(observation) => {
                if let Some(mut state) = STATE.get().and_then(|value| value.lock().ok()) {
                    if !state
                        .latest_occupation_secondary_by_resource
                        .contains_key(&resource)
                        && state.latest_occupation_secondary_by_resource.len()
                            >= MAX_OCCUPATION_SECONDARY_RESOURCES
                    {
                        state.latest_occupation_secondary_by_resource.clear();
                        reject_locked(&mut state);
                    } else {
                        state
                            .latest_occupation_secondary_by_resource
                            .insert(resource, Arc::new(observation));
                    }
                }
            }
            Err(_) => {
                if let Some(mut state) = STATE.get().and_then(|value| value.lock().ok()) {
                    state
                        .latest_occupation_secondary_by_resource
                        .remove(&resource);
                    reject_locked(&mut state);
                }
            }
        }
    }
    unsafe {
        original(
            upload_context,
            cpu_bytes,
            width,
            height,
            bytes_per_texel,
            resource,
        )
    };
}

fn mark_current_snow_mud_rejected() {
    CURRENT_SNOW_MUD_PRODUCER.with(|slot| {
        if let Some(pending) = slot.borrow_mut().as_mut() {
            pending.rejected = true;
        }
    });
}

fn reject_snow_mud_capture() {
    if let Some(mut state) = STATE.get().and_then(|value| value.lock().ok()) {
        state.latest_snow_mud_observation = None;
        reject_locked(&mut state);
    }
}

fn capture_snow_mud_preupload(
    cpu_bytes: *const u8,
    width: u32,
    height: u32,
    bytes_per_texel: u32,
    resource: usize,
) -> Result<RawSnowMudTextureObservation, String> {
    if cpu_bytes.is_null()
        || resource == 0
        || width != portable_render_stream::SNOW_MUD_TEXTURE_WIDTH
        || height != portable_render_stream::SNOW_MUD_TEXTURE_HEIGHT
        || bytes_per_texel != portable_render_stream::SNOW_MUD_TEXTURE_BYTES_PER_TEXEL
    {
        return Err("SnowMudData upload descriptor is not the authenticated a729 descriptor".into());
    }
    let game_hour_before = capture_present_game_hour_raw()?;
    let qpc_before =
        query_performance_counter().ok_or("SnowMudData pre-copy QueryPerformanceCounter failed")?;
    let mut bytes = vec![0; portable_render_stream::SNOW_MUD_TEXTURE_BYTE_LENGTH];
    read_exact(cpu_bytes as usize, &mut bytes)
        .ok_or("SnowMudData CPU pre-upload bytes are unreadable")?;
    let producer_qpc_ticks =
        query_performance_counter().ok_or("SnowMudData QueryPerformanceCounter failed")?;
    let producer_game_hour_raw = capture_present_game_hour_raw()?;
    if producer_qpc_ticks <= qpc_before || producer_game_hour_raw != game_hour_before {
        return Err("SnowMudData game hour/QPC changed during the CPU pre-upload copy".into());
    }
    let (process_start_nonce, hook_epoch_nonce, producer_qpc_frequency_hz) = {
        let epoch = EARLY_CAPTURE_EPOCH
            .get()
            .ok_or("SnowMudData early capture epoch is unavailable")?
            .lock()
            .map_err(|_| "SnowMudData early capture epoch is poisoned")?;
        let process = epoch
            .process
            .as_ref()
            .ok_or("SnowMudData process epoch is unavailable")?;
        if process.executable_sha256 != portable_render_stream::A729_EXECUTABLE_SHA256
            || process.process_start_nonce == 0
            || epoch.hook_epoch_nonce == 0
            || epoch.qpc_frequency_hz == 0
        {
            return Err("SnowMudData capture epoch/binary identity is incomplete".into());
        }
        (
            process.process_start_nonce,
            epoch.hook_epoch_nonce,
            epoch.qpc_frequency_hz,
        )
    };
    let producer_ordinal = NEXT_SNOW_MUD_PRODUCER_ORDINAL
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
            current.checked_add(1)
        })
        .map_err(|_| "SnowMudData producer ordinal overflowed")?;
    if producer_ordinal == 0 {
        return Err("SnowMudData producer ordinal is zero".into());
    }
    let payload_sha256 = format!("{:x}", Sha256::digest(&bytes));
    Ok(RawSnowMudTextureObservation {
        executable_sha256: portable_render_stream::A729_EXECUTABLE_SHA256.into(),
        producer_function_rva: portable_render_stream::SNOW_MUD_PRODUCER_FUNCTION_RVA,
        upload_callsite_rva: portable_render_stream::SNOW_MUD_UPLOAD_CALLSITE_RVA,
        upload_target_rva: portable_render_stream::SNOW_MUD_UPLOAD_TARGET_RVA,
        producer_instructions_sha256:
            portable_render_stream::SNOW_MUD_PRODUCER_INSTRUCTIONS_SHA256.into(),
        upload_instructions_sha256:
            portable_render_stream::SNOW_MUD_UPLOAD_INSTRUCTIONS_SHA256.into(),
        process_start_nonce,
        hook_epoch_nonce,
        producer_ordinal,
        producer_game_hour_raw,
        producer_qpc_ticks,
        producer_qpc_frequency_hz,
        resource_ptr: resource,
        width,
        height,
        row_pitch: portable_render_stream::SNOW_MUD_TEXTURE_ROW_PITCH,
        bytes_per_texel,
        format: "DXGI_FORMAT_B8G8R8A8_UNORM".into(),
        bytes,
        payload_sha256,
    })
}

fn capture_occupation_secondary_preupload(
    cpu_bytes: *const u8,
    width: u32,
    height: u32,
    bytes_per_texel: u32,
    resource: usize,
) -> Result<RawOccupationSecondaryTextureObservation, String> {
    if cpu_bytes.is_null()
        || resource == 0
        || width != crate::OCCUPATION_SECONDARY_WIDTH
        || height != crate::OCCUPATION_SECONDARY_HEIGHT
        || bytes_per_texel != crate::OCCUPATION_SECONDARY_BYTES_PER_TEXEL
    {
        return Err(
            "occupation secondary upload descriptor is not the authenticated a729 descriptor"
                .into(),
        );
    }
    let game_hour_before = capture_present_game_hour_raw()?;
    let qpc_before = query_performance_counter()
        .ok_or("occupation secondary pre-copy QueryPerformanceCounter failed")?;
    let mut bytes = vec![0; crate::OCCUPATION_SECONDARY_BYTE_LENGTH];
    read_exact(cpu_bytes as usize, &mut bytes)
        .ok_or("occupation secondary CPU pre-upload bytes are unreadable")?;
    let producer_qpc_ticks = query_performance_counter()
        .ok_or("occupation secondary QueryPerformanceCounter failed")?;
    let producer_game_hour_raw = capture_present_game_hour_raw()?;
    if producer_qpc_ticks <= qpc_before || producer_game_hour_raw != game_hour_before {
        return Err(
            "occupation secondary game hour/QPC changed during the CPU pre-upload copy".into(),
        );
    }
    let (process_start_nonce, hook_epoch_nonce, producer_qpc_frequency_hz) = {
        let epoch = EARLY_CAPTURE_EPOCH
            .get()
            .ok_or("occupation secondary early capture epoch is unavailable")?
            .lock()
            .map_err(|_| "occupation secondary early capture epoch is poisoned")?;
        let process = epoch
            .process
            .as_ref()
            .ok_or("occupation secondary process epoch is unavailable")?;
        if process.executable_sha256 != portable_render_stream::A729_EXECUTABLE_SHA256
            || process.process_start_nonce == 0
            || epoch.hook_epoch_nonce == 0
            || epoch.qpc_frequency_hz == 0
        {
            return Err("occupation secondary capture epoch/binary identity is incomplete".into());
        }
        (
            process.process_start_nonce,
            epoch.hook_epoch_nonce,
            epoch.qpc_frequency_hz,
        )
    };
    let upload_order = NEXT_OCCUPATION_SECONDARY_UPLOAD_ORDER
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
            current.checked_add(1)
        })
        .map_err(|_| "occupation secondary upload order overflowed")?;
    if upload_order == 0 {
        return Err("occupation secondary upload order is zero".into());
    }
    let payload_sha256 = format!("{:x}", Sha256::digest(&bytes));
    let observation = RawOccupationSecondaryTextureObservation {
        schema: crate::OCCUPATION_SECONDARY_TEXTURE_SCHEMA.into(),
        executable_sha256: portable_render_stream::A729_EXECUTABLE_SHA256.into(),
        raster_worker_rva: crate::OCCUPATION_SECONDARY_RASTER_WORKER_RVA,
        raster_worker_decompile_sha256:
            crate::OCCUPATION_SECONDARY_RASTER_WORKER_DECOMPILE_SHA256.into(),
        upload_target_rva: portable_render_stream::SNOW_MUD_UPLOAD_TARGET_RVA,
        upload_instructions_sha256:
            portable_render_stream::SNOW_MUD_UPLOAD_INSTRUCTIONS_SHA256.into(),
        process_start_nonce,
        hook_epoch_nonce,
        upload_order,
        producer_game_hour_raw,
        producer_qpc_ticks,
        producer_qpc_frequency_hz,
        resource_ptr: resource,
        width,
        height,
        row_pitch: crate::OCCUPATION_SECONDARY_ROW_PITCH,
        bytes_per_texel,
        bytes,
        payload_sha256,
    };
    if !observation.has_exact_nontransparent_alpha_law() {
        return Err(
            "occupation secondary upload is transparent or violates the exact 0/76 alpha law"
                .into(),
        );
    }
    Ok(observation)
}

fn has_pending_render_frame() -> bool {
    STATE
        .get()
        .and_then(|state| state.lock().ok())
        .is_some_and(|state| state.pending_render_frame.is_some())
}

/// Extractor-local candidate identity for the dormant A25 owner-glyph probe.
/// This accessor is valid only inside the authenticated render envelope. It
/// exposes no process pointer and cannot predict which Present the hour latch
/// will select.
pub fn current_a25_owner_glyph_present_candidate(
    frame_hour: u64,
) -> Result<runtime_text_inputs_live_lane::owner_glyph_probe::OwnerGlyphPresentCandidate, String>
{
    let submit_index = CURRENT_SUBMIT
        .with(|slot| slot.borrow().as_ref().map(|submit| submit.submit_index))
        .ok_or("A25 owner render is outside the authenticated render envelope")?;
    let frame_sequence = STATE
        .get()
        .ok_or("render hook state is unavailable")?
        .lock()
        .map_err(|_| "render hook state poisoned")?
        .pending_render_frame
        .filter(|pending| pending.submit_index == submit_index)
        .map(|pending| pending.frame_sequence)
        .ok_or("A25 owner render is outside the active Present candidate")?;
    let process = current_process_epoch();
    let launch = current_capture_launch_epoch()
        .ok_or("A25 owner render has no authenticated early-launch epoch")?;
    if process.process_start_nonce == 0
        || launch.process_start_nonce != process.process_start_nonce
        || launch.hook_epoch_nonce == 0
    {
        return Err("A25 owner render capture epoch is incomplete".into());
    }
    Ok(
        runtime_text_inputs_live_lane::owner_glyph_probe::OwnerGlyphPresentCandidate {
            process_start_nonce: process.process_start_nonce,
            hook_epoch_nonce: launch.hook_epoch_nonce,
            submit_index,
            frame_sequence,
            frame_hour,
        },
    )
}

fn begin_render_frame(submit_index: u64) -> Result<u64, String> {
    let frame_sequence = NEXT_RENDER_FRAME_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    if submit_index == 0 || frame_sequence == 0 {
        return Err("render frame/submit sequence overflowed".into());
    }
    let mut state = STATE
        .get()
        .ok_or("render hook state is unavailable")?
        .lock()
        .map_err(|_| "render hook state poisoned")?;
    if state.pending_render_frame.is_some()
        || !state.render_order_events.is_empty()
        || !state.clear_output_view_snapshots.is_empty()
        || !state.static_d3d_captures.is_empty()
        || !state.render_target_use_observations.is_empty()
    {
        return Err("render frame began with a pending or unframed hook queue".into());
    }
    state.pending_render_frame = Some(PendingRenderFrame {
        frame_sequence,
        submit_index,
    });
    Ok(frame_sequence)
}

fn complete_render_frame() -> Result<PresentationQpcReceipt, String> {
    let mut state = STATE
        .get()
        .ok_or("render hook state is unavailable")?
        .lock()
        .map_err(|_| "render hook state poisoned")?;
    // Present identity and the collector cutoff use the same mutex. A
    // PresentEnd can therefore be wholly before or wholly after an atomic
    // render-hour latch, never captured before it and published after it.
    let receipt = capture_present_qpc_receipt()?;
    complete_render_frame_locked(&mut state, receipt)?;
    Ok(receipt)
}

#[cfg(test)]
fn complete_render_frame_with_receipt(receipt: PresentationQpcReceipt) -> Result<(), String> {
    let mut state = STATE
        .get()
        .ok_or("render hook state is unavailable")?
        .lock()
        .map_err(|_| "render hook state poisoned")?;
    complete_render_frame_locked(&mut state, receipt)
}

fn complete_render_frame_locked(
    state: &mut HookState,
    receipt: PresentationQpcReceipt,
) -> Result<(), String> {
    let pending = state
        .pending_render_frame
        .take()
        .ok_or("PresentEnd has no exact MapSceneBegin frame")?;
    if state.completed_render_frames.len() >= MAX_SUBMITS {
        return Err("completed render-frame queue reached its fail-closed bound".into());
    }
    if receipt.ticks == 0
        || receipt.frequency_hz == 0
        || receipt.process_start_nonce == 0
        || receipt.hook_epoch_nonce == 0
        || state
            .last_presentation_qpc_ticks
            .is_some_and(|last| receipt.ticks <= last)
    {
        return Err("PresentEnd QPC receipt is null, foreign, or non-monotonic".into());
    }
    validate_render_frame_queues(&state)?;
    let static_catalog_fragment = build_static_catalog_fragment(&state)?;
    let snow_mud_texture = match state.latest_snow_mud_observation.as_ref() {
        Some(observation)
            if observation.process_start_nonce == receipt.process_start_nonce
                && observation.hook_epoch_nonce == receipt.hook_epoch_nonce
                && observation.producer_qpc_frequency_hz == receipt.frequency_hz
                && observation.producer_qpc_ticks <= receipt.ticks
                && observation.producer_game_hour_raw <= receipt.game_hour_raw =>
        {
            Some(Arc::clone(observation))
        }
        Some(_) => {
            state.latest_snow_mud_observation = None;
            return Err("SnowMudData producer observation crosses the Present epoch/hour/order".into());
        }
        None => None,
    };
    let occupation_secondary_textures = state
        .latest_occupation_secondary_by_resource
        .values()
        .map(|observation| {
            if observation.process_start_nonce != receipt.process_start_nonce
                || observation.hook_epoch_nonce != receipt.hook_epoch_nonce
                || observation.producer_qpc_frequency_hz != receipt.frequency_hz
                || observation.producer_qpc_ticks > receipt.ticks
                || observation.producer_game_hour_raw > receipt.game_hour_raw
            {
                return Err(
                    "occupation secondary upload crosses the Present epoch/hour/order".into(),
                );
            }
            Ok(Arc::clone(observation))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let weather_membership_updates = state
        .latest_weather_observation_by_region
        .values()
        .filter(|row| row.owner == RawWeatherUpdateOwner::NormalStrategicRegionUpdate)
        .cloned()
        .collect::<Vec<_>>();
    let weather_visibility_updates = state
        .latest_weather_visibility_by_region
        .iter()
        .filter(|(region, row)| {
            row.owner == RawWeatherUpdateOwner::NormalStrategicRegionUpdate
                && state
                    .weather_runtime_codes
                    .get(region)
                    .is_some_and(|code| *code != 0)
        })
        .map(|(_, row)| row)
        .cloned()
        .collect::<Vec<_>>();
    for (region, code) in &state.weather_runtime_codes {
        let membership = state
            .latest_weather_observation_by_region
            .get(region)
            .ok_or("weather region has no initial/checkpoint observation")?;
        if membership.airborne_runtime_code != *code {
            return Err("weather region discriminator differs from its Present snapshot".into());
        }
        if *code != 0 {
            let visibility = state
                .latest_weather_visibility_by_region
                .get(region)
                .ok_or("active weather region has no completed visibility update")?;
            if visibility.update_index < membership.update_index {
                return Err("active weather visibility predates its membership epoch".into());
            }
        }
    }
    let latest_completed_weather_update_index = weather_membership_updates
        .iter()
        .map(|row| row.update_index)
        .chain(weather_visibility_updates.iter().map(|row| row.update_index))
        .max()
        .unwrap_or(0);
    let weather_snapshot_sha256 = weather_snapshot_sha256(
        &weather_membership_updates,
        &weather_visibility_updates,
    )?;
    let frame = RawRenderFrameCapture {
        frame_sequence: pending.frame_sequence,
        submit_index: pending.submit_index,
        global_order: receipt.global_order,
        presentation_qpc_ticks: receipt.ticks,
        presentation_qpc_frequency_hz: receipt.frequency_hz,
        process_start_nonce: receipt.process_start_nonce,
        hook_epoch_nonce: receipt.hook_epoch_nonce,
        expected_weather_region_count: state.expected_weather_region_count.unwrap_or(0) as u64,
        observed_weather_region_count: state.weather_runtime_codes.len() as u64,
        active_airborne_weather_region_count: state
            .weather_runtime_codes
            .values()
            .filter(|code| **code != 0)
            .count() as u64,
        queued_weather_change_count: state.weather_changes.len() as u64,
        weather_membership_updates,
        weather_visibility_updates,
        latest_completed_weather_update_index,
        weather_snapshot_sha256,
        game_hour_raw: receipt.game_hour_raw,
        snow_mud_texture,
        occupation_secondary_textures,
        events: state.render_order_events.drain(..).collect(),
        static_d3d_captures: state.static_d3d_captures.drain(..).collect(),
        static_catalog_fragment,
    };
    state.clear_output_view_snapshots.clear();
    state.render_target_use_observations.clear();
    state.last_presentation_qpc_ticks = Some(receipt.ticks);
    state.completed_render_frames.push_back(frame);
    // Runtime/catalog may have published before PresentEnd. Classify and
    // retire against the active/sealed cutoff before online compaction, just
    // as the runtime-last path does in `hook_envelope`.
    purge_published_sealed_render_submit(state, pending.submit_index)?;
    compact_published_current_hour_candidates(state, pending.submit_index)?;
    Ok(())
}

fn validate_render_frame_queues(state: &HookState) -> Result<(), String> {
    if state.render_order_event_overflows != 0 {
        return Err(format!(
            "render-order event queue overflowed {} times",
            state.render_order_event_overflows
        ));
    }
    if state
        .render_order_events
        .iter()
        .filter(|event| {
            matches!(
                event,
                RawRenderOrderEvent::ClearColor { .. }
                    | RawRenderOrderEvent::ClearDepthStencil { .. }
            )
        })
        .count()
        != state.clear_output_view_snapshots.len()
    {
        return Err("render-order clear event/snapshot queues are misaligned".into());
    }
    if state
        .render_order_events
        .iter()
        .filter(|event| {
            matches!(
                event,
                RawRenderOrderEvent::FinalDraw {
                    family: FinalDrawFamily::StaticOrPostProcess,
                    ..
                }
            )
        })
        .count()
        != state.static_d3d_captures.len()
    {
        return Err("static final-draw event/D3D capture queues are misaligned".into());
    }
    let first = state.render_order_events.front();
    let last = state.render_order_events.back();
    if !matches!(
        first,
        Some(RawRenderOrderEvent::Boundary {
            kind: RenderOrderBoundary::MapSceneBegin,
            ..
        })
    ) || !matches!(
        last,
        Some(RawRenderOrderEvent::Boundary {
            kind: RenderOrderBoundary::PresentEnd,
            ..
        })
    ) {
        return Err("render frame is not exactly MapSceneBegin..PresentEnd".into());
    }
    Ok(())
}

fn record_boundary(kind: RenderOrderBoundary, producer_rva: u32) {
    let _ = record_render_order_event(|sequence| RawRenderOrderEvent::Boundary {
        sequence,
        kind,
        producer_rva,
    });
}

fn record_final_draw(
    call: FinalDrawCall,
    family: FinalDrawFamily,
    producer_rva: u32,
    render_context_ptr: usize,
    shader_program_ptr: usize,
    arguments: [u32; 4],
) -> Option<u64> {
    record_render_order_event(|sequence| RawRenderOrderEvent::FinalDraw {
        sequence,
        call,
        family,
        producer_rva,
        render_context_ptr,
        shader_program_ptr,
        arguments,
    })
}

fn record_render_order_event(build: impl FnOnce(u64) -> RawRenderOrderEvent) -> Option<u64> {
    let sequence = NEXT_RENDER_ORDER_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let Some(mut state) = STATE.get().and_then(|value| value.lock().ok()) else {
        return None;
    };
    if state.pending_render_frame.is_none()
        || sequence == 0
        || state.render_order_events.len() >= MAX_RENDER_ORDER_EVENTS
    {
        state.render_order_event_overflows += 1;
        return None;
    }
    state.render_order_events.push_back(build(sequence));
    Some(sequence)
}

fn record_clear_output_event(
    view_ptr: usize,
    view_kind: RawOutputViewKind,
    created_kind: CreatedD3dObjectKind,
    build: impl FnOnce(u64, u64) -> RawRenderOrderEvent,
) -> Option<u64> {
    let Some(capture_sequence) = next_catalog_observation_sequence() else {
        reject_render_order_event();
        return None;
    };
    let Some(snapshot) =
        capture_output_view_snapshot(view_ptr, view_kind, created_kind, capture_sequence)
    else {
        reject_render_order_event();
        return None;
    };
    let order_sequence = NEXT_RENDER_ORDER_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let Some(mut state) = STATE.get().and_then(|value| value.lock().ok()) else {
        return None;
    };
    if state.pending_render_frame.is_none()
        || order_sequence == 0
        || state.render_order_events.len() >= MAX_RENDER_ORDER_EVENTS
        || state.clear_output_view_snapshots.len() >= MAX_RENDER_ORDER_EVENTS
    {
        state.render_order_event_overflows += 1;
        return None;
    }
    if view_kind == RawOutputViewKind::Color
        && record_registered_color_target_uses_locked(&mut state, capture_sequence, &[view_ptr])
            .is_none()
    {
        state.render_order_event_overflows += 1;
        return None;
    }
    state.clear_output_view_snapshots.push_back(snapshot);
    state
        .render_order_events
        .push_back(build(order_sequence, capture_sequence));
    Some(order_sequence)
}

fn record_registered_color_target_uses_locked(
    state: &mut HookState,
    capture_sequence: u64,
    ordered_color_target_view_ptrs: &[usize],
) -> Option<()> {
    if capture_sequence == 0
        || state
            .render_target_use_observations
            .len()
            .checked_add(ordered_color_target_view_ptrs.len())?
            > MAX_RENDER_ORDER_EVENTS
    {
        return None;
    }
    let mut observations = Vec::with_capacity(ordered_color_target_view_ptrs.len());
    for (attachment_ordinal, view_ptr) in ordered_color_target_view_ptrs.iter().copied().enumerate()
    {
        let mut observed = state.registered_render_targets.get(&view_ptr)?.clone();
        observed.capture_sequence = capture_sequence;
        observed.attachment_ordinal = u32::try_from(attachment_ordinal).ok()?;
        observed.materialize().ok()?;
        observations.push(observed);
    }
    state.render_target_use_observations.extend(observations);
    Some(())
}

fn next_catalog_observation_sequence() -> Option<u64> {
    let sequence = NEXT_CATALOG_OBSERVATION_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    (sequence != 0).then_some(sequence)
}

fn inside_non_ui_render_phase() -> bool {
    CURRENT_MAP_SCENE_PHASE.with(|value| *value.borrow())
        || CURRENT_POST_PROCESS_PHASE.with(|value| *value.borrow())
}

fn bound_render_program(render_context: usize) -> Option<CurrentRenderProgram> {
    BOUND_RENDER_PROGRAMS.with(|value| {
        value
            .borrow()
            .get(&render_context)
            .copied()
            .map(|shader_program_ptr| CurrentRenderProgram {
                render_context_ptr: render_context,
                shader_program_ptr,
            })
    })
}

fn reject_render_order_event() {
    if let Some(mut state) = STATE.get().and_then(|value| value.lock().ok()) {
        state.render_order_event_overflows += 1;
        notify_render_submit_state_change();
    }
}

unsafe extern "system" fn hook_engine_shader_effect_lookup(
    shader_cache: usize,
    render_device: usize,
    logical_shader_path: usize,
    effect_name: usize,
    effect_alias: usize,
    render_state: usize,
    defines: usize,
) -> i32 {
    let original: EngineShaderEffectLookupFn =
        unsafe { mem::transmute(ORIGINAL_ENGINE_SHADER_EFFECT_LOOKUP.load(Ordering::Acquire)) };
    let program_index = unsafe {
        original(
            shader_cache,
            render_device,
            logical_shader_path,
            effect_name,
            effect_alias,
            render_state,
            defines,
        )
    };
    let Some(effect_name) = read_c_string(effect_name, 256) else {
        if let Some(mut state) = STATE.get().and_then(|value| value.lock().ok()) {
            state.rejected_map_shader_lookups += 1;
        }
        return program_index;
    };
    let program_index_u32 = u32::try_from(program_index).ok();
    let shader_program_ptr = (program_index_u32.is_some() && shader_cache != 0)
        .then(|| {
            (program_index_u32.expect("checked above") as usize)
                .checked_mul(0x38)
                .and_then(|offset| shader_cache.checked_add(0x20)?.checked_add(offset))
        })
        .flatten();
    if effect_name.is_empty() || shader_program_ptr.is_none() || program_index_u32.is_none() {
        if let Some(mut state) = STATE.get().and_then(|value| value.lock().ok()) {
            state.rejected_map_shader_lookups += 1;
        }
        return program_index;
    }
    let shader_program_ptr = shader_program_ptr.expect("checked above");
    let engine_effect = EngineEffectIdentity {
        program_index_u32: program_index_u32.expect("checked above"),
        effect_token: effect_name.clone(),
    };
    if let Some(mut state) = STATE.get().and_then(|value| value.lock().ok()) {
        match state
            .engine_effect_tokens_by_program
            .get(&shader_program_ptr)
        {
            Some(existing) if existing != &engine_effect => {
                reject_locked(&mut state);
                return program_index;
            }
            Some(_) => {}
            None if state.engine_effect_tokens_by_program.len() >= MAX_MAP_SHADER_LOOKUPS => {
                reject_locked(&mut state);
                return program_index;
            }
            None => {
                state
                    .engine_effect_tokens_by_program
                    .insert(shader_program_ptr, engine_effect);
            }
        }
    }
    let Some(path) = read_c_string(logical_shader_path, 512) else {
        return program_index;
    };
    let Some(registration_root_rva) =
        CURRENT_MAP_CONTENT_REGISTRATION_ROOT.with(|value| *value.borrow())
    else {
        return program_index;
    };
    let sequence = NEXT_CATALOG_OBSERVATION_SEQUENCE.fetch_add(1, Ordering::AcqRel);
    if sequence == 0 {
        if let Some(mut state) = STATE.get().and_then(|value| value.lock().ok()) {
            state.rejected_map_shader_lookups += 1;
        }
        return program_index;
    }
    if let Some(mut state) = STATE.get().and_then(|value| value.lock().ok()) {
        if state.map_shader_lookups.len() >= MAX_MAP_SHADER_LOOKUPS {
            state.map_shader_lookup_overflows += 1;
        } else {
            let Some(source_open_observation_sequence) = state
                .latest_vfs_open_sequence_by_path
                .get(&path)
                .copied()
                .filter(|open_sequence| *open_sequence < sequence)
            else {
                state.rejected_map_shader_lookups += 1;
                return program_index;
            };
            let lookup = RawMapShaderLookup {
                sequence,
                engine_lookup_rva: ENGINE_SHADER_EFFECT_LOOKUP_RVA,
                registration_root_rva,
                logical_shader_path: path,
                effect_name,
                program_index,
            };
            state.map_shader_lookups.push_back(lookup.clone());
            state
                .map_shader_program_joins
                .push_back(RawMapShaderProgramJoin {
                    lookup,
                    shader_program_ptr,
                    source_open_observation_sequence,
                });
        }
    }
    program_index
}

fn enter_map_content_registration_root(root_rva: u32) -> Option<u32> {
    CURRENT_MAP_CONTENT_REGISTRATION_ROOT.with(|value| value.replace(Some(root_rva)))
}

fn leave_map_content_registration_root(previous: Option<u32>) {
    CURRENT_MAP_CONTENT_REGISTRATION_ROOT.with(|value| *value.borrow_mut() = previous);
}

unsafe extern "system" fn hook_map_effect_registration_root(this: usize, device: usize) -> usize {
    let previous = enter_map_content_registration_root(MAP_EFFECT_REGISTRATION_ROOT_RVA);
    let original: MapEffectRegistrationRootFn =
        unsafe { mem::transmute(ORIGINAL_MAP_EFFECT_REGISTRATION_ROOT.load(Ordering::Acquire)) };
    let result = unsafe { original(this, device) };
    leave_map_content_registration_root(previous);
    result
}

unsafe extern "system" fn hook_railroad_effect_registration_root(this: usize) {
    let previous = enter_map_content_registration_root(RAILROAD_EFFECT_REGISTRATION_ROOT_RVA);
    let original: RailroadEffectRegistrationRootFn = unsafe {
        mem::transmute(ORIGINAL_RAILROAD_EFFECT_REGISTRATION_ROOT.load(Ordering::Acquire))
    };
    unsafe { original(this) };
    leave_map_content_registration_root(previous);
}

unsafe extern "system" fn hook_map_name_effect_registration_root(
    this: usize,
    device: usize,
) -> usize {
    let previous = enter_map_content_registration_root(MAP_NAME_EFFECT_REGISTRATION_ROOT_RVA);
    let original: MapNameEffectRegistrationRootFn = unsafe {
        mem::transmute(ORIGINAL_MAP_NAME_EFFECT_REGISTRATION_ROOT.load(Ordering::Acquire))
    };
    let result = unsafe { original(this, device) };
    leave_map_content_registration_root(previous);
    result
}

unsafe extern "system" fn hook_restore_bloom_effect_registration_root(this: usize, device: usize) {
    let previous = enter_map_content_registration_root(RESTORE_BLOOM_EFFECT_REGISTRATION_ROOT_RVA);
    let original: RestoreBloomEffectRegistrationRootFn = unsafe {
        mem::transmute(ORIGINAL_RESTORE_BLOOM_EFFECT_REGISTRATION_ROOT.load(Ordering::Acquire))
    };
    unsafe { original(this, device) };
    leave_map_content_registration_root(previous);
}

unsafe extern "system" fn hook_lut_blend_effect_registration_root(
    this: usize,
    device: usize,
) -> usize {
    let previous = enter_map_content_registration_root(LUT_BLEND_EFFECT_REGISTRATION_ROOT_RVA);
    let original: LutBlendEffectRegistrationRootFn = unsafe {
        mem::transmute(ORIGINAL_LUT_BLEND_EFFECT_REGISTRATION_ROOT.load(Ordering::Acquire))
    };
    let result = unsafe { original(this, device) };
    leave_map_content_registration_root(previous);
    result
}

fn capture_entity_render_root(
    renderer: usize,
    render_context: usize,
    pass_selector: i32,
    bind_renderer_target: u8,
    alternate_list: u8,
) -> bool {
    let Ok(pass_selector_u32) = u32::try_from(pass_selector) else {
        return false;
    };
    if pass_selector_u32 > 0x100 || renderer == 0 || render_context == 0 || alternate_list > 1 {
        return false;
    }
    let Some(base) = MODULE_BASE
        .load(Ordering::Acquire)
        .checked_add(if alternate_list == 0 {
            ENTITY_RENDER_LISTS_REGULAR_RVA
        } else {
            ENTITY_RENDER_LISTS_ALTERNATE_RVA
        })
    else {
        return false;
    };
    let Some(list_header) = (pass_selector_u32 as usize)
        .checked_mul(0x18)
        .and_then(|offset| base.checked_add(offset))
    else {
        return false;
    };
    let (Some(outer_base), Some(outer_count)) =
        (read_usize(list_header), read_u32(list_header + 0x0c))
    else {
        return false;
    };
    if outer_count > MAX_ENTITY_LIST_ROWS || (outer_count != 0 && outer_base == 0) {
        return false;
    }
    let Some((submit_index, pass_ordinal)) = CURRENT_SUBMIT.with(|slot| {
        let mut slot = slot.borrow_mut();
        let submit = slot.as_mut()?;
        let ordinal = submit.next_entity_pass_ordinal;
        submit.next_entity_pass_ordinal = ordinal.checked_add(1)?;
        Some((submit.submit_index, ordinal))
    }) else {
        return false;
    };
    let mut captured = Vec::new();
    for outer_index in 0..outer_count as usize {
        let Some(row) = outer_index
            .checked_mul(0x50)
            .and_then(|offset| outer_base.checked_add(offset))
        else {
            return false;
        };
        let (Some(group), Some(entity_ptr)) = (read_u32(row), read_usize(row + 8)) else {
            return false;
        };
        let Some(identity) = STATE
            .get()
            .and_then(|value| value.lock().ok())
            .and_then(|state| state.entities.get(&entity_ptr).cloned())
        else {
            return false;
        };
        let mut transform_bytes = [0u8; 64];
        if read_exact(row + 0x10, &mut transform_bytes).is_none() {
            return false;
        }
        let mut transform = [0u32; 16];
        for (index, chunk) in transform_bytes.chunks_exact(4).enumerate() {
            transform[index] = u32::from_le_bytes(chunk.try_into().unwrap());
        }
        let skinning = match read_usize(entity_ptr + 0x70) {
            Some(0) => RawSkinning::NotSkinned,
            Some(owner) => {
                let (Some(count_ptr), Some(data_ptr)) =
                    (read_usize(owner + 8), read_usize(owner + 0x18))
                else {
                    return false;
                };
                let Some(byte_count) = read_u32(count_ptr)
                    .and_then(|count| count.checked_mul(16))
                    .and_then(|count| usize::try_from(count).ok())
                else {
                    return false;
                };
                if byte_count != 3_200 || data_ptr == 0 {
                    return false;
                }
                let mut bytes = vec![0u8; byte_count];
                if read_exact(data_ptr, &mut bytes).is_none() {
                    return false;
                }
                RawSkinning::Palette3200(bytes)
            }
            None => return false,
        };
        let (Some(mesh_owner), Some(binding_lists)) =
            (read_usize(entity_ptr + 0x58), read_usize(entity_ptr + 0xb8))
        else {
            return false;
        };
        let Some(draw_lists) = read_usize(mesh_owner + 0x48) else {
            return false;
        };
        let Some(group_offset) = (group as usize).checked_mul(0x18) else {
            return false;
        };
        let (Some(draw_base), Some(draw_count), Some(binding_base)) = (
            read_usize(draw_lists + group_offset),
            read_u32(draw_lists + group_offset + 0x0c),
            read_usize(binding_lists + group_offset),
        ) else {
            return false;
        };
        if draw_count > MAX_ENTITY_LIST_ROWS
            || (draw_count != 0 && (draw_base == 0 || binding_base == 0))
        {
            return false;
        }
        let Some(entity_variant) = read_u8(entity_ptr + 0x78) else {
            return false;
        };
        for draw_index in 0..draw_count as usize {
            let (Some(mesh_draw_ptr), Some(material_binding_ptr)) = (
                draw_index
                    .checked_mul(0x108)
                    .and_then(|offset| draw_base.checked_add(offset)),
                draw_index
                    .checked_mul(0x60)
                    .and_then(|offset| binding_base.checked_add(offset)),
            ) else {
                return false;
            };
            let Some(draw_variant) = read_u8(mesh_draw_ptr + 0x7d) else {
                return false;
            };
            let variant_class = u8::from(entity_variant != 0 || draw_variant != 0);
            if variant_class != alternate_list {
                continue;
            }
            let shader_index_offset = if entity_variant == 0 {
                let Some(offset) = (pass_selector_u32 as usize)
                    .checked_add(3)
                    .and_then(|value| value.checked_mul(4))
                else {
                    return false;
                };
                offset
            } else {
                0x2c
            };
            let Some(shader_index) = read_i32(material_binding_ptr + shader_index_offset) else {
                return false;
            };
            let shader_program_ptr = if shader_index < 0 {
                MODULE_BASE.load(Ordering::Acquire) + ENTITY_SHADER_FALLBACK_RVA
            } else {
                let Some(offset) = (shader_index as usize).checked_mul(0x38) else {
                    return false;
                };
                let Some(address) = renderer
                    .checked_add(0x558)
                    .and_then(|v| v.checked_add(offset))
                else {
                    return false;
                };
                address
            };
            let state_flags = u32::from(entity_variant)
                | (u32::from(draw_variant) << 8)
                | (u32::from(bind_renderer_target) << 16)
                | (u32::from(alternate_list) << 24);
            captured.push(RawEntitySubmission {
                submit_index,
                pass_ordinal,
                draw_order: u32::MAX,
                entity_instance_id: identity.stable_id,
                entity_registration_token: identity.registration_token.clone(),
                entity_object_ptr: entity_ptr,
                entity_definition_ptr: identity.definition_ptr,
                renderer_ptr: renderer,
                render_context_ptr: render_context,
                mesh_draw_ptr,
                material_binding_ptr,
                entity_group_index: group,
                mesh_draw_list_index: draw_index as u32,
                entity_variant_u8: entity_variant,
                mesh_draw_variant_u8: draw_variant,
                pass_selector_i32: pass_selector,
                bind_renderer_target_u8: bind_renderer_target,
                alternate_list_u8: alternate_list,
                shader_program_ptr,
                additional_shader_stages_authenticated_null: false,
                stream_output_targets_authenticated_null: false,
                world_transform_4x4_f32_bits: transform,
                state_flags,
                skinning: skinning.clone(),
            });
        }
    }
    CURRENT_SUBMIT.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(submit) = slot.as_mut() else {
            return false;
        };
        if submit
            .entity_submissions
            .len()
            .checked_add(captured.len())
            .is_none_or(|count| count > MAX_ENTITY_DRAWS_PER_SUBMIT)
        {
            return false;
        }
        for draw in captured {
            submit.entity_submissions.push(draw);
        }
        true
    })
}

unsafe extern "system" fn hook_seed(words: *mut u32) {
    let original: SeedFn = unsafe { mem::transmute(ORIGINAL_SEED.load(Ordering::Acquire)) };
    unsafe { original(words) };
    if words.is_null() {
        reject();
        return;
    }
    let inside_entity_factory = ENTITY_FACTORY_STACK.with(|stack| !stack.borrow().is_empty());
    if inside_entity_factory {
        return;
    }
    let Some(first) = read_u32(words as usize) else {
        reject();
        return;
    };
    let Some(second) = read_u32(words as usize + 4) else {
        reject();
        return;
    };
    CURRENT_WEATHER.with(|slot| {
        if let Some(weather) = slot.borrow_mut().as_mut() {
            if weather
                .selection_seed_words
                .replace([first, second])
                .is_some()
            {
                weather.rejected = true;
            }
        }
    });
}

unsafe extern "system" fn hook_weather_normal_update_owner(
    region_visual: usize,
    view: usize,
    release_context: usize,
    transition: f32,
    update_visuals: u8,
) {
    let update_index = NEXT_WEATHER_UPDATE_INDEX.fetch_add(1, Ordering::AcqRel);
    let owner = WeatherUpdateOwner {
        update_index,
        owner: RawWeatherUpdateOwner::NormalStrategicRegionUpdate,
    };
    let previous = CURRENT_WEATHER_OWNER.with(|slot| slot.borrow_mut().replace(owner));
    if update_index == 0 || previous.is_some() {
        reject();
    }
    let original: WeatherNormalUpdateOwnerFn = unsafe {
        mem::transmute(ORIGINAL_WEATHER_NORMAL_UPDATE_OWNER.load(Ordering::Acquire))
    };
    unsafe { original(region_visual, view, release_context, transition, update_visuals) };
    let completed = CURRENT_WEATHER_OWNER.with(|slot| slot.borrow_mut().take());
    if completed != Some(owner) {
        reject();
    }
    CURRENT_WEATHER_OWNER.with(|slot| *slot.borrow_mut() = previous);
}

unsafe extern "system" fn hook_weather_editor_refresh_owner(editor: usize, region_index: u32) {
    let update_index = NEXT_WEATHER_UPDATE_INDEX.fetch_add(1, Ordering::AcqRel);
    let owner = WeatherUpdateOwner {
        update_index,
        owner: RawWeatherUpdateOwner::EditorRefresh,
    };
    let previous = CURRENT_WEATHER_OWNER.with(|slot| slot.borrow_mut().replace(owner));
    if update_index == 0 || previous.is_some() {
        reject();
    }
    let original: WeatherEditorRefreshOwnerFn = unsafe {
        mem::transmute(ORIGINAL_WEATHER_EDITOR_REFRESH_OWNER.load(Ordering::Acquire))
    };
    unsafe { original(editor, region_index) };
    let completed = CURRENT_WEATHER_OWNER.with(|slot| slot.borrow_mut().take());
    if completed != Some(owner) {
        reject();
    }
    CURRENT_WEATHER_OWNER.with(|slot| *slot.borrow_mut() = previous);
}

fn weather_entity_role(producer: usize, entity: usize) -> Option<WeatherEntityRole> {
    let pair_count = read_u32(producer.checked_add(0x54)?)?;
    if pair_count > MAX_WEATHER_PAIRS {
        return None;
    }
    let pairs = read_usize(producer.checked_add(0x48)?)?;
    for index in 0..pair_count as usize {
        let pair = pairs.checked_add(index.checked_mul(0x10)?)?;
        if read_usize(pair)? == entity {
            return Some(WeatherEntityRole::Primary);
        }
        if read_usize(pair.checked_add(8)?)? == entity {
            return Some(WeatherEntityRole::Fading);
        }
    }
    None
}

fn record_weather_setter(
    entity: usize,
    update: impl FnOnce(&mut PendingWeatherSetterWrites) -> bool,
) {
    CURRENT_WEATHER.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(pending) = slot.as_mut() else { return };
        if weather_entity_role(pending.producer, entity).is_none() {
            return;
        }
        let row = pending.setter_writes.entry(entity).or_default();
        if !update(row) {
            pending.rejected = true;
        }
    });
}

unsafe extern "system" fn hook_weather_active_setter(entity: usize, active: u8) {
    record_weather_setter(entity, |row| {
        row.active_count = row.active_count.saturating_add(1);
        row.active_value = active != 0;
        row.active_count == 1 && active <= 1
    });
    let original: WeatherActiveSetterFn =
        unsafe { mem::transmute(ORIGINAL_WEATHER_ACTIVE_SETTER.load(Ordering::Acquire)) };
    unsafe { original(entity, active) };
}

unsafe extern "system" fn hook_weather_alpha_setter(entity: usize, alpha: f32) {
    record_weather_setter(entity, |row| {
        row.alpha_count = row.alpha_count.saturating_add(1);
        row.alpha_f32_bits = alpha.to_bits();
        row.alpha_count == 1 && alpha.is_finite()
    });
    let original: WeatherAlphaSetterFn =
        unsafe { mem::transmute(ORIGINAL_WEATHER_ALPHA_SETTER.load(Ordering::Acquire)) };
    unsafe { original(entity, alpha) };
}

unsafe extern "system" fn hook_weather_playback_setter(entity: usize, playback: f64) {
    record_weather_setter(entity, |row| {
        row.playback_count = row.playback_count.saturating_add(1);
        row.playback_f32_bits = (playback as f32).to_bits();
        row.playback_count == 1 && playback.is_finite()
    });
    let original: WeatherPlaybackSetterFn =
        unsafe { mem::transmute(ORIGINAL_WEATHER_PLAYBACK_SETTER.load(Ordering::Acquire)) };
    unsafe { original(entity, playback) };
}

unsafe extern "system" fn hook_weather(producer: usize, view: usize, release_context: usize) {
    let owner = CURRENT_WEATHER_OWNER.with(|slot| *slot.borrow());
    let region_id = read_usize(producer + 0x30).and_then(|region| read_u32(region + 0x58));
    let previous_code = read_u32(producer + 0x60).and_then(|value| u8::try_from(value).ok());
    let view_distance_f32_bits = read_u32(view + 0x184);
    let pending = match (owner, region_id, previous_code, view_distance_f32_bits) {
        (Some(owner), Some(strategic_region_id), Some(previous_code), Some(view_distance_f32_bits))
            if owner.update_index != 0 && !has_pending_render_frame() => Some(PendingWeather {
            producer,
            update_index: owner.update_index,
            owner: owner.owner,
            strategic_region_id,
            previous_code,
            view_distance_f32_bits,
            selection_seed_words: None,
            factory_attempts: Vec::new(),
            setter_writes: HashMap::new(),
            rejected: false,
        }),
        _ => {
            reject();
            None
        }
    };
    CURRENT_WEATHER.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.is_some() {
            if let Some(mut state) = STATE.get().and_then(|value| value.lock().ok()) {
                state.nested_weather_calls += 1;
            }
        }
        *slot = pending;
    });
    let original: WeatherFn = unsafe { mem::transmute(ORIGINAL_WEATHER.load(Ordering::Acquire)) };
    unsafe { original(producer, view, release_context) };
    let pending = CURRENT_WEATHER.with(|slot| slot.borrow_mut().take());
    let Some(pending) = pending else {
        reject();
        return;
    };
    if pending.rejected {
        reject();
        return;
    }
    let Some(current_code) = read_u32(producer + 0x60).and_then(|value| u8::try_from(value).ok())
    else {
        reject();
        return;
    };
    let observed_code = match current_code {
        0 | 2 | 3 | 4 | 5 | 7 => current_code,
        _ => {
            reject();
            return;
        }
    };
    if pending.owner == RawWeatherUpdateOwner::EditorRefresh {
        // The editor refresh root is deliberately authenticated and scoped so
        // it cannot masquerade as a normal strategic-region update. Editor
        // rows are not eligible for production Present ownership.
        return;
    }
    let base = MODULE_BASE.load(Ordering::Acquire);
    let expected_region_count = base
        .checked_add(0x0331_6E90)
        .and_then(read_usize)
        .and_then(|game_state| game_state.checked_add(0x688))
        .and_then(read_usize)
        .and_then(|manager| manager.checked_add(0x4c))
        .and_then(read_u32);
    let Some(expected_region_count) = expected_region_count.filter(|count| {
        *count != 0 && pending.strategic_region_id < *count
    }) else {
        reject();
        return;
    };
    let Some(mut state) = STATE.get().and_then(|value| value.lock().ok()) else {
        reject();
        return;
    };
    let prior_membership = state
        .latest_weather_observation_by_region
        .get(&pending.strategic_region_id)
        .cloned();
    if state
        .expected_weather_region_count
        .is_some_and(|prior| prior != expected_region_count)
    {
        drop(state);
        reject();
        return;
    }
    state.expected_weather_region_count = Some(expected_region_count);
    let initial_observation = prior_membership.is_none();
    state
        .weather_runtime_codes
        .insert(pending.strategic_region_id, observed_code);
    drop(state);
    let membership = if initial_observation || current_code != pending.previous_code {
        let sequence = NEXT_RENDER_ORDER_SEQUENCE.fetch_add(1, Ordering::AcqRel);
        let Some(observation) = capture_weather_change(
            producer,
            current_code,
            &pending,
            sequence,
            initial_observation,
            false,
        ) else {
            reject();
            return;
        };
        Some(observation)
    } else {
        let Some(prior) = prior_membership.as_ref() else {
            reject();
            return;
        };
        let checkpoint_pending = PendingWeather {
            producer: pending.producer,
            update_index: pending.update_index,
            owner: pending.owner,
            strategic_region_id: pending.strategic_region_id,
            previous_code: current_code,
            view_distance_f32_bits: pending.view_distance_f32_bits,
            selection_seed_words: prior.selection_seed_words,
            factory_attempts: prior.ordered_factory_attempts.clone(),
            setter_writes: pending.setter_writes.clone(),
            rejected: false,
        };
        let sequence = NEXT_RENDER_ORDER_SEQUENCE.fetch_add(1, Ordering::AcqRel);
        let Some(candidate) = capture_weather_change(
            producer,
            current_code,
            &checkpoint_pending,
            sequence,
            false,
            true,
        ) else {
            reject();
            return;
        };
        weather_checkpoint_changed(prior, &candidate).then_some(candidate)
    };
    let visibility = if current_code != 0 {
        let sequence = NEXT_RENDER_ORDER_SEQUENCE.fetch_add(1, Ordering::AcqRel);
        let Some(observation) = capture_weather_visibility(&pending, sequence) else {
            reject();
            return;
        };
        Some(observation)
    } else {
        None
    };
    if let Some(mut state) = STATE.get().and_then(|value| value.lock().ok()) {
        if let Some(observation) = membership {
            if state.weather_changes.len() == MAX_SUBMITS {
                state.weather_changes.pop_front();
                state.weather_change_overflows =
                    state.weather_change_overflows.saturating_add(1);
            }
            state
                .latest_weather_observation_by_region
                .insert(observation.strategic_region_id, observation.clone());
            state.weather_changes.push_back(observation);
        }
        if let Some(observation) = visibility {
            if state.weather_visibility_updates.len() == MAX_SUBMITS {
                state.weather_visibility_updates.pop_front();
            }
            state
                .latest_weather_visibility_by_region
                .insert(observation.strategic_region_id, observation.clone());
            state.weather_visibility_updates.push_back(observation);
        }
    }
}

fn weather_checkpoint_changed(
    prior: &RawWeatherObservation,
    current: &RawWeatherObservation,
) -> bool {
    prior.rain_light_active != current.rain_light_active
        || prior.rain_heavy_active != current.rain_heavy_active
        || prior.snow_active != current.snow_active
        || prior.blizzard_active != current.blizzard_active
        || prior.sandstorm_active != current.sandstorm_active
        || prior.arctic_water_active != current.arctic_water_active
        || prior.region_value_i64_fixed_1e5 != current.region_value_i64_fixed_1e5
        || prior.transition_expiry_game_hour_i32 != current.transition_expiry_game_hour_i32
        || prior.airborne_runtime_code != current.airborne_runtime_code
        || prior.ordered_live_pairs != current.ordered_live_pairs
}

fn capture_weather_change(
    producer: usize,
    current_code: u8,
    pending: &PendingWeather,
    sequence: u64,
    initial_observation: bool,
    checkpoint_observation: bool,
) -> Option<RawWeatherObservation> {
    let base = MODULE_BASE.load(Ordering::Acquire);
    let game_state = read_usize(base.checked_add(0x0331_6E90)?)?;
    let manager = read_usize(game_state.checked_add(0x688)?)?;
    let region_count = read_u32(manager.checked_add(0x4c)?)?;
    if pending.strategic_region_id >= region_count {
        return None;
    }
    let records = read_usize(manager.checked_add(0x40)?)?;
    let region = records.checked_add(pending.strategic_region_id as usize * 0x160)?;
    let pair_count = read_u32(producer.checked_add(0x54)?)?;
    let pair_capacity = read_u32(producer.checked_add(0x50)?)?;
    if pair_count > pair_capacity || pair_count > MAX_WEATHER_PAIRS {
        return None;
    }
    if current_code == 0 && pair_count != 0 {
        return None;
    }
    if current_code == 0 && pending.selection_seed_words.is_some() {
        return None;
    }
    let pairs_base = read_usize(producer.checked_add(0x48)?)?;
    let state = STATE.get()?.lock().ok()?;
    let mut pairs = Vec::with_capacity(pair_count as usize);
    for index in 0..pair_count as usize {
        let pair = pairs_base.checked_add(index.checked_mul(0x10)?)?;
        let primary = read_usize(pair)?;
        let fading = read_usize(pair.checked_add(8)?)?;
        pairs.push(RawWeatherEntityPair {
            primary: capture_weather_entity(primary, &state)?,
            fading: capture_weather_entity(fading, &state)?,
        });
    }
    drop(state);
    Some(RawWeatherObservation {
        observation_index: NEXT_WEATHER_OBSERVATION_ID.fetch_add(1, Ordering::AcqRel),
        sequence,
        update_index: pending.update_index,
        owner: pending.owner,
        initial_observation,
        checkpoint_observation,
        strategic_region_id: pending.strategic_region_id,
        rain_light_active: read_u8(region.checked_add(0x108)?)? != 0,
        rain_heavy_active: read_u8(region.checked_add(0x110)?)? != 0,
        snow_active: read_u8(region.checked_add(0x118)?)? != 0,
        blizzard_active: read_u8(region.checked_add(0x120)?)? != 0,
        sandstorm_active: read_u8(region.checked_add(0x128)?)? != 0,
        arctic_water_active: read_u8(region.checked_add(0x130)?)? != 0,
        region_value_i64_fixed_1e5: read_i64(region.checked_add(0x138)?)?,
        transition_expiry_game_hour_i32: read_i32(region.checked_add(0x150)?)?,
        previous_airborne_runtime_code: pending.previous_code,
        airborne_runtime_code: current_code,
        selection_seed_words: pending.selection_seed_words,
        ordered_factory_attempts: pending.factory_attempts.clone(),
        ordered_live_pairs: pairs,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct WeatherEntityPostState {
    active: bool,
    alpha_f32_bits: u32,
    playback_f32_bits: u32,
}

fn read_weather_entity_post_state_once(entity: usize) -> Option<WeatherEntityPostState> {
    let active = read_u8(entity.checked_add(0xe8)?)? != 0;
    let playback = f64::from_bits(read_usize(entity.checked_add(0x78)?)? as u64) as f32;
    if !playback.is_finite() {
        return None;
    }
    let child_count = read_u32(entity.checked_add(0x8c)?)?;
    if child_count == 0 || child_count > MAX_ENTITY_LIST_ROWS {
        return None;
    }
    let child_slots = read_usize(entity.checked_add(0x80)?)?;
    let mut common = None;
    for index in 0..child_count as usize {
        let slot = child_slots.checked_add(index.checked_mul(mem::size_of::<usize>())?)?;
        let slot_ptr = read_usize(slot)?;
        let child = read_usize(slot_ptr)?;
        let child_state = read_usize(child.checked_add(0x70)?)?;
        let child_active = read_u8(child.checked_add(0x60)?)? & 1 != 0;
        let alpha_f32_bits = read_u32(child_state.checked_add(0x4c)?)?;
        let playback_f32_bits = read_u32(child_state.checked_add(0x50)?)?;
        if !f32::from_bits(alpha_f32_bits).is_finite()
            || !f32::from_bits(playback_f32_bits).is_finite()
            || child_active != active
            || playback_f32_bits != playback.to_bits()
        {
            return None;
        }
        let row = WeatherEntityPostState {
            active: child_active,
            alpha_f32_bits,
            playback_f32_bits,
        };
        if common.is_some_and(|prior| prior != row) {
            return None;
        }
        common = Some(row);
    }
    common
}

fn capture_weather_entity_visibility(
    entity: usize,
    role: WeatherEntityRole,
    writes: PendingWeatherSetterWrites,
    state: &HookState,
) -> Option<WeatherEntityVisibility> {
    let stable_id = state.entities.get(&entity)?.stable_id;
    if stable_id == 0
        || writes.active_count > 1
        || writes.alpha_count > 1
        || writes.playback_count != 1
    {
        return None;
    }
    let before = read_weather_entity_post_state_once(entity)?;
    let after = read_weather_entity_post_state_once(entity)?;
    if before != after
        || writes.playback_f32_bits != after.playback_f32_bits
        || (writes.active_count == 1 && writes.active_value != after.active)
        || (writes.alpha_count == 1 && writes.alpha_f32_bits != after.alpha_f32_bits)
    {
        return None;
    }
    Some(WeatherEntityVisibility {
        entity_instance_id: stable_id,
        role,
        post_update_active: after.active,
        post_update_alpha_f32_bits: after.alpha_f32_bits,
        post_update_playback_rate_f32_bits: after.playback_f32_bits,
        producer_wrote_active: writes.active_count == 1,
        producer_wrote_alpha: writes.alpha_count == 1,
    })
}

fn capture_weather_visibility(
    pending: &PendingWeather,
    sequence: u64,
) -> Option<RawWeatherVisibilityObservation> {
    let (camera_branch, computed_fade_alpha_f32_bits, computed_playback_rate_f32_bits) =
        portable_render_stream::weather_visibility_law(pending.view_distance_f32_bits).ok()?;
    let pair_count = read_u32(pending.producer.checked_add(0x54)?)?;
    let pair_capacity = read_u32(pending.producer.checked_add(0x50)?)?;
    if pair_count > pair_capacity || pair_count > MAX_WEATHER_PAIRS {
        return None;
    }
    let pairs = if pair_count == 0 {
        0
    } else {
        read_usize(pending.producer.checked_add(0x48)?)?
    };
    let state = STATE.get()?.lock().ok()?;
    let mut seen = BTreeSet::new();
    let mut ordered_entity_visibility = Vec::new();
    for index in 0..pair_count as usize {
        let pair = pairs.checked_add(index.checked_mul(0x10)?)?;
        for (role, entity) in [
            (WeatherEntityRole::Primary, read_usize(pair)?),
            (WeatherEntityRole::Fading, read_usize(pair.checked_add(8)?)?),
        ] {
            if entity == 0 {
                continue;
            }
            if !seen.insert(entity) {
                return None;
            }
            let writes = pending.setter_writes.get(&entity).copied().unwrap_or_default();
            ordered_entity_visibility.push(capture_weather_entity_visibility(
                entity, role, writes, &state,
            )?);
        }
    }
    if pending.setter_writes.keys().any(|entity| !seen.contains(entity)) {
        return None;
    }
    Some(RawWeatherVisibilityObservation {
        observation_index: NEXT_WEATHER_OBSERVATION_ID.fetch_add(1, Ordering::AcqRel),
        sequence,
        update_index: pending.update_index,
        owner: pending.owner,
        strategic_region_id: pending.strategic_region_id,
        view_distance_f32_bits: pending.view_distance_f32_bits,
        camera_branch,
        computed_fade_alpha_f32_bits,
        computed_playback_rate_f32_bits,
        ordered_entity_visibility,
    })
}

fn capture_weather_entity(entity: usize, state: &HookState) -> Option<Option<RawWeatherEntityRef>> {
    if entity == 0 {
        return Some(None);
    }
    let value = state.entities.get(&entity)?;
    Some(Some(RawWeatherEntityRef {
        entity_instance_id: value.stable_id,
        registration_token: value.registration_token.clone(),
        definition_ptr: value.definition_ptr,
        ordered_effect_instance_ids: value.ordered_effect_instance_ids.clone(),
    }))
}

fn read_particle_type_subsystem_tables(runtime: usize) -> Option<ParticleTypeSubsystemTables> {
    fn read_once(runtime: usize) -> Option<ParticleTypeSubsystemTables> {
        let type_definition_ptr = read_usize(runtime)?;
        if type_definition_ptr == 0 {
            return None;
        }
        let program_indices_ptr = read_usize(type_definition_ptr.checked_add(0x48)?)?;
        let program_count = read_u32(type_definition_ptr.checked_add(0x54)?)?;
        let atlas_rows_ptr = read_usize(type_definition_ptr.checked_add(0x60)?)?;
        let atlas_count = read_u32(type_definition_ptr.checked_add(0x6c)?)?;
        if program_indices_ptr == 0
            || atlas_rows_ptr == 0
            || program_count == 0
            || program_count != atlas_count
            || program_count as usize > MAX_PACKETS_PER_SUBMIT
        {
            return None;
        }
        Some(ParticleTypeSubsystemTables {
            type_definition_ptr,
            program_indices_ptr,
            atlas_rows_ptr,
            subsystem_count: program_count,
        })
    }

    let before = read_once(runtime)?;
    let after = read_once(runtime)?;
    (before == after).then_some(before)
}

fn read_particle_subsystem_row(
    tables: &ParticleTypeSubsystemTables,
    subsystem_selector_u32: u32,
) -> Option<ParticleSubsystemRow> {
    if subsystem_selector_u32 >= tables.subsystem_count {
        return None;
    }
    let selector = subsystem_selector_u32 as usize;
    let program_address = tables
        .program_indices_ptr
        .checked_add(selector.checked_mul(mem::size_of::<u32>())?)?;
    let atlas_address = tables
        .atlas_rows_ptr
        .checked_add(selector.checked_mul(0x14)?)?;

    fn read_once(program_address: usize, atlas_address: usize) -> Option<ParticleSubsystemRow> {
        let shader_program_index_u32 = read_u32(program_address)?;
        let mut atlas = [0u8; 0x14];
        read_exact(atlas_address, &mut atlas)?;
        Some(ParticleSubsystemRow {
            shader_program_index_u32,
            texture_runtime_index_u32: u32::from_le_bytes(atlas[0..4].try_into().ok()?),
            atlas_half_pixel_width_f32_bits: u32::from_le_bytes(atlas[4..8].try_into().ok()?),
            atlas_half_pixel_height_f32_bits: u32::from_le_bytes(atlas[8..12].try_into().ok()?),
            atlas_columns_f32_bits: u32::from_le_bytes(atlas[12..16].try_into().ok()?),
            atlas_rows_f32_bits: u32::from_le_bytes(atlas[16..20].try_into().ok()?),
        })
    }

    let before = read_once(program_address, atlas_address)?;
    let after = read_once(program_address, atlas_address)?;
    if before != after {
        return None;
    }
    let half_pixel_width = f32::from_bits(before.atlas_half_pixel_width_f32_bits);
    let half_pixel_height = f32::from_bits(before.atlas_half_pixel_height_f32_bits);
    let columns = f32::from_bits(before.atlas_columns_f32_bits);
    let rows = f32::from_bits(before.atlas_rows_f32_bits);
    if !half_pixel_width.is_finite()
        || half_pixel_width <= 0.0
        || !half_pixel_height.is_finite()
        || half_pixel_height <= 0.0
        || !columns.is_finite()
        || columns <= 0.0
        || columns.fract() != 0.0
        || columns > u32::MAX as f32
        || !rows.is_finite()
        || rows <= 0.0
        || rows.fract() != 0.0
        || rows > u32::MAX as f32
    {
        return None;
    }
    Some(before)
}

fn capture_packet(
    address: usize,
    identity: &RuntimeIdentity,
    type_tables: &ParticleTypeSubsystemTables,
    builder: u32,
    draw_order: u32,
    ribbon_tail_order: Option<u32>,
) -> Option<RawParticlePacket> {
    let mut bytes = [0u8; PACKET_BYTES];
    read_exact(address, &mut bytes)?;
    let live_count = u32::from_le_bytes(bytes[0x40..0x44].try_into().ok()?);
    let records_ptr = usize::from_le_bytes(bytes[0x48..0x50].try_into().ok()?);
    let stride = u32::from_le_bytes(bytes[0x50..0x54].try_into().ok()?);
    let max_count = u32::from_le_bytes(bytes[0x54..0x58].try_into().ok()?);
    if stride != GPU_INSTANCE_STRIDE
        || live_count > max_count
        || live_count > MAX_LIVE_PARTICLES_PER_PACKET
    {
        return None;
    }
    let byte_count = live_count.checked_mul(stride)? as usize;
    let mut records = vec![0u8; byte_count];
    if byte_count != 0 {
        read_exact(records_ptr, &mut records)?;
    }
    let mut matrix = [0u32; 16];
    for (index, word) in matrix.iter_mut().enumerate() {
        *word = u32::from_le_bytes(bytes[index * 4..index * 4 + 4].try_into().ok()?);
    }
    let subsystem_selector_u32 = u32::from_le_bytes(bytes[0x60..0x64].try_into().ok()?);
    let subsystem = read_particle_subsystem_row(type_tables, subsystem_selector_u32)?;
    Some(RawParticlePacket {
        effect_instance_id: identity.stable_id,
        definition: identity.definition.clone(),
        render_list_class: identity.render_list_class,
        render_subbucket: identity.render_subbucket,
        builder_call_ordinal: builder,
        packet_ordinal: subsystem_selector_u32,
        particle_type_definition_ptr: type_tables.type_definition_ptr,
        subsystem_selector_u32,
        shader_program_index_u32: subsystem.shader_program_index_u32,
        texture_runtime_index_u32: subsystem.texture_runtime_index_u32,
        engine_effect_token: String::new(),
        atlas_half_pixel_width_f32_bits: subsystem.atlas_half_pixel_width_f32_bits,
        atlas_half_pixel_height_f32_bits: subsystem.atlas_half_pixel_height_f32_bits,
        atlas_columns_f32_bits: subsystem.atlas_columns_f32_bits,
        atlas_rows_f32_bits: subsystem.atlas_rows_f32_bits,
        draw_order,
        shader_program_ptr: 0,
        additional_shader_stages_authenticated_null: false,
        stream_output_targets_authenticated_null: false,
        ribbon_tail_order,
        canonical_packet_112_bytes: canonicalize_packet(bytes).to_vec(),
        packet_matrix_4x4_f32_bits: matrix,
        live_count,
        max_count,
        ordered_gpu_instance_records: records,
        elapsed_f32_bits: u32::from_le_bytes(bytes[0x58..0x5c].try_into().ok()?),
        scale_f32_bits: identity.scale_f32_bits,
        flags_u16: u16::from_le_bytes(bytes[0x5c..0x5e].try_into().ok()?),
        parent_entity_instance_id: identity.parent_entity_instance_id,
    })
}

fn read_definition(address: usize) -> Option<DefinitionIdentity> {
    let token = read_fixed_string(address, 64)?;
    let source = read_fixed_string(address + 0x40, 64)?;
    let stored_hash = read_u32(address + 0x80)?;
    if token.is_empty() || source.is_empty() || stored_hash != fnv1a32(token.as_bytes()) {
        return None;
    }
    Some(DefinitionIdentity {
        registration_token: token,
        definition_source_token: source,
        registration_hash_u32: stored_hash,
    })
}

fn read_fixed_string(address: usize, len: usize) -> Option<String> {
    let mut bytes = vec![0u8; len];
    read_exact(address, &mut bytes)?;
    let end = bytes.iter().position(|byte| *byte == 0)?;
    String::from_utf8(bytes[..end].to_vec()).ok()
}
fn read_c_string(address: usize, max_len: usize) -> Option<String> {
    if address == 0 || max_len == 0 {
        return None;
    }
    let mut bytes = Vec::with_capacity(max_len.min(128));
    for offset in 0..max_len {
        let byte = read_u8(address.checked_add(offset)?)?;
        if byte == 0 {
            return String::from_utf8(bytes).ok();
        }
        bytes.push(byte);
    }
    None
}

fn read_msvc_string(address: usize, max_len: usize) -> Option<String> {
    if address == 0 || max_len == 0 {
        return None;
    }
    let mut first = [0u8; 32];
    let mut second = [0u8; 32];
    read_exact(address, &mut first)?;
    read_exact(address, &mut second)?;
    if first != second {
        return None;
    }
    let length = usize::from_le_bytes(first[0x10..0x18].try_into().ok()?);
    let capacity = usize::from_le_bytes(first[0x18..0x20].try_into().ok()?);
    if length == 0 || length > max_len || capacity < length {
        return None;
    }
    let bytes = if capacity < 16 {
        if length >= 16 {
            return None;
        }
        first[..length].to_vec()
    } else {
        let data_ptr = usize::from_le_bytes(first[..8].try_into().ok()?);
        if data_ptr == 0 {
            return None;
        }
        let mut before = vec![0u8; length];
        let mut after = vec![0u8; length];
        read_exact(data_ptr, &mut before)?;
        read_exact(data_ptr, &mut after)?;
        if before != after {
            return None;
        }
        before
    };
    if bytes.contains(&0) {
        return None;
    }
    String::from_utf8(bytes).ok()
}
fn read_u16(address: usize) -> Option<u16> {
    let mut bytes = [0u8; 2];
    read_exact(address, &mut bytes)?;
    Some(u16::from_le_bytes(bytes))
}
fn read_u8(address: usize) -> Option<u8> {
    let mut bytes = [0u8; 1];
    read_exact(address, &mut bytes)?;
    Some(bytes[0])
}
fn read_u32(address: usize) -> Option<u32> {
    let mut bytes = [0u8; 4];
    read_exact(address, &mut bytes)?;
    Some(u32::from_le_bytes(bytes))
}
fn read_u32x4(address: usize) -> Option<[u32; 4]> {
    let mut bytes = [0u8; 16];
    read_exact(address, &mut bytes)?;
    Some([
        u32::from_le_bytes(bytes[0..4].try_into().ok()?),
        u32::from_le_bytes(bytes[4..8].try_into().ok()?),
        u32::from_le_bytes(bytes[8..12].try_into().ok()?),
        u32::from_le_bytes(bytes[12..16].try_into().ok()?),
    ])
}
fn read_i32(address: usize) -> Option<i32> {
    let mut bytes = [0u8; 4];
    read_exact(address, &mut bytes)?;
    Some(i32::from_le_bytes(bytes))
}
fn read_i64(address: usize) -> Option<i64> {
    let mut bytes = [0u8; 8];
    read_exact(address, &mut bytes)?;
    Some(i64::from_le_bytes(bytes))
}
fn read_usize(address: usize) -> Option<usize> {
    let mut bytes = [0u8; 8];
    read_exact(address, &mut bytes)?;
    Some(usize::from_le_bytes(bytes))
}
fn read_bytes(address: usize, length: usize) -> Option<Vec<u8>> {
    let mut bytes = vec![0u8; length];
    read_exact(address, &mut bytes)?;
    Some(bytes)
}
fn read_exact(address: usize, out: &mut [u8]) -> Option<()> {
    if address == 0 || out.is_empty() {
        return out.is_empty().then_some(());
    }
    let mut read = 0usize;
    let ok = unsafe {
        ReadProcessMemory(
            GetCurrentProcess(),
            address as *const c_void,
            out.as_mut_ptr().cast(),
            out.len(),
            &mut read,
        )
    };
    (ok != 0 && read == out.len()).then_some(())
}

fn capture_authenticated_camera_carrier(
    carrier: usize,
    carrier_ordinal: u8,
) -> Result<Option<RawA729CameraCarrier>, String> {
    const ROOT_BYTES: usize = 0x740;
    const CARRIER_BYTES: usize = 0x1b4;
    let carrier_offset = [0x10usize, 0x230, 0x450]
        .get(usize::from(carrier_ordinal))
        .copied()
        .ok_or_else(|| "a729 camera carrier ordinal is outside 0..=2".to_string())?;
    let Some(root) = carrier.checked_sub(carrier_offset) else {
        return Ok(None);
    };
    let mut first = vec![0u8; ROOT_BYTES];
    let mut second = vec![0u8; ROOT_BYTES];
    if read_exact(root, &mut first).is_none() || read_exact(root, &mut second).is_none() {
        return Ok(None);
    }
    if first != second {
        return Err("a729 camera root changed during capture".into());
    }
    let owner = usize::from_le_bytes(
        first[0x08..0x10]
            .try_into()
            .map_err(|_| "a729 camera owner pointer decode")?,
    );
    if owner == 0 {
        return Ok(None);
    }
    let Some(owner_vtable) = read_usize(owner) else {
        return Ok(None);
    };
    let base = MODULE_BASE.load(Ordering::Acquire);
    let Some(owner_vtable_rva) = owner_vtable.checked_sub(base) else {
        return Ok(None);
    };
    let (owner_kind, owner_camera_offset) = match owner_vtable_rva {
        0x0293_3710 => (RawA729CameraOwnerKind::FrontEnd, 0x610usize),
        0x0295_2510 => (RawA729CameraOwnerKind::InGame, 0x750usize),
        0x0299_3698 => (RawA729CameraOwnerKind::Third, 0x638usize),
        _ => return Ok(None),
    };
    if owner.checked_add(owner_camera_offset) != Some(root) {
        return Ok(None);
    }
    if read_usize(owner) != Some(owner_vtable) {
        return Err("a729 camera owner vtable changed during capture".into());
    }
    let u32_at = |offset: usize| -> Result<u32, String> {
        Ok(u32::from_le_bytes(
            first[offset..offset + 4]
                .try_into()
                .map_err(|_| "a729 camera u32 field decode")?,
        ))
    };
    let i32_at = |offset: usize| -> Result<i32, String> {
        Ok(i32::from_le_bytes(
            first[offset..offset + 4]
                .try_into()
                .map_err(|_| "a729 camera i32 field decode")?,
        ))
    };
    let vec3_at = |offset: usize| -> Result<[u32; 3], String> {
        Ok([u32_at(offset)?, u32_at(offset + 4)?, u32_at(offset + 8)?])
    };
    let min_height_i32 = i32_at(0x6d0)?;
    let max_height_i32 = i32_at(0x6d4)?;
    let update_produced_u8 = first[0x6d8];
    let special_free_state_u8 = first[0x6d9];
    let wrap_branch_u8 = first[0x6f8];
    if min_height_i32 > max_height_i32
        || update_produced_u8 > 1
        || special_free_state_u8 > 1
        || wrap_branch_u8 > 1
        || vec3_at(0x190)? != vec3_at(0x680)?
        || vec3_at(0x19c)? != vec3_at(0x68c)?
    {
        return Err("a729 camera root state invariant mismatch".into());
    }
    Ok(Some(RawA729CameraCarrier {
        carrier_ordinal,
        carrier_offset: carrier_offset as u32,
        owner_kind,
        owner_vtable_rva: owner_vtable_rva as u32,
        render_view_436_bytes: first[carrier_offset..carrier_offset + CARRIER_BYTES].to_vec(),
        canonical_render_view_436_bytes: first[0x10..0x10 + CARRIER_BYTES].to_vec(),
        previous_current_eye_f32_bits: vec3_at(0x674)?,
        current_eye_f32_bits: vec3_at(0x680)?,
        current_look_at_f32_bits: vec3_at(0x68c)?,
        target_eye_f32_bits: vec3_at(0x698)?,
        target_look_at_f32_bits: vec3_at(0x6a4)?,
        viewport_i32: [
            i32_at(0x6c0)?,
            i32_at(0x6c4)?,
            i32_at(0x6c8)?,
            i32_at(0x6cc)?,
        ],
        min_height_i32,
        max_height_i32,
        update_produced_u8,
        special_free_state_u8,
        ramp_f32_bits: u32_at(0x6f4)?,
        wrap_branch_u8,
        camera_event_batches: Vec::new(),
    }))
}
fn verify_sentinel(address: usize, expected: &[u8; 16], label: &str) -> Result<(), String> {
    let mut actual = [0u8; 16];
    read_exact(address, &mut actual).ok_or_else(|| format!("could not read {label} sentinel"))?;
    if &actual != expected {
        return Err(format!("{label} sentinel mismatch at {address:#x}"));
    }
    Ok(())
}
fn reject() {
    if let Some(mut state) = STATE.get().and_then(|value| value.lock().ok()) {
        reject_locked(&mut state);
    }
}
fn reject_locked(state: &mut HookState) {
    state.rejected_observations += 1;
    notify_render_submit_state_change();
}
fn notify_render_submit_state_change() {
    RENDER_SUBMIT_PUBLICATION
        .get_or_init(Condvar::new)
        .notify_all();
}
fn reject_current() {
    CURRENT_SUBMIT.with(|slot| {
        if let Some(value) = slot.borrow_mut().as_mut() {
            value.rejected = true;
        } else {
            reject();
        }
    });
}
fn completeness_violated(state: &HookState) -> bool {
    state.rejected_observations != 0
        || state.evicted_submits != 0
        || state.duplicate_runtime_ids != 0
        || state.nested_envelopes != 0
        || state.duplicate_entity_ids != 0
        || state.nested_weather_calls != 0
        || state.render_order_event_overflows != 0
}
fn completeness_message(state: &HookState) -> String {
    format!("render hook completeness violated: rejected={} evicted={} duplicate_runtime={} nested_submit={} duplicate_entity={} nested_weather={} render_order_overflow={}",
        state.rejected_observations, state.evicted_submits, state.duplicate_runtime_ids,
        state.nested_envelopes, state.duplicate_entity_ids, state.nested_weather_calls,
        state.render_order_event_overflows)
}

fn current_process_epoch() -> RawProcessEpoch {
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    let process = unsafe { GetCurrentProcess() };
    let has_times =
        unsafe { GetProcessTimes(process, &mut creation, &mut exit, &mut kernel, &mut user) } != 0;
    let process_start_nonce = if has_times {
        (u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime)
    } else {
        0
    };
    RawProcessEpoch {
        process_id: unsafe { GetCurrentProcessId() },
        process_start_nonce,
        executable_sha256: A729_EXECUTABLE_SHA256.to_owned(),
    }
}

fn select_latest_creation_rows<T: Clone>(
    references: impl IntoIterator<Item = (usize, u64)>,
    rows: &[T],
    key: impl Fn(&T) -> (usize, u64),
) -> Option<Vec<T>> {
    let mut selected = BTreeMap::new();
    for (pointer, draw_sequence) in references {
        let row = rows
            .iter()
            .filter(|row| {
                let (candidate_pointer, sequence) = key(row);
                candidate_pointer == pointer && sequence <= draw_sequence
            })
            .max_by_key(|row| key(row).1)?;
        selected.insert(key(row), row.clone());
    }
    Some(selected.into_values().collect())
}

fn materialize_shader_creations(
    draws: &[RawD3dDrawState],
    rows: &[ObservedShaderCreation],
) -> Option<Vec<RawShaderCreation>> {
    let references = draws.iter().flat_map(|draw| {
        [
            (draw.vertex_shader_ptr, draw.final_draw_sequence),
            (draw.pixel_shader_ptr, draw.final_draw_sequence),
        ]
    });
    let selected = select_latest_creation_rows(references, rows, |row| {
        (row.shader_ptr, row.creation_sequence)
    })?;
    selected
        .into_iter()
        .map(materialize_shader_creation)
        .collect()
}

fn materialize_shader_creation(row: ObservedShaderCreation) -> Option<RawShaderCreation> {
    let compiler = row.compiler?;
    let captured_sha256 = format!("{:x}", Sha256::digest(&row.bytecode));
    let compiler_invocation = RawShaderCompilerInvocation {
        source_name: compiler.source_name.clone(),
        source_payload_sha256: compiler.source_payload_sha256.clone(),
        source_payload_bytes: compiler.source_payload_bytes,
        entry_token: compiler.entry_token.clone(),
        ordered_defines: compiler
            .ordered_defines
            .iter()
            .map(|(name, value)| RawShaderDefine {
                name: name.clone(),
                value: value.clone(),
                scope: "entry_point".into(),
            })
            .collect(),
    };
    Some(RawShaderCreation {
        creation_sequence: row.creation_sequence,
        shader_ptr: row.shader_ptr,
        stage: row.stage,
        compiler_identity: compiler.compiler_identity,
        target_profile: compiler.target_profile,
        compiler_flags_u64: compiler.compiler_flags_u64,
        compiler_invocation: Some(compiler_invocation),
        bytecode: RawPayloadSnapshot {
            captured_bytes: row.bytecode,
            captured_sha256,
            source: RawPayloadSource::RuntimeGenerated {
                reason: "d3dcompile_output".into(),
            },
        },
    })
}

fn build_live_shader_reflections(
    catalog: &RawRenderCatalogCapture,
) -> Result<Vec<RawShaderBytecodeReflectionObservation>, String> {
    let shaders = catalog
        .shader_creations
        .iter()
        .map(|creation| ((creation.shader_ptr, creation.creation_sequence), creation))
        .collect::<BTreeMap<_, _>>();
    let mut reflection_keys = BTreeMap::new();
    for draw in &catalog.draw_states {
        for (shader_ptr, stage) in [
            (draw.vertex_shader_ptr, RawShaderStage::Vertex),
            (draw.pixel_shader_ptr, RawShaderStage::Pixel),
        ] {
            let creation = shaders
                .iter()
                .filter(|((pointer, sequence), creation)| {
                    *pointer == shader_ptr
                        && *sequence <= draw.final_draw_sequence
                        && creation.stage == stage
                })
                .max_by_key(|((_, sequence), _)| *sequence)
                .map(|(_, creation)| *creation)
                .ok_or("runtime VS/PS lacks a selected exact creation receipt")?;
            reflection_keys.insert(
                (
                    draw.shader_program_ptr,
                    creation.shader_ptr,
                    creation.creation_sequence,
                ),
                creation,
            );
        }
    }
    reflect_program_shader_pairs(reflection_keys)
}

fn reflect_program_shader_pairs<'a>(
    reflection_keys: BTreeMap<(usize, usize, u64), &'a RawShaderCreation>,
) -> Result<Vec<RawShaderBytecodeReflectionObservation>, String> {
    let mut output = Vec::with_capacity(reflection_keys.len());
    for ((shader_program_ptr, _, _), creation) in reflection_keys {
        let entry_token = creation
            .compiler_invocation
            .as_ref()
            .map(|invocation| invocation.entry_token.as_str())
            .ok_or("shader reflection lacks an exact compiler entry token")?;
        let reflection_sequence =
            next_catalog_observation_sequence().ok_or("shader reflection sequence overflowed")?;
        output.push(crate::shader_reflection::reflect_shader_creation(
            creation,
            reflection_sequence,
            shader_program_ptr,
            entry_token,
        )?);
    }
    Ok(output)
}

fn build_static_catalog_fragment(state: &HookState) -> Result<RawStaticCatalogFragment, String> {
    let shader_references = state.static_d3d_captures.iter().flat_map(|capture| {
        let draw = &capture.state;
        [
            Some((draw.vertex_shader_ptr, draw.capture_sequence)),
            Some((draw.pixel_shader_ptr, draw.capture_sequence)),
            draw.geometry_shader_ptr
                .map(|pointer| (pointer, draw.capture_sequence)),
            draw.hull_shader_ptr
                .map(|pointer| (pointer, draw.capture_sequence)),
            draw.domain_shader_ptr
                .map(|pointer| (pointer, draw.capture_sequence)),
            draw.compute_shader_ptr
                .map(|pointer| (pointer, draw.capture_sequence)),
        ]
        .into_iter()
        .flatten()
    });
    let observed_shader_creations = select_latest_creation_rows(
        shader_references,
        &state.shader_creation_observations,
        |row| (row.shader_ptr, row.creation_sequence),
    )
    .ok_or("static draw references a shader without an exact creation receipt")?;
    let shader_creations = observed_shader_creations
        .into_iter()
        .map(materialize_shader_creation)
        .collect::<Option<Vec<_>>>()
        .ok_or("static shader creation has no exact D3DCompile output receipt")?;

    let input_layout_creations = select_latest_creation_rows(
        state.static_d3d_captures.iter().filter_map(|capture| {
            capture
                .state
                .input_layout_ptr
                .map(|pointer| (pointer, capture.state.capture_sequence))
        }),
        &state.input_layout_creations,
        |row| (row.input_layout_ptr, row.creation_sequence),
    )
    .ok_or("static draw references an input layout without an exact creation receipt")?;
    let blend_state_creations = select_latest_creation_rows(
        state.static_d3d_captures.iter().filter_map(|capture| {
            capture
                .state
                .blend_state_ptr
                .map(|pointer| (pointer, capture.state.capture_sequence))
        }),
        &state.blend_state_creations,
        |row| (row.blend_state_ptr, row.creation_sequence),
    )
    .ok_or("static draw references a blend state without an exact creation receipt")?;
    let depth_stencil_state_creations = select_latest_creation_rows(
        state.static_d3d_captures.iter().filter_map(|capture| {
            capture
                .state
                .depth_stencil_state_ptr
                .map(|pointer| (pointer, capture.state.capture_sequence))
        }),
        &state.depth_stencil_state_creations,
        |row| (row.depth_stencil_state_ptr, row.creation_sequence),
    )
    .ok_or("static draw references a depth/stencil state without an exact creation receipt")?;
    let rasterizer_state_creations = select_latest_creation_rows(
        state.static_d3d_captures.iter().filter_map(|capture| {
            capture
                .state
                .rasterizer_state_ptr
                .map(|pointer| (pointer, capture.state.capture_sequence))
        }),
        &state.rasterizer_state_creations,
        |row| (row.rasterizer_state_ptr, row.creation_sequence),
    )
    .ok_or("static draw references a rasterizer state without an exact creation receipt")?;
    let sampler_state_creations = select_latest_creation_rows(
        state.static_d3d_captures.iter().flat_map(|capture| {
            capture
                .state
                .ordered_sampler_objects
                .iter()
                .map(move |slot| (slot.object_ptr, capture.state.capture_sequence))
        }),
        &state.sampler_state_creations,
        |row| (row.sampler_state_ptr, row.creation_sequence),
    )
    .ok_or("static draw references a sampler state without an exact creation receipt")?;

    let shaders = shader_creations
        .iter()
        .map(|creation| ((creation.shader_ptr, creation.creation_sequence), creation))
        .collect::<BTreeMap<_, _>>();
    let mut reflection_keys = BTreeMap::new();
    for capture in &state.static_d3d_captures {
        for (shader_ptr, stage) in [
            (capture.state.vertex_shader_ptr, RawShaderStage::Vertex),
            (capture.state.pixel_shader_ptr, RawShaderStage::Pixel),
        ] {
            let creation = shaders
                .iter()
                .filter(|((pointer, sequence), creation)| {
                    *pointer == shader_ptr
                        && *sequence <= capture.state.capture_sequence
                        && creation.stage == stage
                })
                .max_by_key(|((_, sequence), _)| *sequence)
                .map(|(_, creation)| *creation)
                .ok_or("static VS/PS lacks a selected exact creation receipt")?;
            reflection_keys.insert(
                (
                    capture.state.shader_program_ptr,
                    creation.shader_ptr,
                    creation.creation_sequence,
                ),
                creation,
            );
        }
    }
    let shader_reflections = reflect_program_shader_pairs(reflection_keys)?;

    Ok(RawStaticCatalogFragment {
        map_shader_program_joins: state.map_shader_program_joins.iter().cloned().collect(),
        render_target_registrations: state
            .render_target_use_observations
            .iter()
            .cloned()
            .collect(),
        texture_vfs_resource_joins: state.texture_vfs_resource_joins.values().cloned().collect(),
        shader_creations,
        shader_reflections,
        input_layout_creations,
        blend_state_creations,
        depth_stencil_state_creations,
        rasterizer_state_creations,
        sampler_state_creations,
        buffer_snapshots: state
            .static_d3d_captures
            .iter()
            .flat_map(|capture| capture.buffer_snapshots.iter().cloned())
            .collect(),
        texture_resource_snapshots: state
            .static_d3d_captures
            .iter()
            .flat_map(|capture| capture.texture_resource_snapshots.iter().cloned())
            .collect(),
        shader_resource_view_snapshots: state
            .static_d3d_captures
            .iter()
            .flat_map(|capture| capture.shader_resource_view_snapshots.iter().cloned())
            .collect(),
        render_target_view_snapshots: state
            .static_d3d_captures
            .iter()
            .flat_map(|capture| capture.render_target_view_snapshots.iter().cloned())
            .collect(),
        clear_output_view_snapshots: state.clear_output_view_snapshots.iter().cloned().collect(),
    })
}

fn query_performance_counter() -> Option<u64> {
    let mut value = 0i64;
    let ok = unsafe { QueryPerformanceCounter(&mut value) } != 0;
    (ok && value > 0).then_some(value as u64)
}

fn query_performance_frequency() -> Option<u64> {
    let mut value = 0i64;
    let ok = unsafe { QueryPerformanceFrequency(&mut value) } != 0;
    (ok && value > 0).then_some(value as u64)
}

fn capture_present_qpc_receipt() -> Result<PresentationQpcReceipt, String> {
    let ticks =
        query_performance_counter().ok_or("QueryPerformanceCounter failed at PresentEnd")?;
    let game_hour_raw = capture_present_game_hour_raw()?;
    let epoch = EARLY_CAPTURE_EPOCH
        .get()
        .ok_or("early capture epoch state is unavailable at PresentEnd")?
        .lock()
        .map_err(|_| "early capture epoch state poisoned")?;
    let process = epoch
        .process
        .as_ref()
        .ok_or("early capture process epoch is absent at PresentEnd")?;
    if epoch.qpc_frequency_hz == 0 || epoch.hook_epoch_nonce == 0 {
        return Err("PresentEnd QPC epoch binding is incomplete".into());
    }
    let global_order = NEXT_PRESENT_GLOBAL_ORDER
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
            current.checked_add(1)
        })
        .map_err(|_| "PresentEnd global-order ordinal overflowed")?;
    Ok(PresentationQpcReceipt {
        global_order,
        ticks,
        frequency_hz: epoch.qpc_frequency_hz,
        process_start_nonce: process.process_start_nonce,
        hook_epoch_nonce: epoch.hook_epoch_nonce,
        game_hour_raw,
    })
}

fn capture_present_game_hour_raw() -> Result<i32, String> {
    let root_slot = PRESENT_GAME_STATE_ROOT_SLOT.load(Ordering::Acquire);
    if root_slot == 0 {
        return Err("Present game-hour root slot is unbound".into());
    }
    let game_state_before = read_usize(root_slot)
        .filter(|pointer| *pointer != 0)
        .ok_or("Present game-hour CGameState pointee is null or unreadable")?;
    let hour_address = game_state_before
        .checked_add(compact_contract::build_profile::CGAMESTATE_CURRENT_HOUR)
        .ok_or("Present game-hour field address overflowed")?;
    let hour_before = read_i32(hour_address).ok_or("Present game-hour field is unreadable")?;
    let hour_after = read_i32(hour_address).ok_or("Present game-hour field is unreadable")?;
    let game_state_after = read_usize(root_slot)
        .filter(|pointer| *pointer != 0)
        .ok_or("Present game-hour CGameState pointee is null or unreadable")?;
    if game_state_before != game_state_after || hour_before != hour_after {
        return Err("Present game-hour root or value changed during stable read".into());
    }
    compact_contract::calendar::hour_index_from_raw(hour_before)
        .ok_or("Present game-hour value is outside the canonical calendar range")?;
    Ok(hour_before)
}

fn initialize_early_capture_epoch(all_targets: &[usize]) -> Result<(), String> {
    let process = current_process_epoch();
    let observer_installed_qpc =
        query_performance_counter().ok_or("QueryPerformanceCounter failed at hook readiness")?;
    let qpc_frequency_hz = query_performance_frequency()
        .ok_or("QueryPerformanceFrequency failed at hook readiness")?;
    if process.process_id == 0 || process.process_start_nonce == 0 {
        return Err("process epoch is incomplete at hook readiness".into());
    }
    let hook_epoch_nonce = observer_installed_qpc
        ^ process.process_start_nonce
        ^ u64::from(process.process_id).rotate_left(17);
    if hook_epoch_nonce == 0 {
        return Err("hook epoch nonce is zero".into());
    }
    let mut hasher = Sha256::new();
    hasher.update(b"hoi4-a729-early-render-hook-set-v1");
    hasher.update(process.process_id.to_le_bytes());
    hasher.update(process.process_start_nonce.to_le_bytes());
    hasher.update(observer_installed_qpc.to_le_bytes());
    hasher.update(qpc_frequency_hz.to_le_bytes());
    hasher.update(hook_epoch_nonce.to_le_bytes());
    hasher.update(process.executable_sha256.as_bytes());
    for target in all_targets {
        hasher.update((*target as u64).to_le_bytes());
    }
    let hook_set_receipt_sha256 = format!("{:x}", hasher.finalize());
    let hook_ready_sequence =
        next_catalog_observation_sequence().ok_or("early capture hook-ready sequence exhausted")?;
    *EARLY_CAPTURE_EPOCH
        .get_or_init(|| Mutex::new(EarlyCaptureEpochState::default()))
        .lock()
        .map_err(|_| "early capture epoch state poisoned")? = EarlyCaptureEpochState {
        process: Some(process),
        observer_installed_qpc,
        qpc_frequency_hz,
        hook_ready_sequence,
        d3d_device_created_qpc: 0,
        game_device_count: 0,
        first_creation_sequence: 0,
        hook_epoch_nonce,
        hook_set_receipt_sha256,
        capture_receipt_sha256: None,
        // No collector probe device is created. Hardware COM targets are
        // derived only from the successful game device inside its detour.
        excluded_probe_device_ptr: 0,
        staged_hook_generation: 0,
        staged_hook_count: 0,
        staged_hook_set_sha256: String::new(),
        diagnostic_stage_probe: false,
        staged_hook_group_mask: 0,
        game_device_ptr: 0,
        immediate_context_ptr: 0,
    };
    Ok(())
}

fn mark_first_creation_sequence(sequence: u64) {
    let stage = HOOK_STAGE
        .get()
        .and_then(|value| value.lock().ok().map(|value| *value));
    if stage == Some(HookStage::Installing) {
        let _ = PENDING_STAGE_FIRST_CREATION.fetch_update(
            Ordering::AcqRel,
            Ordering::Acquire,
            |current| {
                Some(if current == 0 {
                    sequence
                } else {
                    current.min(sequence)
                })
            },
        );
        return;
    }
    if !stage.is_some_and(HookStage::admits_first_creation) {
        // D3DCompile is deliberately available as a bootstrap observer, but
        // pre-device compiler output cannot prove the post-device hook set.
        return;
    }
    if let Some(mut epoch) = EARLY_CAPTURE_EPOCH
        .get()
        .and_then(|value| value.lock().ok())
    {
        if epoch.first_creation_sequence == 0 {
            epoch.first_creation_sequence = sequence;
        }
    }
}

fn runtime_dlc_raw_name_roster_sha256(raw_names: &[String]) -> Result<String, String> {
    if raw_names.len() > MAX_RUNTIME_DLC_NAMES as usize {
        return Err("runtime DLC roster count is outside the proven bound".into());
    }
    let mut seen = BTreeSet::new();
    let mut hasher = Sha256::new();
    hasher.update(b"hoi4-a729-runtime-dlc-roster-v1\0");
    hasher.update((raw_names.len() as u32).to_le_bytes());
    for (ordinal, name) in raw_names.iter().enumerate() {
        if name.is_empty()
            || name.len() > MAX_RUNTIME_DLC_NAME_BYTES
            || name
                .bytes()
                .any(|byte| byte == 0 || byte.is_ascii_control())
            || !seen.insert(name.as_bytes().to_vec())
        {
            return Err("runtime DLC roster contains an invalid or duplicate raw name".into());
        }
        hasher.update((ordinal as u32).to_le_bytes());
        hasher.update((name.len() as u32).to_le_bytes());
        hasher.update(name.as_bytes());
    }
    Ok(format!("{:x}", hasher.finalize()))
}

unsafe fn release_runtime_dlc_string_vector(
    base: usize,
    vector: &mut RuntimeDlcStringVector,
) -> Result<(), String> {
    if vector.data == 0 {
        return (vector.count == 0)
            .then_some(())
            .ok_or_else(|| "runtime DLC vector has count without storage".into());
    }
    let allocator_vtable = read_usize(vector.allocator)
        .filter(|value| *value != 0)
        .ok_or("runtime DLC vector allocator vtable is null or unreadable")?;
    let free_target = read_usize(
        allocator_vtable
            .checked_add(0x10)
            .ok_or("runtime DLC allocator free slot overflowed")?,
    )
    .filter(|value| *value != 0)
    .ok_or("runtime DLC allocator free method is null or unreadable")?;
    type DestroyStringsFn = unsafe extern "C" fn(*mut RuntimeDlcStringVector, u64, u64);
    type FreeVectorFn = unsafe extern "C" fn(usize, usize);
    let destroy: DestroyStringsFn = unsafe {
        mem::transmute(
            base.checked_add(RUNTIME_DLC_STRING_VECTOR_TEARDOWN_RVA)
                .ok_or("runtime DLC string teardown address overflowed")?,
        )
    };
    let free: FreeVectorFn = unsafe { mem::transmute(free_target) };
    let data = vector.data;
    let count = vector.count;
    unsafe {
        destroy(vector, 0, u64::from(count));
        free(vector.allocator, data);
    }
    vector.data = 0;
    vector.capacity = 0;
    vector.count = 0;
    Ok(())
}

unsafe fn sample_runtime_dlc_names(base: usize) -> Result<(usize, Vec<String>), String> {
    let allocator = read_usize(
        base.checked_add(RUNTIME_DLC_VECTOR_ALLOCATOR_RVA)
            .ok_or("runtime DLC allocator global address overflowed")?,
    )
    .filter(|value| *value != 0)
    .ok_or("runtime DLC vector allocator is null or unreadable")?;
    let allocator_aux = read_usize(
        base.checked_add(RUNTIME_DLC_VECTOR_ALLOCATOR_RVA + mem::size_of::<usize>())
            .ok_or("runtime DLC allocator auxiliary global address overflowed")?,
    )
    .ok_or("runtime DLC vector allocator auxiliary state is unreadable")?;
    let mut vector = RuntimeDlcStringVector {
        data: 0,
        capacity: 0,
        count: 0,
        allocator,
        allocator_aux,
    };
    type ManagerGetterFn = unsafe extern "C" fn() -> usize;
    type EnumeratorFn =
        unsafe extern "C" fn(usize, *mut RuntimeDlcStringVector) -> *mut RuntimeDlcStringVector;
    let manager_getter: ManagerGetterFn = unsafe {
        mem::transmute(
            base.checked_add(RUNTIME_DLC_MANAGER_GETTER_RVA)
                .ok_or("runtime DLC manager getter address overflowed")?,
        )
    };
    let enumerator: EnumeratorFn = unsafe {
        mem::transmute(
            base.checked_add(RUNTIME_ACTIVE_DLC_ENUMERATOR_RVA)
                .ok_or("runtime DLC enumerator address overflowed")?,
        )
    };
    let manager = unsafe { manager_getter() };
    if manager == 0 {
        return Err("runtime DLC manager getter returned null".into());
    }
    let returned = unsafe { enumerator(manager, &mut vector) };
    let parsed = (|| {
        if !ptr::eq(returned, &mut vector)
            || vector.count > MAX_RUNTIME_DLC_NAMES
            || vector.capacity < vector.count
            || vector.capacity > MAX_RUNTIME_DLC_NAMES
            || (vector.count != 0 && vector.data == 0)
        {
            return Err("runtime DLC enumerator returned an invalid vector shape".into());
        }
        let mut names = Vec::with_capacity(vector.count as usize);
        let mut seen = BTreeSet::new();
        for ordinal in 0..vector.count as usize {
            let row = vector
                .data
                .checked_add(
                    ordinal
                        .checked_mul(0x20)
                        .ok_or("runtime DLC vector row offset overflowed")?,
                )
                .ok_or("runtime DLC vector row address overflowed")?;
            let name = read_msvc_string(row, MAX_RUNTIME_DLC_NAME_BYTES)
                .ok_or("runtime DLC vector contains an invalid or unstable MSVC string")?;
            if name.bytes().any(|byte| byte.is_ascii_control())
                || !seen.insert(name.as_bytes().to_vec())
            {
                return Err("runtime DLC vector contains a control byte or duplicate name".into());
            }
            names.push(name);
        }
        Ok(names)
    })();
    let released = unsafe { release_runtime_dlc_string_vector(base, &mut vector) };
    match (parsed, released) {
        (Ok(names), Ok(())) => Ok((manager, names)),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(error)) => Err(error),
        (Err(parse), Err(release)) => Err(format!("{parse}; {release}")),
    }
}

pub fn runtime_dlc_roster_observation() -> Result<RuntimeDlcRosterObservation, String> {
    let stage = HOOK_STAGE
        .get()
        .ok_or("hook stage state is unavailable")?
        .lock()
        .map_err(|_| "hook stage state poisoned")?;
    if *stage != HookStage::Ready {
        return Err("runtime DLC roster requested before staged hooks were ready".into());
    }
    drop(stage);
    let epoch = EARLY_CAPTURE_EPOCH
        .get()
        .ok_or("early capture epoch state is unavailable")?
        .lock()
        .map_err(|_| "early capture epoch state poisoned")?
        .clone();
    let process = epoch
        .process
        .ok_or("early capture process epoch is absent")?;
    if epoch.staged_hook_generation != 1
        || epoch.game_device_ptr == 0
        || epoch.first_creation_sequence == 0
    {
        return Err("runtime DLC roster requested before FIRST was complete".into());
    }
    let mut cached = RUNTIME_DLC_ROSTER
        .get_or_init(|| Mutex::new(None))
        .lock()
        .map_err(|_| "runtime DLC roster state poisoned")?;
    if let Some(observation) = cached.as_ref() {
        if observation.process_id == process.process_id
            && observation.process_start_nonce == process.process_start_nonce
            && observation.hook_epoch_nonce == epoch.hook_epoch_nonce
            && observation.staged_hook_generation == epoch.staged_hook_generation
            && observation.game_device_ptr == epoch.game_device_ptr
        {
            return Ok(observation.clone());
        }
        return Err("cached runtime DLC roster belongs to a foreign hook epoch".into());
    }

    let base = MODULE_BASE.load(Ordering::Acquire);
    if base == 0 {
        return Err("module base is absent at runtime DLC enumeration".into());
    }
    let first = unsafe { sample_runtime_dlc_names(base) }?;
    let second = unsafe { sample_runtime_dlc_names(base) }?;
    if first != second {
        return Err(
            "runtime DLC roster was not double-stable in manager identity, order, or bytes".into(),
        );
    }
    let raw_name_roster_sha256 = runtime_dlc_raw_name_roster_sha256(&first.1)?;
    let observation = RuntimeDlcRosterObservation {
        process_id: process.process_id,
        process_start_nonce: process.process_start_nonce,
        hook_epoch_nonce: epoch.hook_epoch_nonce,
        staged_hook_generation: epoch.staged_hook_generation,
        game_device_ptr: epoch.game_device_ptr,
        manager_getter_rva: RUNTIME_DLC_MANAGER_GETTER_RVA,
        active_dlc_enumerator_rva: RUNTIME_ACTIVE_DLC_ENUMERATOR_RVA,
        string_vector_teardown_rva: RUNTIME_DLC_STRING_VECTOR_TEARDOWN_RVA,
        roster_count: first.1.len() as u32,
        raw_names: first.1,
        raw_name_roster_sha256,
        sample_count: 2,
        double_stable: true,
    };
    *cached = Some(observation.clone());
    Ok(observation)
}

fn current_capture_launch_epoch() -> Option<RawCaptureLaunchEpoch> {
    let epoch = EARLY_CAPTURE_EPOCH.get()?.lock().ok()?.clone();
    let process = epoch.process?;
    let capture_receipt_sha256 = epoch.capture_receipt_sha256?;
    if epoch.observer_installed_qpc == 0
        || epoch.qpc_frequency_hz == 0
        || epoch.d3d_device_created_qpc < epoch.observer_installed_qpc
        || epoch.first_creation_sequence == 0
        || epoch.hook_epoch_nonce == 0
        || epoch.hook_ready_sequence == 0
        || epoch.hook_set_receipt_sha256.len() != 64
    {
        return None;
    }
    Some(RawCaptureLaunchEpoch {
        process_id: process.process_id,
        process_start_nonce: process.process_start_nonce,
        observer_installed_qpc: epoch.observer_installed_qpc,
        d3d_device_created_qpc: epoch.d3d_device_created_qpc,
        first_creation_sequence: epoch.first_creation_sequence,
        hook_epoch_nonce: epoch.hook_epoch_nonce,
        hook_set_receipt_sha256: epoch.hook_set_receipt_sha256,
        capture_receipt_sha256,
    })
}

pub fn commit_capture_receipt(
    process_id: u32,
    process_start_nonce: u64,
    hook_epoch_nonce: u64,
    capture_receipt_sha256: &str,
) -> Result<(), String> {
    if capture_receipt_sha256.len() != 64
        || capture_receipt_sha256
            .bytes()
            .any(|byte| !byte.is_ascii_digit() && !(b'a'..=b'f').contains(&byte))
    {
        return Err("capture receipt SHA-256 is not canonical lowercase hex".into());
    }
    let mut epoch = EARLY_CAPTURE_EPOCH
        .get()
        .ok_or("early capture epoch state is unavailable")?
        .lock()
        .map_err(|_| "early capture epoch state poisoned")?;
    if epoch.diagnostic_stage_probe {
        return Err("diagnostic staged-hook probes cannot commit capture receipts".into());
    }
    let process = epoch
        .process
        .as_ref()
        .ok_or("early capture process epoch is absent")?;
    if process.process_id != process_id
        || process.process_start_nonce != process_start_nonce
        || epoch.hook_epoch_nonce != hook_epoch_nonce
        || epoch.d3d_device_created_qpc == 0
        || epoch.first_creation_sequence == 0
    {
        return Err("capture receipt belongs to a foreign, late, or incomplete hook epoch".into());
    }
    match &epoch.capture_receipt_sha256 {
        Some(existing) if existing != capture_receipt_sha256 => {
            return Err("capture receipt epoch was already committed to a different digest".into())
        }
        Some(_) => {}
        None => epoch.capture_receipt_sha256 = Some(capture_receipt_sha256.to_owned()),
    }
    Ok(())
}

/// Accept one H4CT-authenticated G2 arm after the exact capture receipt has
/// been committed. The existing production hook graph remains the only D3D
/// owner; no auxiliary capture DLL participates.
pub fn configure_g2_capture_expected(expected: bool) -> Result<(), String> {
    g2_capture::configure_expected(expected)
}

pub fn arm_g2_capture(
    arm: compact_contract::early_render_control::EarlyG2CaptureArmV1,
) -> Result<(), String> {
    g2_capture::arm(arm)
}

pub fn early_capture_readiness() -> Result<EarlyCaptureReadiness, String> {
    if !INSTALLED.load(Ordering::Acquire) {
        return Err("a729 render hooks are not installed".into());
    }
    let epoch = EARLY_CAPTURE_EPOCH
        .get()
        .ok_or("early capture epoch state is unavailable")?
        .lock()
        .map_err(|_| "early capture epoch state poisoned")?
        .clone();
    let process = epoch
        .process
        .ok_or("early capture process epoch is absent")?;
    let bootstrap_hook_count = BOOTSTRAP_HOOK_TARGETS
        .get()
        .and_then(|value| value.lock().ok().map(|value| value.len() as u32))
        .unwrap_or(0);
    let staged_hook_count = STAGED_HOOK_TARGETS
        .get()
        .and_then(|value| value.lock().ok().map(|value| value.len() as u32))
        .unwrap_or(0);
    let creation_hook_count = bootstrap_hook_count.saturating_add(staged_hook_count);
    if epoch.observer_installed_qpc == 0
        || epoch.qpc_frequency_hz == 0
        || epoch.hook_epoch_nonce == 0
        || epoch.hook_set_receipt_sha256.len() != 64
        || bootstrap_hook_count != BOOTSTRAP_HOOK_COUNT as u32
    {
        return Err("early capture hook readiness is incomplete".into());
    }
    Ok(EarlyCaptureReadiness {
        process,
        observer_installed_qpc: epoch.observer_installed_qpc,
        qpc_frequency_hz: epoch.qpc_frequency_hz,
        hook_ready_sequence: epoch.hook_ready_sequence,
        d3d_device_created_qpc: (epoch.d3d_device_created_qpc != 0)
            .then_some(epoch.d3d_device_created_qpc),
        game_device_count: epoch.game_device_count,
        first_creation_sequence: (epoch.first_creation_sequence != 0)
            .then_some(epoch.first_creation_sequence),
        hook_epoch_nonce: epoch.hook_epoch_nonce,
        hook_set_receipt_sha256: epoch.hook_set_receipt_sha256,
        creation_hook_count,
        excluded_probe_device_ptr: epoch.excluded_probe_device_ptr,
        staged_hook_generation: epoch.staged_hook_generation,
        staged_hook_count: epoch.staged_hook_count,
        staged_hook_set_sha256: epoch.staged_hook_set_sha256,
        diagnostic_stage_probe: epoch.diagnostic_stage_probe,
        staged_hook_group_mask: epoch.staged_hook_group_mask,
        game_device_ptr: epoch.game_device_ptr,
        immediate_context_ptr: epoch.immediate_context_ptr,
    })
}

pub fn early_capture_liveness_sample() -> Result<EarlyCaptureLivenessSample, String> {
    let readiness = early_capture_readiness()?;
    let sampled_qpc = query_performance_counter()
        .ok_or("QueryPerformanceCounter failed for early capture liveness sample")?;
    if sampled_qpc < readiness.observer_installed_qpc
        || readiness
            .d3d_device_created_qpc
            .is_some_and(|creation_qpc| sampled_qpc < creation_qpc)
    {
        return Err("early capture liveness sample QPC predates its hook epoch".into());
    }
    Ok(EarlyCaptureLivenessSample {
        readiness,
        sampled_qpc,
    })
}

#[allow(clippy::too_many_arguments)]
fn capture_runtime_draw_state(
    final_draw_sequence: u64,
    identity: RawDrawIdentity,
    family: RawDrawFamily,
    submit_index: u64,
    draw_order: u32,
    render_context_ptr: usize,
    shader_program_ptr: usize,
    draw_call: RawDrawCall,
) -> Option<CapturedRuntimeDraw> {
    if submit_index == 0 {
        return None;
    }
    let captured = capture_d3d_core(
        final_draw_sequence,
        render_context_ptr,
        shader_program_ptr,
        draw_call,
    )?;
    let core = captured.state;
    {
        let mut hook_state = STATE.get()?.lock().ok()?;
        record_registered_color_target_uses_locked(
            &mut hook_state,
            final_draw_sequence,
            &core.ordered_color_target_view_ptrs,
        )?;
    }
    let state = RawD3dDrawState {
        final_draw_sequence: core.final_draw_sequence,
        identity,
        family,
        submit_index,
        draw_order,
        render_context_ptr: core.render_context_ptr,
        device_context_ptr: core.device_context_ptr,
        shader_program_ptr: core.shader_program_ptr,
        vertex_shader_ptr: core.vertex_shader_ptr,
        pixel_shader_ptr: core.pixel_shader_ptr,
        geometry_shader_ptr: core.geometry_shader_ptr,
        hull_shader_ptr: core.hull_shader_ptr,
        domain_shader_ptr: core.domain_shader_ptr,
        compute_shader_ptr: core.compute_shader_ptr,
        dynamic_shader_linkage_present: core.dynamic_shader_linkage_present,
        input_layout_ptr: core.input_layout_ptr,
        blend_state_ptr: core.blend_state_ptr,
        blend_factor_rgba_f32_bits: core.blend_factor_rgba_f32_bits,
        sample_mask_u32: core.sample_mask_u32,
        depth_stencil_state_ptr: core.depth_stencil_state_ptr,
        stencil_reference_u32: core.stencil_reference_u32,
        rasterizer_state_ptr: core.rasterizer_state_ptr,
        primitive_topology_u32: core.primitive_topology_u32,
        ordered_texture_objects: core.ordered_texture_objects,
        ordered_sampler_objects: core.ordered_sampler_objects,
        ordered_constant_buffer_objects: core.ordered_constant_buffer_objects,
        ordered_vertex_buffer_objects: core.ordered_vertex_buffer_objects,
        index_buffer: core.index_buffer,
        ordered_stream_output_buffer_ptrs: core.ordered_stream_output_buffer_ptrs,
        ordered_color_target_view_ptrs: core.ordered_color_target_view_ptrs,
        depth_stencil_view_ptr: core.depth_stencil_view_ptr,
        ordered_viewports_f32_bits: core.ordered_viewports_f32_bits,
        ordered_scissor_rects_i32: core.ordered_scissor_rects_i32,
        draw_call: core.draw_call,
    };
    Some(CapturedRuntimeDraw {
        state,
        buffer_snapshots: captured.buffer_snapshots,
        texture_resource_snapshots: captured.texture_resource_snapshots,
        shader_resource_view_snapshots: captured.shader_resource_view_snapshots,
        render_target_view_snapshots: captured.render_target_view_snapshots,
    })
}

fn capture_d3d_core(
    final_draw_sequence: u64,
    render_context_ptr: usize,
    shader_program_ptr: usize,
    draw_call: RawDrawCall,
) -> Option<CapturedD3dCore> {
    if final_draw_sequence == 0 || render_context_ptr == 0 || shader_program_ptr == 0 {
        return None;
    }
    // Proven a729 render-context field containing the real
    // ID3D11DeviceContext*. Every observation below is made before the matched
    // final Draw* wrapper calls through the native context vtable.
    let device_context_ptr = read_usize(render_context_ptr.checked_add(0x2a0)?)?;
    if device_context_ptr == 0 {
        return None;
    }

    let mut acquired = ComRefs::default();
    let (vertex_shader_ptr, vertex_linkage) = capture_shader(device_context_ptr, 80)?;
    let (pixel_shader_ptr, pixel_linkage) = capture_shader(device_context_ptr, 78)?;
    acquired.push(vertex_shader_ptr);
    acquired.push(pixel_shader_ptr);
    if vertex_shader_ptr == 0 || pixel_shader_ptr == 0 {
        return None;
    }
    let (geometry_shader_raw, geometry_linkage) = capture_shader(device_context_ptr, 86)?;
    let (hull_shader_raw, hull_linkage) = capture_shader(device_context_ptr, 102)?;
    let (domain_shader_raw, domain_linkage) = capture_shader(device_context_ptr, 106)?;
    let (compute_shader_raw, compute_linkage) = capture_shader(device_context_ptr, 111)?;
    acquired.extend([
        geometry_shader_raw,
        hull_shader_raw,
        domain_shader_raw,
        compute_shader_raw,
    ]);

    let input_layout_raw = capture_single_object(device_context_ptr, 82)?;
    acquired.push(input_layout_raw);
    let ordered_vertex_buffer_objects = capture_vertex_buffers(device_context_ptr)?;
    acquired.extend(
        ordered_vertex_buffer_objects
            .iter()
            .map(|value| value.buffer_ptr),
    );
    let index_buffer = capture_index_buffer(device_context_ptr)?;
    if let Some(value) = &index_buffer {
        acquired.push(value.buffer_ptr);
    }
    let primitive_topology_u32 = capture_primitive_topology(device_context_ptr)?;
    let blend = capture_blend_state(device_context_ptr)?;
    let depth = capture_depth_stencil_state(device_context_ptr)?;
    let rasterizer_raw = capture_single_object(device_context_ptr, 98)?;
    acquired.extend([blend.state_ptr, depth.state_ptr, rasterizer_raw]);

    let stages = [
        (RawShaderStage::Vertex, 88usize, 89usize, 76usize),
        (RawShaderStage::Pixel, 77, 79, 81),
        (RawShaderStage::Geometry, 91, 92, 85),
        (RawShaderStage::Hull, 101, 103, 104),
        (RawShaderStage::Domain, 105, 107, 108),
        (RawShaderStage::Compute, 109, 112, 113),
    ];
    let mut ordered_texture_objects = Vec::new();
    let mut ordered_sampler_objects = Vec::new();
    let mut ordered_constant_buffer_objects = Vec::new();
    for (stage, srv_method, sampler_method, constant_method) in stages {
        ordered_texture_objects.extend(capture_textures(device_context_ptr, stage, srv_method)?);
        let samplers = capture_stage_objects(
            device_context_ptr,
            stage,
            sampler_method,
            MAX_D3D_SAMPLER_SLOTS,
        )?;
        acquired.extend(samplers.iter().map(|value| value.object_ptr));
        ordered_sampler_objects.extend(samplers);
        let constants = capture_stage_objects(
            device_context_ptr,
            stage,
            constant_method,
            MAX_D3D_CONSTANT_BUFFER_SLOTS,
        )?;
        acquired.extend(constants.iter().map(|value| value.object_ptr));
        ordered_constant_buffer_objects.extend(constants);
    }
    let ordered_stream_output_buffer_ptrs = capture_stream_output(device_context_ptr)?;
    acquired.extend(ordered_stream_output_buffer_ptrs.iter().copied());
    let (ordered_color_target_view_ptrs, depth_stencil_view_raw) =
        capture_render_targets(device_context_ptr)?;
    acquired.extend(ordered_color_target_view_ptrs.iter().copied());
    acquired.push(depth_stencil_view_raw);
    let ordered_viewports_f32_bits = capture_viewports(device_context_ptr)?;
    let ordered_scissor_rects_i32 = capture_scissors(device_context_ptr)?;

    let state = NeutralD3dDrawState {
        final_draw_sequence,
        render_context_ptr,
        device_context_ptr,
        shader_program_ptr,
        vertex_shader_ptr,
        pixel_shader_ptr,
        geometry_shader_ptr: nonzero(geometry_shader_raw),
        hull_shader_ptr: nonzero(hull_shader_raw),
        domain_shader_ptr: nonzero(domain_shader_raw),
        compute_shader_ptr: nonzero(compute_shader_raw),
        dynamic_shader_linkage_present: vertex_linkage
            || pixel_linkage
            || geometry_linkage
            || hull_linkage
            || domain_linkage
            || compute_linkage,
        input_layout_ptr: nonzero(input_layout_raw),
        blend_state_ptr: nonzero(blend.state_ptr),
        blend_factor_rgba_f32_bits: blend.factor_bits,
        sample_mask_u32: blend.sample_mask,
        depth_stencil_state_ptr: nonzero(depth.state_ptr),
        stencil_reference_u32: depth.stencil_reference,
        rasterizer_state_ptr: nonzero(rasterizer_raw),
        primitive_topology_u32,
        ordered_texture_objects,
        ordered_sampler_objects,
        ordered_constant_buffer_objects,
        ordered_vertex_buffer_objects,
        index_buffer,
        ordered_stream_output_buffer_ptrs,
        ordered_color_target_view_ptrs,
        depth_stencil_view_ptr: nonzero(depth_stencil_view_raw),
        ordered_viewports_f32_bits,
        ordered_scissor_rects_i32,
        draw_call,
    };

    let buffer_snapshots = capture_draw_buffers(&state)?;
    let (texture_resource_snapshots, shader_resource_view_snapshots) =
        capture_draw_textures(&state)?;
    let render_target_view_snapshots = capture_draw_output_views(&state)?;
    Some(CapturedD3dCore {
        state,
        buffer_snapshots,
        texture_resource_snapshots,
        shader_resource_view_snapshots,
        render_target_view_snapshots,
    })
}

fn record_static_d3d_capture(
    sequence: Option<u64>,
    render_context_ptr: usize,
    shader_program_ptr: usize,
    draw_call: RawStaticDrawCall,
) {
    let Some(capture_sequence) = sequence else {
        reject_render_order_event();
        return;
    };
    let Some(captured) = capture_static_d3d_state(
        capture_sequence,
        render_context_ptr,
        shader_program_ptr,
        draw_call,
    ) else {
        reject_render_order_event();
        return;
    };
    let Some(mut state) = STATE.get().and_then(|value| value.lock().ok()) else {
        return;
    };
    if state.pending_render_frame.is_none()
        || state.static_d3d_captures.len() >= MAX_RENDER_ORDER_EVENTS
    {
        state.render_order_event_overflows += 1;
        return;
    }
    if record_registered_color_target_uses_locked(
        &mut state,
        capture_sequence,
        &captured.state.ordered_color_target_view_ptrs,
    )
    .is_none()
    {
        state.render_order_event_overflows += 1;
        return;
    }
    state.static_d3d_captures.push_back(captured);
}

fn capture_static_d3d_state(
    capture_sequence: u64,
    render_context_ptr: usize,
    shader_program_ptr: usize,
    draw_call: RawStaticDrawCall,
) -> Option<RawStaticD3dCapture> {
    let runtime_call = match draw_call {
        RawStaticDrawCall::Draw {
            vertex_count,
            start_vertex,
        } => RawDrawCall::Draw {
            vertex_count,
            start_vertex,
        },
        RawStaticDrawCall::DrawIndexed {
            index_count,
            start_index,
            base_vertex,
        } => RawDrawCall::DrawIndexed {
            index_count,
            start_index,
            base_vertex,
        },
        RawStaticDrawCall::DrawInstanced {
            vertex_count_per_instance,
            instance_count,
            start_vertex,
            start_instance,
        } => RawDrawCall::DrawInstanced {
            vertex_count_per_instance,
            instance_count,
            start_vertex,
            start_instance,
        },
        RawStaticDrawCall::DrawIndexedInstanced {
            index_count_per_instance,
            instance_count,
            start_index,
            base_vertex,
            start_instance,
        } => RawDrawCall::DrawIndexedInstanced {
            index_count_per_instance,
            instance_count,
            start_index,
            base_vertex,
            start_instance,
        },
    };
    let captured = capture_d3d_core(
        capture_sequence,
        render_context_ptr,
        shader_program_ptr,
        runtime_call,
    )?;
    let core = captured.state;
    if core.geometry_shader_ptr.is_some()
        || core.hull_shader_ptr.is_some()
        || core.domain_shader_ptr.is_some()
        || core.compute_shader_ptr.is_some()
        || core.dynamic_shader_linkage_present
    {
        return None;
    }
    let ordered_stream_output_buffers = STATE
        .get()?
        .lock()
        .ok()?
        .stream_output_bindings
        .get(&core.device_context_ptr)
        .cloned()
        .unwrap_or_default();
    if ordered_stream_output_buffers
        .iter()
        .map(|row| row.buffer_ptr)
        .collect::<Vec<_>>()
        != core.ordered_stream_output_buffer_ptrs
    {
        return None;
    }
    let state = RawStaticD3dDrawState {
        capture_sequence,
        shader_program_ptr: core.shader_program_ptr,
        render_context_ptr: core.render_context_ptr,
        device_context_ptr: core.device_context_ptr,
        vertex_shader_ptr: core.vertex_shader_ptr,
        pixel_shader_ptr: core.pixel_shader_ptr,
        geometry_shader_ptr: None,
        hull_shader_ptr: None,
        domain_shader_ptr: None,
        compute_shader_ptr: None,
        geometry_entry_token: None,
        hull_entry_token: None,
        domain_entry_token: None,
        compute_entry_token: None,
        dynamic_shader_linkage_present: false,
        input_layout_ptr: core.input_layout_ptr,
        blend_state_ptr: core.blend_state_ptr,
        blend_factor_rgba_f32_bits: core.blend_factor_rgba_f32_bits,
        sample_mask_u32: core.sample_mask_u32,
        depth_stencil_state_ptr: core.depth_stencil_state_ptr,
        stencil_reference_u32: core.stencil_reference_u32,
        rasterizer_state_ptr: core.rasterizer_state_ptr,
        primitive_topology_u32: core.primitive_topology_u32,
        ordered_texture_objects: core.ordered_texture_objects,
        ordered_sampler_objects: core.ordered_sampler_objects,
        ordered_constant_buffer_objects: core.ordered_constant_buffer_objects,
        ordered_vertex_buffer_objects: core.ordered_vertex_buffer_objects,
        index_buffer: core.index_buffer,
        ordered_stream_output_buffers,
        ordered_color_target_view_ptrs: core.ordered_color_target_view_ptrs,
        depth_stencil_view_ptr: core.depth_stencil_view_ptr,
        ordered_viewports_f32_bits: core.ordered_viewports_f32_bits,
        ordered_scissor_rects_i32: core.ordered_scissor_rects_i32,
        draw_call,
        ordered_texture_semantics: Vec::new(),
        ordered_sampler_semantics: Vec::new(),
        ordered_constant_buffer_semantics: Vec::new(),
        ordered_color_targets: Vec::new(),
        depth_target: None,
        ordered_content: Vec::new(),
    };
    Some(RawStaticD3dCapture {
        state,
        buffer_snapshots: captured.buffer_snapshots,
        texture_resource_snapshots: captured.texture_resource_snapshots,
        shader_resource_view_snapshots: captured.shader_resource_view_snapshots,
        render_target_view_snapshots: captured.render_target_view_snapshots,
    })
}

fn capture_draw_textures(
    state: &NeutralD3dDrawState,
) -> Option<(
    Vec<RawTextureResourceSnapshot>,
    Vec<RawShaderResourceViewSnapshot>,
)> {
    let mut resources = BTreeMap::new();
    let mut views = Vec::with_capacity(state.ordered_texture_objects.len());
    for binding in &state.ordered_texture_objects {
        if !created_object_precedes(
            binding.shader_resource_view_ptr,
            CreatedD3dObjectKind::ShaderResourceView,
            state.final_draw_sequence,
        ) || !resource_creation_precedes(binding.resource_ptr, state.final_draw_sequence)
        {
            return None;
        }
        let (resource_ptr, desc) = capture_view_desc(
            binding.shader_resource_view_ptr,
            CreatedD3dObjectKind::ShaderResourceView,
        )?;
        if resource_ptr != binding.resource_ptr {
            return None;
        }
        views.push(RawShaderResourceViewSnapshot {
            snapshot_sequence: state.final_draw_sequence,
            shader_resource_view_ptr: binding.shader_resource_view_ptr,
            resource_ptr,
            desc,
        });
        if !resources.contains_key(&resource_ptr) {
            resources.insert(
                resource_ptr,
                capture_texture_snapshot(
                    state.device_context_ptr,
                    resource_ptr,
                    state.final_draw_sequence,
                )?,
            );
        }
    }
    Some((resources.into_values().collect(), views))
}

fn capture_draw_output_views(
    state: &NeutralD3dDrawState,
) -> Option<Vec<RawRenderTargetViewSnapshot>> {
    let mut output = Vec::new();
    for &view in &state.ordered_color_target_view_ptrs {
        output.push(capture_output_view_snapshot(
            view,
            RawOutputViewKind::Color,
            CreatedD3dObjectKind::RenderTargetView,
            state.final_draw_sequence,
        )?);
    }
    if let Some(view) = state.depth_stencil_view_ptr {
        output.push(capture_output_view_snapshot(
            view,
            RawOutputViewKind::DepthStencil,
            CreatedD3dObjectKind::DepthStencilView,
            state.final_draw_sequence,
        )?);
    }
    Some(output)
}

fn capture_output_view_snapshot(
    view: usize,
    view_kind: RawOutputViewKind,
    created_kind: CreatedD3dObjectKind,
    snapshot_sequence: u64,
) -> Option<RawRenderTargetViewSnapshot> {
    if !created_object_precedes(view, created_kind, snapshot_sequence) {
        return None;
    }
    let (resource_ptr, desc) = capture_view_desc(view, created_kind)?;
    if !resource_creation_precedes(resource_ptr, snapshot_sequence) {
        return None;
    }
    let meta = capture_texture_meta(resource_ptr)?;
    Some(RawRenderTargetViewSnapshot {
        snapshot_sequence,
        view_ptr: view,
        resource_ptr,
        view_kind,
        desc,
        resource_dimension_u32: meta.dimension,
        resource_width: meta.width,
        resource_height: meta.height,
        resource_depth: meta.depth,
        resource_mip_levels: meta.mip_levels,
        resource_array_size: meta.array_size,
        resource_sample_count: meta.sample_count,
        resource_sample_quality: meta.sample_quality,
    })
}

fn capture_view_desc(
    view: usize,
    kind: CreatedD3dObjectKind,
) -> Option<(usize, RawResourceViewDesc)> {
    type GetResourceFn = unsafe extern "system" fn(usize, *mut usize);
    let get_resource: GetResourceFn = unsafe { mem::transmute(com_method(view, 7)?) };
    let mut resource = 0usize;
    unsafe { get_resource(view, &mut resource) };
    if resource == 0 {
        return None;
    }
    let mut acquired = ComRefs::default();
    acquired.push(resource);
    let desc = match kind {
        CreatedD3dObjectKind::ShaderResourceView => {
            type GetDescFn = unsafe extern "system" fn(usize, *mut D3D11_SHADER_RESOURCE_VIEW_DESC);
            let get_desc: GetDescFn = unsafe { mem::transmute(com_method(view, 8)?) };
            let mut raw = unsafe { mem::zeroed::<D3D11_SHADER_RESOURCE_VIEW_DESC>() };
            unsafe { get_desc(view, &mut raw) };
            RawResourceViewDesc {
                dxgi_format_u32: raw.Format,
                view_dimension_u32: raw.ViewDimension,
                flags_u32: 0,
                union_words_u32: view_union_words(
                    (&raw as *const D3D11_SHADER_RESOURCE_VIEW_DESC).cast::<u8>(),
                    8,
                    16,
                ),
            }
        }
        CreatedD3dObjectKind::RenderTargetView => {
            type GetDescFn = unsafe extern "system" fn(usize, *mut D3D11_RENDER_TARGET_VIEW_DESC);
            let get_desc: GetDescFn = unsafe { mem::transmute(com_method(view, 8)?) };
            let mut raw = unsafe { mem::zeroed::<D3D11_RENDER_TARGET_VIEW_DESC>() };
            unsafe { get_desc(view, &mut raw) };
            RawResourceViewDesc {
                dxgi_format_u32: raw.Format,
                view_dimension_u32: raw.ViewDimension,
                flags_u32: 0,
                union_words_u32: view_union_words(
                    (&raw as *const D3D11_RENDER_TARGET_VIEW_DESC).cast::<u8>(),
                    8,
                    12,
                ),
            }
        }
        CreatedD3dObjectKind::DepthStencilView => {
            type GetDescFn = unsafe extern "system" fn(usize, *mut D3D11_DEPTH_STENCIL_VIEW_DESC);
            let get_desc: GetDescFn = unsafe { mem::transmute(com_method(view, 8)?) };
            let mut raw = unsafe { mem::zeroed::<D3D11_DEPTH_STENCIL_VIEW_DESC>() };
            unsafe { get_desc(view, &mut raw) };
            RawResourceViewDesc {
                dxgi_format_u32: raw.Format,
                view_dimension_u32: raw.ViewDimension,
                flags_u32: raw.Flags,
                union_words_u32: view_union_words(
                    (&raw as *const D3D11_DEPTH_STENCIL_VIEW_DESC).cast::<u8>(),
                    12,
                    12,
                ),
            }
        }
        _ => return None,
    };
    Some((resource, desc))
}

fn view_union_words(bytes: *const u8, offset: usize, byte_count: usize) -> [u32; 4] {
    let mut words = [0u32; 4];
    if byte_count > mem::size_of_val(&words) {
        return words;
    }
    unsafe {
        ptr::copy_nonoverlapping(
            bytes.add(offset),
            words.as_mut_ptr().cast::<u8>(),
            byte_count,
        );
    }
    words
}

#[derive(Clone, Copy)]
struct TextureMeta {
    dimension: u32,
    width: u32,
    height: u32,
    depth: u32,
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

fn capture_texture_meta(resource: usize) -> Option<TextureMeta> {
    type GetTypeFn = unsafe extern "system" fn(usize, *mut u32);
    let get_type: GetTypeFn = unsafe { mem::transmute(com_method(resource, 7)?) };
    let mut dimension = 0u32;
    unsafe { get_type(resource, &mut dimension) };
    match dimension {
        2 => {
            type GetDescFn = unsafe extern "system" fn(usize, *mut D3D11_TEXTURE1D_DESC);
            let get_desc: GetDescFn = unsafe { mem::transmute(com_method(resource, 10)?) };
            let mut desc = unsafe { mem::zeroed::<D3D11_TEXTURE1D_DESC>() };
            unsafe { get_desc(resource, &mut desc) };
            Some(TextureMeta {
                dimension,
                width: desc.Width,
                height: 1,
                depth: 1,
                mip_levels: desc.MipLevels,
                array_size: desc.ArraySize,
                format: desc.Format,
                sample_count: 1,
                sample_quality: 0,
                usage: desc.Usage,
                bind_flags: desc.BindFlags,
                cpu_access_flags: desc.CPUAccessFlags,
                misc_flags: desc.MiscFlags,
            })
        }
        3 => {
            type GetDescFn = unsafe extern "system" fn(usize, *mut D3D11_TEXTURE2D_DESC);
            let get_desc: GetDescFn = unsafe { mem::transmute(com_method(resource, 10)?) };
            let mut desc = unsafe { mem::zeroed::<D3D11_TEXTURE2D_DESC>() };
            unsafe { get_desc(resource, &mut desc) };
            Some(TextureMeta {
                dimension,
                width: desc.Width,
                height: desc.Height,
                depth: 1,
                mip_levels: desc.MipLevels,
                array_size: desc.ArraySize,
                format: desc.Format,
                sample_count: desc.SampleDesc.Count,
                sample_quality: desc.SampleDesc.Quality,
                usage: desc.Usage,
                bind_flags: desc.BindFlags,
                cpu_access_flags: desc.CPUAccessFlags,
                misc_flags: desc.MiscFlags,
            })
        }
        4 => {
            type GetDescFn = unsafe extern "system" fn(usize, *mut D3D11_TEXTURE3D_DESC);
            let get_desc: GetDescFn = unsafe { mem::transmute(com_method(resource, 10)?) };
            let mut desc = unsafe { mem::zeroed::<D3D11_TEXTURE3D_DESC>() };
            unsafe { get_desc(resource, &mut desc) };
            Some(TextureMeta {
                dimension,
                width: desc.Width,
                height: desc.Height,
                depth: desc.Depth,
                mip_levels: desc.MipLevels,
                array_size: 1,
                format: desc.Format,
                sample_count: 1,
                sample_quality: 0,
                usage: desc.Usage,
                bind_flags: desc.BindFlags,
                cpu_access_flags: desc.CPUAccessFlags,
                misc_flags: desc.MiscFlags,
            })
        }
        _ => None,
    }
}

fn capture_texture_snapshot(
    device_context: usize,
    resource: usize,
    snapshot_sequence: u64,
) -> Option<RawTextureResourceSnapshot> {
    let meta = capture_texture_meta(resource)?;
    // A multisampled resource cannot be copied to a staging texture without
    // an explicit format-aware resolve. Fail closed instead of manufacturing
    // bytes for a semantically different resource.
    if meta.sample_count != 1 || meta.mip_levels == 0 || meta.array_size == 0 {
        return None;
    }
    let staging = create_staging_texture(resource, meta)?;
    let mut acquired = ComRefs::default();
    acquired.push(staging);
    type CopyResourceFn = unsafe extern "system" fn(usize, usize, usize);
    let copy: CopyResourceFn = unsafe { mem::transmute(com_method(device_context, 47)?) };
    unsafe { copy(device_context, staging, resource) };
    let mut ordered_subresources = Vec::new();
    for array_slice in 0..meta.array_size {
        for mip_level in 0..meta.mip_levels {
            let subresource_index = mip_level + array_slice * meta.mip_levels;
            ordered_subresources.push(map_texture_subresource(
                device_context,
                staging,
                subresource_index,
                mip_level,
                array_slice,
                meta,
            )?);
        }
    }
    Some(RawTextureResourceSnapshot {
        snapshot_sequence,
        resource_ptr: resource,
        dimension_u32: meta.dimension,
        width: meta.width,
        height: meta.height,
        depth: meta.depth,
        mip_levels: meta.mip_levels,
        array_size: meta.array_size,
        dxgi_format_u32: meta.format,
        sample_count: meta.sample_count,
        sample_quality: meta.sample_quality,
        usage_u32: meta.usage,
        bind_flags_u32: meta.bind_flags,
        cpu_access_flags_u32: meta.cpu_access_flags,
        misc_flags_u32: meta.misc_flags,
        content: None,
        ordered_subresources,
    })
}

fn create_staging_texture(resource: usize, meta: TextureMeta) -> Option<usize> {
    type GetDeviceFn = unsafe extern "system" fn(usize, *mut usize);
    let get_device: GetDeviceFn = unsafe { mem::transmute(com_method(resource, 3)?) };
    let mut device = 0usize;
    unsafe { get_device(resource, &mut device) };
    if device == 0 {
        return None;
    }
    let mut acquired = ComRefs::default();
    acquired.push(device);
    let mut staging = 0usize;
    let hr = match meta.dimension {
        2 => {
            type CreateFn = unsafe extern "system" fn(
                usize,
                *const D3D11_TEXTURE1D_DESC,
                *const D3D11_SUBRESOURCE_DATA,
                *mut usize,
            ) -> i32;
            let mut desc = unsafe { mem::zeroed::<D3D11_TEXTURE1D_DESC>() };
            type GetDescFn = unsafe extern "system" fn(usize, *mut D3D11_TEXTURE1D_DESC);
            let get_desc: GetDescFn = unsafe { mem::transmute(com_method(resource, 10)?) };
            unsafe { get_desc(resource, &mut desc) };
            desc.Usage = 3;
            desc.BindFlags = 0;
            desc.CPUAccessFlags = 0x0002_0000;
            desc.MiscFlags = 0;
            let original = ORIGINAL_D3D_CREATE_TEXTURE1D.load(Ordering::Acquire);
            if original == 0 {
                return None;
            }
            let create: CreateFn = unsafe { mem::transmute(original) };
            unsafe { create(device, &desc, ptr::null(), &mut staging) }
        }
        3 => {
            type CreateFn = unsafe extern "system" fn(
                usize,
                *const D3D11_TEXTURE2D_DESC,
                *const D3D11_SUBRESOURCE_DATA,
                *mut usize,
            ) -> i32;
            let mut desc = unsafe { mem::zeroed::<D3D11_TEXTURE2D_DESC>() };
            type GetDescFn = unsafe extern "system" fn(usize, *mut D3D11_TEXTURE2D_DESC);
            let get_desc: GetDescFn = unsafe { mem::transmute(com_method(resource, 10)?) };
            unsafe { get_desc(resource, &mut desc) };
            desc.Usage = 3;
            desc.BindFlags = 0;
            desc.CPUAccessFlags = 0x0002_0000;
            desc.MiscFlags = 0;
            let original = ORIGINAL_D3D_CREATE_TEXTURE2D.load(Ordering::Acquire);
            if original == 0 {
                return None;
            }
            let create: CreateFn = unsafe { mem::transmute(original) };
            unsafe { create(device, &desc, ptr::null(), &mut staging) }
        }
        4 => {
            type CreateFn = unsafe extern "system" fn(
                usize,
                *const D3D11_TEXTURE3D_DESC,
                *const D3D11_SUBRESOURCE_DATA,
                *mut usize,
            ) -> i32;
            let mut desc = unsafe { mem::zeroed::<D3D11_TEXTURE3D_DESC>() };
            type GetDescFn = unsafe extern "system" fn(usize, *mut D3D11_TEXTURE3D_DESC);
            let get_desc: GetDescFn = unsafe { mem::transmute(com_method(resource, 10)?) };
            unsafe { get_desc(resource, &mut desc) };
            desc.Usage = 3;
            desc.BindFlags = 0;
            desc.CPUAccessFlags = 0x0002_0000;
            desc.MiscFlags = 0;
            let original = ORIGINAL_D3D_CREATE_TEXTURE3D.load(Ordering::Acquire);
            if original == 0 {
                return None;
            }
            let create: CreateFn = unsafe { mem::transmute(original) };
            unsafe { create(device, &desc, ptr::null(), &mut staging) }
        }
        _ => return None,
    };
    (hr >= 0 && staging != 0).then_some(staging)
}

fn map_texture_subresource(
    device_context: usize,
    staging: usize,
    subresource_index: u32,
    mip_level: u32,
    array_slice: u32,
    meta: TextureMeta,
) -> Option<RawTextureSubresourceSnapshot> {
    type MapFn =
        unsafe extern "system" fn(usize, usize, u32, u32, u32, *mut D3dMappedSubresource) -> i32;
    type UnmapFn = unsafe extern "system" fn(usize, usize, u32);
    let map: MapFn = unsafe { mem::transmute(com_method(device_context, 14)?) };
    let unmap: UnmapFn = unsafe { mem::transmute(com_method(device_context, 15)?) };
    let mut mapped = D3dMappedSubresource::default();
    if unsafe {
        map(
            device_context,
            staging,
            subresource_index,
            1,
            0,
            &mut mapped,
        )
    } < 0
        || mapped.data.is_null()
        || mapped.row_pitch == 0
    {
        return None;
    }
    let mip_height = (meta.height >> mip_level).max(1);
    let mip_depth = (meta.depth >> mip_level).max(1);
    let rows = if is_block_compressed(meta.format) {
        mip_height.div_ceil(4)
    } else {
        mip_height
    };
    let slice_pitch = if mapped.depth_pitch != 0 {
        mapped.depth_pitch
    } else {
        mapped.row_pitch.checked_mul(rows)?
    };
    let byte_count = usize::try_from(slice_pitch.checked_mul(mip_depth)?).ok()?;
    let bytes = unsafe { slice::from_raw_parts(mapped.data.cast::<u8>(), byte_count) }.to_vec();
    unsafe { unmap(device_context, staging, subresource_index) };
    Some(RawTextureSubresourceSnapshot {
        subresource_index,
        mip_level,
        array_slice,
        row_pitch: mapped.row_pitch,
        slice_pitch,
        payload: RawPayloadSnapshot {
            captured_sha256: format!("{:x}", Sha256::digest(&bytes)),
            captured_bytes: bytes,
            source: RawPayloadSource::RuntimeGenerated {
                reason: "draw_time_d3d11_texture_readback".into(),
            },
        },
    })
}

fn is_block_compressed(format: u32) -> bool {
    matches!(format, 70..=84 | 94..=99)
}

fn resource_creation_precedes(pointer: usize, observation_sequence: u64) -> bool {
    [
        CreatedD3dObjectKind::Texture1D,
        CreatedD3dObjectKind::Texture2D,
        CreatedD3dObjectKind::Texture3D,
    ]
    .into_iter()
    .any(|kind| created_object_precedes(pointer, kind, observation_sequence))
}

fn capture_draw_buffers(state: &NeutralD3dDrawState) -> Option<Vec<RawBufferSnapshot>> {
    let mut pointers = state
        .ordered_vertex_buffer_objects
        .iter()
        .map(|value| value.buffer_ptr)
        .chain(state.index_buffer.iter().map(|value| value.buffer_ptr))
        .chain(
            state
                .ordered_constant_buffer_objects
                .iter()
                .map(|value| value.object_ptr),
        )
        .collect::<Vec<_>>();
    pointers.sort_unstable();
    pointers.dedup();
    pointers
        .into_iter()
        .map(|buffer| {
            capture_buffer_snapshot(state.device_context_ptr, buffer, state.final_draw_sequence)
        })
        .collect()
}

fn capture_buffer_snapshot(
    device_context: usize,
    buffer: usize,
    snapshot_sequence: u64,
) -> Option<RawBufferSnapshot> {
    if !created_object_precedes(buffer, CreatedD3dObjectKind::Buffer, snapshot_sequence) {
        return None;
    }
    type GetDescFn = unsafe extern "system" fn(usize, *mut D3D11_BUFFER_DESC);
    type GetDeviceFn = unsafe extern "system" fn(usize, *mut usize);
    type CreateBufferFn = unsafe extern "system" fn(
        usize,
        *const D3D11_BUFFER_DESC,
        *const D3D11_SUBRESOURCE_DATA,
        *mut usize,
    ) -> i32;
    type CopyResourceFn = unsafe extern "system" fn(usize, usize, usize);
    type MapFn =
        unsafe extern "system" fn(usize, usize, u32, u32, u32, *mut D3dMappedSubresource) -> i32;
    type UnmapFn = unsafe extern "system" fn(usize, usize, u32);

    let get_desc: GetDescFn = unsafe { mem::transmute(com_method(buffer, 10)?) };
    let mut desc = unsafe { mem::zeroed::<D3D11_BUFFER_DESC>() };
    unsafe { get_desc(buffer, &mut desc) };
    if desc.ByteWidth == 0 {
        return None;
    }

    let get_device: GetDeviceFn = unsafe { mem::transmute(com_method(device_context, 3)?) };
    let mut device = 0usize;
    unsafe { get_device(device_context, &mut device) };
    if device == 0 {
        return None;
    }
    let mut acquired = ComRefs::default();
    acquired.push(device);

    let mut staging_desc = desc;
    // D3D11_USAGE_STAGING, no bind flags, CPU read. Structured/raw identity
    // flags and stride remain unchanged so CopyResource has a compatible
    // destination.
    staging_desc.Usage = 3;
    staging_desc.BindFlags = 0;
    staging_desc.CPUAccessFlags = 0x0002_0000;
    let original_create = ORIGINAL_D3D_CREATE_BUFFER.load(Ordering::Acquire);
    if original_create == 0 {
        return None;
    }
    let create: CreateBufferFn = unsafe { mem::transmute(original_create) };
    let mut staging = 0usize;
    if unsafe { create(device, &staging_desc, ptr::null(), &mut staging) } < 0 || staging == 0 {
        return None;
    }
    acquired.push(staging);

    let copy: CopyResourceFn = unsafe { mem::transmute(com_method(device_context, 47)?) };
    unsafe { copy(device_context, staging, buffer) };
    let map: MapFn = unsafe { mem::transmute(com_method(device_context, 14)?) };
    let unmap: UnmapFn = unsafe { mem::transmute(com_method(device_context, 15)?) };
    let mut mapped = D3dMappedSubresource::default();
    // D3D11_MAP_READ = 1. Map provides the synchronization boundary proving
    // that the bytes copied immediately before this draw are complete.
    if unsafe { map(device_context, staging, 0, 1, 0, &mut mapped) } < 0 || mapped.data.is_null() {
        return None;
    }
    let bytes = unsafe { slice::from_raw_parts(mapped.data.cast::<u8>(), desc.ByteWidth as usize) }
        .to_vec();
    unsafe { unmap(device_context, staging, 0) };
    Some(RawBufferSnapshot {
        snapshot_sequence,
        buffer_ptr: buffer,
        byte_width: desc.ByteWidth,
        usage_u32: desc.Usage,
        bind_flags_u32: desc.BindFlags,
        cpu_access_flags_u32: desc.CPUAccessFlags,
        misc_flags_u32: desc.MiscFlags,
        structure_byte_stride: desc.StructureByteStride,
        payload: RawPayloadSnapshot {
            captured_sha256: format!("{:x}", Sha256::digest(&bytes)),
            captured_bytes: bytes,
            source: RawPayloadSource::RuntimeGenerated {
                reason: "draw_time_d3d11_buffer_readback".into(),
            },
        },
    })
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct D3dMappedSubresource {
    data: *mut c_void,
    row_pitch: u32,
    depth_pitch: u32,
}

fn created_object_precedes(
    pointer: usize,
    expected_kind: CreatedD3dObjectKind,
    observation_sequence: u64,
) -> bool {
    pointer != 0
        && STATE
            .get()
            .and_then(|value| value.lock().ok())
            .and_then(|state| state.created_d3d_objects.get(&pointer).copied())
            .is_some_and(|created| {
                created.kind == expected_kind
                    && created.creation_sequence != 0
                    && created.creation_sequence <= observation_sequence
            })
}

#[derive(Default)]
struct ComRefs(Vec<usize>);

impl ComRefs {
    fn push(&mut self, object: usize) {
        if object != 0 {
            self.0.push(object);
        }
    }

    fn extend(&mut self, objects: impl IntoIterator<Item = usize>) {
        for object in objects {
            self.push(object);
        }
    }
}

impl Drop for ComRefs {
    fn drop(&mut self) {
        unsafe {
            for object in self.0.drain(..) {
                release_com(object);
            }
        }
    }
}

#[derive(Clone, Copy)]
struct CapturedBlendState {
    state_ptr: usize,
    factor_bits: [u32; 4],
    sample_mask: u32,
}

#[derive(Clone, Copy)]
struct CapturedDepthStencilState {
    state_ptr: usize,
    stencil_reference: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct D3dViewport {
    top_left_x: f32,
    top_left_y: f32,
    width: f32,
    height: f32,
    min_depth: f32,
    max_depth: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct D3dRect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

fn nonzero(value: usize) -> Option<usize> {
    (value != 0).then_some(value)
}

fn com_method(object: usize, method_index: usize) -> Option<usize> {
    let vtable = read_usize(object)?;
    let method =
        read_usize(vtable.checked_add(method_index.checked_mul(mem::size_of::<usize>())?)?)?;
    (method != 0).then_some(method)
}

unsafe fn release_com(object: usize) {
    if object == 0 {
        return;
    }
    type ReleaseFn = unsafe extern "system" fn(usize) -> u32;
    if let Some(method) = com_method(object, 2) {
        let release: ReleaseFn = unsafe { mem::transmute(method) };
        let _ = unsafe { release(object) };
    }
}

fn capture_shader(device_context: usize, method_index: usize) -> Option<(usize, bool)> {
    type GetShaderFn = unsafe extern "system" fn(usize, *mut usize, *mut usize, *mut u32);
    let method = com_method(device_context, method_index)?;
    let get_shader: GetShaderFn = unsafe { mem::transmute(method) };
    let mut shader = 0usize;
    let mut classes = [0usize; MAX_D3D_CLASS_INSTANCES];
    let mut class_count = MAX_D3D_CLASS_INSTANCES as u32;
    unsafe {
        get_shader(
            device_context,
            &mut shader,
            classes.as_mut_ptr(),
            &mut class_count,
        );
    }
    if class_count as usize > classes.len() {
        unsafe {
            release_com(shader);
            for class in classes {
                release_com(class);
            }
        }
        return None;
    }
    let dynamic = class_count != 0;
    unsafe {
        for class in classes.into_iter().take(class_count as usize) {
            release_com(class);
        }
    }
    Some((shader, dynamic))
}

fn capture_single_object(device_context: usize, method_index: usize) -> Option<usize> {
    type GetObjectFn = unsafe extern "system" fn(usize, *mut usize);
    let method = com_method(device_context, method_index)?;
    let get_object: GetObjectFn = unsafe { mem::transmute(method) };
    let mut object = 0usize;
    unsafe { get_object(device_context, &mut object) };
    Some(object)
}

fn capture_stage_objects(
    device_context: usize,
    stage: RawShaderStage,
    method_index: usize,
    slot_count: usize,
) -> Option<Vec<RawStageSlotObject>> {
    type GetObjectsFn = unsafe extern "system" fn(usize, u32, u32, *mut usize);
    let method = com_method(device_context, method_index)?;
    let get_objects: GetObjectsFn = unsafe { mem::transmute(method) };
    let mut objects = vec![0usize; slot_count];
    unsafe {
        get_objects(
            device_context,
            0,
            u32::try_from(slot_count).ok()?,
            objects.as_mut_ptr(),
        );
    }
    Some(
        objects
            .into_iter()
            .enumerate()
            .filter_map(|(slot, object_ptr)| {
                (object_ptr != 0).then_some(RawStageSlotObject {
                    stage,
                    slot: slot as u8,
                    object_ptr,
                })
            })
            .collect(),
    )
}

fn capture_textures(
    device_context: usize,
    stage: RawShaderStage,
    method_index: usize,
) -> Option<Vec<RawTextureObject>> {
    type GetObjectsFn = unsafe extern "system" fn(usize, u32, u32, *mut usize);
    type GetResourceFn = unsafe extern "system" fn(usize, *mut usize);
    let method = com_method(device_context, method_index)?;
    let get_objects: GetObjectsFn = unsafe { mem::transmute(method) };
    let mut views = [0usize; MAX_D3D_SHADER_RESOURCE_SLOTS];
    unsafe {
        get_objects(
            device_context,
            0,
            MAX_D3D_SHADER_RESOURCE_SLOTS as u32,
            views.as_mut_ptr(),
        );
    }
    let mut output = Vec::new();
    for (slot, view) in views.into_iter().enumerate() {
        if view == 0 {
            continue;
        }
        let Some(resource_method) = com_method(view, 7) else {
            unsafe { release_com(view) };
            return None;
        };
        let get_resource: GetResourceFn = unsafe { mem::transmute(resource_method) };
        let mut resource = 0usize;
        unsafe { get_resource(view, &mut resource) };
        if resource == 0 {
            unsafe { release_com(view) };
            return None;
        }
        output.push(RawTextureObject {
            stage,
            slot: slot as u8,
            shader_resource_view_ptr: view,
            resource_ptr: resource,
        });
        // The numeric resource identity remains in the raw row. The reference
        // acquired by GetResource is released immediately.
        unsafe {
            release_com(resource);
            release_com(view);
        }
    }
    Some(output)
}

fn capture_vertex_buffers(device_context: usize) -> Option<Vec<RawVertexBufferObject>> {
    type GetVertexBuffersFn =
        unsafe extern "system" fn(usize, u32, u32, *mut usize, *mut u32, *mut u32);
    let method = com_method(device_context, 83)?;
    let get_buffers: GetVertexBuffersFn = unsafe { mem::transmute(method) };
    let mut buffers = [0usize; MAX_D3D_VERTEX_BUFFER_SLOTS];
    let mut strides = [0u32; MAX_D3D_VERTEX_BUFFER_SLOTS];
    let mut offsets = [0u32; MAX_D3D_VERTEX_BUFFER_SLOTS];
    unsafe {
        get_buffers(
            device_context,
            0,
            MAX_D3D_VERTEX_BUFFER_SLOTS as u32,
            buffers.as_mut_ptr(),
            strides.as_mut_ptr(),
            offsets.as_mut_ptr(),
        );
    }
    Some(
        buffers
            .into_iter()
            .enumerate()
            .filter_map(|(input_slot, buffer_ptr)| {
                (buffer_ptr != 0).then_some(RawVertexBufferObject {
                    input_slot: input_slot as u32,
                    buffer_ptr,
                    stride_bytes: strides[input_slot],
                    byte_offset: offsets[input_slot],
                })
            })
            .collect(),
    )
}

fn capture_index_buffer(device_context: usize) -> Option<Option<RawIndexBufferObject>> {
    type GetIndexBufferFn = unsafe extern "system" fn(usize, *mut usize, *mut u32, *mut u32);
    let method = com_method(device_context, 84)?;
    let get_buffer: GetIndexBufferFn = unsafe { mem::transmute(method) };
    let mut buffer_ptr = 0usize;
    let mut dxgi_format_u32 = 0u32;
    let mut byte_offset = 0u32;
    unsafe {
        get_buffer(
            device_context,
            &mut buffer_ptr,
            &mut dxgi_format_u32,
            &mut byte_offset,
        );
    }
    Some((buffer_ptr != 0).then_some(RawIndexBufferObject {
        buffer_ptr,
        dxgi_format_u32,
        byte_offset,
    }))
}

fn capture_primitive_topology(device_context: usize) -> Option<u32> {
    type GetTopologyFn = unsafe extern "system" fn(usize, *mut u32);
    let method = com_method(device_context, 87)?;
    let get_topology: GetTopologyFn = unsafe { mem::transmute(method) };
    let mut topology = 0u32;
    unsafe { get_topology(device_context, &mut topology) };
    Some(topology)
}

fn capture_blend_state(device_context: usize) -> Option<CapturedBlendState> {
    type GetBlendFn = unsafe extern "system" fn(usize, *mut usize, *mut f32, *mut u32);
    let method = com_method(device_context, 95)?;
    let get_blend: GetBlendFn = unsafe { mem::transmute(method) };
    let mut state_ptr = 0usize;
    let mut factors = [0f32; 4];
    let mut sample_mask = 0u32;
    unsafe {
        get_blend(
            device_context,
            &mut state_ptr,
            factors.as_mut_ptr(),
            &mut sample_mask,
        );
    }
    Some(CapturedBlendState {
        state_ptr,
        factor_bits: factors.map(f32::to_bits),
        sample_mask,
    })
}

fn capture_depth_stencil_state(device_context: usize) -> Option<CapturedDepthStencilState> {
    type GetDepthFn = unsafe extern "system" fn(usize, *mut usize, *mut u32);
    let method = com_method(device_context, 96)?;
    let get_depth: GetDepthFn = unsafe { mem::transmute(method) };
    let mut state_ptr = 0usize;
    let mut stencil_reference = 0u32;
    unsafe { get_depth(device_context, &mut state_ptr, &mut stencil_reference) };
    Some(CapturedDepthStencilState {
        state_ptr,
        stencil_reference,
    })
}

fn capture_stream_output(device_context: usize) -> Option<Vec<usize>> {
    type GetTargetsFn = unsafe extern "system" fn(usize, u32, *mut usize);
    let method = com_method(device_context, 97)?;
    let get_targets: GetTargetsFn = unsafe { mem::transmute(method) };
    let mut targets = [0usize; MAX_D3D_STREAM_OUTPUT_SLOTS];
    unsafe {
        get_targets(
            device_context,
            MAX_D3D_STREAM_OUTPUT_SLOTS as u32,
            targets.as_mut_ptr(),
        );
    }
    Some(targets.into_iter().filter(|value| *value != 0).collect())
}

fn capture_render_targets(device_context: usize) -> Option<(Vec<usize>, usize)> {
    type GetTargetsFn = unsafe extern "system" fn(usize, u32, *mut usize, *mut usize);
    let method = com_method(device_context, 93)?;
    let get_targets: GetTargetsFn = unsafe { mem::transmute(method) };
    let mut color = [0usize; MAX_D3D_RENDER_TARGET_SLOTS];
    let mut depth = 0usize;
    unsafe {
        get_targets(
            device_context,
            MAX_D3D_RENDER_TARGET_SLOTS as u32,
            color.as_mut_ptr(),
            &mut depth,
        );
    }
    Some((
        color.into_iter().filter(|value| *value != 0).collect(),
        depth,
    ))
}

fn capture_viewports(device_context: usize) -> Option<Vec<[u32; 6]>> {
    type GetViewportsFn = unsafe extern "system" fn(usize, *mut u32, *mut D3dViewport);
    let method = com_method(device_context, 99)?;
    let get_viewports: GetViewportsFn = unsafe { mem::transmute(method) };
    let mut count = MAX_D3D_VIEWPORTS_AND_SCISSORS as u32;
    let mut values = [D3dViewport::default(); MAX_D3D_VIEWPORTS_AND_SCISSORS];
    unsafe { get_viewports(device_context, &mut count, values.as_mut_ptr()) };
    if count as usize > values.len() {
        return None;
    }
    Some(
        values
            .into_iter()
            .take(count as usize)
            .map(|value| {
                [
                    value.top_left_x.to_bits(),
                    value.top_left_y.to_bits(),
                    value.width.to_bits(),
                    value.height.to_bits(),
                    value.min_depth.to_bits(),
                    value.max_depth.to_bits(),
                ]
            })
            .collect(),
    )
}

fn capture_scissors(device_context: usize) -> Option<Vec<[i32; 4]>> {
    type GetScissorsFn = unsafe extern "system" fn(usize, *mut u32, *mut D3dRect);
    let method = com_method(device_context, 100)?;
    let get_scissors: GetScissorsFn = unsafe { mem::transmute(method) };
    let mut count = MAX_D3D_VIEWPORTS_AND_SCISSORS as u32;
    let mut values = [D3dRect::default(); MAX_D3D_VIEWPORTS_AND_SCISSORS];
    unsafe { get_scissors(device_context, &mut count, values.as_mut_ptr()) };
    if count as usize > values.len() {
        return None;
    }
    Some(
        values
            .into_iter()
            .take(count as usize)
            .map(|value| [value.left, value.top, value.right, value.bottom])
            .collect(),
    )
}
fn addresses(base: usize) -> InstalledAddresses {
    InstalledAddresses {
        vfs_open_constructor: base + VFS_OPEN_CONSTRUCTOR_RVA,
        window_backbuffer_rtv_producer: base + WINDOW_BACKBUFFER_RTV_PRODUCER_RVA,
        opaque_texture_target_producer: base + OPAQUE_TEXTURE_TARGET_PRODUCER_RVA,
        texture_cache_load_root: base + TEXTURE_CACHE_LOAD_ROOT_RVA,
        global_envelope: base + GLOBAL_RENDER_ENVELOPE_RVA as usize,
        object_ctor: base + PARTICLE_OBJECT_CTOR_RVA as usize,
        bucket_submit: base + PARTICLE_BUCKET_SUBMIT_RVA as usize,
        runtime_create: base + PARTICLE_RUNTIME_CREATE_RVA as usize,
        runtime_destroy: base + PARTICLE_RUNTIME_DESTROY_RVA as usize,
        packet_builder: base + PARTICLE_PACKET_BUILDER_RVA as usize,
        particle_render_submit: base + PARTICLE_RENDER_SUBMIT_RVA as usize,
        particle_final_draw: base + PARTICLE_FINAL_DRAW_RVA as usize,
        entity_factory: base + ENTITY_FACTORY_RVA as usize,
        entity_named_factory: base + ENTITY_NAMED_FACTORY_RVA,
        entity_definition_lookup: base + ENTITY_DEFINITION_LOOKUP_RVA,
        entity_destroy: base + ENTITY_DESTROY_RVA as usize,
        entity_render_root: base + ENTITY_RENDER_ROOT_RVA as usize,
        entity_final_draw: base + ENTITY_FINAL_DRAW_RVA as usize,
        indexed_instanced_final_draw: base + INDEXED_INSTANCED_FINAL_DRAW_RVA as usize,
        instanced_final_draw: base + INSTANCED_FINAL_DRAW_RVA as usize,
        render_target_clear: base + RENDER_TARGET_CLEAR_RVA as usize,
        shader_program_binder: base + SHADER_PROGRAM_BINDER_RVA as usize,
        engine_shader_effect_lookup: base + ENGINE_SHADER_EFFECT_LOOKUP_RVA as usize,
        map_effect_registration_root: base + MAP_EFFECT_REGISTRATION_ROOT_RVA as usize,
        railroad_effect_registration_root: base + RAILROAD_EFFECT_REGISTRATION_ROOT_RVA as usize,
        map_name_effect_registration_root: base + MAP_NAME_EFFECT_REGISTRATION_ROOT_RVA as usize,
        restore_bloom_effect_registration_root: base
            + RESTORE_BLOOM_EFFECT_REGISTRATION_ROOT_RVA as usize,
        lut_blend_effect_registration_root: base + LUT_BLEND_EFFECT_REGISTRATION_ROOT_RVA as usize,
        weather_normal_update_owner: base + WEATHER_NORMAL_UPDATE_OWNER_RVA as usize,
        weather_editor_refresh_owner: base + WEATHER_EDITOR_REFRESH_OWNER_RVA as usize,
        weather_visual_producer: base + WEATHER_VISUAL_PRODUCER_RVA as usize,
        weather_entity_active_setter: base + WEATHER_ENTITY_ACTIVE_SETTER_RVA as usize,
        weather_entity_alpha_setter: base + WEATHER_ENTITY_ALPHA_SETTER_RVA as usize,
        weather_entity_playback_setter: base + WEATHER_ENTITY_PLAYBACK_SETTER_RVA as usize,
        high_resolution_seed: base + HIGH_RESOLUTION_SEED_RVA as usize,
        map_scene_render_root: base + MAP_SCENE_RENDER_ROOT_RVA as usize,
        post_process_render_root: base + POST_PROCESS_RENDER_ROOT_RVA as usize,
        present_render_root: base + PRESENT_RENDER_ROOT_RVA as usize,
        snow_mud_producer: base + SNOW_MUD_PRODUCER_RVA,
        snow_mud_upload: base + SNOW_MUD_UPLOAD_RVA,
        camera_wheel_update: base + CAMERA_WHEEL_UPDATE_RVA as usize,
        camera_vertical_event_producer: base + CAMERA_VERTICAL_EVENT_PRODUCER_RVA as usize,
        camera_map_pick: base + CAMERA_MAP_PICK_RVA as usize,
        o05_update_provinces: base + o05_building_capture::UPDATE_PROVINCES_RVA,
        o05_building_constructor: base + o05_building_capture::BUILDING_CONSTRUCTOR_RVA,
        o05_building_vector_update: base + o05_building_capture::BUILDING_VECTOR_UPDATE_RVA,
        o05_entity_transform_setter: base
            + o05_building_capture::ENTITY_TRANSFORM_SETTER_RVA,
    }
}
fn targets(a: InstalledAddresses) -> [usize; ENGINE_HOOK_COUNT] {
    [
        a.vfs_open_constructor,
        a.window_backbuffer_rtv_producer,
        a.opaque_texture_target_producer,
        a.texture_cache_load_root,
        a.global_envelope,
        a.object_ctor,
        a.bucket_submit,
        a.runtime_create,
        a.runtime_destroy,
        a.packet_builder,
        a.particle_render_submit,
        a.particle_final_draw,
        a.entity_factory,
        a.entity_named_factory,
        a.entity_definition_lookup,
        a.entity_destroy,
        a.entity_render_root,
        a.entity_final_draw,
        a.indexed_instanced_final_draw,
        a.instanced_final_draw,
        a.render_target_clear,
        a.shader_program_binder,
        a.engine_shader_effect_lookup,
        a.map_effect_registration_root,
        a.railroad_effect_registration_root,
        a.map_name_effect_registration_root,
        a.restore_bloom_effect_registration_root,
        a.lut_blend_effect_registration_root,
        a.weather_normal_update_owner,
        a.weather_editor_refresh_owner,
        a.weather_visual_producer,
        a.weather_entity_active_setter,
        a.weather_entity_alpha_setter,
        a.weather_entity_playback_setter,
        a.high_resolution_seed,
        a.map_scene_render_root,
        a.post_process_render_root,
        a.present_render_root,
        a.snow_mud_producer,
        a.snow_mud_upload,
        a.camera_wheel_update,
        a.camera_vertical_event_producer,
        a.camera_map_pick,
        a.o05_update_provinces,
        a.o05_building_constructor,
        a.o05_building_vector_update,
        a.o05_entity_transform_setter,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{mpsc, Mutex};
    use std::thread;

    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn test_game_hour(offset: i32) -> i32 {
        compact_contract::calendar::DATE_EPOCH_CONST + (1936 * 365 * 24) + offset
    }

    fn reset_frame_state() {
        INSTALLED.store(false, Ordering::Release);
        PRESENT_GAME_STATE_ROOT_SLOT.store(0, Ordering::Release);
        *STATE
            .get_or_init(|| Mutex::new(HookState::default()))
            .lock()
            .unwrap() = HookState::default();
        NEXT_RENDER_ORDER_SEQUENCE.store(1, Ordering::Release);
        NEXT_RENDER_FRAME_SEQUENCE.store(1, Ordering::Release);
        NEXT_PRESENT_GLOBAL_ORDER.store(0, Ordering::Release);
        NEXT_CATALOG_OBSERVATION_SEQUENCE.store(1, Ordering::Release);
    }

    #[test]
    fn create_buffer_rejects_a_missing_original_instead_of_calling_zero() {
        let _serial = TEST_LOCK.lock().unwrap();
        COM_DETOUR_TEARDOWN.store(false, Ordering::Release);
        ACTIVE_COM_DETOURS.store(0, Ordering::Release);
        ORIGINAL_D3D_CREATE_BUFFER.store(0, Ordering::Release);
        let hr = unsafe { hook_d3d_create_buffer(0, ptr::null(), ptr::null(), ptr::null_mut()) };
        assert_eq!(hr, 0x8000_4005u32 as i32);
        assert_eq!(ACTIVE_COM_DETOURS.load(Ordering::Acquire), 0);
    }

    #[test]
    fn com_teardown_waits_for_entered_detours_before_clearing_originals() {
        let _serial = TEST_LOCK.lock().unwrap();
        COM_DETOUR_TEARDOWN.store(false, Ordering::Release);
        ACTIVE_COM_DETOURS.store(0, Ordering::Release);
        ORIGINAL_D3D_CREATE_BUFFER.store(0x1234_5678, Ordering::Release);
        let active = ActiveComDetour::enter().expect("test detour must enter before teardown");
        let (started_tx, started_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            begin_com_detour_teardown();
            started_tx.send(()).unwrap();
            wait_for_com_detour_quiescence().unwrap();
            let original_before_clear = ORIGINAL_D3D_CREATE_BUFFER.load(Ordering::Acquire);
            ORIGINAL_D3D_CREATE_BUFFER.store(0, Ordering::Release);
            finish_com_detour_teardown();
            done_tx.send(original_before_clear).unwrap();
        });
        started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(ActiveComDetour::enter().is_none());
        assert!(done_rx.recv_timeout(Duration::from_millis(25)).is_err());
        assert_eq!(
            ORIGINAL_D3D_CREATE_BUFFER.load(Ordering::Acquire),
            0x1234_5678,
            "the trampoline must remain populated while an entered detour is active"
        );
        drop(active);
        assert_eq!(
            done_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            0x1234_5678
        );
        worker.join().unwrap();
        assert_eq!(ACTIVE_COM_DETOURS.load(Ordering::Acquire), 0);
        assert!(!COM_DETOUR_TEARDOWN.load(Ordering::Acquire));
    }

    #[test]
    fn d3d_export_sentinels_accept_dump_prologues_and_reject_collector_import_thunks() {
        let d3d11 = [
            0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24,
            0x18, 0x57, 0x48, 0x83, 0xec, 0x60,
        ];
        let d3d_compile = [
            0x4c, 0x8b, 0xdc, 0x48, 0x83, 0xec, 0x78, 0x48, 0x8b, 0x84, 0x24, 0xd0,
        ];
        let collector_d3d11_import = [0xff, 0x25, 0x04, 0xe5, 0x00, 0x00];
        let collector_compile_import = [0xff, 0x25, 0x66, 0xe1, 0x00, 0x00];
        assert!(validate_export_sentinel_bytes(
            &d3d11,
            &D3D11_CREATE_DEVICE_EXPORT_SENTINEL,
            "d3d11!D3D11CreateDevice"
        )
        .is_ok());
        assert!(validate_export_sentinel_bytes(
            &d3d_compile,
            &D3D_COMPILE_EXPORT_SENTINEL,
            "d3dcompiler_47!D3DCompile"
        )
        .is_ok());
        assert!(validate_export_sentinel_bytes(
            &collector_d3d11_import,
            &D3D11_CREATE_DEVICE_EXPORT_SENTINEL,
            "collector!D3D11CreateDevice"
        )
        .unwrap_err()
        .contains("import thunk"));
        assert!(validate_export_sentinel_bytes(
            &collector_compile_import,
            &D3D_COMPILE_EXPORT_SENTINEL,
            "collector!D3DCompile"
        )
        .unwrap_err()
        .contains("import thunk"));
    }

    #[test]
    fn d3d_export_target_must_fit_inside_loaded_module_image() {
        assert!(export_target_within_image(0x1000, 0x9000, 0x1800, 16));
        assert!(!export_target_within_image(0x1000, 0x9000, 0x0fff, 16));
        assert!(!export_target_within_image(0x1000, 0x9000, 0x9ff8, 16));
        assert!(!export_target_within_image(
            usize::MAX - 0x100,
            0x200,
            usize::MAX - 0x80,
            16
        ));
    }

    fn registered_target(
        view_ptr: usize,
        registration_sequence: u64,
    ) -> RawRenderTargetRegistrationObservation {
        RawRenderTargetRegistrationObservation {
            registration_sequence,
            capture_sequence: registration_sequence,
            producer_rva: OPAQUE_TEXTURE_TARGET_PRODUCER_RVA as u64,
            recreate_generation: 0,
            attachment_ordinal: 0,
            kind: RawRegisteredRenderTargetKind::OpaqueRenderTarget {
                source_flags_u32: 2,
            },
            view_ptr,
            resource_ptr: view_ptr + 1,
            resource_desc: RawTexture2dDesc {
                width: 64,
                height: 32,
                mip_levels: 1,
                array_size: 1,
                dxgi_format_u32: 28,
                sample_count: 1,
                sample_quality: 0,
                usage_u32: 0,
                bind_flags_u32: 0x28,
                cpu_access_flags_u32: 0,
                misc_flags_u32: 0,
            },
            rtv_desc: RawResourceViewDesc {
                dxgi_format_u32: 28,
                view_dimension_u32: 4,
                flags_u32: 0,
                union_words_u32: [0; 4],
            },
        }
    }

    #[test]
    fn transient_target_registry_materializes_exact_om_attachment_order() {
        let _guard = TEST_LOCK.lock().unwrap();
        let mut state = HookState::default();
        state
            .registered_render_targets
            .insert(0x500, registered_target(0x500, 2));
        state
            .registered_render_targets
            .insert(0x600, registered_target(0x600, 3));
        record_registered_color_target_uses_locked(&mut state, 10, &[0x500, 0x600]).unwrap();
        let rows = state
            .render_target_use_observations
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].attachment_ordinal, 0);
        assert_eq!(rows[1].attachment_ordinal, 1);
        assert!(rows.iter().all(|row| row.capture_sequence == 10));
        assert!(rows.into_iter().all(|row| row.materialize().is_ok()));
    }

    #[test]
    fn transient_target_registry_rejects_unregistered_view_without_partial_rows() {
        let _guard = TEST_LOCK.lock().unwrap();
        let mut state = HookState::default();
        state
            .registered_render_targets
            .insert(0x500, registered_target(0x500, 2));
        assert!(
            record_registered_color_target_uses_locked(&mut state, 10, &[0x500, 0x700]).is_none()
        );
        assert!(state.render_target_use_observations.is_empty());
    }

    #[test]
    fn texture_load_join_requires_root_scoped_open_resource_and_same_resource_srv() {
        let _guard = TEST_LOCK.lock().unwrap();
        let mut state = HookState::default();
        let miss = PendingTextureLoad {
            entry_sequence: 10,
            logical_path: "gfx/map/modded.dds".into(),
            nested_open_sequences: vec![11],
            nested_texture_creations: vec![(12, 0x700)],
            nested_srv_creations: vec![(13, 0x701, 0x700)],
        };
        finalize_texture_load_locked(&mut state, miss, 14, 3).unwrap();
        let join = state.texture_vfs_resource_joins.get(&0x700).unwrap();
        assert_eq!(join.source_open_observation_sequence, 11);
        assert_eq!(join.shader_resource_view_ptr, 0x701);

        let hit = PendingTextureLoad {
            entry_sequence: 15,
            logical_path: "gfx/map/modded.dds".into(),
            nested_open_sequences: Vec::new(),
            nested_texture_creations: Vec::new(),
            nested_srv_creations: Vec::new(),
        };
        finalize_texture_load_locked(&mut state, hit, 16, 3).unwrap();
        assert_eq!(state.texture_vfs_resource_joins.len(), 1);
    }

    #[test]
    fn texture_load_join_rejects_foreign_resource_srv_and_unproven_cache_hit() {
        let _guard = TEST_LOCK.lock().unwrap();
        let mut state = HookState::default();
        let foreign_srv = PendingTextureLoad {
            entry_sequence: 10,
            logical_path: "gfx/map/modded.dds".into(),
            nested_open_sequences: vec![11],
            nested_texture_creations: vec![(12, 0x700)],
            nested_srv_creations: vec![(13, 0x701, 0x999)],
        };
        assert!(finalize_texture_load_locked(&mut state, foreign_srv, 14, 3).is_none());
        assert!(state.texture_vfs_resource_joins.is_empty());

        let unproven_hit = PendingTextureLoad {
            entry_sequence: 20,
            logical_path: "gfx/map/modded.dds".into(),
            nested_open_sequences: Vec::new(),
            nested_texture_creations: Vec::new(),
            nested_srv_creations: Vec::new(),
        };
        assert!(finalize_texture_load_locked(&mut state, unproven_hit, 21, 3).is_none());
    }

    fn record_empty_frame(submit_index: u64, presentation_qpc_ticks: u64) {
        record_empty_frame_for_hour(submit_index, presentation_qpc_ticks, test_game_hour(41));
    }

    fn record_empty_frame_for_hour(
        submit_index: u64,
        presentation_qpc_ticks: u64,
        game_hour_raw: i32,
    ) {
        begin_render_frame(submit_index).unwrap();
        record_boundary(
            RenderOrderBoundary::MapSceneBegin,
            MAP_SCENE_RENDER_ROOT_RVA,
        );
        record_boundary(RenderOrderBoundary::MapSceneEnd, MAP_SCENE_RENDER_ROOT_RVA);
        record_boundary(
            RenderOrderBoundary::PostProcessBegin,
            POST_PROCESS_RENDER_ROOT_RVA,
        );
        record_boundary(
            RenderOrderBoundary::PostProcessEnd,
            POST_PROCESS_RENDER_ROOT_RVA,
        );
        record_boundary(RenderOrderBoundary::PresentBegin, PRESENT_RENDER_ROOT_RVA);
        record_boundary(RenderOrderBoundary::PresentEnd, PRESENT_RENDER_ROOT_RVA);
        complete_render_frame_with_receipt(PresentationQpcReceipt {
            global_order: NEXT_PRESENT_GLOBAL_ORDER.fetch_add(1, Ordering::AcqRel),
            ticks: presentation_qpc_ticks,
            frequency_hz: 10_000_003,
            process_start_nonce: 71,
            hook_epoch_nonce: 73,
            game_hour_raw,
        })
        .unwrap();
    }

    fn enqueue_empty_runtime(state: &mut HookState, submit_index: u64) {
        state.completed.push_back(RawParticleSubmit {
            submit_index,
            camera_carrier_ordinal: 0,
            camera_carrier: None,
            packets: Vec::new(),
            entity_submissions: Vec::new(),
        });
        state.completed_catalogs.push_back(RawRenderCatalogCapture {
            process: RawProcessEpoch {
                process_id: 1,
                process_start_nonce: 71,
                executable_sha256: "30".repeat(32),
            },
            launch_epoch: RawCaptureLaunchEpoch {
                process_id: 1,
                process_start_nonce: 71,
                observer_installed_qpc: 1,
                d3d_device_created_qpc: 2,
                first_creation_sequence: 1,
                hook_epoch_nonce: 73,
                hook_set_receipt_sha256: "33".repeat(32),
                capture_receipt_sha256: "34".repeat(32),
            },
            submit_index,
            frame_sequence: submit_index,
            draw_states: Vec::new(),
            draw_semantics: Vec::new(),
            program_identities: Vec::new(),
            shader_creations: Vec::new(),
            input_layout_creations: Vec::new(),
            blend_state_creations: Vec::new(),
            depth_stencil_state_creations: Vec::new(),
            rasterizer_state_creations: Vec::new(),
            sampler_state_creations: Vec::new(),
            buffer_snapshots: Vec::new(),
            texture_resource_snapshots: Vec::new(),
            shader_resource_view_snapshots: Vec::new(),
            render_target_view_snapshots: Vec::new(),
        });
    }

    fn set_last_frame_hour(game_hour_raw: i32) {
        STATE
            .get()
            .unwrap()
            .lock()
            .unwrap()
            .completed_render_frames
            .back_mut()
            .unwrap()
            .game_hour_raw = game_hour_raw;
    }

    #[test]
    fn render_frames_keep_explicit_submit_join_even_without_runtime_draws() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_frame_state();
        record_empty_frame(41, 101);
        record_empty_frame(42, 9_001);
        let state = STATE.get().unwrap().lock().unwrap();
        assert_eq!(state.completed_render_frames.len(), 2);
        assert_eq!(state.completed_render_frames[0].frame_sequence, 1);
        assert_eq!(state.completed_render_frames[0].submit_index, 41);
        assert_eq!(state.completed_render_frames[0].global_order, 0);
        assert_eq!(state.completed_render_frames[0].presentation_qpc_ticks, 101);
        assert_eq!(
            state.completed_render_frames[0].game_hour_raw,
            test_game_hour(41)
        );
        assert_eq!(
            state.completed_render_frames[0].presentation_qpc_frequency_hz,
            10_000_003
        );
        assert_eq!(state.completed_render_frames[1].frame_sequence, 2);
        assert_eq!(state.completed_render_frames[1].submit_index, 42);
        assert_eq!(state.completed_render_frames[1].global_order, 1);
        assert_eq!(
            state.completed_render_frames[1].presentation_qpc_ticks,
            9_001
        );
        assert_ne!(
            state.completed_render_frames[1].presentation_qpc_ticks
                - state.completed_render_frames[0].presentation_qpc_ticks,
            state.completed_render_frames[1].frame_sequence
                - state.completed_render_frames[0].frame_sequence
        );
        assert!(state.completed_render_frames[0]
            .static_catalog_fragment
            .shader_creations
            .is_empty());
        assert!(state.render_order_events.is_empty());
    }

    #[test]
    fn render_order_event_without_frame_fails_closed() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_frame_state();
        assert!(
            record_render_order_event(|sequence| RawRenderOrderEvent::Boundary {
                sequence,
                kind: RenderOrderBoundary::MapSceneBegin,
                producer_rva: MAP_SCENE_RENDER_ROOT_RVA,
            })
            .is_none()
        );
        assert_eq!(
            STATE
                .get()
                .unwrap()
                .lock()
                .unwrap()
                .render_order_event_overflows,
            1
        );
    }

    #[test]
    fn joined_drain_retains_frame_during_runtime_publication_gap() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_frame_state();
        record_empty_frame(41, 101);

        let mut state = STATE.get().unwrap().lock().unwrap();
        assert!(matches!(
            drain_complete_render_frame_submit_captures_locked(&mut state).unwrap(),
            RawJoinedRenderFrameDrain::PendingIncomplete {
                publication_generation: 0,
                completed_frame_count: 1,
                runtime_submit_count: 0,
            }
        ));
        assert_eq!(state.completed_render_frames.len(), 1);
        assert!(state.completed.is_empty());
        assert!(state.completed_catalogs.is_empty());

        enqueue_empty_runtime(&mut state, 41);
        let RawJoinedRenderFrameDrain::Ready(joined) =
            drain_complete_render_frame_submit_captures_locked(&mut state).unwrap()
        else {
            panic!("exact frame/runtime join was not ready");
        };
        assert_eq!(joined.len(), 1);
        assert_eq!(joined[0].frame.submit_index, 41);
        assert_eq!(joined[0].runtime.submit.submit_index, 41);
        assert_eq!(joined[0].runtime.catalog.submit_index, 41);
        assert!(joined[0].runtime.shader_reflections.is_empty());
        assert!(state.completed_render_frames.is_empty());
        assert!(state.completed.is_empty());
        assert!(state.completed_catalogs.is_empty());
    }

    #[test]
    fn joined_drain_retains_unmatched_rows_and_orders_by_completed_frame() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_frame_state();
        {
            let mut state = STATE.get().unwrap().lock().unwrap();
            enqueue_empty_runtime(&mut state, 40);
            assert!(matches!(
                drain_complete_render_frame_submit_captures_locked(&mut state).unwrap(),
                RawJoinedRenderFrameDrain::PendingIncomplete {
                    completed_frame_count: 0,
                    runtime_submit_count: 1,
                    ..
                }
            ));
            assert_eq!(state.completed.len(), 1);
            assert_eq!(state.completed_catalogs.len(), 1);
        }

        record_empty_frame(41, 101);
        record_empty_frame(42, 9_001);
        {
            let mut state = STATE.get().unwrap().lock().unwrap();
            enqueue_empty_runtime(&mut state, 42);
            let RawJoinedRenderFrameDrain::Ready(joined) =
                drain_complete_render_frame_submit_captures_locked(&mut state).unwrap()
            else {
                panic!("exact frame/runtime join was not ready");
            };
            assert_eq!(joined.len(), 1);
            assert_eq!(joined[0].frame.submit_index, 42);
            assert_eq!(joined[0].runtime.submit.submit_index, 42);
            assert_eq!(state.completed_render_frames.len(), 1);
            assert_eq!(state.completed_render_frames[0].submit_index, 41);
            assert_eq!(state.completed.len(), 1);
            assert_eq!(state.completed[0].submit_index, 40);
            assert_eq!(state.completed_catalogs.len(), 1);
            assert_eq!(state.completed_catalogs[0].submit_index, 40);
        }

        record_empty_frame(40, 10_001);
        let mut state = STATE.get().unwrap().lock().unwrap();
        enqueue_empty_runtime(&mut state, 41);
        let RawJoinedRenderFrameDrain::Ready(joined) =
            drain_complete_render_frame_submit_captures_locked(&mut state).unwrap()
        else {
            panic!("exact frame/runtime joins were not ready");
        };
        assert_eq!(
            joined
                .iter()
                .map(|row| row.frame.submit_index)
                .collect::<Vec<_>>(),
            vec![41, 40]
        );
        assert!(joined.iter().all(
            |row| row.frame.submit_index == row.runtime.submit.submit_index
                && row.frame.submit_index == row.runtime.catalog.submit_index
        ));
        assert!(state.completed_render_frames.is_empty());
        assert!(state.completed.is_empty());
        assert!(state.completed_catalogs.is_empty());
    }

    #[test]
    fn joined_drain_rejects_duplicate_or_misaligned_ids_without_mutation() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_frame_state();
        record_empty_frame(41, 101);
        let mut state = STATE.get().unwrap().lock().unwrap();
        enqueue_empty_runtime(&mut state, 41);
        state.completed_catalogs[0].submit_index = 42;
        let before = (
            state.completed_render_frames.clone(),
            state.completed.clone(),
            state.completed_catalogs.clone(),
        );
        assert!(drain_complete_render_frame_submit_captures_locked(&mut state).is_err());
        assert_eq!(state.completed_render_frames, before.0);
        assert_eq!(state.completed, before.1);
        assert_eq!(state.completed_catalogs, before.2);
    }

    #[test]
    fn joined_drain_distinguishes_empty_from_publication_gap() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_frame_state();
        let mut state = STATE.get().unwrap().lock().unwrap();
        assert_eq!(
            drain_complete_render_frame_submit_captures_locked(&mut state).unwrap(),
            RawJoinedRenderFrameDrain::EmptyNoCapture
        );
        enqueue_empty_runtime(&mut state, 41);
        state.runtime_publication_generation = 9;
        assert_eq!(
            drain_complete_render_frame_submit_captures_locked(&mut state).unwrap(),
            RawJoinedRenderFrameDrain::PendingIncomplete {
                publication_generation: 9,
                completed_frame_count: 0,
                runtime_submit_count: 1,
            }
        );
    }

    #[test]
    fn targeted_hour_drain_selects_last_cutoff_present_and_retains_other_rows() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_frame_state();
        let hour = test_game_hour(1_000);
        for (submit_index, qpc, frame_hour) in [
            (41, 100, hour),
            (42, 200, hour),
            (43, 300, hour),
            (44, 400, hour + 1),
            (45, 500, hour),
        ] {
            record_empty_frame(submit_index, qpc);
            set_last_frame_hour(frame_hour);
        }
        let mut state = STATE.get().unwrap().lock().unwrap();
        for submit_index in 41..=45 {
            enqueue_empty_runtime(&mut state, submit_index);
        }
        let RawSelectedRenderFrameDrain::Ready(selected) =
            drain_selected_render_frame_submit_capture_locked(&mut state, hour, 350, 10_000_003)
                .unwrap()
        else {
            panic!("target hour did not select a ready Present");
        };
        assert_eq!(selected.receipt.candidate_count, 3);
        assert_eq!(selected.receipt.selected_submit_index, 43);
        assert_eq!(selected.receipt.selected_global_order, 2);
        assert_eq!(selected.receipt.selected_presentation_qpc_ticks, 300);
        assert_eq!(selected.selected.frame.submit_index, 43);
        assert_eq!(selected.receipt.discarded_candidate_count, 2);
        assert_eq!(selected.consumed_unselected.len(), 2);
        assert_eq!(selected.receipt.candidate_set_sha256.len(), 64);
        assert_eq!(selected.receipt.consumed_unselected_sha256.len(), 64);
        assert_eq!(selected.receipt.selection_receipt_sha256.len(), 64);
        assert_eq!(selected.sealed_purges.len(), 1);
        assert_eq!(selected.sealed_purges[0].submit_index, 45);
        assert_eq!(
            state
                .completed_render_frames
                .iter()
                .map(|frame| frame.submit_index)
                .collect::<Vec<_>>(),
            vec![44]
        );
        assert_eq!(
            state
                .completed
                .iter()
                .map(|submit| submit.submit_index)
                .collect::<Vec<_>>(),
            vec![44]
        );
        assert_eq!(
            state
                .completed_catalogs
                .iter()
                .map(|catalog| catalog.submit_index)
                .collect::<Vec<_>>(),
            vec![44]
        );
        drop(state);

        // A same-hour frame that publishes after sealing is retired in place
        // and cannot accumulate into the next hour. The future-hour row stays.
        record_empty_frame(46, 600);
        set_last_frame_hour(hour);
        let mut state = STATE.get().unwrap().lock().unwrap();
        enqueue_empty_runtime(&mut state, 46);
        purge_published_sealed_render_submit(&mut state, 46).unwrap();
        assert_eq!(
            state
                .completed_render_frames
                .iter()
                .map(|frame| frame.submit_index)
                .collect::<Vec<_>>(),
            vec![44]
        );
        assert_eq!(state.sealed_purge_receipt_count, 1);
        assert_ne!(state.sealed_purge_rolling_sha256, [0; 32]);
    }

    #[test]
    fn paused_sealed_hour_purges_more_than_submit_bound_without_retention_or_failure() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_frame_state();
        let hour = test_game_hour(1_000);
        {
            let mut state = STATE.get().unwrap().lock().unwrap();
            state.sealed_render_hour = Some(SealedRenderHour {
                game_hour_raw: hour,
                cutoff_qpc_ticks: 100,
                cutoff_qpc_frequency_hz: 10_000_003,
            });
        }
        let purge_count = MAX_SUBMITS + 257;
        for ordinal in 0..purge_count {
            let submit_index = 1_000 + ordinal as u64;
            record_empty_frame(submit_index, 101 + ordinal as u64);
            set_last_frame_hour(hour);
            let mut state = STATE.get().unwrap().lock().unwrap();
            enqueue_empty_runtime(&mut state, submit_index);
            purge_published_sealed_render_submit(&mut state, submit_index).unwrap();
            assert!(state.completed_render_frames.is_empty());
            assert!(state.completed.is_empty());
            assert!(state.completed_catalogs.is_empty());
        }
        INSTALLED.store(true, Ordering::Release);
        let telemetry = drain_completed_sealed_render_purge_receipts().unwrap();
        INSTALLED.store(false, Ordering::Release);
        assert_eq!(telemetry.receipt_count, purge_count as u128);
        assert!(!telemetry.receipt_count_overflowed);
        assert_eq!(telemetry.rolling_receipt_sha256.len(), 64);
        assert_ne!(telemetry.rolling_receipt_sha256, "0".repeat(64));
        let state = STATE.get().unwrap().lock().unwrap();
        assert_eq!(state.sealed_purge_receipt_count, 0);
        assert_eq!(state.sealed_purge_rolling_sha256, [0; 32]);
    }

    #[test]
    fn paused_current_hour_compacts_more_than_submit_bound_to_latest_candidate() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_frame_state();
        let hour = test_game_hour(1_001);
        let candidate_count = MAX_SUBMITS + 257;
        for ordinal in 0..candidate_count {
            let submit_index = 10_000 + ordinal as u64;
            let qpc = 1_000 + ordinal as u64;
            record_empty_frame(submit_index, qpc);
            set_last_frame_hour(hour);
            let mut state = STATE.get().unwrap().lock().unwrap();
            enqueue_empty_runtime(&mut state, submit_index);
            compact_published_current_hour_candidates(&mut state, submit_index).unwrap();
            assert_eq!(state.completed_render_frames.len(), 1);
            assert_eq!(state.completed.len(), 1);
            assert_eq!(state.completed_catalogs.len(), 1);
            assert_eq!(state.completed_render_frames[0].submit_index, submit_index);
        }

        let mut state = STATE.get().unwrap().lock().unwrap();
        let proof = state.online_candidate_proofs.get(&hour).unwrap();
        assert_eq!(
            proof.purged_candidate_count,
            u32::try_from(candidate_count - 1).unwrap()
        );
        assert_eq!(proof.purged_receipt_count, proof.purged_candidate_count);
        assert_ne!(proof.candidate_rolling_sha256, [0; 32]);
        assert_ne!(proof.consumed_rolling_sha256, [0; 32]);

        let RawSelectedRenderFrameDrain::Ready(selected) =
            drain_selected_render_frame_submit_capture_locked(
                &mut state,
                hour,
                1_000 + candidate_count as u64,
                10_000_003,
            )
            .unwrap()
        else {
            panic!("compacted current hour did not retain its latest candidate");
        };
        assert_eq!(
            selected.receipt.candidate_count,
            u32::try_from(candidate_count).unwrap()
        );
        assert_eq!(
            selected.receipt.discarded_candidate_count,
            u32::try_from(candidate_count - 1).unwrap()
        );
        assert_eq!(
            selected.selected.frame.submit_index,
            10_000 + candidate_count as u64 - 1
        );
        assert!(selected.consumed_unselected.is_empty());
        assert_eq!(selected.receipt.candidate_set_sha256.len(), 64);
        assert_eq!(selected.receipt.consumed_unselected_sha256.len(), 64);
        assert!(state.completed_render_frames.is_empty());
        assert!(state.completed.is_empty());
        assert!(state.completed_catalogs.is_empty());
        assert!(!state.online_candidate_proofs.contains_key(&hour));
    }

    #[test]
    fn delayed_runtime_publication_folds_only_contiguous_present_prefix_in_exact_order() {
        let _guard = TEST_LOCK.lock().unwrap();
        let hour = test_game_hour(1_002);

        let run = |publication_order: [u64; 3]| {
            reset_frame_state();
            for (submit_index, qpc) in [(41, 100), (42, 200), (43, 300)] {
                record_empty_frame_for_hour(submit_index, qpc, hour);
            }
            let mut state = STATE.get().unwrap().lock().unwrap();
            for submit_index in publication_order {
                enqueue_empty_runtime(&mut state, submit_index);
                compact_published_current_hour_candidates(&mut state, submit_index).unwrap();
                if submit_index != 41 && publication_order[0] != 41 {
                    assert!(state.online_candidate_proofs.get(&hour).is_none());
                }
            }
            assert_eq!(
                state
                    .completed_render_frames
                    .iter()
                    .map(|frame| frame.submit_index)
                    .collect::<Vec<_>>(),
                vec![43]
            );
            state.online_candidate_proofs.get(&hour).unwrap().clone()
        };

        let delayed = run([42, 43, 41]);
        let in_order = run([41, 42, 43]);
        assert_eq!(delayed, in_order);
        assert_eq!(delayed.purged_candidate_count, 2);
        assert_eq!(delayed.last_purged_order, Some((200, 1, 2)));
    }

    #[test]
    fn atomic_cutoff_retains_pre_cutoff_gap_and_purges_post_cutoff_present() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_frame_state();
        let hour = test_game_hour(1_003);
        record_empty_frame_for_hour(41, 100, hour);
        let cutoff = SealedRenderHour {
            game_hour_raw: hour,
            cutoff_qpc_ticks: 150,
            cutoff_qpc_frequency_hz: 10_000_003,
        };
        {
            let mut state = STATE.get().unwrap().lock().unwrap();
            assert!(matches!(
                latch_selected_render_frame_submit_capture_locked(&mut state, cutoff).unwrap(),
                RawSelectedRenderFrameDrain::PendingIncomplete { .. }
            ));
            assert_eq!(state.latched_render_hour, Some(cutoff));
        }

        record_empty_frame_for_hour(42, 200, hour);
        {
            let mut state = STATE.get().unwrap().lock().unwrap();
            enqueue_empty_runtime(&mut state, 42);
            purge_published_sealed_render_submit(&mut state, 42).unwrap();
            assert!(state
                .completed_render_frames
                .iter()
                .all(|frame| frame.submit_index != 42));
            enqueue_empty_runtime(&mut state, 41);
            purge_published_sealed_render_submit(&mut state, 41).unwrap();
            compact_published_current_hour_candidates(&mut state, 41).unwrap();
            let RawSelectedRenderFrameDrain::Ready(selected) =
                latch_selected_render_frame_submit_capture_locked(&mut state, cutoff).unwrap()
            else {
                panic!("atomic cutoff did not retain its pre-cutoff candidate");
            };
            assert_eq!(selected.selected.frame.submit_index, 41);
            assert_eq!(selected.receipt.candidate_count, 1);
            assert_eq!(state.sealed_render_hour, Some(cutoff));
            assert_eq!(state.latched_render_hour, None);
            assert!(state.completed_render_frames.is_empty());
            assert!(state.completed.is_empty());
            assert!(state.completed_catalogs.is_empty());
        }
    }

    #[test]
    fn present_receipt_cannot_publish_across_an_already_completed_cutoff_latch() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_frame_state();
        let hour = test_game_hour(1_004);
        begin_render_frame(41).unwrap();
        record_boundary(
            RenderOrderBoundary::MapSceneBegin,
            MAP_SCENE_RENDER_ROOT_RVA,
        );
        record_boundary(RenderOrderBoundary::MapSceneEnd, MAP_SCENE_RENDER_ROOT_RVA);
        record_boundary(
            RenderOrderBoundary::PostProcessBegin,
            POST_PROCESS_RENDER_ROOT_RVA,
        );
        record_boundary(
            RenderOrderBoundary::PostProcessEnd,
            POST_PROCESS_RENDER_ROOT_RVA,
        );
        record_boundary(RenderOrderBoundary::PresentBegin, PRESENT_RENDER_ROOT_RVA);
        record_boundary(RenderOrderBoundary::PresentEnd, PRESENT_RENDER_ROOT_RVA);
        let cutoff = SealedRenderHour {
            game_hour_raw: hour,
            cutoff_qpc_ticks: 150,
            cutoff_qpc_frequency_hz: 10_000_003,
        };
        {
            let mut state = STATE.get().unwrap().lock().unwrap();
            assert!(matches!(
                latch_selected_render_frame_submit_capture_locked(&mut state, cutoff).unwrap(),
                RawSelectedRenderFrameDrain::EmptyNoCandidate { .. }
            ));
        }
        complete_render_frame_with_receipt(PresentationQpcReceipt {
            global_order: NEXT_PRESENT_GLOBAL_ORDER.fetch_add(1, Ordering::AcqRel),
            ticks: 100,
            frequency_hz: 10_000_003,
            process_start_nonce: 71,
            hook_epoch_nonce: 73,
            game_hour_raw: hour,
        })
        .unwrap();
        let mut state = STATE.get().unwrap().lock().unwrap();
        enqueue_empty_runtime(&mut state, 41);
        assert!(purge_published_sealed_render_submit(&mut state, 41).is_err());
    }

    #[test]
    fn paused_sealed_hour_runtime_first_frames_purge_beyond_submit_bound() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_frame_state();
        let hour = test_game_hour(1_005);
        {
            let mut state = STATE.get().unwrap().lock().unwrap();
            state.sealed_render_hour = Some(SealedRenderHour {
                game_hour_raw: hour,
                cutoff_qpc_ticks: 100,
                cutoff_qpc_frequency_hz: 10_000_003,
            });
        }
        let purge_count = MAX_SUBMITS + 257;
        for ordinal in 0..purge_count {
            let submit_index = 20_000 + ordinal as u64;
            {
                let mut state = STATE.get().unwrap().lock().unwrap();
                enqueue_empty_runtime(&mut state, submit_index);
            }
            record_empty_frame_for_hour(submit_index, 101 + ordinal as u64, hour);
            let state = STATE.get().unwrap().lock().unwrap();
            assert!(state.completed_render_frames.is_empty());
            assert!(state.completed.is_empty());
            assert!(state.completed_catalogs.is_empty());
        }
        let state = STATE.get().unwrap().lock().unwrap();
        assert_eq!(state.sealed_purge_receipt_count, purge_count as u128);
        assert_ne!(state.sealed_purge_rolling_sha256, [0; 32]);
    }

    #[test]
    fn targeted_hour_drain_reports_pending_without_consuming_gap_rows() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_frame_state();
        let hour = test_game_hour(1_000);
        record_empty_frame(41, 100);
        set_last_frame_hour(hour);
        let mut state = STATE.get().unwrap().lock().unwrap();
        let before = state.completed_render_frames.clone();
        assert_eq!(
            drain_selected_render_frame_submit_capture_locked(&mut state, hour, 350, 10_000_003,)
                .unwrap(),
            RawSelectedRenderFrameDrain::PendingIncomplete {
                publication_generation: 0,
                target_frame_count: 1,
                target_ready_count: 0,
            }
        );
        assert_eq!(state.completed_render_frames, before);
        assert!(state.completed.is_empty());
        assert!(state.completed_catalogs.is_empty());
    }

    #[test]
    fn targeted_hour_drain_ignores_unrelated_runtime_only_rows() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_frame_state();
        let hour = test_game_hour(1_000);
        record_empty_frame(41, 100);
        set_last_frame_hour(hour);
        let mut state = STATE.get().unwrap().lock().unwrap();
        enqueue_empty_runtime(&mut state, 40);
        enqueue_empty_runtime(&mut state, 41);
        let RawSelectedRenderFrameDrain::Ready(selected) =
            drain_selected_render_frame_submit_capture_locked(&mut state, hour, 350, 10_000_003)
                .unwrap()
        else {
            panic!("unrelated runtime-only row blocked target selection");
        };
        assert_eq!(selected.selected.frame.submit_index, 41);
        assert_eq!(state.completed.len(), 1);
        assert_eq!(state.completed[0].submit_index, 40);
        assert_eq!(state.completed_catalogs.len(), 1);
        assert_eq!(state.completed_catalogs[0].submit_index, 40);
    }

    #[test]
    fn unframed_present_is_not_a_capture_candidate() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_frame_state();
        assert!(!has_pending_render_frame());
        assert_eq!(
            STATE
                .get()
                .unwrap()
                .lock()
                .unwrap()
                .render_order_event_overflows,
            0
        );
        begin_render_frame(41).unwrap();
        assert!(has_pending_render_frame());
    }

    #[test]
    fn bounded_publication_wait_distinguishes_advance_and_timeout_without_drain() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_frame_state();
        {
            let mut state = STATE.get().unwrap().lock().unwrap();
            enqueue_empty_runtime(&mut state, 41);
            state.runtime_publication_generation = 9;
        }
        INSTALLED.store(true, Ordering::Release);
        assert_eq!(
            wait_for_render_submit_publication_timeout(8, Duration::ZERO).unwrap(),
            RenderSubmitPublicationWait::PublicationAdvanced {
                publication_generation: 9,
            }
        );
        assert_eq!(
            wait_for_render_submit_publication_timeout(9, Duration::ZERO).unwrap(),
            RenderSubmitPublicationWait::TimedOut {
                publication_generation: 9,
            }
        );
        INSTALLED.store(false, Ordering::Release);
        let state = STATE.get().unwrap().lock().unwrap();
        assert_eq!(state.completed.len(), 1);
        assert_eq!(state.completed_catalogs.len(), 1);
    }

    #[test]
    fn staged_hook_lifecycle_is_fail_closed_and_first_requires_ready() {
        let _guard = TEST_LOCK.lock().unwrap();
        let mut stage = HookStage::Bootstrap;
        assert!(!stage.admits_first_creation());
        stage.begin_staging().unwrap();
        assert_eq!(stage, HookStage::Installing);
        assert!(!stage.admits_first_creation());
        assert_eq!(
            stage.begin_staging(),
            Err("recursive game-device hook staging")
        );
        stage.finish_staging(true);
        assert!(stage.admits_first_creation());
        assert_eq!(
            stage.begin_staging(),
            Err("game-device hook staging already completed")
        );

        let mut failed = HookStage::Bootstrap;
        failed.begin_staging().unwrap();
        failed.finish_staging(false);
        assert_eq!(failed, HookStage::Failed);
        assert!(!failed.admits_first_creation());
        assert_eq!(
            failed.begin_staging(),
            Err("game-device hook staging previously failed")
        );

        let mut epoch = EarlyCaptureEpochState::default();
        epoch.diagnostic_stage_probe = true;
        *EARLY_CAPTURE_EPOCH
            .get_or_init(|| Mutex::new(EarlyCaptureEpochState::default()))
            .lock()
            .unwrap() = epoch;
        assert_eq!(
            commit_capture_receipt(1, 2, 3, &"ab".repeat(32)),
            Err("diagnostic staged-hook probes cannot commit capture receipts".into())
        );
        *EARLY_CAPTURE_EPOCH.get().unwrap().lock().unwrap() = EarlyCaptureEpochState::default();
    }

    #[test]
    fn staged_hook_inventory_and_partial_original_rollback_are_exact() {
        static ORIGINAL_A: AtomicUsize = AtomicUsize::new(0);
        static ORIGINAL_B: AtomicUsize = AtomicUsize::new(0);
        static ORIGINAL_C: AtomicUsize = AtomicUsize::new(0);
        let specs: [HookSpec; 3] = [
            (1, ptr::null_mut(), &ORIGINAL_A),
            (2, ptr::null_mut(), &ORIGINAL_B),
            (3, ptr::null_mut(), &ORIGINAL_C),
        ];
        ORIGINAL_A.store(0x11, Ordering::Release);
        ORIGINAL_B.store(0x22, Ordering::Release);
        ORIGINAL_C.store(0x33, Ordering::Release);
        clear_created_originals(&specs, 2);
        assert_eq!(ORIGINAL_A.load(Ordering::Acquire), 0);
        assert_eq!(ORIGINAL_B.load(Ordering::Acquire), 0);
        assert_eq!(ORIGINAL_C.load(Ordering::Acquire), 0x33);
        ORIGINAL_C.store(0, Ordering::Release);

        assert_eq!(engine_hook_specs(addresses(0)).len(), ENGINE_HOOK_COUNT);
        assert_eq!(BOOTSTRAP_HOOK_COUNT, 2);
        assert_eq!(HARDWARE_D3D_HOOK_COUNT, 19);
        assert_eq!(STAGED_HOOK_COUNT, 66);
        assert_eq!(READY_HOOK_COUNT, 68);
        assert_eq!(staged_hook_count_for_group_mask(0x01), Some(19));
        assert_eq!(staged_hook_count_for_group_mask(0x03), Some(21));
        // Every staged hook except the three camera-event taps is in the
        // low nine groups.  The four O05 hooks replaced four COM-owned G2
        // hooks in the fixed 66-hook inventory, but they are entity-owned and
        // therefore remain selected by 0x1ff.
        assert_eq!(staged_hook_count_for_group_mask(0x1ff), Some(63));
        assert_eq!(staged_hook_count_for_group_mask(0x3ff), Some(66));
        assert_eq!(staged_hook_count_for_group_mask(0x02), None);
        assert_eq!(staged_hook_count_for_group_mask(0x101), Some(21));
        assert_eq!(staged_hook_count_for_group_mask(0x201), Some(22));
        assert_eq!(
            parse_diagnostic_stage_mask(None).unwrap(),
            StagedHookSelection {
                diagnostic_stage_probe: false,
                group_mask: PRODUCTION_STAGED_HOOK_GROUP_MASK,
            }
        );
        assert_eq!(
            parse_diagnostic_stage_mask(Some("0x01")).unwrap(),
            StagedHookSelection {
                diagnostic_stage_probe: true,
                group_mask: STAGED_HOOK_GROUP_COM,
            }
        );
        assert_eq!(
            parse_diagnostic_stage_mask(Some("0x1ff")).unwrap(),
            StagedHookSelection {
                diagnostic_stage_probe: true,
                group_mask: 0x1ff,
            }
        );
        assert!(parse_diagnostic_stage_mask(Some("0x3ff")).is_err());
        assert!(parse_diagnostic_stage_mask(Some("0x02")).is_err());
    }

    #[test]
    fn staged_hook_receipt_binds_count_order_targets_and_device_identity() {
        let targets: Vec<_> = (0..STAGED_HOOK_COUNT)
            .map(|index| 0x1000usize + index * 0x100)
            .collect();
        let labels = staged_hook_identity_labels();
        let receipt = staged_hook_set_receipt_sha256(
            7,
            11,
            13,
            &"ab".repeat(32),
            0x2000_0000,
            0x3000_0000,
            1,
            false,
            PRODUCTION_STAGED_HOOK_GROUP_MASK,
            &labels,
            &targets,
        )
        .unwrap();
        assert_eq!(receipt.len(), 64);
        assert_eq!(
            receipt,
            staged_hook_set_receipt_sha256(
                7,
                11,
                13,
                &"ab".repeat(32),
                0x2000_0000,
                0x3000_0000,
                1,
                false,
                PRODUCTION_STAGED_HOOK_GROUP_MASK,
                &labels,
                &targets,
            )
            .unwrap()
        );

        let mut reordered = targets.clone();
        reordered.swap(0, 1);
        assert_ne!(
            receipt,
            staged_hook_set_receipt_sha256(
                7,
                11,
                13,
                &"ab".repeat(32),
                0x2000_0000,
                0x3000_0000,
                1,
                false,
                PRODUCTION_STAGED_HOOK_GROUP_MASK,
                &labels,
                &reordered,
            )
            .unwrap()
        );
        assert_ne!(
            receipt,
            staged_hook_set_receipt_sha256(
                7,
                11,
                13,
                &"ab".repeat(32),
                0x2000_0001,
                0x3000_0000,
                1,
                false,
                PRODUCTION_STAGED_HOOK_GROUP_MASK,
                &labels,
                &targets,
            )
            .unwrap()
        );

        let mut duplicated = targets.clone();
        duplicated[1] = duplicated[0];
        assert!(staged_hook_set_receipt_sha256(
            7,
            11,
            13,
            &"ab".repeat(32),
            0x2000_0000,
            0x3000_0000,
            1,
            false,
            PRODUCTION_STAGED_HOOK_GROUP_MASK,
            &labels,
            &duplicated,
        )
        .is_err());
        assert!(staged_hook_set_receipt_sha256(
            7,
            11,
            13,
            &"ab".repeat(32),
            0x2000_0000,
            0x3000_0000,
            1,
            false,
            PRODUCTION_STAGED_HOOK_GROUP_MASK,
            &labels[..labels.len() - 1],
            &targets[..targets.len() - 1],
        )
        .is_err());

        let diagnostic_targets = &targets[..HARDWARE_D3D_HOOK_COUNT];
        let diagnostic_labels = &labels[..HARDWARE_D3D_HOOK_COUNT];
        let diagnostic_receipt = staged_hook_set_receipt_sha256(
            7,
            11,
            13,
            &"ab".repeat(32),
            0x2000_0000,
            0x3000_0000,
            1,
            true,
            STAGED_HOOK_GROUP_COM,
            diagnostic_labels,
            diagnostic_targets,
        )
        .unwrap();
        assert_eq!(diagnostic_receipt.len(), 64);
        assert_ne!(receipt, diagnostic_receipt);
    }

    #[test]
    fn runtime_dlc_vector_layout_and_raw_name_receipt_are_frozen() {
        assert_eq!(mem::size_of::<RuntimeDlcStringVector>(), 0x20);
        assert_eq!(mem::offset_of!(RuntimeDlcStringVector, data), 0x00);
        assert_eq!(mem::offset_of!(RuntimeDlcStringVector, capacity), 0x08);
        assert_eq!(mem::offset_of!(RuntimeDlcStringVector, count), 0x0c);
        assert_eq!(mem::offset_of!(RuntimeDlcStringVector, allocator), 0x10);
        assert_eq!(mem::offset_of!(RuntimeDlcStringVector, allocator_aux), 0x18);

        let names = vec!["Expansion Pass 1".to_owned(), "Modern Armor".to_owned()];
        assert_eq!(
            runtime_dlc_raw_name_roster_sha256(&names).unwrap(),
            "77b6b4af72b76d9d8daa6c693f3b4dbcfb8a2569bdeb0de7ed19e988bf1d76f3"
        );
        assert_eq!(
            runtime_dlc_raw_name_roster_sha256(&[]).unwrap(),
            "7c5b297d06f4143cf7183489c7ad4335e85fad47348ec75306a79089223992ec"
        );
        let mut reordered = names.clone();
        reordered.swap(0, 1);
        assert_ne!(
            runtime_dlc_raw_name_roster_sha256(&names).unwrap(),
            runtime_dlc_raw_name_roster_sha256(&reordered).unwrap()
        );
        assert!(runtime_dlc_raw_name_roster_sha256(&[names[0].clone(), names[0].clone()]).is_err());
        assert!(runtime_dlc_raw_name_roster_sha256(&["bad\nname".to_owned()]).is_err());
    }
}
