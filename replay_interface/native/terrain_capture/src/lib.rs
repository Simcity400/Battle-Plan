#![deny(unsafe_op_in_unsafe_fn)]

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Map as JsonMap, Value};

#[cfg(windows)]
mod d3d11_runtime;

pub const TERRAIN_ID_SRV_SLOT: u32 = 5;
pub const TERRAIN_ID_DXGI_FORMAT: u32 = DXGI_FORMAT_R8G8B8A8_UNORM;
pub const TERRAIN_ID_RESOURCE_SYMBOL: &str = "TerrainIDMap";
pub const SNOW_MUD_DATA_RESOURCE_SYMBOL: &str = "SnowMudData";
pub const PDXMAP_RUNTIME_CAPTURE_RESOURCE_SYMBOLS: [&str; 2] =
    [TERRAIN_ID_RESOURCE_SYMBOL, SNOW_MUD_DATA_RESOURCE_SYMBOL];
pub const WATER_HEIGHT_TEXTURE_SYMBOL: &str = "HeightTexture";
pub const WATER_LEAN_TEXTURE1_SYMBOL: &str = "LeanTexture1";
pub const WATER_LEAN_TEXTURE2_SYMBOL: &str = "LeanTexture2";
pub const WATER_SPECULAR_MAP_SYMBOL: &str = "SpecularMap";
pub const WATER_REFRACTION_RESOURCE_SYMBOL: &str = "WaterRefraction";
pub const PDXWATER_REQUIRED_RESOURCE_SYMBOLS: [&str; 5] = [
    WATER_HEIGHT_TEXTURE_SYMBOL,
    WATER_LEAN_TEXTURE1_SYMBOL,
    WATER_LEAN_TEXTURE2_SYMBOL,
    WATER_SPECULAR_MAP_SYMBOL,
    WATER_REFRACTION_RESOURCE_SYMBOL,
];
pub const RIVER_DATA_RESOURCE_SYMBOL: &str = "RiverData";
/// `gfx/FX/river.shader:85` declares `SnowMudTexture`, NOT pdxmap's
/// `SnowMudData` — the 2026-07-02 session proved rivers rendered for minutes
/// without ever matching the old `SnowMudData`-based signature (shader-cache
/// entry `ps_5_0/0008000014C21000EB884362.bin` confirms the declared set).
pub const RIVER_SNOW_MUD_TEXTURE_SYMBOL: &str = "SnowMudTexture";
pub const RIVER_REQUIRED_RESOURCE_SYMBOLS: [&str; 6] = [
    RIVER_DATA_RESOURCE_SYMBOL,
    WATER_LEAN_TEXTURE1_SYMBOL,
    WATER_LEAN_TEXTURE2_SYMBOL,
    RIVER_SNOW_MUD_TEXTURE_SYMBOL,
    "GradientBorderChannel1",
    "GradientBorderChannel2",
];
pub const BORDER_DIFFUSE_RESOURCE_SYMBOL: &str = "BorderDiffuse";
/// BORD5 (2026-07-07): `BorderDiffuse` ALONE. The old 4-symbol requirement
/// (`+ SnowMudData + IntelMap + ShadowMap`) was read off a border.shader
/// define variant the engine never compiles: the REAL compiled
/// `gfx/FX/border.shader:border` PS (5248 bytes, byte-compiled via OFF-3)
/// declares only `BorderDiffuse_*` and `ShadowMap_*`, so the detector could
/// NEVER match — which is how the engine's border vertex shaders (two 3284-
/// byte variants observed in session_BORDERZOOM_1/2, full RDEF/ISGN string
/// match to our compile) went misfiled under the generic map family and
/// BORD4 wrongly concluded "border.shader is never used". `BorderDiffuse` is
/// SOUND alone: no other shipped gfx/FX effect declares that texture symbol.
pub const BORDER_REQUIRED_RESOURCE_SYMBOLS: [&str; 1] = [BORDER_DIFFUSE_RESOURCE_SYMBOL];
/// `pdxmap.shader` `Effect underwater` → `PixelShaderUnderwater` (lines 407-454).
/// The underwater seafloor-refraction PS samples exactly these six textures (the
/// same slots the OFF-3/WATER2 byte-exact recompile of `PixelShaderUnderwater`
/// reflects — `map_viewer/src/water_law.rs::UNDERWATER_PS_REQUIRED_SRV_SLOTS`:
/// `TerrainDiffuse`(0), `HeightNormal`(1), `TerrainColorTint`(2),
/// `LightIndexMap`(10), `LightDataMap`(11), `ShadowMap`(12)). A genuine
/// underwater PS declares ALL six in its DXBC string table (`tex2D`/`GetShadow`/
/// `CalculatePointLights` reference every one), so requiring their presence never
/// false-rejects the real shader.
pub const UNDERWATER_REQUIRED_RESOURCE_SYMBOLS: [&str; 6] = [
    "TerrainDiffuse",
    "HeightNormal",
    "TerrainColorTint",
    "LightIndexMap",
    "LightDataMap",
    "ShadowMap",
];
/// The DISCRIMINATOR marker that separates `PixelShaderUnderwater` from the main
/// terrain `PixelShaderTerrain`. The main terrain PS samples `TerrainIDMap`
/// (pdxmap.shader:239 `calculate_map_tex_index( tex2D( TerrainIDMap, … ) )`);
/// `PixelShaderUnderwater` does NOT (it reads `TerrainDiffuse` directly at a fixed
/// 32-world-px repeat, never via the ID indirection). So an underwater PS is a PS
/// that binds all of [`UNDERWATER_REQUIRED_RESOURCE_SYMBOLS`] AND does not bind
/// `TerrainIDMap`. This also rejects `PixelShaderTerrainUnlit` (which omits the
/// `LightIndexMap`/`LightDataMap` point-light slots this set requires).
pub const UNDERWATER_EXCLUDED_RESOURCE_SYMBOL: &str = TERRAIN_ID_RESOURCE_SYMBOL;

pub const TREE_MASK_RESOURCE_SYMBOL: &str = "TreeMaskTexture";
pub const TREE_REQUIRED_RESOURCE_SYMBOLS: [&str; 7] = [
    "DiffuseMap",
    "NormalMap",
    "TintMap",
    "SeasonMap",
    TREE_MASK_RESOURCE_SYMBOL,
    "GradientBorderChannel1",
    "GradientBorderChannel2",
];
pub const RESTORE_MAIN_SCENE_RESOURCE_SYMBOL: &str = "MainScene";
pub const RESTORE_BLOOM_RESOURCE_SYMBOL: &str = "RestoreBloom";
pub const RESTORE_COLOR_CUBE_RESOURCE_SYMBOL: &str = "ColorCube";
pub const RESTORE_AVERAGE_LUMINANCE_RESOURCE_SYMBOL: &str = "AverageLuminanceTexture";
pub const RESTORE_REQUIRED_RESOURCE_SYMBOLS: [&str; 4] = [
    RESTORE_MAIN_SCENE_RESOURCE_SYMBOL,
    RESTORE_BLOOM_RESOURCE_SYMBOL,
    RESTORE_COLOR_CUBE_RESOURCE_SYMBOL,
    RESTORE_AVERAGE_LUMINANCE_RESOURCE_SYMBOL,
];
pub const RESTORE_POSTEFFECT_BASE_CB_SOURCE: &str =
    "gfx/FX/posteffect_base.fxh:ConstantBuffer(1,32)";
pub const RESTORE_POSTEFFECT_BASE_CB_BIND_POINT: u32 = 1;

/// Distinctive family marker for the mapname (country-label) pass. Both the
/// mapname VS and PS read `Transp_OffsetX` from the slot-1 CB, so the symbol is
/// present in both shaders' DXBC string tables. It is far more distinctive than
/// the PS's single generic `DiffuseTexture`, so it is the marker that lets the
/// mapname PS survive the lock-free create-shader prefilter without weakening
/// the load-freeze protection (only the few `Transp_OffsetX`-bearing shaders —
/// mapname/particle/arrow-family — pay the reflection cost).
pub const MAPNAME_TRANSP_OFFSET_MARKER: &str = "Transp_OffsetX";

/// `gfx/FX/mapname.shader` (Effect `mapname`) draws the curved country-label
/// glyph quads. Its distinctive fingerprint is the VERTEX shader input
/// signature — `float3 vPosition : POSITION` + `float2 vTexCoord : TEXCOORD0`
/// — paired with `ConstantBuffer(1,32){ float4 Transp_OffsetX; }` at bind
/// point 1. That float3-world-position input is the whole point of the pass:
/// it is the engine's byte ground truth for the label placement law. The PS
/// samples a single font-atlas `DiffuseTexture` at slot 0 and reads
/// `Transp_OffsetX.x` (transparency) from the same CB1; the VS reads
/// `Transp_OffsetX.y` (wrap offset) and `ViewProjectionMatrix`/`vCamLookAtDir`
/// from the scene CB at slot 0.
pub const MAPNAME_DIFFUSE_TEXTURE_RESOURCE_SYMBOL: &str = "DiffuseTexture";
pub const MAPNAME_TRANSP_OFFSET_CB_SOURCE: &str = "gfx/FX/mapname.shader:ConstantBuffer(1,32)";
pub const MAPNAME_TRANSP_OFFSET_CB_BIND_POINT: u32 = 1;
pub const MAPNAME_SCENE_CB_BIND_POINT: u32 = 0;
/// Corroborating hash from live session evidence. NOT a gate — reflection is
/// the primary discriminator; this only annotates events so an operator can
/// confirm the reflected match is the same VS seen in prior censuses.
pub const MAPNAME_KNOWN_VERTEX_SHADER_FNV1A64: &str = "4e1b73f39c128df0";

/// One distinctive, always-required resource-name marker per capture family.
/// Each marker is a member of its family's `*_REQUIRED_RESOURCE_SYMBOLS`, so a
/// genuine family pixel shader ALWAYS declares it. If a shader's bytecode
/// contains none of these markers it cannot satisfy any family's required set
/// and matches no family — a SOUND (never-false-reject) result.
///
/// This exists to keep the game's map-load shader-creation storm off the global
/// capture lock: [`pixel_shader_bytecode_could_be_capture_family`] rejects the
/// ~all non-map shaders created during load with a cheap byte scan, before any
/// expensive D3D reflection or lock acquisition (2026-07-02 load-freeze fix).
pub const PIXEL_SHADER_CAPTURE_FAMILY_MARKERS: [&str; 8] = [
    TERRAIN_ID_RESOURCE_SYMBOL,         // pdxmap terrain
    WATER_REFRACTION_RESOURCE_SYMBOL,   // pdxwater
    RIVER_DATA_RESOURCE_SYMBOL,         // river
    BORDER_DIFFUSE_RESOURCE_SYMBOL,     // border
    TREE_MASK_RESOURCE_SYMBOL,          // tree
    RESTORE_COLOR_CUBE_RESOURCE_SYMBOL, // restorescene postfx restore
    MAPNAME_TRANSP_OFFSET_MARKER,       // mapname country-label glyph pass
    // `PixelShaderUnderwater` binds none of the above markers (notably NOT
    // `TerrainIDMap`, which is exactly what separates it from the main terrain
    // PS). It always declares `HeightNormal` (`tex2D( HeightNormal, … )`,
    // pdxmap.shader:413), a required member of its resource set, so this marker
    // lets the underwater PS survive the lock-free prefilter. `HeightNormal` is
    // a map-family texture (pdxmap terrain / underwater / terrainunlit), so
    // admitting it costs reflection on only those few shaders; the exact
    // classifier (`underwater_resource_slots_from_shader_bytecode`) still gates
    // the family, so this stays a SOUND over-approximation.
    "HeightNormal", // underwater seafloor-refraction pass
];

pub const DXGI_FORMAT_R32G32B32A32_FLOAT: u32 = 2;
pub const DXGI_FORMAT_R8G8B8A8_UNORM: u32 = 28;
pub const DXGI_FORMAT_B8G8R8A8_UNORM: u32 = 87;
pub const DXGI_FORMAT_B8G8R8A8_UNORM_SRGB: u32 = 91;
pub const DXGI_FORMAT_R32G32B32A32_SINT: u32 = 4;
pub const DXGI_FORMAT_R16G16B16A16_SINT: u32 = 14;
pub const DXGI_FORMAT_R32G32_SINT: u32 = 18;
pub const DXGI_FORMAT_R8G8B8A8_SINT: u32 = 32;
pub const DXGI_FORMAT_R16G16_SNORM: u32 = 37;
pub const DXGI_FORMAT_R16G16_SINT: u32 = 38;
pub const DXGI_FORMAT_R8G8_SINT: u32 = 52;
pub const D3D11_USAGE_DEFAULT: u32 = 0;
pub const D3D11_USAGE_STAGING: u32 = 3;
pub const D3D11_BIND_VERTEX_BUFFER: u32 = 0x1;
pub const D3D11_BIND_CONSTANT_BUFFER: u32 = 0x4;
pub const D3D11_BIND_SHADER_RESOURCE: u32 = 0x8;
pub const D3D11_CPU_ACCESS_READ: u32 = 0x20000;
pub const D3D11_MAP_READ: u32 = 1;
pub const D3D11_INPUT_PER_VERTEX_DATA: u32 = 0;
pub const D3D11_APPEND_ALIGNED_ELEMENT: u32 = 0xFFFF_FFFF;
pub const D3D11_IA_VERTEX_INPUT_STRUCTURE_ELEMENT_COUNT: u32 = 32;
pub const D3D_REGISTER_COMPONENT_SINT32: u32 = 2;
pub const D3D_REGISTER_COMPONENT_FLOAT32: u32 = 3;
pub const TERRAIN_VERTEX_CAPTURE_SAMPLE_BYTES: usize = 64 * 1024;
pub const TERRAIN_INPUT_LAYOUT_SHADER_SOURCE: &str =
    "gfx/FX/pdxmap.shader:VS_INPUT_TERRAIN_NOTEXTURE:int4 position + int2 height";
pub const TERRAIN_CAPTURE_MOMENT: &str = "terrain_active_pass";
pub const TERRAIN_CHUNK_LIBRARY_MOMENT: &str = "terrain_chunk_library";
pub const WATER_CAPTURE_MOMENT: &str = "water_pass";
pub const TERRAIN_SCENE_CB0_SYMBOL: &str = "TerrainSceneCB0";
pub const TERRAIN_SCENE_CB0_BIND_POINT: u32 = 0;
pub const TERRAIN_SCENE_CB0_FILE: &str = "terrain_scene_cb0.bin";
pub const TERRAIN_SCENE_CB0_META_FILE: &str = "terrain_scene_cb0_capture.json";
pub const TERRAIN_DRAW_CB1_LAYOUT_SOURCE: &str = "gfx/FX/pdxmap.fxh:ConstantBuffer(1,32)";
pub const TERRAIN_DRAW_CB1_BIND_POINT: u32 = 1;
pub const PDXMAP_FXH_SOURCE: &str = "gfx/FX/pdxmap.fxh";
pub const MAP_FRAME_BUNDLE_FILE: &str = "map_frame_bundle.json";
pub const WATER_MAP_FRAME_BUNDLE_FILE: &str = "map_frame_water_pass_bundle.json";

pub const IUNKNOWN_QUERY_INTERFACE_INDEX: usize = 0;
pub const IUNKNOWN_ADD_REF_INDEX: usize = 1;
pub const IUNKNOWN_RELEASE_INDEX: usize = 2;
pub const SWAP_CHAIN_GET_DEVICE_INDEX: usize = 7;
pub const SWAP_CHAIN_PRESENT_INDEX: usize = 8;
pub const SWAP_CHAIN_RESIZE_BUFFERS_INDEX: usize = 13;
pub const DEVICE_CREATE_BUFFER_INDEX: usize = 3;
pub const DEVICE_CREATE_TEXTURE2D_INDEX: usize = 5;
pub const DEVICE_CREATE_SHADER_RESOURCE_VIEW_INDEX: usize = 7;
pub const DEVICE_CREATE_INPUT_LAYOUT_INDEX: usize = 11;
pub const DEVICE_CREATE_VERTEX_SHADER_INDEX: usize = 12;
pub const DEVICE_CREATE_PIXEL_SHADER_INDEX: usize = 15;
pub const CONTEXT_VS_SET_CONSTANT_BUFFERS_INDEX: usize = 7;
pub const CONTEXT_PS_SET_SHADER_RESOURCES_INDEX: usize = 8;
pub const CONTEXT_PS_SET_SHADER_INDEX: usize = 9;
pub const CONTEXT_VS_SET_SHADER_INDEX: usize = 11;
pub const CONTEXT_DRAW_INDEXED_INDEX: usize = 12;
pub const CONTEXT_DRAW_INDEX: usize = 13;
pub const CONTEXT_MAP_INDEX: usize = 14;
pub const CONTEXT_UNMAP_INDEX: usize = 15;
pub const CONTEXT_PS_SET_CONSTANT_BUFFERS_INDEX: usize = 16;
pub const CONTEXT_IA_SET_INPUT_LAYOUT_INDEX: usize = 17;
pub const CONTEXT_IA_SET_VERTEX_BUFFERS_INDEX: usize = 18;
pub const CONTEXT_DRAW_INDEXED_INSTANCED_INDEX: usize = 20;
pub const CONTEXT_DRAW_INSTANCED_INDEX: usize = 21;
pub const CONTEXT_OM_SET_RENDER_TARGETS_INDEX: usize = 33;
pub const CONTEXT_OM_SET_RENDER_TARGETS_AND_UAVS_INDEX: usize = 34;
pub const CONTEXT_OM_SET_BLEND_STATE_INDEX: usize = 35;
pub const CONTEXT_OM_SET_DEPTH_STENCIL_STATE_INDEX: usize = 36;
pub const CONTEXT_COPY_SUBRESOURCE_REGION_INDEX: usize = 46;
pub const CONTEXT_COPY_RESOURCE_INDEX: usize = 47;
pub const CONTEXT_CLEAR_RENDER_TARGET_VIEW_INDEX: usize = 50;
pub const CONTEXT_CLEAR_DEPTH_STENCIL_VIEW_INDEX: usize = 53;
pub const CONTEXT_PS_GET_SHADER_RESOURCES_INDEX: usize = 73;
pub const CONTEXT_IA_GET_INPUT_LAYOUT_INDEX: usize = 78;
pub const CONTEXT_IA_GET_VERTEX_BUFFERS_INDEX: usize = 79;
pub const SHADER_RESOURCE_VIEW_GET_RESOURCE_INDEX: usize = 7;
pub const SHADER_RESOURCE_VIEW_GET_DESC_INDEX: usize = 8;
pub const TEXTURE2D_GET_DESC_INDEX: usize = 10;

const D3D11_TEXTURE2D_DESC_FIELD_NAMES: [&str; 11] = [
    "Width",
    "Height",
    "MipLevels",
    "ArraySize",
    "Format",
    "SampleDesc.Count",
    "SampleDesc.Quality",
    "Usage",
    "BindFlags",
    "CPUAccessFlags",
    "MiscFlags",
];

fn fnv1a64_bytes(bytes: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Texture2DDesc {
    pub width: u32,
    pub height: u32,
    pub mip_levels: u32,
    pub array_size: u32,
    pub format: u32,
    pub sample_count: u32,
    pub sample_quality: u32,
    pub usage: u32,
    pub bind_flags: u32,
    pub cpu_access_flags: u32,
    pub misc_flags: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BufferDesc {
    pub byte_width: u32,
    pub usage: u32,
    pub bind_flags: u32,
    pub cpu_access_flags: u32,
    pub misc_flags: u32,
    pub structure_byte_stride: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TerrainCaptureError {
    EmptyTexture,
    UnsupportedFormat(u32),
    UnsupportedArraySize(u32),
    UnsupportedSampleCount(u32),
    MissingShaderResourceBind,
    SourceCpuReadable,
    InvalidMappedPitch { row_pitch: usize, row_bytes: usize },
    MappedRowsTooShort { available: usize, required: usize },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TerrainVertexBufferError {
    EmptyBuffer,
    MissingVertexBufferBind,
    SourceCpuReadable,
    StrideTooSmall {
        stride: u32,
        minimum: u32,
    },
    StrideNotU32Aligned(u32),
    OffsetPastEnd {
        offset: u32,
        byte_width: u32,
    },
    FirstVertexPastEnd {
        offset: u32,
        stride: u32,
        byte_width: u32,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TerrainInputElementDesc {
    pub semantic_name: String,
    pub semantic_index: u32,
    pub format: u32,
    pub input_slot: u32,
    pub aligned_byte_offset: u32,
    pub input_slot_class: u32,
    pub instance_data_step_rate: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TerrainInputLayoutError {
    EmptyLayout,
    MissingElement {
        semantic_name: &'static str,
        semantic_index: u32,
    },
    WrongFormat {
        semantic_name: &'static str,
        semantic_index: u32,
        expected: &'static str,
        actual: u32,
    },
    WrongInputSlot {
        semantic_name: &'static str,
        semantic_index: u32,
        expected: u32,
        actual: u32,
    },
    AppendAlignedOffset {
        semantic_name: &'static str,
        semantic_index: u32,
    },
    WrongInputSlotClass {
        semantic_name: &'static str,
        semantic_index: u32,
        expected: u32,
        actual: u32,
    },
    WrongInstanceDataStepRate {
        semantic_name: &'static str,
        semantic_index: u32,
        expected: u32,
        actual: u32,
    },
    ElementOverrunsStride {
        semantic_name: &'static str,
        semantic_index: u32,
        offset: u32,
        byte_width: u32,
        stride: u32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerrainShaderBinding {
    pub pixel_shader: usize,
    pub terrain_id_srv_slot: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MapPixelShaderBinding {
    pub pixel_shader: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerrainVertexShaderBinding {
    pub vertex_shader: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerrainSrvBinding {
    pub srv: usize,
    pub slot: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PdxmapShaderMetadata {
    pub resource_slots: BTreeMap<String, u32>,
    pub constant_buffers: BTreeMap<String, u32>,
    pub byte_len: usize,
    pub fnv1a64: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ShaderInputParameter {
    pub semantic_name: String,
    pub semantic_index: u32,
    pub register: u32,
    pub component_type: u32,
    pub mask: u8,
    pub read_write_mask: u8,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TerrainVertexShaderMetadata {
    pub input_parameters: Vec<ShaderInputParameter>,
    pub constant_buffers: BTreeMap<String, u32>,
    pub byte_len: usize,
    pub fnv1a64: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TerrainCaptureState {
    pdxmap_pixel_shaders: BTreeMap<usize, PdxmapShaderMetadata>,
    pdxwater_pixel_shaders: BTreeMap<usize, PdxmapShaderMetadata>,
    river_pixel_shaders: BTreeMap<usize, PdxmapShaderMetadata>,
    border_pixel_shaders: BTreeMap<usize, PdxmapShaderMetadata>,
    underwater_pixel_shaders: BTreeMap<usize, PdxmapShaderMetadata>,
    tree_pixel_shaders: BTreeMap<usize, PdxmapShaderMetadata>,
    restore_pixel_shaders: BTreeMap<usize, PdxmapShaderMetadata>,
    mapname_pixel_shaders: BTreeMap<usize, PdxmapShaderMetadata>,
    map_vertex_shaders: BTreeMap<usize, TerrainVertexShaderMetadata>,
    terrain_vertex_shaders: BTreeMap<usize, TerrainVertexShaderMetadata>,
    mapname_vertex_shaders: BTreeMap<usize, TerrainVertexShaderMetadata>,
    current_pixel_shader: Option<usize>,
    current_vertex_shader: Option<usize>,
    shader_resource_srvs: BTreeMap<String, TerrainSrvBinding>,
    input_layouts: BTreeMap<usize, Vec<TerrainInputElementDesc>>,
    current_input_layout: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShaderCacheSignature {
    pub map_size_x: u32,
    pub map_size_y: u32,
    pub map_pow2_x_milli: u32,
    pub map_pow2_y_milli: u32,
    pub fow_pow2_x_milli: u32,
    pub fow_pow2_y_milli: u32,
    pub terrain_id_srv_slot: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadbackPlan {
    pub source_srv_slot: u32,
    pub source_format: u32,
    pub staging_desc: Texture2DDesc,
    pub map_type: u32,
    pub row_bytes: usize,
    pub total_bytes: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerrainVertexBufferReadbackPlan {
    pub staging_desc: BufferDesc,
    pub map_type: u32,
    pub captured_byte_len: usize,
    pub vertex_stride: u32,
    pub vertex_offset: u32,
}

impl TerrainCaptureState {
    pub fn observe_pixel_shader_created(&mut self, shader_ptr: usize, bytecode: &[u8]) -> bool {
        let Some(resource_slots) = pdxmap_resource_slots_from_shader_bytecode(bytecode) else {
            return false;
        };
        if shader_ptr == 0 {
            return false;
        }
        self.pdxmap_pixel_shaders.insert(
            shader_ptr,
            PdxmapShaderMetadata {
                resource_slots,
                constant_buffers: shader_constant_buffer_slots_from_shader_bytecode(bytecode),
                byte_len: bytecode.len(),
                fnv1a64: fnv1a64_bytes(bytecode),
            },
        );
        true
    }

    pub fn observe_water_pixel_shader_created(
        &mut self,
        shader_ptr: usize,
        bytecode: &[u8],
    ) -> bool {
        let Some(resource_slots) = pdxwater_resource_slots_from_shader_bytecode(bytecode) else {
            return false;
        };
        if shader_ptr == 0 {
            return false;
        }
        self.pdxwater_pixel_shaders.insert(
            shader_ptr,
            PdxmapShaderMetadata {
                resource_slots,
                constant_buffers: shader_constant_buffer_slots_from_shader_bytecode(bytecode),
                byte_len: bytecode.len(),
                fnv1a64: fnv1a64_bytes(bytecode),
            },
        );
        true
    }

    pub fn observe_river_pixel_shader_created(
        &mut self,
        shader_ptr: usize,
        bytecode: &[u8],
    ) -> bool {
        let Some(resource_slots) = river_resource_slots_from_shader_bytecode(bytecode) else {
            return false;
        };
        if shader_ptr == 0 {
            return false;
        }
        self.river_pixel_shaders.insert(
            shader_ptr,
            PdxmapShaderMetadata {
                resource_slots,
                constant_buffers: shader_constant_buffer_slots_from_shader_bytecode(bytecode),
                byte_len: bytecode.len(),
                fnv1a64: fnv1a64_bytes(bytecode),
            },
        );
        true
    }

    pub fn observe_border_pixel_shader_created(
        &mut self,
        shader_ptr: usize,
        bytecode: &[u8],
    ) -> bool {
        let Some(resource_slots) = border_resource_slots_from_shader_bytecode(bytecode) else {
            return false;
        };
        if shader_ptr == 0 {
            return false;
        }
        self.border_pixel_shaders.insert(
            shader_ptr,
            PdxmapShaderMetadata {
                resource_slots,
                constant_buffers: shader_constant_buffer_slots_from_shader_bytecode(bytecode),
                byte_len: bytecode.len(),
                fnv1a64: fnv1a64_bytes(bytecode),
            },
        );
        true
    }

    pub fn observe_underwater_pixel_shader_created(
        &mut self,
        shader_ptr: usize,
        bytecode: &[u8],
    ) -> bool {
        let Some(resource_slots) = underwater_resource_slots_from_shader_bytecode(bytecode) else {
            return false;
        };
        if shader_ptr == 0 {
            return false;
        }
        self.underwater_pixel_shaders.insert(
            shader_ptr,
            PdxmapShaderMetadata {
                resource_slots,
                constant_buffers: shader_constant_buffer_slots_from_shader_bytecode(bytecode),
                byte_len: bytecode.len(),
                fnv1a64: fnv1a64_bytes(bytecode),
            },
        );
        true
    }

    pub fn observe_tree_pixel_shader_created(
        &mut self,
        shader_ptr: usize,
        bytecode: &[u8],
    ) -> bool {
        let Some(resource_slots) = tree_resource_slots_from_shader_bytecode(bytecode) else {
            return false;
        };
        if shader_ptr == 0 {
            return false;
        }
        self.tree_pixel_shaders.insert(
            shader_ptr,
            PdxmapShaderMetadata {
                resource_slots,
                constant_buffers: shader_constant_buffer_slots_from_shader_bytecode(bytecode),
                byte_len: bytecode.len(),
                fnv1a64: fnv1a64_bytes(bytecode),
            },
        );
        true
    }

    pub fn observe_restore_pixel_shader_created(
        &mut self,
        shader_ptr: usize,
        bytecode: &[u8],
    ) -> bool {
        let Some(resource_slots) = restore_resource_slots_from_shader_bytecode(bytecode) else {
            return false;
        };
        if shader_ptr == 0 {
            return false;
        }
        let mut constant_buffers = shader_constant_buffer_slots_from_shader_bytecode(bytecode);
        constant_buffers
            .entry(RESTORE_POSTEFFECT_BASE_CB_SOURCE.to_string())
            .or_insert(RESTORE_POSTEFFECT_BASE_CB_BIND_POINT);
        self.restore_pixel_shaders.insert(
            shader_ptr,
            PdxmapShaderMetadata {
                resource_slots,
                constant_buffers,
                byte_len: bytecode.len(),
                fnv1a64: fnv1a64_bytes(bytecode),
            },
        );
        true
    }

    pub fn observe_mapname_pixel_shader_created(
        &mut self,
        shader_ptr: usize,
        bytecode: &[u8],
    ) -> bool {
        let Some(resource_slots) = mapname_pixel_resource_slots_from_shader_bytecode(bytecode)
        else {
            return false;
        };
        // The texture-slot shape alone is weak (a generic font-atlas PS). The
        // classification invariant (2026-07-06 adversarial review) is: a
        // mapname PS must ALSO carry the `Transp_OffsetX` symbol in its DXBC
        // (the PS reads `Transp_OffsetX.x` as transparency, so a genuine
        // mapname PS always contains it). A shader that reached this observe
        // via another family's prefilter marker and merely reflects as a
        // DiffuseTexture-at-t0 PS is rejected here.
        if !shader_bytecode_has_mapname_transp_offset_marker(bytecode) {
            return false;
        }
        if shader_ptr == 0 {
            return false;
        }
        let mut constant_buffers = shader_constant_buffer_slots_from_shader_bytecode(bytecode);
        // When reflection surfaced constant buffers at all, the Transp_OffsetX
        // CB must be bound at slot 1 (the shipped ConstantBuffer(1,32)). An
        // empty reflection map (non-DXBC unit fixtures / reflection
        // unavailable) falls back to the byte-marker evidence above.
        if !constant_buffers.is_empty()
            && !constant_buffers
                .values()
                .any(|bind_point| *bind_point == MAPNAME_TRANSP_OFFSET_CB_BIND_POINT)
        {
            return false;
        }
        constant_buffers
            .entry(MAPNAME_TRANSP_OFFSET_CB_SOURCE.to_string())
            .or_insert(MAPNAME_TRANSP_OFFSET_CB_BIND_POINT);
        self.mapname_pixel_shaders.insert(
            shader_ptr,
            PdxmapShaderMetadata {
                resource_slots,
                constant_buffers,
                byte_len: bytecode.len(),
                fnv1a64: fnv1a64_bytes(bytecode),
            },
        );
        true
    }

    pub fn observe_vertex_shader_created(
        &mut self,
        shader_ptr: usize,
        bytecode: &[u8],
        reflection: Option<TerrainVertexShaderMetadata>,
    ) -> bool {
        if shader_ptr != 0 {
            if let Some(reflection) = reflection
                .clone()
                .or_else(|| reflect_terrain_vertex_shader_metadata(bytecode))
            {
                let recorded = TerrainVertexShaderMetadata {
                    byte_len: bytecode.len(),
                    fnv1a64: fnv1a64_bytes(bytecode),
                    ..reflection
                };
                if terrain_vertex_shader_metadata_matches_mapname(&recorded)
                    && shader_bytecode_has_mapname_transp_offset_marker(bytecode)
                {
                    let mut mapname = recorded.clone();
                    mapname
                        .constant_buffers
                        .insert(MAPNAME_TRANSP_OFFSET_CB_SOURCE.to_string(), 1);
                    self.mapname_vertex_shaders.insert(shader_ptr, mapname);
                }
                self.map_vertex_shaders.insert(shader_ptr, recorded);
            }
        }
        if shader_ptr == 0 || !shader_bytecode_has_pdxmap_terrain_vertex_signature(bytecode) {
            return false;
        }
        let Some(mut reflection) =
            reflection.or_else(|| terrain_vertex_shader_metadata_from_shader_bytecode(bytecode))
        else {
            return false;
        };
        if !terrain_vertex_shader_metadata_matches_pdxmap(&reflection) {
            return false;
        }
        reflection
            .constant_buffers
            .insert(TERRAIN_DRAW_CB1_LAYOUT_SOURCE.to_string(), 1);
        self.terrain_vertex_shaders.insert(
            shader_ptr,
            TerrainVertexShaderMetadata {
                byte_len: bytecode.len(),
                fnv1a64: fnv1a64_bytes(bytecode),
                ..reflection
            },
        );
        true
    }

    pub fn observe_ps_set_shader(&mut self, shader_ptr: usize) {
        self.current_pixel_shader = (shader_ptr != 0).then_some(shader_ptr);
    }

    pub fn observe_vs_set_shader(&mut self, shader_ptr: usize) {
        self.current_vertex_shader = (shader_ptr != 0).then_some(shader_ptr);
    }

    pub fn current_pixel_shader(&self) -> Option<usize> {
        self.current_pixel_shader
    }

    pub fn current_vertex_shader(&self) -> Option<usize> {
        self.current_vertex_shader
    }

    pub fn observe_input_layout_created(
        &mut self,
        layout_ptr: usize,
        elements: Vec<TerrainInputElementDesc>,
    ) -> bool {
        if layout_ptr == 0 || elements.is_empty() {
            return false;
        }
        self.input_layouts.insert(layout_ptr, elements);
        true
    }

    pub fn observe_ia_set_input_layout(&mut self, layout_ptr: usize) {
        self.current_input_layout = (layout_ptr != 0).then_some(layout_ptr);
    }

    pub fn observe_ps_set_shader_resources(&mut self, start_slot: u32, srvs: &[usize]) -> bool {
        self.observe_ps_set_named_shader_resource(TERRAIN_ID_RESOURCE_SYMBOL, start_slot, srvs)
            .is_some()
    }

    pub fn observe_ps_set_named_shader_resource(
        &mut self,
        resource: &str,
        start_slot: u32,
        srvs: &[usize],
    ) -> Option<TerrainSrvBinding> {
        let shader_slot = self
            .current_pixel_shader
            .and_then(|shader| self.pdxmap_pixel_shaders.get(&shader))
            .and_then(|shader| shader.resource_slots.get(resource).copied())?;
        let Some(index) = shader_slot.checked_sub(start_slot) else {
            return None;
        };
        let Some(&srv) = srvs.get(index as usize) else {
            return None;
        };
        if srv == 0 {
            return None;
        }
        let binding = TerrainSrvBinding {
            srv,
            slot: shader_slot,
        };
        self.shader_resource_srvs
            .insert(resource.to_string(), binding);
        Some(binding)
    }

    pub fn terrain_id_srv(&self) -> Option<TerrainSrvBinding> {
        self.shader_resource_srv(TERRAIN_ID_RESOURCE_SYMBOL)
    }

    pub fn shader_resource_srv(&self, resource: &str) -> Option<TerrainSrvBinding> {
        self.shader_resource_srvs.get(resource).copied()
    }

    pub fn pdxmap_pixel_shader(&self) -> Option<TerrainShaderBinding> {
        self.current_pixel_shader.and_then(|pixel_shader| {
            self.pdxmap_pixel_shaders
                .get(&pixel_shader)
                .and_then(|shader| shader.resource_slots.get(TERRAIN_ID_RESOURCE_SYMBOL))
                .map(|terrain_id_srv_slot| TerrainShaderBinding {
                    pixel_shader,
                    terrain_id_srv_slot: *terrain_id_srv_slot,
                })
        })
    }

    pub fn pdxwater_pixel_shader(&self) -> Option<MapPixelShaderBinding> {
        self.current_pixel_shader
            .filter(|pixel_shader| self.pdxwater_pixel_shaders.contains_key(pixel_shader))
            .map(|pixel_shader| MapPixelShaderBinding { pixel_shader })
    }

    pub fn river_pixel_shader(&self) -> Option<MapPixelShaderBinding> {
        self.current_pixel_shader
            .filter(|pixel_shader| self.river_pixel_shaders.contains_key(pixel_shader))
            .map(|pixel_shader| MapPixelShaderBinding { pixel_shader })
    }

    pub fn border_pixel_shader(&self) -> Option<MapPixelShaderBinding> {
        self.current_pixel_shader
            .filter(|pixel_shader| self.border_pixel_shaders.contains_key(pixel_shader))
            .map(|pixel_shader| MapPixelShaderBinding { pixel_shader })
    }

    pub fn underwater_pixel_shader(&self) -> Option<MapPixelShaderBinding> {
        self.current_pixel_shader
            .filter(|pixel_shader| self.underwater_pixel_shaders.contains_key(pixel_shader))
            .map(|pixel_shader| MapPixelShaderBinding { pixel_shader })
    }

    pub fn tree_pixel_shader(&self) -> Option<MapPixelShaderBinding> {
        self.current_pixel_shader
            .filter(|pixel_shader| self.tree_pixel_shaders.contains_key(pixel_shader))
            .map(|pixel_shader| MapPixelShaderBinding { pixel_shader })
    }

    pub fn restore_pixel_shader(&self) -> Option<MapPixelShaderBinding> {
        self.current_pixel_shader
            .filter(|pixel_shader| self.restore_pixel_shaders.contains_key(pixel_shader))
            .map(|pixel_shader| MapPixelShaderBinding { pixel_shader })
    }

    pub fn mapname_pixel_shader(&self) -> Option<MapPixelShaderBinding> {
        self.current_pixel_shader
            .filter(|pixel_shader| self.mapname_pixel_shaders.contains_key(pixel_shader))
            .map(|pixel_shader| MapPixelShaderBinding { pixel_shader })
    }

    pub fn pdxmap_pixel_shader_metadata(
        &self,
        pixel_shader: usize,
    ) -> Option<&PdxmapShaderMetadata> {
        self.pdxmap_pixel_shaders.get(&pixel_shader)
    }

    pub fn pdxwater_pixel_shader_metadata(
        &self,
        pixel_shader: usize,
    ) -> Option<&PdxmapShaderMetadata> {
        self.pdxwater_pixel_shaders.get(&pixel_shader)
    }

    pub fn river_pixel_shader_metadata(
        &self,
        pixel_shader: usize,
    ) -> Option<&PdxmapShaderMetadata> {
        self.river_pixel_shaders.get(&pixel_shader)
    }

    pub fn border_pixel_shader_metadata(
        &self,
        pixel_shader: usize,
    ) -> Option<&PdxmapShaderMetadata> {
        self.border_pixel_shaders.get(&pixel_shader)
    }

    pub fn underwater_pixel_shader_metadata(
        &self,
        pixel_shader: usize,
    ) -> Option<&PdxmapShaderMetadata> {
        self.underwater_pixel_shaders.get(&pixel_shader)
    }

    pub fn tree_pixel_shader_metadata(&self, pixel_shader: usize) -> Option<&PdxmapShaderMetadata> {
        self.tree_pixel_shaders.get(&pixel_shader)
    }

    pub fn restore_pixel_shader_metadata(
        &self,
        pixel_shader: usize,
    ) -> Option<&PdxmapShaderMetadata> {
        self.restore_pixel_shaders.get(&pixel_shader)
    }

    pub fn mapname_pixel_shader_metadata(
        &self,
        pixel_shader: usize,
    ) -> Option<&PdxmapShaderMetadata> {
        self.mapname_pixel_shaders.get(&pixel_shader)
    }

    pub fn terrain_vertex_shader(&self) -> Option<TerrainVertexShaderBinding> {
        self.current_vertex_shader
            .filter(|vertex_shader| self.terrain_vertex_shaders.contains_key(vertex_shader))
            .map(|vertex_shader| TerrainVertexShaderBinding { vertex_shader })
    }

    pub fn map_vertex_shader(&self) -> Option<TerrainVertexShaderBinding> {
        self.current_vertex_shader
            .filter(|vertex_shader| self.map_vertex_shaders.contains_key(vertex_shader))
            .map(|vertex_shader| TerrainVertexShaderBinding { vertex_shader })
    }

    pub fn mapname_vertex_shader(&self) -> Option<TerrainVertexShaderBinding> {
        self.current_vertex_shader
            .filter(|vertex_shader| self.mapname_vertex_shaders.contains_key(vertex_shader))
            .map(|vertex_shader| TerrainVertexShaderBinding { vertex_shader })
    }

    pub fn mapname_vertex_shader_metadata(
        &self,
        vertex_shader: usize,
    ) -> Option<&TerrainVertexShaderMetadata> {
        self.mapname_vertex_shaders.get(&vertex_shader)
    }

    pub fn mapname_vertex_shader_known(&self, vertex_shader: usize) -> bool {
        self.mapname_vertex_shaders.contains_key(&vertex_shader)
    }

    pub fn terrain_vertex_shader_metadata(
        &self,
        vertex_shader: usize,
    ) -> Option<&TerrainVertexShaderMetadata> {
        self.terrain_vertex_shaders.get(&vertex_shader)
    }

    pub fn map_vertex_shader_metadata(
        &self,
        vertex_shader: usize,
    ) -> Option<&TerrainVertexShaderMetadata> {
        self.map_vertex_shaders.get(&vertex_shader)
    }

    pub fn current_pixel_shader_constant_buffer_name(&self, slot: u32) -> Option<String> {
        self.current_pixel_shader_constant_buffers()
            .and_then(|constant_buffers| constant_buffer_name_for_slot(&constant_buffers, slot))
    }

    pub fn current_vertex_shader_constant_buffer_name(&self, slot: u32) -> Option<String> {
        self.current_vertex_shader_constant_buffers()
            .and_then(|constant_buffers| constant_buffer_name_for_slot(&constant_buffers, slot))
    }

    pub fn current_pixel_shader_constant_buffers(&self) -> Option<BTreeMap<String, u32>> {
        let pixel_shader = self.current_pixel_shader?;
        self.pdxwater_pixel_shaders
            .get(&pixel_shader)
            .or_else(|| self.pdxmap_pixel_shaders.get(&pixel_shader))
            .or_else(|| self.river_pixel_shaders.get(&pixel_shader))
            .or_else(|| self.border_pixel_shaders.get(&pixel_shader))
            .or_else(|| self.underwater_pixel_shaders.get(&pixel_shader))
            .or_else(|| self.tree_pixel_shaders.get(&pixel_shader))
            .or_else(|| self.restore_pixel_shaders.get(&pixel_shader))
            .or_else(|| self.mapname_pixel_shaders.get(&pixel_shader))
            .map(|shader| shader.constant_buffers.clone())
    }

    pub fn current_vertex_shader_constant_buffers(&self) -> Option<BTreeMap<String, u32>> {
        let vertex_shader = self.current_vertex_shader?;
        self.terrain_vertex_shaders
            .get(&vertex_shader)
            .or_else(|| self.mapname_vertex_shaders.get(&vertex_shader))
            .or_else(|| self.map_vertex_shaders.get(&vertex_shader))
            .map(|shader| shader.constant_buffers.clone())
    }

    pub fn pdxmap_pixel_shader_count(&self) -> usize {
        self.pdxmap_pixel_shaders.len()
    }

    pub fn pdxwater_pixel_shader_count(&self) -> usize {
        self.pdxwater_pixel_shaders.len()
    }

    pub fn river_pixel_shader_count(&self) -> usize {
        self.river_pixel_shaders.len()
    }

    pub fn border_pixel_shader_count(&self) -> usize {
        self.border_pixel_shaders.len()
    }

    pub fn underwater_pixel_shader_count(&self) -> usize {
        self.underwater_pixel_shaders.len()
    }

    pub fn tree_pixel_shader_count(&self) -> usize {
        self.tree_pixel_shaders.len()
    }

    pub fn restore_pixel_shader_count(&self) -> usize {
        self.restore_pixel_shaders.len()
    }

    pub fn mapname_pixel_shader_count(&self) -> usize {
        self.mapname_pixel_shaders.len()
    }

    pub fn mapname_vertex_shader_count(&self) -> usize {
        self.mapname_vertex_shaders.len()
    }

    pub fn terrain_vertex_shader_count(&self) -> usize {
        self.terrain_vertex_shaders.len()
    }

    pub fn map_vertex_shader_count(&self) -> usize {
        self.map_vertex_shaders.len()
    }

    pub fn input_layout(&self, layout_ptr: usize) -> Option<&[TerrainInputElementDesc]> {
        self.input_layouts.get(&layout_ptr).map(Vec::as_slice)
    }

    pub fn current_input_layout(&self) -> Option<(usize, &[TerrainInputElementDesc])> {
        let layout_ptr = self.current_input_layout?;
        self.input_layout(layout_ptr)
            .map(|elements| (layout_ptr, elements))
    }

    pub fn input_layout_count(&self) -> usize {
        self.input_layouts.len()
    }
}

fn constant_buffer_name_for_slot(
    constant_buffers: &BTreeMap<String, u32>,
    slot: u32,
) -> Option<String> {
    constant_buffers
        .iter()
        .find(|(_, bind_point)| **bind_point == slot)
        .map(|(name, _)| name.clone())
}

pub fn terrain_scene_cb0_evidence_matches_binding(bundle_dir: &Path, bytes: &[u8]) -> bool {
    let Ok(meta_bytes) = fs::read(bundle_dir.join(TERRAIN_SCENE_CB0_META_FILE)) else {
        return false;
    };
    let Ok(meta) = serde_json::from_slice::<Value>(&meta_bytes) else {
        return false;
    };
    if meta.get("symbol").and_then(Value::as_str) != Some(TERRAIN_SCENE_CB0_SYMBOL) {
        return false;
    }
    if meta.get("file").and_then(Value::as_str) != Some(TERRAIN_SCENE_CB0_FILE) {
        return false;
    }
    if meta
        .get("shader_constant_buffer_slot")
        .and_then(Value::as_u64)
        != Some(u64::from(TERRAIN_SCENE_CB0_BIND_POINT))
    {
        return false;
    }
    let expected_hash = fnv1a64_bytes(bytes);
    if meta.get("fnv1a64").and_then(Value::as_str) != Some(expected_hash.as_str()) {
        return false;
    }
    if meta.get("byte_len").and_then(Value::as_u64) != Some(bytes.len() as u64) {
        return false;
    }
    fs::read(bundle_dir.join(TERRAIN_SCENE_CB0_FILE))
        .map(|captured| captured == bytes)
        .unwrap_or(false)
}

pub fn canonicalize_map_frame_shader_constant_buffers<'a, I>(
    capture_moment: &str,
    stage: &str,
    source_evidence: I,
    constant_buffers: &mut BTreeMap<String, u32>,
    terrain_scene_cb0_matches_binding: bool,
) -> usize
where
    I: IntoIterator<Item = &'a str>,
{
    let mut changed = 0;
    if matches!(
        capture_moment,
        TERRAIN_CAPTURE_MOMENT | TERRAIN_CHUNK_LIBRARY_MOMENT
    ) && terrain_scene_cb0_matches_binding
        && matches!(stage, "VS" | "PS")
        && replace_constant_buffer_slot_name(
            constant_buffers,
            TERRAIN_SCENE_CB0_BIND_POINT,
            TERRAIN_SCENE_CB0_SYMBOL,
        )
    {
        changed += 1;
    }

    let cites_pdxmap_fxh = source_evidence
        .into_iter()
        .any(|source| source == PDXMAP_FXH_SOURCE);
    if matches!(
        capture_moment,
        TERRAIN_CAPTURE_MOMENT | TERRAIN_CHUNK_LIBRARY_MOMENT
    ) && stage == "VS"
        && cites_pdxmap_fxh
        && replace_constant_buffer_slot_name(
            constant_buffers,
            TERRAIN_DRAW_CB1_BIND_POINT,
            TERRAIN_DRAW_CB1_LAYOUT_SOURCE,
        )
    {
        changed += 1;
    }
    changed
}

pub fn canonical_map_frame_constant_buffer_name_for_binding<'a, I>(
    capture_moment: &str,
    stage: &str,
    slot: u32,
    source_evidence: I,
    reflected_constant_buffers: &BTreeMap<String, u32>,
    bundle_dir: &Path,
    bytes: &[u8],
) -> Option<String>
where
    I: IntoIterator<Item = &'a str>,
{
    let mut constant_buffers = reflected_constant_buffers.clone();
    canonicalize_map_frame_shader_constant_buffers(
        capture_moment,
        stage,
        source_evidence,
        &mut constant_buffers,
        slot == TERRAIN_SCENE_CB0_BIND_POINT
            && terrain_scene_cb0_evidence_matches_binding(bundle_dir, bytes),
    );
    constant_buffer_name_for_slot(&constant_buffers, slot)
}

fn replace_constant_buffer_slot_name(
    constant_buffers: &mut BTreeMap<String, u32>,
    slot: u32,
    canonical_name: &str,
) -> bool {
    if !constant_buffers
        .values()
        .any(|bind_point| *bind_point == slot)
    {
        return false;
    }
    let before = constant_buffers.clone();
    constant_buffers.retain(|_, bind_point| *bind_point != slot);
    constant_buffers.insert(canonical_name.to_string(), slot);
    *constant_buffers != before
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BundleConstantBufferRepairReport {
    pub binding_names_changed: usize,
    pub shader_names_changed: usize,
}

impl BundleConstantBufferRepairReport {
    pub fn changed(&self) -> bool {
        self.binding_names_changed != 0 || self.shader_names_changed != 0
    }
}

pub fn repair_map_frame_bundle_constant_buffer_names(
    bundle_path: &Path,
) -> Result<BundleConstantBufferRepairReport, Box<dyn std::error::Error>> {
    let bundle_dir = bundle_path
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", bundle_path.display()))?;
    let mut bundle: Value = serde_json::from_slice(&fs::read(bundle_path)?)?;
    let capture_moment = bundle
        .get("capture_moment")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let mut report = BundleConstantBufferRepairReport::default();
    if let Some(active_draw) = bundle.get_mut("active_draw") {
        repair_map_frame_draw_constant_buffer_names(
            active_draw,
            &capture_moment,
            bundle_dir,
            &mut report,
        )?;
    }
    if let Some(draws) = bundle.get_mut("draws").and_then(Value::as_array_mut) {
        for draw in draws {
            repair_map_frame_draw_constant_buffer_names(
                draw,
                &capture_moment,
                bundle_dir,
                &mut report,
            )?;
        }
    }
    if report.changed() {
        let bytes = serde_json::to_vec_pretty(&bundle)?;
        fs::write(bundle_path, bytes)?;
    }
    Ok(report)
}

fn repair_map_frame_draw_constant_buffer_names(
    draw: &mut Value,
    capture_moment: &str,
    bundle_dir: &Path,
    report: &mut BundleConstantBufferRepairReport,
) -> Result<(), Box<dyn std::error::Error>> {
    let ps_meta = draw_stage_shader_meta(draw, "PS");
    let vs_meta = draw_stage_shader_meta(draw, "VS");
    if let Some(bindings) = draw
        .get_mut("constant_buffers")
        .and_then(Value::as_array_mut)
    {
        for binding in bindings {
            let Some(stage) = binding.get("stage").and_then(Value::as_str) else {
                continue;
            };
            let Some(slot) = binding
                .get("slot")
                .and_then(Value::as_u64)
                .and_then(|slot| u32::try_from(slot).ok())
            else {
                continue;
            };
            let Some(file) = binding.get("file").and_then(Value::as_str) else {
                continue;
            };
            let Ok(bytes) = fs::read(bundle_dir.join(file)) else {
                continue;
            };
            let Some((constant_buffers, source_evidence)) = (match stage {
                "PS" => ps_meta.as_ref(),
                "VS" => vs_meta.as_ref(),
                _ => None,
            }) else {
                continue;
            };
            let Some(name) = canonical_map_frame_constant_buffer_name_for_binding(
                capture_moment,
                stage,
                slot,
                source_evidence.iter().map(String::as_str),
                constant_buffers,
                bundle_dir,
                &bytes,
            ) else {
                continue;
            };
            if binding.get("declared_name").and_then(Value::as_str) != Some(name.as_str()) {
                set_object_string(binding, "declared_name", name);
                report.binding_names_changed += 1;
            }
        }
    }
    repair_map_frame_draw_shader_constant_buffer_names(
        draw,
        capture_moment,
        bundle_dir,
        "PS",
        report,
    );
    repair_map_frame_draw_shader_constant_buffer_names(
        draw,
        capture_moment,
        bundle_dir,
        "VS",
        report,
    );
    Ok(())
}

fn draw_stage_shader_meta(
    draw: &Value,
    stage: &str,
) -> Option<(BTreeMap<String, u32>, Vec<String>)> {
    let shader = draw_stage_shader(draw, stage)?;
    Some((
        shader_constant_buffers_from_value(shader),
        shader_source_evidence_from_value(shader),
    ))
}

fn repair_map_frame_draw_shader_constant_buffer_names(
    draw: &mut Value,
    capture_moment: &str,
    bundle_dir: &Path,
    stage: &str,
    report: &mut BundleConstantBufferRepairReport,
) {
    let terrain_scene_cb0_matches_binding =
        draw_stage_slot0_matches_terrain_scene_cb0(draw, stage, bundle_dir);
    let Some(shader) = draw_stage_shader_mut(draw, stage) else {
        return;
    };
    let source_evidence = shader_source_evidence_from_value(shader);
    let mut constant_buffers = shader_constant_buffers_from_value(shader);
    let changed = canonicalize_map_frame_shader_constant_buffers(
        capture_moment,
        stage,
        source_evidence.iter().map(String::as_str),
        &mut constant_buffers,
        terrain_scene_cb0_matches_binding,
    );
    if changed != 0 {
        set_shader_constant_buffers(shader, &constant_buffers);
        report.shader_names_changed += changed;
    }
}

fn draw_stage_slot0_matches_terrain_scene_cb0(
    draw: &Value,
    stage: &str,
    bundle_dir: &Path,
) -> bool {
    draw.get("constant_buffers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .any(|binding| {
            binding.get("stage").and_then(Value::as_str) == Some(stage)
                && binding.get("slot").and_then(Value::as_u64)
                    == Some(u64::from(TERRAIN_SCENE_CB0_BIND_POINT))
                && binding
                    .get("file")
                    .and_then(Value::as_str)
                    .and_then(|file| fs::read(bundle_dir.join(file)).ok())
                    .is_some_and(|bytes| {
                        terrain_scene_cb0_evidence_matches_binding(bundle_dir, &bytes)
                    })
        })
}

fn draw_stage_shader<'a>(draw: &'a Value, stage: &str) -> Option<&'a Value> {
    match stage {
        "PS" => draw.get("shader"),
        "VS" => draw.get("vertex_shader"),
        _ => None,
    }
}

fn draw_stage_shader_mut<'a>(draw: &'a mut Value, stage: &str) -> Option<&'a mut Value> {
    match stage {
        "PS" => draw.get_mut("shader"),
        "VS" => draw.get_mut("vertex_shader"),
        _ => None,
    }
}

fn shader_constant_buffers_from_value(shader: &Value) -> BTreeMap<String, u32> {
    shader
        .get("constant_buffers")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|object| object.iter())
        .filter_map(|(name, slot)| {
            slot.as_u64()
                .and_then(|slot| u32::try_from(slot).ok())
                .map(|slot| (name.clone(), slot))
        })
        .collect()
}

fn shader_source_evidence_from_value(shader: &Value) -> Vec<String> {
    shader
        .get("source_evidence")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(ToOwned::to_owned)
        .collect()
}

fn set_shader_constant_buffers(shader: &mut Value, constant_buffers: &BTreeMap<String, u32>) {
    let object = constant_buffers
        .iter()
        .map(|(name, slot)| (name.clone(), Value::from(*slot)))
        .collect::<JsonMap<String, Value>>();
    if let Some(shader) = shader.as_object_mut() {
        shader.insert("constant_buffers".to_string(), Value::Object(object));
    }
}

fn set_object_string(value: &mut Value, key: &str, string: String) {
    if let Some(object) = value.as_object_mut() {
        object.insert(key.to_string(), Value::String(string));
    }
}

pub fn shader_cache_signature(text: &str) -> Option<ShaderCacheSignature> {
    let terrain_id_srv_slot = terrain_id_slot_from_shader_text(text)?;
    Some(ShaderCacheSignature {
        map_size_x: parse_define_u32(text, "MAP_SIZE_X")?,
        map_size_y: parse_define_u32(text, "MAP_SIZE_Y")?,
        map_pow2_x_milli: parse_define_milli(text, "MAP_POW2_X")?,
        map_pow2_y_milli: parse_define_milli(text, "MAP_POW2_Y")?,
        fow_pow2_x_milli: parse_define_milli(text, "FOW_POW2_X")?,
        fow_pow2_y_milli: parse_define_milli(text, "FOW_POW2_Y")?,
        terrain_id_srv_slot,
    })
}

pub fn terrain_id_slot_from_shader_text(text: &str) -> Option<u32> {
    shader_resource_slot_from_shader_text(text, TERRAIN_ID_RESOURCE_SYMBOL)
}

pub fn shader_resource_slot_from_shader_text(text: &str, resource: &str) -> Option<u32> {
    let texture_name = format!("{resource}_Texture");
    text.lines().find_map(|line| {
        let resource_offset = line.find(&texture_name).or_else(|| line.find(resource))?;
        parse_register_t_slot(&line[resource_offset..])
    })
}

pub fn shader_resource_slots_from_shader_text(text: &str) -> BTreeMap<String, u32> {
    let mut out = BTreeMap::new();
    for segment in text.split(';') {
        let Some(register_offset) = segment.find("register") else {
            continue;
        };
        let Some(slot) = parse_register_t_slot(&segment[register_offset..]) else {
            continue;
        };
        let name_prefix = segment[..register_offset]
            .trim_end_matches(|c: char| c == ':' || c.is_ascii_whitespace());
        let Some(raw_name) = name_prefix.split_whitespace().last() else {
            continue;
        };
        let name = normalize_shader_resource_name(raw_name);
        if !name.is_empty() {
            out.insert(name, slot);
        }
    }
    out
}

pub fn normalize_shader_resource_name(name: &str) -> String {
    name.trim_matches(|c: char| {
        c == '\0' || c == ':' || c == ';' || c == ',' || c.is_ascii_whitespace()
    })
    .trim_end_matches("_Texture")
    .to_string()
}

pub fn terrain_id_slot_from_shader_bytecode(bytecode: &[u8]) -> Option<u32> {
    shader_resource_slot_from_shader_bytecode(bytecode, TERRAIN_ID_RESOURCE_SYMBOL)
}

pub fn shader_resource_slot_from_shader_bytecode(bytecode: &[u8], resource: &str) -> Option<u32> {
    let text = String::from_utf8_lossy(bytecode);
    shader_resource_slot_from_shader_text(&text, resource)
        .or_else(|| reflect_shader_resource_slot(bytecode, resource))
        .filter(|_| shader_bytecode_has_terrain_idmap_signature(bytecode))
}

#[cfg(windows)]
fn reflect_shader_resource_slot(bytecode: &[u8], resource: &str) -> Option<u32> {
    d3d11_runtime::reflect_shader_resource_slot(bytecode, resource)
}

#[cfg(not(windows))]
fn reflect_shader_resource_slot(_bytecode: &[u8], _resource: &str) -> Option<u32> {
    None
}

pub fn shader_texture_resource_slots_from_shader_bytecode(
    bytecode: &[u8],
) -> BTreeMap<String, u32> {
    let text = String::from_utf8_lossy(bytecode);
    let mut out = shader_resource_slots_from_shader_text(&text);
    #[cfg(windows)]
    {
        for (name, slot) in d3d11_runtime::reflect_shader_texture_resource_slots(bytecode) {
            out.insert(name, slot);
        }
    }
    out
}

pub fn shader_constant_buffer_slots_from_shader_bytecode(bytecode: &[u8]) -> BTreeMap<String, u32> {
    #[cfg(windows)]
    {
        d3d11_runtime::reflect_shader_constant_buffer_slots(bytecode)
    }
    #[cfg(not(windows))]
    {
        let _ = bytecode;
        BTreeMap::new()
    }
}

pub fn pdxmap_resource_slots_from_shader_bytecode(
    bytecode: &[u8],
) -> Option<BTreeMap<String, u32>> {
    if !shader_bytecode_has_terrain_idmap_signature(bytecode) {
        return None;
    }
    let mut out = shader_texture_resource_slots_from_shader_bytecode(bytecode);
    let terrain_id_slot =
        shader_resource_slot_from_shader_bytecode(bytecode, TERRAIN_ID_RESOURCE_SYMBOL)?;
    out.insert(TERRAIN_ID_RESOURCE_SYMBOL.to_string(), terrain_id_slot);
    for resource in PDXMAP_RUNTIME_CAPTURE_RESOURCE_SYMBOLS {
        if let Some(slot) = shader_resource_slot_from_shader_bytecode(bytecode, resource) {
            out.insert(resource.to_string(), slot);
        }
    }
    Some(out)
}

pub fn pdxwater_resource_slots_from_shader_bytecode(
    bytecode: &[u8],
) -> Option<BTreeMap<String, u32>> {
    required_resource_slots_from_shader_bytecode(bytecode, &PDXWATER_REQUIRED_RESOURCE_SYMBOLS)
}

pub fn river_resource_slots_from_shader_bytecode(bytecode: &[u8]) -> Option<BTreeMap<String, u32>> {
    required_resource_slots_from_shader_bytecode(bytecode, &RIVER_REQUIRED_RESOURCE_SYMBOLS)
}

pub fn border_resource_slots_from_shader_bytecode(
    bytecode: &[u8],
) -> Option<BTreeMap<String, u32>> {
    required_resource_slots_from_shader_bytecode(bytecode, &BORDER_REQUIRED_RESOURCE_SYMBOLS)
}

pub fn tree_resource_slots_from_shader_bytecode(bytecode: &[u8]) -> Option<BTreeMap<String, u32>> {
    required_resource_slots_from_shader_bytecode(bytecode, &TREE_REQUIRED_RESOURCE_SYMBOLS)
}

/// Classify a PS as `pdxmap.shader:PixelShaderUnderwater` from its DXBC texture
/// bindings. SOUND (never-false-reject) discriminator: the PS must declare EVERY
/// member of [`UNDERWATER_REQUIRED_RESOURCE_SYMBOLS`] and must NOT declare
/// [`UNDERWATER_EXCLUDED_RESOURCE_SYMBOL`] (`TerrainIDMap`). The `TerrainIDMap`
/// exclusion is what separates the underwater PS from the main terrain PS, which
/// shares the same VS and several of the same texture slots but reaches the
/// terrain diffuse through the ID-map indirection. `PixelShaderTerrainUnlit` is
/// rejected because it omits the point-light `LightIndexMap`/`LightDataMap` slots
/// this set requires. Returns the reflected texture slot map on a match.
pub fn underwater_resource_slots_from_shader_bytecode(
    bytecode: &[u8],
) -> Option<BTreeMap<String, u32>> {
    let out = shader_texture_resource_slots_from_shader_bytecode(bytecode);
    if out.contains_key(UNDERWATER_EXCLUDED_RESOURCE_SYMBOL) {
        return None;
    }
    UNDERWATER_REQUIRED_RESOURCE_SYMBOLS
        .iter()
        .all(|resource| out.contains_key(*resource))
        .then_some(out)
}

pub fn restore_resource_slots_from_shader_bytecode(
    bytecode: &[u8],
) -> Option<BTreeMap<String, u32>> {
    let out = shader_texture_resource_slots_from_shader_bytecode(bytecode);
    restore_resource_slots_match(&out).then_some(out)
}

pub fn restore_resource_slots_match(resource_slots: &BTreeMap<String, u32>) -> bool {
    resource_slots
        .get(RESTORE_MAIN_SCENE_RESOURCE_SYMBOL)
        .copied()
        == Some(0)
        && resource_slots.get(RESTORE_BLOOM_RESOURCE_SYMBOL).copied() == Some(1)
        && resource_slots
            .get(RESTORE_COLOR_CUBE_RESOURCE_SYMBOL)
            .copied()
            == Some(2)
        && resource_slots
            .get(RESTORE_AVERAGE_LUMINANCE_RESOURCE_SYMBOL)
            .copied()
            == Some(3)
}

/// Reflected-signature discriminator for the `mapname` vertex shader — the
/// PRIMARY identifier for the label pass (the spec's mandate). A mapname VS
/// has EXACTLY two vertex inputs:
///   - `POSITION[0]` as `float3` (component type FLOAT32, xyz components read)
///   - `TEXCOORD[0]` as `float2` (component type FLOAT32, xy components read)
/// and binds a constant buffer at slot 1 (the `Transp_OffsetX` CB).
///
/// This rejects every other map/UI vertex shader in the game:
///   - terrain VS: POSITION is `int4`/`int2` SINT32 (not FLOAT32);
///   - UI VS: POSITION is `float2` (mask 0x03, not the xyz 0x07 of a float3);
///   - restorescene/fullscreen VS: no POSITION input (uses SV_VertexID);
///   - `traderoute.shader`: a THIRD input (`TANGENT`) — excluded by the
///     exactly-two-inputs constraint (also by the paired PS having 2 SRVs);
///   - `maskedflag`/`static_button`/`buttonstate_*`: 2-input float3+float2 VS
///     but NO slot-1 CB — excluded by the CB1 constraint (also by the paired
///     PS not being a single `DiffuseTexture`);
///   - `particle.shader`/`arrow.shader` have a slot-1 32-byte CB but their VS
///     inputs differ (instancing / extra inputs) — excluded by the input set.
/// Masks use exact equality (`== 0x07` for `float3`, `== 0x03` for `float2`):
/// D3D reflection reports the declared component set, so a `float4` position
/// would report `0x0F` and must NOT match. This is stricter than the terrain
/// predicate (which uses `& 0x0f`), because here float-width IS the
/// discriminating signal against float4-position mesh shaders. The shipped
/// mapname VS reads `v.vPosition.xyz` and `v.vTexCoord.xy`, giving exactly
/// `0x07`/`0x03`.
pub fn terrain_vertex_shader_metadata_matches_mapname(
    metadata: &TerrainVertexShaderMetadata,
) -> bool {
    if metadata.input_parameters.len() != 2 {
        return false;
    }
    let has_position = metadata.input_parameters.iter().any(|parameter| {
        parameter.semantic_name.eq_ignore_ascii_case("POSITION")
            && parameter.semantic_index == 0
            && parameter.component_type == D3D_REGISTER_COMPONENT_FLOAT32
            && parameter.mask == 0x07
    });
    let has_texcoord = metadata.input_parameters.iter().any(|parameter| {
        parameter.semantic_name.eq_ignore_ascii_case("TEXCOORD")
            && parameter.semantic_index == 0
            && parameter.component_type == D3D_REGISTER_COMPONENT_FLOAT32
            && parameter.mask == 0x03
    });
    let has_cb1 = metadata
        .constant_buffers
        .values()
        .any(|bind_point| *bind_point == MAPNAME_TRANSP_OFFSET_CB_BIND_POINT);
    has_position && has_texcoord && has_cb1
}

/// Byte-level corroboration that a candidate mapname vertex shader's slot-1 CB
/// is specifically `Transp_OffsetX` and not some other 32-byte b1 CB. Live
/// evidence (session_20260702d) proved the reflected float3-position + b1-CB
/// signature ALONE collides with other 2-input float3+float2 map shaders whose
/// slot-1 CB is a different struct (`Offset`/`NextOffset`); only the true
/// mapname VS carries the `Transp_OffsetX` symbol in its DXBC string table.
/// D3D reflection surfaces a name only if it is in the bytecode, so this marker
/// is a sound (never-false-reject) tightener: a genuine mapname VS always
/// contains it, and the colliders never do.
pub fn shader_bytecode_has_mapname_transp_offset_marker(bytecode: &[u8]) -> bool {
    byte_slice_contains(bytecode, b"Transp_OffsetX")
}

/// The exact vertex-input signature the REAL `border.shader` VS produces, derived
/// from the shipped source `VS_INPUT_BORDER { float3 position : POSITION; int2 uv
/// : TEXCOORD0; }` (`gfx/FX/border.shader:57`-`:60`) by compiling that struct
/// through `fxc /T vs_5_0` and reading the ISGN out of the DXBC:
///
/// ```text
/// // Name        Index  Mask  Register  SysValue  Format  Used
/// // POSITION        0   xyz         0      NONE   float   xyz   (component_type FLOAT32=3, mask 0x07)
/// // TEXCOORD        0   xy          1      NONE     int   xy    (component_type SINT32=2,  mask 0x03)
/// ```
///
/// This is the load-bearing discriminator that the diag23b contamination violated:
/// the mis-captured terrain patch draw ran the TERRAIN vertex shader, whose
/// POSITION is `int4` (component_type SINT32=2, mask 0x0f) — the opposite of the
/// border VS's `float3` (FLOAT32=3, mask 0x07) position. Masks use exact equality:
/// D3D reflection reports the declared component set, so a `float4` position would
/// report `0x0f` and must NOT match. BORDER_LAW.md's earlier prose "float3 + int2"
/// was a source-level guess; THIS is the fxc-compiled bytecode truth.
///
/// A genuine engine border draw always runs this VS (the shipped `Effect border`
/// binds `VS_INPUT_BORDER`), so the gate never false-rejects a real border draw.
pub fn terrain_vertex_shader_metadata_matches_border(
    metadata: &TerrainVertexShaderMetadata,
) -> bool {
    if metadata.input_parameters.len() != 2 {
        return false;
    }
    let has_position = metadata.input_parameters.iter().any(|parameter| {
        parameter.semantic_name.eq_ignore_ascii_case("POSITION")
            && parameter.semantic_index == 0
            && parameter.component_type == D3D_REGISTER_COMPONENT_FLOAT32
            && parameter.mask == 0x07
    });
    let has_uv = metadata.input_parameters.iter().any(|parameter| {
        parameter.semantic_name.eq_ignore_ascii_case("TEXCOORD")
            && parameter.semantic_index == 0
            && parameter.component_type == D3D_REGISTER_COMPONENT_SINT32
            && parameter.mask == 0x03
    });
    has_position && has_uv
}

/// The structural facts a candidate `borders_pass` draw must present, distilled to
/// exactly what [`validate_border_candidate`] gates on. All fields are cheap to
/// gather on the render thread from state the DLL already tracks (VS reflection,
/// the BorderDiffuse SRV readback, the pre/post render-target snapshots), so the
/// validation is a pure function that is unit-testable off-line against the
/// diag23b contamination evidence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BorderCandidateFacts<'a> {
    /// Reflected input signature of the vertex shader bound on the draw.
    pub vertex_shader: &'a TerrainVertexShaderMetadata,
    /// `true` iff the texture bound at the BorderDiffuse slot (t0) read back as
    /// entirely zero bytes (the diag23b contamination signature).
    pub border_diffuse_srv_all_zero: bool,
    /// `true` iff the BorderDiffuse-slot SRV was actually observed/read back. When
    /// `false` the all-zero flag is meaningless and the candidate is rejected
    /// (we cannot prove the load-bearing texture carries ink → fail closed).
    pub border_diffuse_srv_observed: bool,
    /// `true` iff the pre-draw render target and post-pass render target differ
    /// (the draw actually wrote pixels). `false` == a no-op draw (diag23b).
    pub render_target_changed: bool,
    /// `true` iff both the pre-draw and post-pass render-target snapshots were
    /// captured, so `render_target_changed` is meaningful. When `false` the
    /// candidate is rejected (cannot prove the draw was not a no-op → fail closed).
    pub render_target_snapshots_observed: bool,
}

/// Common prefix on every [`validate_border_candidate`] rejection reason. The
/// runtime keys off it to distinguish a CAPFIX1 structural border rejection from
/// an ordinary observation-miss, so it can discard the no-op draw and keep the
/// borders moment listening.
pub const BORDER_REJECT_MESSAGE_PREFIX: &str = "border candidate rejected:";

/// FAIL-CLOSED border-draw validation. Returns `Ok(())` only when the candidate
/// passes EVERY structural check mined from the shipped `border.shader`; on any
/// failure returns `Err(reason)` so the caller REJECTS the candidate, writes a
/// truthful blocked event, and keeps the borders moment listening (MOMFIX1
/// convention — never finalize a contaminated bundle).
///
/// The three checks correspond one-to-one to the diag23b contamination proof:
/// 1. **VS input signature** must match the real border VS ISGN
///    ([`terrain_vertex_shader_metadata_matches_border`]) — rejects the terrain
///    patch draw, whose `int4` POSITION is not the border VS's `float3`.
/// 2. **BorderDiffuse SRV non-all-zero** — the load-bearing border texture the PS
///    samples (`tex2D( BorderDiffuse, … )`, border.shader:127) must carry ink;
///    diag23b recorded an all-zero t0.
/// 3. **Render target changed** — the draw must have written pixels; diag23b's
///    pre==post render targets prove a no-op.
///
/// Never panics; pure over its inputs.
pub fn validate_border_candidate(facts: &BorderCandidateFacts) -> Result<(), String> {
    if !terrain_vertex_shader_metadata_matches_border(facts.vertex_shader) {
        return Err(format!(
            "border candidate rejected: vertex-shader input signature does not match \
             gfx/FX/border.shader VS_INPUT_BORDER (float3 POSITION + int2 TEXCOORD0); \
             got {} — this is the diag23b terrain-patch contamination signature (int4 POSITION)",
            describe_vs_input_signature(facts.vertex_shader)
        ));
    }
    if !facts.border_diffuse_srv_observed {
        return Err(
            "border candidate rejected: the BorderDiffuse (t0) SRV was not \
             observed/read back, so its ink content cannot be proven (fail closed)"
                .to_string(),
        );
    }
    if facts.border_diffuse_srv_all_zero {
        return Err(
            "border candidate rejected: the bound BorderDiffuse (t0) SRV is \
             all-zero — the diag23b contamination signature; a genuine border draw \
             samples an inked BorderDiffuse texture (border.shader:127)"
                .to_string(),
        );
    }
    if !facts.render_target_snapshots_observed {
        return Err(
            "border candidate rejected: pre-draw/post-pass render-target \
             snapshots were not both captured, so a no-op draw cannot be ruled out \
             (fail closed)"
                .to_string(),
        );
    }
    if !facts.render_target_changed {
        return Err(
            "border candidate rejected: the pre-draw and post-pass render \
             targets are byte-identical — the draw was a no-op on the RT (the diag23b \
             contamination signature); a genuine border draw writes pixels"
                .to_string(),
        );
    }
    Ok(())
}

/// Human-readable one-line summary of a VS's input signature for truthful reject
/// logs (semantic[index]:componentType:mask per input).
fn describe_vs_input_signature(metadata: &TerrainVertexShaderMetadata) -> String {
    if metadata.input_parameters.is_empty() {
        return "<no inputs>".to_string();
    }
    metadata
        .input_parameters
        .iter()
        .map(|parameter| {
            let component = match parameter.component_type {
                D3D_REGISTER_COMPONENT_SINT32 => "sint32",
                D3D_REGISTER_COMPONENT_FLOAT32 => "float32",
                other => {
                    return format!(
                        "{}[{}]:type{}:0x{:02x}",
                        parameter.semantic_name, parameter.semantic_index, other, parameter.mask
                    )
                }
            };
            format!(
                "{}[{}]:{}:0x{:02x}",
                parameter.semantic_name, parameter.semantic_index, component, parameter.mask
            )
        })
        .collect::<Vec<_>>()
        .join(" + ")
}

/// Corroborating PS anchor for the mapname pass. This slot-shape predicate is
/// deliberately NOT a standalone family discriminator — it never gates trust,
/// and classification additionally requires the `Transp_OffsetX` byte marker
/// (see [`TerrainCaptureState::observe_mapname_pixel_shader_created`]) plus a
/// confirmed mapname VS on the draw itself. Returns the resource slots iff the
/// PS declares `DiffuseTexture` at slot 0 (the font atlas). Extra SRVs are
/// TOLERATED — a modded mapname.shader may add e.g. an outline/FoW texture and
/// must not silently break the label capture (mod-support mandate); any extra
/// bindings are recorded in the bundle as evidence. The vanilla uniqueness
/// burden is carried by the `Transp_OffsetX` marker, which in the shipped
/// gfx/FX tree appears ONLY in mapname.shader (verified 2026-07-06; notably
/// NOT in traderoute/arrow/particle, whose DiffuseTexture-bearing pixel
/// shaders this predicate alone would otherwise admit).
pub fn mapname_pixel_resource_slots_from_shader_bytecode(
    bytecode: &[u8],
) -> Option<BTreeMap<String, u32>> {
    let out = shader_texture_resource_slots_from_shader_bytecode(bytecode);
    mapname_pixel_resource_slots_match(&out).then_some(out)
}

pub fn mapname_pixel_resource_slots_match(resource_slots: &BTreeMap<String, u32>) -> bool {
    resource_slots
        .get(MAPNAME_DIFFUSE_TEXTURE_RESOURCE_SYMBOL)
        .copied()
        == Some(0)
}

/// SOUND over-approximation: `true` if `bytecode` might declare a
/// capture-family pixel shader, `false` only when it definitely matches no
/// family. Cheap by design — a raw byte-substring scan for the distinctive
/// family markers, with NO UTF-8 allocation and NO D3D reflection — so it can
/// gate the create-shader hook before any lock or reflection during the game's
/// load-time shader storm. Reflection can only surface resources whose names
/// appear in the bytecode string table, so a marker's absence as raw bytes
/// proves that resource is unbound; a real family shader always contains its
/// required marker and is therefore never rejected.
pub fn pixel_shader_bytecode_could_be_capture_family(bytecode: &[u8]) -> bool {
    PIXEL_SHADER_CAPTURE_FAMILY_MARKERS
        .iter()
        .any(|marker| byte_slice_contains(bytecode, marker.as_bytes()))
}

/// Naive substring search over raw bytes (no UTF-8 decoding). An empty needle
/// matches; a needle longer than the haystack cannot match.
fn byte_slice_contains(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }
    if haystack.len() < needle.len() {
        return false;
    }
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn required_resource_slots_from_shader_bytecode(
    bytecode: &[u8],
    required_resources: &[&str],
) -> Option<BTreeMap<String, u32>> {
    let out = shader_texture_resource_slots_from_shader_bytecode(bytecode);
    required_resources
        .iter()
        .all(|resource| out.contains_key(*resource))
        .then_some(out)
}

pub fn terrain_vertex_shader_metadata_from_shader_bytecode(
    bytecode: &[u8],
) -> Option<TerrainVertexShaderMetadata> {
    if !shader_bytecode_has_pdxmap_terrain_vertex_signature(bytecode) {
        return None;
    }
    reflect_terrain_vertex_shader_metadata(bytecode)
        .filter(terrain_vertex_shader_metadata_matches_pdxmap)
}

#[cfg(windows)]
fn reflect_terrain_vertex_shader_metadata(bytecode: &[u8]) -> Option<TerrainVertexShaderMetadata> {
    d3d11_runtime::reflect_terrain_vertex_shader_metadata(bytecode)
}

#[cfg(not(windows))]
fn reflect_terrain_vertex_shader_metadata(_bytecode: &[u8]) -> Option<TerrainVertexShaderMetadata> {
    None
}

pub fn terrain_vertex_shader_metadata_matches_pdxmap(
    metadata: &TerrainVertexShaderMetadata,
) -> bool {
    let has_position = metadata.input_parameters.iter().any(|parameter| {
        parameter.semantic_name.eq_ignore_ascii_case("POSITION")
            && parameter.semantic_index == 0
            && parameter.component_type == D3D_REGISTER_COMPONENT_SINT32
            && parameter.mask == 0x0f
    });
    let has_height = metadata.input_parameters.iter().any(|parameter| {
        parameter.semantic_name.eq_ignore_ascii_case("TEXCOORD")
            && parameter.semantic_index == 0
            && parameter.component_type == D3D_REGISTER_COMPONENT_SINT32
            && parameter.mask & 0x03 == 0x03
    });
    let has_cb1 = metadata
        .constant_buffers
        .values()
        .any(|bind_point| *bind_point == 1);
    has_position && has_height && has_cb1
}

pub fn shader_bytecode_has_pdxmap_terrain_vertex_signature(bytecode: &[u8]) -> bool {
    contains_ascii(bytecode, b"POSITION")
        && contains_ascii(bytecode, b"TEXCOORD")
        && contains_ascii(bytecode, b"QuadOffset_Scale_IsDetail")
        && contains_ascii(bytecode, b"vSnap")
}

pub fn shader_bytecode_has_terrain_idmap_signature(bytecode: &[u8]) -> bool {
    contains_ascii(bytecode, b"TerrainIDMap")
        && contains_ascii(bytecode, b"TerrainDiffuse")
        && contains_ascii(bytecode, b"TerrainColorTint")
}

pub fn validate_terrain_id_source_desc(desc: &Texture2DDesc) -> Result<(), TerrainCaptureError> {
    validate_rgba8_shader_resource_source_desc(desc)
}

pub fn validate_rgba8_shader_resource_source_desc(
    desc: &Texture2DDesc,
) -> Result<(), TerrainCaptureError> {
    if desc.width == 0 || desc.height == 0 {
        return Err(TerrainCaptureError::EmptyTexture);
    }
    if !is_supported_terrain_id_format(desc.format) {
        return Err(TerrainCaptureError::UnsupportedFormat(desc.format));
    }
    if desc.array_size != 1 {
        return Err(TerrainCaptureError::UnsupportedArraySize(desc.array_size));
    }
    if desc.sample_count != 1 {
        return Err(TerrainCaptureError::UnsupportedSampleCount(
            desc.sample_count,
        ));
    }
    if desc.bind_flags & D3D11_BIND_SHADER_RESOURCE == 0 {
        return Err(TerrainCaptureError::MissingShaderResourceBind);
    }
    if desc.cpu_access_flags != 0 {
        return Err(TerrainCaptureError::SourceCpuReadable);
    }
    Ok(())
}

pub fn validate_r32g32b32a32_float_shader_resource_source_desc(
    desc: &Texture2DDesc,
) -> Result<(), TerrainCaptureError> {
    if desc.width == 0 || desc.height == 0 {
        return Err(TerrainCaptureError::EmptyTexture);
    }
    if desc.format != DXGI_FORMAT_R32G32B32A32_FLOAT {
        return Err(TerrainCaptureError::UnsupportedFormat(desc.format));
    }
    if desc.array_size != 1 {
        return Err(TerrainCaptureError::UnsupportedArraySize(desc.array_size));
    }
    if desc.sample_count != 1 {
        return Err(TerrainCaptureError::UnsupportedSampleCount(
            desc.sample_count,
        ));
    }
    if desc.bind_flags & D3D11_BIND_SHADER_RESOURCE == 0 {
        return Err(TerrainCaptureError::MissingShaderResourceBind);
    }
    if desc.cpu_access_flags != 0 {
        return Err(TerrainCaptureError::SourceCpuReadable);
    }
    Ok(())
}

pub fn staging_desc_for_readback(
    source: &Texture2DDesc,
) -> Result<Texture2DDesc, TerrainCaptureError> {
    validate_terrain_id_source_desc(source)?;
    Ok(Texture2DDesc {
        width: source.width,
        height: source.height,
        mip_levels: 1,
        array_size: 1,
        format: source.format,
        sample_count: 1,
        sample_quality: 0,
        usage: D3D11_USAGE_STAGING,
        bind_flags: 0,
        cpu_access_flags: D3D11_CPU_ACCESS_READ,
        misc_flags: 0,
    })
}

pub fn readback_plan(source: &Texture2DDesc) -> Result<ReadbackPlan, TerrainCaptureError> {
    readback_plan_for_slot(source, TERRAIN_ID_SRV_SLOT)
}

pub fn readback_plan_for_slot(
    source: &Texture2DDesc,
    source_srv_slot: u32,
) -> Result<ReadbackPlan, TerrainCaptureError> {
    let staging_desc = staging_desc_for_readback(source)?;
    let row_bytes = rgba8_row_bytes(source.width);
    Ok(ReadbackPlan {
        source_srv_slot,
        source_format: source.format,
        staging_desc,
        map_type: D3D11_MAP_READ,
        row_bytes,
        total_bytes: row_bytes * source.height as usize,
    })
}

pub fn terrain_vertex_buffer_readback_plan(
    source: &BufferDesc,
    stride: u32,
    offset: u32,
) -> Result<TerrainVertexBufferReadbackPlan, TerrainVertexBufferError> {
    validate_terrain_vertex_buffer_source_desc(source, stride, offset)?;
    Ok(TerrainVertexBufferReadbackPlan {
        staging_desc: BufferDesc {
            byte_width: source.byte_width,
            usage: D3D11_USAGE_STAGING,
            bind_flags: 0,
            cpu_access_flags: D3D11_CPU_ACCESS_READ,
            misc_flags: 0,
            structure_byte_stride: 0,
        },
        map_type: D3D11_MAP_READ,
        captured_byte_len: terrain_vertex_sample_byte_len(source.byte_width),
        vertex_stride: stride,
        vertex_offset: offset,
    })
}

pub fn validate_terrain_vertex_buffer_source_desc(
    desc: &BufferDesc,
    stride: u32,
    offset: u32,
) -> Result<(), TerrainVertexBufferError> {
    if desc.byte_width == 0 {
        return Err(TerrainVertexBufferError::EmptyBuffer);
    }
    if desc.bind_flags & D3D11_BIND_VERTEX_BUFFER == 0 {
        return Err(TerrainVertexBufferError::MissingVertexBufferBind);
    }
    if desc.cpu_access_flags != 0 {
        return Err(TerrainVertexBufferError::SourceCpuReadable);
    }
    if stride == 0 {
        return Err(TerrainVertexBufferError::StrideTooSmall { stride, minimum: 1 });
    }
    if stride % 4 != 0 {
        return Err(TerrainVertexBufferError::StrideNotU32Aligned(stride));
    }
    if offset >= desc.byte_width {
        return Err(TerrainVertexBufferError::OffsetPastEnd {
            offset,
            byte_width: desc.byte_width,
        });
    }
    if offset.saturating_add(stride) > desc.byte_width {
        return Err(TerrainVertexBufferError::FirstVertexPastEnd {
            offset,
            stride,
            byte_width: desc.byte_width,
        });
    }
    Ok(())
}

pub fn terrain_vertex_sample_byte_len(byte_width: u32) -> usize {
    (byte_width as usize).min(TERRAIN_VERTEX_CAPTURE_SAMPLE_BYTES)
}

pub fn validate_terrain_input_layout(
    elements: &[TerrainInputElementDesc],
    vertex_stride: u32,
) -> Result<(), TerrainInputLayoutError> {
    if elements.is_empty() {
        return Err(TerrainInputLayoutError::EmptyLayout);
    }
    validate_terrain_input_element(
        elements,
        TerrainInputElementContract {
            semantic_name: "POSITION",
            semantic_index: 0,
            input_slot: 0,
            component_count: 4,
            allow_signed_normalized: false,
        },
        vertex_stride,
    )?;
    validate_terrain_input_element(
        elements,
        TerrainInputElementContract {
            semantic_name: "TEXCOORD",
            semantic_index: 0,
            input_slot: 1,
            component_count: 2,
            allow_signed_normalized: false,
        },
        vertex_stride,
    )
}

struct TerrainInputElementContract {
    semantic_name: &'static str,
    semantic_index: u32,
    input_slot: u32,
    component_count: u32,
    allow_signed_normalized: bool,
}

fn validate_terrain_input_element(
    elements: &[TerrainInputElementDesc],
    contract: TerrainInputElementContract,
    vertex_stride: u32,
) -> Result<(), TerrainInputLayoutError> {
    let element = find_input_element(elements, contract.semantic_name, contract.semantic_index)
        .ok_or(TerrainInputLayoutError::MissingElement {
            semantic_name: contract.semantic_name,
            semantic_index: contract.semantic_index,
        })?;
    let Some(format) = terrain_numeric_input_format(element.format) else {
        return Err(TerrainInputLayoutError::WrongFormat {
            semantic_name: contract.semantic_name,
            semantic_index: contract.semantic_index,
            expected: contract.expected_format_label(),
            actual: element.format,
        });
    };
    if format.component_count != contract.component_count {
        return Err(TerrainInputLayoutError::WrongFormat {
            semantic_name: contract.semantic_name,
            semantic_index: contract.semantic_index,
            expected: contract.expected_format_label(),
            actual: element.format,
        });
    }
    if format.class == NumericInputFormatClass::SignedNormalized
        && !contract.allow_signed_normalized
    {
        return Err(TerrainInputLayoutError::WrongFormat {
            semantic_name: contract.semantic_name,
            semantic_index: contract.semantic_index,
            expected: contract.expected_format_label(),
            actual: element.format,
        });
    }
    if element.input_slot != contract.input_slot {
        return Err(TerrainInputLayoutError::WrongInputSlot {
            semantic_name: contract.semantic_name,
            semantic_index: contract.semantic_index,
            expected: contract.input_slot,
            actual: element.input_slot,
        });
    }
    if element.aligned_byte_offset == D3D11_APPEND_ALIGNED_ELEMENT {
        return Err(TerrainInputLayoutError::AppendAlignedOffset {
            semantic_name: contract.semantic_name,
            semantic_index: contract.semantic_index,
        });
    }
    if element.input_slot_class != D3D11_INPUT_PER_VERTEX_DATA {
        return Err(TerrainInputLayoutError::WrongInputSlotClass {
            semantic_name: contract.semantic_name,
            semantic_index: contract.semantic_index,
            expected: D3D11_INPUT_PER_VERTEX_DATA,
            actual: element.input_slot_class,
        });
    }
    if element.instance_data_step_rate != 0 {
        return Err(TerrainInputLayoutError::WrongInstanceDataStepRate {
            semantic_name: contract.semantic_name,
            semantic_index: contract.semantic_index,
            expected: 0,
            actual: element.instance_data_step_rate,
        });
    }
    if element
        .aligned_byte_offset
        .saturating_add(format.byte_width)
        > vertex_stride
    {
        return Err(TerrainInputLayoutError::ElementOverrunsStride {
            semantic_name: contract.semantic_name,
            semantic_index: contract.semantic_index,
            offset: element.aligned_byte_offset,
            byte_width: format.byte_width,
            stride: vertex_stride,
        });
    }
    Ok(())
}

impl TerrainInputElementContract {
    fn expected_format_label(&self) -> &'static str {
        match self.component_count {
            4 => "signed integer DXGI format with 4 components for HLSL int4",
            2 => "signed integer DXGI format with 2 components for HLSL int2",
            _ => "signed integer DXGI format matching the HLSL input component count",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct NumericInputFormat {
    component_count: u32,
    byte_width: u32,
    class: NumericInputFormatClass,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NumericInputFormatClass {
    SignedInteger,
    SignedNormalized,
}

fn terrain_numeric_input_format(format: u32) -> Option<NumericInputFormat> {
    match format {
        DXGI_FORMAT_R32G32B32A32_SINT => Some(NumericInputFormat {
            component_count: 4,
            byte_width: 16,
            class: NumericInputFormatClass::SignedInteger,
        }),
        DXGI_FORMAT_R16G16B16A16_SINT => Some(NumericInputFormat {
            component_count: 4,
            byte_width: 8,
            class: NumericInputFormatClass::SignedInteger,
        }),
        DXGI_FORMAT_R8G8B8A8_SINT => Some(NumericInputFormat {
            component_count: 4,
            byte_width: 4,
            class: NumericInputFormatClass::SignedInteger,
        }),
        DXGI_FORMAT_R32G32_SINT => Some(NumericInputFormat {
            component_count: 2,
            byte_width: 8,
            class: NumericInputFormatClass::SignedInteger,
        }),
        DXGI_FORMAT_R16G16_SNORM => Some(NumericInputFormat {
            component_count: 2,
            byte_width: 4,
            class: NumericInputFormatClass::SignedNormalized,
        }),
        DXGI_FORMAT_R16G16_SINT => Some(NumericInputFormat {
            component_count: 2,
            byte_width: 4,
            class: NumericInputFormatClass::SignedInteger,
        }),
        DXGI_FORMAT_R8G8_SINT => Some(NumericInputFormat {
            component_count: 2,
            byte_width: 2,
            class: NumericInputFormatClass::SignedInteger,
        }),
        _ => None,
    }
}

fn find_input_element<'a>(
    elements: &'a [TerrainInputElementDesc],
    semantic_name: &str,
    semantic_index: u32,
) -> Option<&'a TerrainInputElementDesc> {
    elements.iter().find(|element| {
        element.semantic_name.eq_ignore_ascii_case(semantic_name)
            && element.semantic_index == semantic_index
    })
}

pub fn validate_mapped_row_pitch(row_pitch: usize, width: u32) -> Result<(), TerrainCaptureError> {
    let row_bytes = rgba8_row_bytes(width);
    if row_pitch < row_bytes {
        Err(TerrainCaptureError::InvalidMappedPitch {
            row_pitch,
            row_bytes,
        })
    } else {
        Ok(())
    }
}

pub fn copy_pitched_rgba_rows(
    rows: &[u8],
    row_pitch: usize,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, TerrainCaptureError> {
    copy_pitched_terrain_id_rows(rows, row_pitch, width, height, DXGI_FORMAT_R8G8B8A8_UNORM)
}

pub fn copy_pitched_terrain_id_rows(
    rows: &[u8],
    row_pitch: usize,
    width: u32,
    height: u32,
    format: u32,
) -> Result<Vec<u8>, TerrainCaptureError> {
    if !is_supported_terrain_id_format(format) {
        return Err(TerrainCaptureError::UnsupportedFormat(format));
    }
    validate_mapped_row_pitch(row_pitch, width)?;
    let row_bytes = rgba8_row_bytes(width);
    let required = row_pitch * height as usize;
    if rows.len() < required {
        return Err(TerrainCaptureError::MappedRowsTooShort {
            available: rows.len(),
            required,
        });
    }
    let mut out = Vec::with_capacity(row_bytes * height as usize);
    for row in 0..height as usize {
        let start = row * row_pitch;
        let source_row = &rows[start..start + row_bytes];
        match format {
            DXGI_FORMAT_R8G8B8A8_UNORM => out.extend_from_slice(source_row),
            DXGI_FORMAT_B8G8R8A8_UNORM | DXGI_FORMAT_B8G8R8A8_UNORM_SRGB => {
                for pixel in source_row.chunks_exact(4) {
                    out.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
                }
            }
            _ => unreachable!("format was checked above"),
        }
    }
    Ok(out)
}

pub fn copy_pitched_r32g32b32a32_float_rows(
    rows: &[u8],
    row_pitch: usize,
    width: u32,
    height: u32,
    format: u32,
) -> Result<Vec<u8>, TerrainCaptureError> {
    if format != DXGI_FORMAT_R32G32B32A32_FLOAT {
        return Err(TerrainCaptureError::UnsupportedFormat(format));
    }
    let row_bytes = rgba32f_row_bytes(width);
    if row_pitch < row_bytes {
        return Err(TerrainCaptureError::InvalidMappedPitch {
            row_pitch,
            row_bytes,
        });
    }
    let required = row_pitch * height as usize;
    if rows.len() < required {
        return Err(TerrainCaptureError::MappedRowsTooShort {
            available: rows.len(),
            required,
        });
    }
    let mut out = Vec::with_capacity(row_bytes * height as usize);
    for row in 0..height as usize {
        let start = row * row_pitch;
        out.extend_from_slice(&rows[start..start + row_bytes]);
    }
    Ok(out)
}

pub fn d3d11_texture2d_desc_fields() -> &'static [&'static str; 11] {
    &D3D11_TEXTURE2D_DESC_FIELD_NAMES
}

fn rgba8_row_bytes(width: u32) -> usize {
    width as usize * 4
}

fn rgba32f_row_bytes(width: u32) -> usize {
    width as usize * 16
}

fn is_supported_terrain_id_format(format: u32) -> bool {
    matches!(
        format,
        DXGI_FORMAT_R8G8B8A8_UNORM | DXGI_FORMAT_B8G8R8A8_UNORM | DXGI_FORMAT_B8G8R8A8_UNORM_SRGB
    )
}

fn parse_define_u32(text: &str, key: &str) -> Option<u32> {
    parse_define_float_token(text, key).and_then(|value| {
        let parsed = value.parse::<f32>().ok()?;
        (parsed.is_finite() && parsed > 0.0).then_some(parsed.round() as u32)
    })
}

fn parse_define_milli(text: &str, key: &str) -> Option<u32> {
    parse_define_float_token(text, key).and_then(|value| {
        let parsed = value.parse::<f32>().ok()?;
        (parsed.is_finite() && parsed > 0.0).then_some((parsed * 1000.0).round() as u32)
    })
}

fn parse_define_float_token<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        if parts.next() != Some("#define") {
            continue;
        }
        if parts.next() != Some(key) {
            continue;
        }
        return parts.next().map(|token| token.trim_end_matches('f'));
    }
    None
}

fn parse_register_t_slot(line: &str) -> Option<u32> {
    let register = line.find("register")?;
    let after_register = &line[register..];
    let open = after_register.find('(')?;
    let mut chars = after_register[open + 1..].trim_start().chars();
    if chars.next()? != 't' {
        return None;
    }
    let digits: String = chars.take_while(|ch| ch.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse().ok()
}

fn contains_ascii(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || haystack.len() < needle.len() {
        return false;
    }
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

#[cfg(windows)]
mod injected {
    use super::*;
    use core::ffi::c_void;
    use core::ptr;
    use std::ffi::{OsStr, OsString};
    use std::fs::OpenOptions;
    use std::io::Write;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::path::PathBuf;

    const DLL_PROCESS_DETACH: u32 = 0;
    const DLL_PROCESS_ATTACH: u32 = 1;
    const TERRAIN_CAPTURE_TIMEOUT_MS: u32 = 180_000;
    const FILE_NAME_CAP: usize = 32768;

    type Bool = i32;
    type Dword = u32;
    type Handle = *mut c_void;
    type Hmodule = *mut c_void;
    type Lpvoid = *mut c_void;
    type ThreadStart = Option<unsafe extern "system" fn(Lpvoid) -> Dword>;

    #[link(name = "kernel32")]
    extern "system" {
        fn CloseHandle(handle: Handle) -> Bool;
        fn CreateThread(
            attrs: Lpvoid,
            stack_size: usize,
            start: ThreadStart,
            param: Lpvoid,
            flags: Dword,
            thread_id: *mut Dword,
        ) -> Handle;
        fn DisableThreadLibraryCalls(module: Hmodule) -> Bool;
        fn GetCurrentProcessId() -> Dword;
        fn GetModuleFileNameW(module: Hmodule, filename: *mut u16, size: Dword) -> Dword;
        fn GetModuleHandleW(module_name: *const u16) -> Hmodule;
    }

    #[link(name = "user32")]
    extern "system" {
        fn CallNextHookEx(hook: *mut c_void, code: i32, wparam: usize, lparam: isize) -> isize;
    }

    static mut DLL_MODULE: Hmodule = ptr::null_mut();

    pub unsafe fn dll_main(module: Hmodule, reason: Dword) -> Bool {
        if reason == DLL_PROCESS_ATTACH {
            unsafe {
                DLL_MODULE = module;
                DisableThreadLibraryCalls(module);
                let handle = CreateThread(
                    ptr::null_mut(),
                    0,
                    Some(attach_worker),
                    ptr::null_mut(),
                    0,
                    ptr::null_mut(),
                );
                if !handle.is_null() {
                    CloseHandle(handle);
                }
            }
        } else if reason == DLL_PROCESS_DETACH {
            super::d3d11_runtime::record_process_detach_hook_lock_stats();
        }
        1
    }

    pub unsafe fn get_msg_proc(code: i32, wparam: usize, lparam: isize) -> isize {
        unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) }
    }

    unsafe extern "system" fn attach_worker(_param: Lpvoid) -> Dword {
        if !is_supported_capture_process() {
            return 0;
        }
        let Some(path) = module_path() else {
            return 0;
        };
        write_probe_frame(&path);
        if let Some(dir) = path.parent() {
            super::d3d11_runtime::run_one_shot_capture(dir, TERRAIN_CAPTURE_TIMEOUT_MS);
        }
        0
    }

    fn write_probe_frame(module_path: &PathBuf) {
        let mut path = module_path.clone();
        let pid = unsafe { GetCurrentProcessId() };
        path.set_file_name("terrain_capture_probe.jsonl");
        let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) else {
            return;
        };
        let frame = format!(
            "{{\"type\":\"terrain_capture_started\",\"backend\":\"terrain_capture_d3d11\",\"process_id\":{},\"d3d11_loaded\":{},\"dxgi_loaded\":{},\"terrain_id_srv_slot\":{},\"device_create_pixel_shader_index\":{},\"context_ps_set_shader_resources_index\":{},\"context_draw_indexed_index\":{},\"context_draw_index\":{},\"context_draw_indexed_instanced_index\":{},\"context_draw_instanced_index\":{},\"context_ia_get_vertex_buffers_index\":{},\"context_copy_resource_index\":{},\"context_map_index\":{},\"context_unmap_index\":{}}}\n",
            pid,
            module_loaded("d3d11.dll"),
            module_loaded("dxgi.dll"),
            TERRAIN_ID_SRV_SLOT,
            DEVICE_CREATE_PIXEL_SHADER_INDEX,
            CONTEXT_PS_SET_SHADER_RESOURCES_INDEX,
            CONTEXT_DRAW_INDEXED_INDEX,
            CONTEXT_DRAW_INDEX,
            CONTEXT_DRAW_INDEXED_INSTANCED_INDEX,
            CONTEXT_DRAW_INSTANCED_INDEX,
            CONTEXT_IA_GET_VERTEX_BUFFERS_INDEX,
            CONTEXT_COPY_RESOURCE_INDEX,
            CONTEXT_MAP_INDEX,
            CONTEXT_UNMAP_INDEX,
        );
        let _ = file.write_all(frame.as_bytes());
    }

    fn module_path() -> Option<PathBuf> {
        let mut buf = vec![0u16; FILE_NAME_CAP];
        let len = unsafe {
            GetModuleFileNameW(DLL_MODULE, buf.as_mut_ptr(), u32::try_from(buf.len()).ok()?)
        } as usize;
        if len == 0 || len >= buf.len() {
            return None;
        }
        buf.truncate(len);
        Some(PathBuf::from(OsString::from_wide(&buf)))
    }

    fn module_loaded(name: &str) -> bool {
        let wide = wide_null(name);
        !unsafe { GetModuleHandleW(wide.as_ptr()) }.is_null()
    }

    fn is_supported_capture_process() -> bool {
        process_path()
            .and_then(|path| path.file_name().map(|name| name.to_os_string()))
            .and_then(|name| name.to_str().map(|name| name.to_string()))
            .is_some_and(|name| {
                name.eq_ignore_ascii_case("hoi4.exe")
                    || name.eq_ignore_ascii_case("capture_harness.exe")
            })
    }

    fn process_path() -> Option<PathBuf> {
        let mut buf = vec![0u16; FILE_NAME_CAP];
        let len = unsafe {
            GetModuleFileNameW(
                ptr::null_mut(),
                buf.as_mut_ptr(),
                u32::try_from(buf.len()).ok()?,
            )
        } as usize;
        if len == 0 || len >= buf.len() {
            return None;
        }
        buf.truncate(len);
        Some(PathBuf::from(OsString::from_wide(&buf)))
    }

    fn wide_null(value: &str) -> Vec<u16> {
        OsStr::new(value).encode_wide().chain([0]).collect()
    }
}

#[cfg(windows)]
#[no_mangle]
pub unsafe extern "system" fn DllMain(
    module: *mut core::ffi::c_void,
    reason: u32,
    _reserved: *mut core::ffi::c_void,
) -> i32 {
    unsafe { injected::dll_main(module, reason) }
}

#[cfg(windows)]
#[no_mangle]
pub unsafe extern "system" fn Hoi4TerrainCaptureGetMsgProc(
    code: i32,
    wparam: usize,
    lparam: isize,
) -> isize {
    unsafe { injected::get_msg_proc(code, wparam, lparam) }
}

#[cfg(windows)]
#[no_mangle]
pub unsafe extern "system" fn Hoi4TerrainCaptureGetHookLockStats(
    out: *mut d3d11_runtime::HookLockStatsSnapshot,
) -> i32 {
    if out.is_null() {
        return 0;
    }
    unsafe {
        *out = d3d11_runtime::hook_lock_stats_snapshot();
    }
    1
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    const TERRAIN_CAPTURE_FIXTURE_DIRS_ENV: &str = "BATTLE_PLAN_TERRAIN_CAPTURE_FIXTURE_DIRS";

    fn first_capture_fixture(file_name: &str) -> Option<PathBuf> {
        std::env::var_os(TERRAIN_CAPTURE_FIXTURE_DIRS_ENV)
            .into_iter()
            .flat_map(|value| std::env::split_paths(&value).collect::<Vec<_>>())
            .map(|dir| dir.join(file_name))
            .find(|path| path.is_file())
    }

    #[test]
    fn d3d11_vtable_indices_match_sdk_order_needed_for_capture() {
        assert_eq!(SWAP_CHAIN_GET_DEVICE_INDEX, 7);
        assert_eq!(SWAP_CHAIN_PRESENT_INDEX, 8);
        assert_eq!(SWAP_CHAIN_RESIZE_BUFFERS_INDEX, 13);
        assert_eq!(DEVICE_CREATE_INPUT_LAYOUT_INDEX, 11);
        assert_eq!(DEVICE_CREATE_PIXEL_SHADER_INDEX, 15);
        assert_eq!(DEVICE_CREATE_SHADER_RESOURCE_VIEW_INDEX, 7);
        assert_eq!(CONTEXT_PS_SET_SHADER_RESOURCES_INDEX, 8);
        assert_eq!(CONTEXT_PS_SET_SHADER_INDEX, 9);
        assert_eq!(CONTEXT_DRAW_INDEXED_INDEX, 12);
        assert_eq!(CONTEXT_DRAW_INDEX, 13);
        assert_eq!(CONTEXT_IA_SET_INPUT_LAYOUT_INDEX, 17);
        assert_eq!(CONTEXT_IA_SET_VERTEX_BUFFERS_INDEX, 18);
        assert_eq!(CONTEXT_DRAW_INDEXED_INSTANCED_INDEX, 20);
        assert_eq!(CONTEXT_DRAW_INSTANCED_INDEX, 21);
        assert_eq!(CONTEXT_OM_SET_RENDER_TARGETS_INDEX, 33);
        assert_eq!(CONTEXT_OM_SET_RENDER_TARGETS_AND_UAVS_INDEX, 34);
        assert_eq!(CONTEXT_OM_SET_BLEND_STATE_INDEX, 35);
        assert_eq!(CONTEXT_OM_SET_DEPTH_STENCIL_STATE_INDEX, 36);
        assert_eq!(CONTEXT_COPY_SUBRESOURCE_REGION_INDEX, 46);
        assert_eq!(CONTEXT_COPY_RESOURCE_INDEX, 47);
        assert_eq!(CONTEXT_CLEAR_RENDER_TARGET_VIEW_INDEX, 50);
        assert_eq!(CONTEXT_CLEAR_DEPTH_STENCIL_VIEW_INDEX, 53);
        assert_eq!(CONTEXT_PS_GET_SHADER_RESOURCES_INDEX, 73);
        assert_eq!(CONTEXT_IA_GET_INPUT_LAYOUT_INDEX, 78);
        assert_eq!(CONTEXT_IA_GET_VERTEX_BUFFERS_INDEX, 79);
        assert_eq!(CONTEXT_MAP_INDEX, 14);
        assert_eq!(CONTEXT_UNMAP_INDEX, 15);
        assert_eq!(SHADER_RESOURCE_VIEW_GET_RESOURCE_INDEX, 7);
        assert_eq!(SHADER_RESOURCE_VIEW_GET_DESC_INDEX, 8);
        assert_eq!(TEXTURE2D_GET_DESC_INDEX, 10);
        assert_eq!(D3D11_IA_VERTEX_INPUT_STRUCTURE_ELEMENT_COUNT, 32);
    }

    #[test]
    fn map_viewer_accepts_shared_constant_buffer_naming_contract() {
        let root = temp_test_dir("cb_name_contract");
        let bundle_path = write_stale_constant_buffer_name_contract_fixture(&root);

        let stale_error = map_viewer::validate_terrain_d3d11_bundle(&bundle_path)
            .expect_err("stale fixture must reach the renderer constant-buffer naming checks")
            .to_string();
        assert!(
            has_constant_buffer_naming_complaint(&stale_error),
            "stale fixture did not exercise the naming contract, got: {stale_error}"
        );

        let report = repair_map_frame_bundle_constant_buffer_names(&bundle_path)
            .expect("repair constant-buffer names through shared terrain_capture logic");
        assert!(
            report.changed(),
            "repair should rewrite stale constant-buffer names"
        );

        if let Err(error) = map_viewer::validate_terrain_d3d11_bundle(&bundle_path) {
            let message = error.to_string();
            assert!(
                !has_constant_buffer_naming_complaint(&message),
                "repaired fixture still trips constant-buffer naming: {message}"
            );
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn shader_cache_signature_records_declared_terrain_id_slot() {
        let text = r#"
            #define MAP_SIZE_X 5632.000
            #define MAP_SIZE_Y 2048.000
            #define MAP_POW2_X 1.000
            #define MAP_POW2_Y 1.000
            #define FOW_POW2_X 0.500
            #define FOW_POW2_Y 0.250
            Texture2D TerrainIDMap_Texture : register(t5);
        "#;
        let signature = shader_cache_signature(text).expect("pdxmap terrain cache signature");
        assert_eq!(
            signature,
            ShaderCacheSignature {
                map_size_x: 5632,
                map_size_y: 2048,
                map_pow2_x_milli: 1000,
                map_pow2_y_milli: 1000,
                fow_pow2_x_milli: 500,
                fow_pow2_y_milli: 250,
                terrain_id_srv_slot: 5,
            }
        );
        assert_eq!(
            shader_cache_signature(&text.replace("register(t5)", "register(t3)"))
                .expect("t3 variant is valid")
                .terrain_id_srv_slot,
            3
        );
        assert!(
            shader_cache_signature(&text.replace("TerrainIDMap_Texture", "OtherTexture")).is_none()
        );
    }

    #[test]
    fn shader_bytecode_signature_uses_reflection_names() {
        let bytes = b"DXBC....RDEF....TerrainDiffuse\0TerrainIDMap_Texture : register(t3)\0TerrainColorTint\0";
        assert!(shader_bytecode_has_terrain_idmap_signature(bytes));
        assert_eq!(terrain_id_slot_from_shader_bytecode(bytes), Some(3));
        assert_eq!(
            shader_resource_slot_from_shader_bytecode(bytes, TERRAIN_ID_RESOURCE_SYMBOL),
            Some(3)
        );
        assert!(!shader_bytecode_has_terrain_idmap_signature(
            b"DXBC TerrainDiffuse TerrainColorTint"
        ));
        assert_eq!(
            terrain_id_slot_from_shader_bytecode(
                b"DXBC TerrainDiffuse TerrainIDMap_Texture TerrainColorTint"
            ),
            None
        );
    }

    #[test]
    fn pdxmap_resource_slots_include_snow_mud_when_declared_by_shader() {
        let bytes = b"DXBC TerrainDiffuse TerrainColorTint \
            TerrainIDMap_Texture : register(t5); \
            SnowMudData_Texture : register(t7); \
            MudDiffuseGloss_Texture : register(t13);";
        let slots = pdxmap_resource_slots_from_shader_bytecode(bytes)
            .expect("pdxmap terrain shader resources");
        assert_eq!(slots.get(TERRAIN_ID_RESOURCE_SYMBOL), Some(&5));
        assert_eq!(slots.get(SNOW_MUD_DATA_RESOURCE_SYMBOL), Some(&7));
        assert_eq!(slots.get("MudDiffuseGloss"), Some(&13));
        assert_eq!(
            shader_resource_slot_from_shader_text(
                "Texture2D SnowMudData_Texture : register(t6);",
                SNOW_MUD_DATA_RESOURCE_SYMBOL
            ),
            Some(6)
        );
    }

    #[test]
    fn pdxmap_resource_slots_require_terrain_shader_signature() {
        assert!(pdxmap_resource_slots_from_shader_bytecode(
            b"SnowMudData_Texture : register(t7); TerrainDiffuse TerrainColorTint",
        )
        .is_none());
    }

    #[test]
    fn pdxwater_resource_slots_require_water_shader_signature() {
        let slots = pdxwater_resource_slots_from_shader_bytecode(
            b"HeightTexture_Texture : register(t0); LeanTexture1_Texture : register(t1); LeanTexture2_Texture : register(t2); SpecularMap_Texture : register(t4); WaterRefraction_Texture : register(t5)",
        )
        .expect("pdxwater shader resources");
        assert_eq!(slots.get(WATER_HEIGHT_TEXTURE_SYMBOL), Some(&0));
        assert_eq!(slots.get(WATER_LEAN_TEXTURE1_SYMBOL), Some(&1));
        assert_eq!(slots.get(WATER_LEAN_TEXTURE2_SYMBOL), Some(&2));
        assert_eq!(slots.get(WATER_SPECULAR_MAP_SYMBOL), Some(&4));
        assert_eq!(slots.get(WATER_REFRACTION_RESOURCE_SYMBOL), Some(&5));
        assert!(pdxwater_resource_slots_from_shader_bytecode(
            b"TerrainIDMap_Texture : register(t5); TerrainDiffuse TerrainColorTint"
        )
        .is_none());
        assert!(pdxwater_resource_slots_from_shader_bytecode(
            b"HeightTexture_Texture : register(t0); LeanTexture1_Texture : register(t1); SpecularMap_Texture : register(t4)"
        )
        .is_none());
        assert!(pdxwater_resource_slots_from_shader_bytecode(
            b"HeightTexture_Texture : register(t0); LeanTexture1_Texture : register(t1); LeanTexture2_Texture : register(t2); SpecularMap_Texture : register(t4)"
        )
        .is_none());
    }

    #[test]
    fn map_layer_resource_slots_require_distinct_shader_signatures() {
        // Rivers declare SnowMudTexture (river.shader:85), not SnowMudData.
        let river_slots = river_resource_slots_from_shader_bytecode(
            b"RiverData_Texture : register(t3); LeanTexture1_Texture : register(t4); \
              LeanTexture2_Texture : register(t5); SnowMudTexture_Texture : register(t8); \
              GradientBorderChannel1_Texture : register(t12); \
              GradientBorderChannel2_Texture : register(t13);",
        )
        .expect("river shader resources");
        assert_eq!(river_slots.get(RIVER_DATA_RESOURCE_SYMBOL), Some(&3));
        assert_eq!(river_slots.get(WATER_LEAN_TEXTURE1_SYMBOL), Some(&4));

        let border_slots = border_resource_slots_from_shader_bytecode(
            b"BorderDiffuse_Texture : register(t0); SnowMudData_Texture : register(t1); \
              IntelMap_Texture : register(t2); ShadowMap_Texture : register(t3);",
        )
        .expect("border shader resources");
        assert_eq!(border_slots.get(BORDER_DIFFUSE_RESOURCE_SYMBOL), Some(&0));

        let tree_slots = tree_resource_slots_from_shader_bytecode(
            b"DiffuseMap_Texture : register(t0); NormalMap_Texture : register(t1); \
              TintMap_Texture : register(t2); SeasonMap_Texture : register(t3); \
              TreeMaskTexture_Texture : register(t14); \
              GradientBorderChannel1_Texture : register(t12); \
              GradientBorderChannel2_Texture : register(t13);",
        )
        .expect("tree shader resources");
        assert_eq!(tree_slots.get(TREE_MASK_RESOURCE_SYMBOL), Some(&14));

        assert!(river_resource_slots_from_shader_bytecode(
            b"HeightTexture_Texture : register(t0); LeanTexture1_Texture : register(t1); LeanTexture2_Texture : register(t2); SpecularMap_Texture : register(t4);"
        )
        .is_none());
        assert!(border_resource_slots_from_shader_bytecode(
            b"TerrainIDMap_Texture : register(t5); TerrainDiffuse TerrainColorTint"
        )
        .is_none());
        assert!(tree_resource_slots_from_shader_bytecode(
            b"TintMap_Texture : register(t2); SeasonMap_Texture : register(t3); TreeMaskTexture_Texture : register(t14);"
        )
        .is_none());
    }

    #[test]
    fn terrain_vertex_shader_metadata_requires_d3d_reflection_not_ascii_fallback() {
        let bytes = b"DXBC POSITION TEXCOORD QuadOffset_Scale_IsDetail vSnap";
        assert!(shader_bytecode_has_pdxmap_terrain_vertex_signature(bytes));
        assert!(terrain_vertex_shader_metadata_from_shader_bytecode(bytes).is_none());
    }

    #[test]
    fn terrain_vertex_shader_metadata_rejects_unproven_signature() {
        assert!(terrain_vertex_shader_metadata_from_shader_bytecode(
            b"DXBC POSITION TEXCOORD TerrainDiffuse TerrainColorTint"
        )
        .is_none());
        assert!(!terrain_vertex_shader_metadata_matches_pdxmap(
            &TerrainVertexShaderMetadata {
                input_parameters: vec![ShaderInputParameter {
                    semantic_name: "POSITION".to_string(),
                    semantic_index: 0,
                    register: 0,
                    component_type: D3D_REGISTER_COMPONENT_SINT32,
                    mask: 0x0f,
                    read_write_mask: 0x0f,
                }],
                constant_buffers: BTreeMap::new(),
                byte_len: 32,
                fnv1a64: "abc".to_string(),
            }
        ));
    }

    #[test]
    fn capture_state_tracks_active_terrain_vertex_shader_by_reflection() {
        let mut state = TerrainCaptureState::default();
        let metadata = TerrainVertexShaderMetadata {
            input_parameters: vec![
                ShaderInputParameter {
                    semantic_name: "POSITION".to_string(),
                    semantic_index: 0,
                    register: 0,
                    component_type: D3D_REGISTER_COMPONENT_SINT32,
                    mask: 0x0f,
                    read_write_mask: 0x0f,
                },
                ShaderInputParameter {
                    semantic_name: "TEXCOORD".to_string(),
                    semantic_index: 0,
                    register: 1,
                    component_type: D3D_REGISTER_COMPONENT_SINT32,
                    mask: 0x03,
                    read_write_mask: 0x03,
                },
            ],
            constant_buffers: BTreeMap::from([(TERRAIN_DRAW_CB1_LAYOUT_SOURCE.to_string(), 1)]),
            byte_len: 0,
            fnv1a64: String::new(),
        };
        assert!(!state.observe_vertex_shader_created(0, b"DXBC", Some(metadata.clone())));
        assert!(!state.observe_vertex_shader_created(
            0x70,
            b"terrain vs bytes",
            Some(metadata.clone())
        ));
        let bytecode = b"DXBC POSITION TEXCOORD QuadOffset_Scale_IsDetail vSnap";
        assert!(state.observe_vertex_shader_created(0x70, bytecode, Some(metadata)));
        assert_eq!(state.terrain_vertex_shader_count(), 1);
        assert!(state.terrain_vertex_shader().is_none());
        state.observe_vs_set_shader(0x80);
        assert!(state.terrain_vertex_shader().is_none());
        state.observe_vs_set_shader(0x70);
        assert_eq!(
            state.terrain_vertex_shader(),
            Some(TerrainVertexShaderBinding {
                vertex_shader: 0x70
            })
        );
        let metadata = state
            .terrain_vertex_shader_metadata(0x70)
            .expect("tracked vertex shader metadata");
        assert_eq!(metadata.byte_len, bytecode.len());
        assert_eq!(metadata.fnv1a64, fnv1a64_bytes(bytecode));
    }

    #[test]
    fn capture_state_tracks_pdxmap_shader_then_slot_five_srv() {
        let mut state = TerrainCaptureState::default();
        assert!(!state.observe_pixel_shader_created(0x10, b"unrelated"));
        assert!(state.observe_pixel_shader_created(
            0x20,
            b"TerrainIDMap_Texture : register(t5); TerrainDiffuse TerrainColorTint"
        ));
        state.observe_ps_set_shader(0x10);
        assert!(!state.observe_ps_set_shader_resources(4, &[0x44, 0x55, 0x66]));
        state.observe_ps_set_shader(0x20);
        assert!(state.observe_ps_set_shader_resources(4, &[0x44, 0x55, 0x66]));
        assert_eq!(
            state.terrain_id_srv(),
            Some(TerrainSrvBinding { srv: 0x55, slot: 5 })
        );
    }

    #[test]
    fn capture_state_tracks_active_pdxwater_shader_without_terrain_id() {
        let mut state = TerrainCaptureState::default();
        let water_shader = b"HeightTexture_Texture : register(t0); LeanTexture1_Texture : register(t1); LeanTexture2_Texture : register(t2); SpecularMap_Texture : register(t4); WaterRefraction_Texture : register(t5)";
        assert!(!state.observe_pixel_shader_created(0x90, water_shader));
        assert!(state.observe_water_pixel_shader_created(0x90, water_shader));
        assert_eq!(state.pdxmap_pixel_shader_count(), 0);
        assert_eq!(state.pdxwater_pixel_shader_count(), 1);
        state.observe_ps_set_shader(0x90);
        assert_eq!(
            state.pdxwater_pixel_shader(),
            Some(MapPixelShaderBinding { pixel_shader: 0x90 })
        );
        assert!(state.pdxmap_pixel_shader().is_none());
        assert_eq!(
            state
                .pdxwater_pixel_shader_metadata(0x90)
                .expect("water shader metadata")
                .resource_slots
                .get(WATER_HEIGHT_TEXTURE_SYMBOL),
            Some(&0)
        );
        assert_eq!(
            state
                .pdxwater_pixel_shader_metadata(0x90)
                .expect("water shader metadata")
                .resource_slots
                .get(WATER_REFRACTION_RESOURCE_SYMBOL),
            Some(&5)
        );
    }

    #[test]
    fn capture_state_tracks_active_restore_shader_and_posteffect_cb() {
        let mut state = TerrainCaptureState::default();
        let restore_shader = b"MainScene_Texture : register(t0); RestoreBloom_Texture : register(t1); ColorCube_Texture : register(t2); AverageLuminanceTexture_Texture : register(t3)";
        assert!(!state.observe_pixel_shader_created(0xA0, restore_shader));
        assert!(state.observe_restore_pixel_shader_created(0xA0, restore_shader));
        assert_eq!(state.restore_pixel_shader_count(), 1);
        state.observe_ps_set_shader(0xA0);
        assert_eq!(
            state.restore_pixel_shader(),
            Some(MapPixelShaderBinding { pixel_shader: 0xA0 })
        );
        assert_eq!(
            state.current_pixel_shader_constant_buffer_name(RESTORE_POSTEFFECT_BASE_CB_BIND_POINT),
            Some(RESTORE_POSTEFFECT_BASE_CB_SOURCE.to_string())
        );
    }

    #[test]
    fn capture_family_prefilter_never_rejects_a_real_family_shader() {
        // Every genuine family fixture must survive the lock-free pre-filter
        // (its distinctive required marker is present), or the load-freeze fix
        // would silently drop that layer's capture.
        let pdxmap = b"TerrainIDMap_Texture : register(t5); SnowMudData_Texture : register(t7); TerrainDiffuse TerrainColorTint";
        let pdxwater = b"HeightTexture : register(t0); LeanTexture1 : register(t1); LeanTexture2 : register(t2); SpecularMap : register(t3); WaterRefraction : register(t5);";
        let river = b"RiverData_Texture : register(t3); LeanTexture1_Texture : register(t4); LeanTexture2_Texture : register(t5); SnowMudTexture_Texture : register(t8);";
        let border = b"BorderDiffuse_Texture : register(t0); SnowMudData_Texture : register(t1); IntelMap_Texture : register(t2); ShadowMap_Texture : register(t3);";
        let tree = b"DiffuseMap_Texture : register(t0); TreeMaskTexture_Texture : register(t14); TintMap SeasonMap";
        let restore = b"MainScene_Texture : register(t0); RestoreBloom_Texture : register(t1); ColorCube_Texture : register(t2); AverageLuminanceTexture_Texture : register(t3);";
        let mapname = b"DiffuseTexture_Texture : register(t0); Transp_OffsetX";
        for (name, bytes) in [
            ("pdxmap", pdxmap.as_slice()),
            ("pdxwater", pdxwater.as_slice()),
            ("river", river.as_slice()),
            ("border", border.as_slice()),
            ("tree", tree.as_slice()),
            ("restore", restore.as_slice()),
            ("mapname", mapname.as_slice()),
        ] {
            assert!(
                pixel_shader_bytecode_could_be_capture_family(bytes),
                "pre-filter must pass the {name} family shader"
            );
        }
    }

    #[test]
    fn capture_family_prefilter_rejects_non_family_shaders() {
        // The overwhelming majority of the game load shaders declare none of the
        // distinctive family markers and must be rejected without a lock.
        let generic_ui = b"DiffuseMap : register(t0); ColorMask : register(t1); vBlendFactor";
        let empty: &[u8] = b"";
        let unrelated = b"CloudTexture NoiseMap FontAtlas register(t0) register(t1)";
        assert!(!pixel_shader_bytecode_could_be_capture_family(generic_ui));
        assert!(!pixel_shader_bytecode_could_be_capture_family(empty));
        assert!(!pixel_shader_bytecode_could_be_capture_family(unrelated));
    }

    #[test]
    fn capture_family_prefilter_matches_suffixed_resource_names() {
        // Reflection keys on the base name (TerrainIDMap) while bytecode may
        // spell it TerrainIDMap_Texture; the raw substring scan must still see
        // the marker inside the suffixed declaration.
        let suffixed = b"...garbage... TerrainIDMap_Texture : register(t5) ...more...";
        assert!(pixel_shader_bytecode_could_be_capture_family(suffixed));
    }

    #[test]
    fn capfix2_prefilter_admits_synthetic_pdxmap_pixel_shader_blob() {
        // CAPFIX2 INVESTIGATION 3: the pdxmap terrain PS declares TerrainIDMap +
        // SnowMudData. The lock-free create-shader prefilter MUST always admit a
        // blob carrying those markers (its distinctive TerrainIDMap marker is a
        // member of PIXEL_SHADER_CAPTURE_FAMILY_MARKERS), or the terrain PS is
        // never observed -> no active pdxmap PS -> every map frame rejected
        // (exactly the "no active pdxmap pixel shader for map-frame capture"
        // rejection seen in all four LAW sessions tonight).
        let pdxmap_like =
            b"DXBC....TerrainIDMap....SnowMudData....TerrainDiffuse....TerrainColorTint";
        assert!(
            pixel_shader_bytecode_could_be_capture_family(pdxmap_like),
            "prefilter must admit a pdxmap-like PS (TerrainIDMap + SnowMudData)"
        );
        assert!(
            shader_bytecode_has_terrain_idmap_signature(pdxmap_like),
            "the synthetic blob must satisfy the terrain-idmap signature"
        );
    }

    #[test]
    fn capfix2_prefilter_admits_real_captured_pdxmap_pixel_shader() {
        // CAPFIX2 INVESTIGATION 3: prove the prefilter admits the REAL captured
        // pdxmap terrain PS bytecode when an external capture corpus is supplied.
        // The repo does not carry harness run artifacts as permanent fixtures.
        let Some(path) = first_capture_fixture("pdxmap_pixel_shader_ce5e5185d46e214b.bin") else {
            eprintln!(
                "capfix2_prefilter_admits_real_captured_pdxmap_pixel_shader skipped: set {TERRAIN_CAPTURE_FIXTURE_DIRS_ENV} to a capture run containing pdxmap_pixel_shader_ce5e5185d46e214b.bin"
            );
            return;
        };
        let bytes = fs::read(&path).expect("read captured pdxmap pixel shader bytecode");
        assert!(
            pixel_shader_bytecode_could_be_capture_family(&bytes),
            "prefilter must admit the real captured pdxmap PS {}",
            path.display()
        );
        assert!(
            terrain_id_slot_from_shader_bytecode(&bytes).is_some(),
            "the real captured pdxmap PS must expose a TerrainIDMap SRV slot"
        );
    }

    #[test]
    fn restore_pass_identifier_matches_exact_resource_registers() {
        let restore = b"
            Texture2D MainScene_Texture : register(t0);
            Texture2D RestoreBloom_Texture : register(t1);
            Texture2D ColorCube_Texture : register(t2);
            Texture2D AverageLuminanceTexture_Texture : register(t3);
        ";
        let slots =
            restore_resource_slots_from_shader_bytecode(restore).expect("restore signature");
        assert_eq!(
            slots.get(RESTORE_MAIN_SCENE_RESOURCE_SYMBOL).copied(),
            Some(0)
        );
        assert_eq!(slots.get(RESTORE_BLOOM_RESOURCE_SYMBOL).copied(), Some(1));
        assert_eq!(
            slots.get(RESTORE_COLOR_CUBE_RESOURCE_SYMBOL).copied(),
            Some(2)
        );
        assert_eq!(
            slots
                .get(RESTORE_AVERAGE_LUMINANCE_RESOURCE_SYMBOL)
                .copied(),
            Some(3)
        );
    }

    #[test]
    fn restore_pass_identifier_rejects_browser_overlay_small_draw_shader() {
        let browser_overlay = b"
            Texture2D BrowserTexture : register(t0);
            Texture2D ColorMask : register(t1);
            SamplerState BrowserSampler : register(s0);
        ";
        let draw_call = "Draw";
        let draw_count = 3;
        assert_eq!((draw_call, draw_count), ("Draw", 3));
        assert!(restore_resource_slots_from_shader_bytecode(browser_overlay).is_none());
    }

    #[test]
    fn restore_pass_identifier_is_not_shape_gated() {
        let fullscreen_triangle = b"
            Texture2D MainScene : register(t0);
            Texture2D RestoreBloom : register(t1);
            Texture2D ColorCube : register(t2);
            Texture2D AverageLuminanceTexture : register(t3);
        ";
        let draw_call = "Draw";
        let draw_count = 3;
        assert_eq!((draw_call, draw_count), ("Draw", 3));
        assert!(restore_resource_slots_from_shader_bytecode(fullscreen_triangle).is_some());
    }

    fn mapname_vs_input(
        semantic_name: &str,
        component_type: u32,
        mask: u8,
    ) -> ShaderInputParameter {
        ShaderInputParameter {
            semantic_name: semantic_name.to_string(),
            semantic_index: 0,
            register: 0,
            component_type,
            mask,
            read_write_mask: mask,
        }
    }

    fn mapname_vs_metadata(
        inputs: Vec<ShaderInputParameter>,
        cb_bind_point: Option<u32>,
    ) -> TerrainVertexShaderMetadata {
        let mut constant_buffers = BTreeMap::new();
        if let Some(bind_point) = cb_bind_point {
            constant_buffers.insert("Transp_OffsetX".to_string(), bind_point);
        }
        TerrainVertexShaderMetadata {
            input_parameters: inputs,
            constant_buffers,
            byte_len: 0,
            fnv1a64: String::new(),
        }
    }

    #[test]
    fn mapname_vertex_discriminator_matches_float3_position_texcoord_cb1() {
        let metadata = mapname_vs_metadata(
            vec![
                mapname_vs_input("POSITION", D3D_REGISTER_COMPONENT_FLOAT32, 0x07),
                mapname_vs_input("TEXCOORD", D3D_REGISTER_COMPONENT_FLOAT32, 0x03),
            ],
            Some(1),
        );
        assert!(terrain_vertex_shader_metadata_matches_mapname(&metadata));
    }

    #[test]
    fn mapname_vertex_discriminator_rejects_terrain_sint_position() {
        // Terrain VS: int4 POSITION + int2 TEXCOORD (SINT32) — not the label VS.
        let metadata = mapname_vs_metadata(
            vec![
                mapname_vs_input("POSITION", D3D_REGISTER_COMPONENT_SINT32, 0x0f),
                mapname_vs_input("TEXCOORD", D3D_REGISTER_COMPONENT_SINT32, 0x03),
            ],
            Some(1),
        );
        assert!(!terrain_vertex_shader_metadata_matches_mapname(&metadata));
    }

    // ===================================================================
    // CAPFIX1 — fail-closed borders_pass validation regression tests.
    // ===================================================================

    fn vs_input(semantic_name: &str, component_type: u32, mask: u8) -> ShaderInputParameter {
        ShaderInputParameter {
            semantic_name: semantic_name.to_string(),
            semantic_index: 0,
            register: 0,
            component_type,
            mask,
            read_write_mask: mask,
        }
    }

    fn vs_metadata(inputs: Vec<ShaderInputParameter>) -> TerrainVertexShaderMetadata {
        TerrainVertexShaderMetadata {
            input_parameters: inputs,
            constant_buffers: BTreeMap::new(),
            byte_len: 0,
            fnv1a64: String::new(),
        }
    }

    /// The REAL border VS ISGN, exactly as fxc compiles `VS_INPUT_BORDER`
    /// (float3 POSITION mask 0x07, int2 TEXCOORD mask 0x03).
    fn real_border_vs() -> TerrainVertexShaderMetadata {
        vs_metadata(vec![
            vs_input("POSITION", D3D_REGISTER_COMPONENT_FLOAT32, 0x07),
            vs_input("TEXCOORD", D3D_REGISTER_COMPONENT_SINT32, 0x03),
        ])
    }

    /// The diag23b contamination VS: the TERRAIN VS (int4 POSITION mask 0x0f,
    /// int2 TEXCOORD mask 0x03), as recorded in
    /// `map_frame_borders_pass_bundle.json`.
    fn diag23b_terrain_vs() -> TerrainVertexShaderMetadata {
        vs_metadata(vec![
            vs_input("POSITION", D3D_REGISTER_COMPONENT_SINT32, 0x0f),
            vs_input("TEXCOORD", D3D_REGISTER_COMPONENT_SINT32, 0x03),
        ])
    }

    #[test]
    fn border_vs_matcher_accepts_real_border_vs_rejects_terrain_vs() {
        assert!(terrain_vertex_shader_metadata_matches_border(
            &real_border_vs()
        ));
        // The diag23b terrain VS (int4 POSITION) must NOT match.
        assert!(!terrain_vertex_shader_metadata_matches_border(
            &diag23b_terrain_vs()
        ));
        // A float4 POSITION (mask 0x0f) must NOT match either — the border VS is
        // float3 (mask 0x07), so exact-mask equality is required.
        assert!(!terrain_vertex_shader_metadata_matches_border(
            &vs_metadata(vec![
                vs_input("POSITION", D3D_REGISTER_COMPONENT_FLOAT32, 0x0f),
                vs_input("TEXCOORD", D3D_REGISTER_COMPONENT_SINT32, 0x03),
            ])
        ));
    }

    #[test]
    fn border_candidate_rejects_diag23b_contamination() {
        // The diag23b capture: terrain VS + all-zero BorderDiffuse + no-op RT.
        // Every one of the three criteria is violated; validation must REJECT.
        let facts = BorderCandidateFacts {
            vertex_shader: &diag23b_terrain_vs(),
            border_diffuse_srv_all_zero: true,
            border_diffuse_srv_observed: true,
            render_target_changed: false,
            render_target_snapshots_observed: true,
        };
        let err = validate_border_candidate(&facts).expect_err("diag23b must be rejected");
        assert!(err.starts_with(BORDER_REJECT_MESSAGE_PREFIX), "{err}");
        // The FIRST gate that trips is the VS signature (the decisive one).
        assert!(err.contains("input signature"), "{err}");
    }

    #[test]
    fn border_candidate_accepts_valid_border_draw() {
        // A genuine border draw: real border VS, inked BorderDiffuse, RT changed.
        let facts = BorderCandidateFacts {
            vertex_shader: &real_border_vs(),
            border_diffuse_srv_all_zero: false,
            border_diffuse_srv_observed: true,
            render_target_changed: true,
            render_target_snapshots_observed: true,
        };
        assert!(validate_border_candidate(&facts).is_ok());
    }

    #[test]
    fn border_candidate_rejects_each_failure_mode_independently() {
        // (1) All-zero BorderDiffuse even with a real border VS + RT change.
        let all_zero = BorderCandidateFacts {
            vertex_shader: &real_border_vs(),
            border_diffuse_srv_all_zero: true,
            border_diffuse_srv_observed: true,
            render_target_changed: true,
            render_target_snapshots_observed: true,
        };
        let err = validate_border_candidate(&all_zero).expect_err("all-zero must reject");
        assert!(err.contains("BorderDiffuse"), "{err}");

        // (2) BorderDiffuse never observed → cannot prove ink → fail closed.
        let unobserved = BorderCandidateFacts {
            vertex_shader: &real_border_vs(),
            border_diffuse_srv_all_zero: false,
            border_diffuse_srv_observed: false,
            render_target_changed: true,
            render_target_snapshots_observed: true,
        };
        assert!(validate_border_candidate(&unobserved).is_err());

        // (3) No-op RT even with real VS + inked BorderDiffuse.
        let noop_rt = BorderCandidateFacts {
            vertex_shader: &real_border_vs(),
            border_diffuse_srv_all_zero: false,
            border_diffuse_srv_observed: true,
            render_target_changed: false,
            render_target_snapshots_observed: true,
        };
        let err = validate_border_candidate(&noop_rt).expect_err("no-op RT must reject");
        assert!(
            err.contains("no-op") || err.contains("byte-identical"),
            "{err}"
        );

        // (4) RT snapshots not both captured → cannot prove change → fail closed.
        let no_rt_snapshots = BorderCandidateFacts {
            vertex_shader: &real_border_vs(),
            border_diffuse_srv_all_zero: false,
            border_diffuse_srv_observed: true,
            render_target_changed: true,
            render_target_snapshots_observed: false,
        };
        assert!(validate_border_candidate(&no_rt_snapshots).is_err());
    }

    #[test]
    fn underwater_classifier_matches_underwater_ps_rejects_terrain_and_terrainunlit() {
        // PixelShaderUnderwater: samples the six required textures, NOT TerrainIDMap.
        let underwater =
            b"TerrainDiffuse_Texture : register(t0); HeightNormal_Texture : register(t1); \
            TerrainColorTint_Texture : register(t2); LightIndexMap_Texture : register(t10); \
            LightDataMap_Texture : register(t11); ShadowMap_Texture : register(t12);";
        let slots =
            underwater_resource_slots_from_shader_bytecode(underwater).expect("underwater match");
        assert_eq!(slots.get("TerrainDiffuse"), Some(&0));
        assert_eq!(slots.get("HeightNormal"), Some(&1));
        assert_eq!(slots.get("ShadowMap"), Some(&12));

        // Main terrain PS: binds TerrainIDMap(5) → excluded discriminator → reject.
        let terrain =
            b"TerrainDiffuse_Texture : register(t0); HeightNormal_Texture : register(t1); \
            TerrainColorTint_Texture : register(t2); TerrainIDMap_Texture : register(t5); \
            LightIndexMap_Texture : register(t10); LightDataMap_Texture : register(t11); \
            ShadowMap_Texture : register(t12);";
        assert!(underwater_resource_slots_from_shader_bytecode(terrain).is_none());

        // PixelShaderTerrainUnlit: no point-light slots → missing required set → reject.
        let terrainunlit =
            b"TerrainDiffuse_Texture : register(t0); HeightNormal_Texture : register(t1); \
            TerrainColorTint_Texture : register(t2); ShadowMap_Texture : register(t12);";
        assert!(underwater_resource_slots_from_shader_bytecode(terrainunlit).is_none());
    }

    #[test]
    fn underwater_ps_survives_the_lockfree_prefilter() {
        // The underwater PS carries none of the historical markers, so without a
        // dedicated marker the load-freeze prefilter would drop it. HeightNormal
        // admits it.
        let underwater =
            b"TerrainDiffuse HeightNormal TerrainColorTint LightIndexMap LightDataMap ShadowMap";
        assert!(pixel_shader_bytecode_could_be_capture_family(underwater));
    }

    #[test]
    fn capture_state_tracks_active_underwater_shader() {
        let mut state = TerrainCaptureState::default();
        let underwater =
            b"TerrainDiffuse_Texture : register(t0); HeightNormal_Texture : register(t1); \
            TerrainColorTint_Texture : register(t2); LightIndexMap_Texture : register(t10); \
            LightDataMap_Texture : register(t11); ShadowMap_Texture : register(t12);";
        assert!(state.observe_underwater_pixel_shader_created(0xB0, underwater));
        assert_eq!(state.underwater_pixel_shader_count(), 1);
        state.observe_ps_set_shader(0xB0);
        assert_eq!(
            state.underwater_pixel_shader(),
            Some(MapPixelShaderBinding { pixel_shader: 0xB0 })
        );
        // A terrain PS (binds TerrainIDMap) must NOT be classified as underwater.
        let terrain =
            b"TerrainDiffuse_Texture : register(t0); HeightNormal_Texture : register(t1); \
            TerrainColorTint_Texture : register(t2); TerrainIDMap_Texture : register(t5); \
            LightIndexMap_Texture : register(t10); LightDataMap_Texture : register(t11); \
            ShadowMap_Texture : register(t12);";
        assert!(!state.observe_underwater_pixel_shader_created(0xB1, terrain));
    }

    #[test]
    fn mapname_vertex_discriminator_rejects_ui_float2_position() {
        // A plain UI shader: float2 POSITION (mask 0x03) must be rejected.
        let metadata = mapname_vs_metadata(
            vec![
                mapname_vs_input("POSITION", D3D_REGISTER_COMPONENT_FLOAT32, 0x03),
                mapname_vs_input("TEXCOORD", D3D_REGISTER_COMPONENT_FLOAT32, 0x03),
            ],
            Some(1),
        );
        assert!(!terrain_vertex_shader_metadata_matches_mapname(&metadata));
    }

    #[test]
    fn mapname_vertex_discriminator_rejects_float4_position_mesh() {
        // A float4-position mesh VS reports mask 0x0F, not 0x07 — exact-mask
        // equality is what excludes it.
        let metadata = mapname_vs_metadata(
            vec![
                mapname_vs_input("POSITION", D3D_REGISTER_COMPONENT_FLOAT32, 0x0f),
                mapname_vs_input("TEXCOORD", D3D_REGISTER_COMPONENT_FLOAT32, 0x03),
            ],
            Some(1),
        );
        assert!(!terrain_vertex_shader_metadata_matches_mapname(&metadata));
    }

    #[test]
    fn mapname_vertex_discriminator_rejects_three_input_traderoute() {
        // traderoute.shader adds a third TANGENT input — excluded by the
        // exactly-two-inputs rule.
        let metadata = mapname_vs_metadata(
            vec![
                mapname_vs_input("POSITION", D3D_REGISTER_COMPONENT_FLOAT32, 0x07),
                mapname_vs_input("TEXCOORD", D3D_REGISTER_COMPONENT_FLOAT32, 0x03),
                mapname_vs_input("TANGENT", D3D_REGISTER_COMPONENT_FLOAT32, 0x07),
            ],
            Some(1),
        );
        assert!(!terrain_vertex_shader_metadata_matches_mapname(&metadata));
    }

    #[test]
    fn mapname_vertex_discriminator_rejects_missing_cb1() {
        // maskedflag/static_button/buttonstate: float3+float2 VS but no slot-1
        // CB — excluded by the CB1 requirement.
        let metadata = mapname_vs_metadata(
            vec![
                mapname_vs_input("POSITION", D3D_REGISTER_COMPONENT_FLOAT32, 0x07),
                mapname_vs_input("TEXCOORD", D3D_REGISTER_COMPONENT_FLOAT32, 0x03),
            ],
            None,
        );
        assert!(!terrain_vertex_shader_metadata_matches_mapname(&metadata));
        // Same inputs but a slot-0-only CB is also rejected.
        let metadata = mapname_vs_metadata(
            vec![
                mapname_vs_input("POSITION", D3D_REGISTER_COMPONENT_FLOAT32, 0x07),
                mapname_vs_input("TEXCOORD", D3D_REGISTER_COMPONENT_FLOAT32, 0x03),
            ],
            Some(0),
        );
        assert!(!terrain_vertex_shader_metadata_matches_mapname(&metadata));
    }

    #[test]
    fn mapname_pixel_discriminator_matches_single_diffuse_texture() {
        let mapname = b"Texture2D DiffuseTexture_Texture : register(t0);";
        let slots = mapname_pixel_resource_slots_from_shader_bytecode(mapname)
            .expect("mapname PS signature");
        assert_eq!(
            slots.get(MAPNAME_DIFFUSE_TEXTURE_RESOURCE_SYMBOL).copied(),
            Some(0)
        );
        // Mod tolerance: extra SRVs alongside DiffuseTexture@t0 still match the
        // slot shape (a modded mapname.shader must not silently break the label
        // capture). Vanilla uniqueness is enforced by the Transp_OffsetX byte
        // marker at observe time, not by an exactly-one-SRV constraint.
        let modded = b"
            Texture2D DiffuseTexture_Texture : register(t0);
            Texture2D OutlineTexture_Texture : register(t1);
        ";
        assert!(mapname_pixel_resource_slots_from_shader_bytecode(modded).is_some());
    }

    #[test]
    fn mapname_pixel_discriminator_rejects_browser_overlay_two_textures() {
        let browser_overlay = b"
            Texture2D BrowserTexture : register(t0);
            Texture2D ColorMask : register(t1);
            SamplerState BrowserSampler : register(s0);
        ";
        assert!(mapname_pixel_resource_slots_from_shader_bytecode(browser_overlay).is_none());
    }

    #[test]
    fn mapname_pixel_discriminator_rejects_multi_texture_map_shader() {
        // A pdxmap-family PS declares no DiffuseTexture at slot 0 — never a
        // mapname font-atlas PS.
        let pdxmap = b"
            Texture2D TerrainIDMap_Texture : register(t5);
            Texture2D TerrainDiffuse_Texture : register(t6);
            Texture2D TerrainColorTint_Texture : register(t7);
        ";
        assert!(mapname_pixel_resource_slots_from_shader_bytecode(pdxmap).is_none());
        // A DiffuseTexture at a NON-zero slot is also rejected.
        let wrong_slot = b"Texture2D DiffuseTexture_Texture : register(t3);";
        assert!(mapname_pixel_resource_slots_from_shader_bytecode(wrong_slot).is_none());
    }

    #[cfg(windows)]
    #[test]
    fn mapname_discriminator_matches_live_captured_vertex_shader() {
        // Ground truth: the census captured the real mapname VS as a generic
        // map_vertex_shader with fnv 4e1b73f39c128df0. Reflect that exact DXBC
        // and confirm the mapname discriminator classifies it — proving the
        // reflected float3-position signature targets the live shader, not just
        // synthetic HLSL. Fixture-gated so CI without an external capture still
        // passes.
        let Some(fixture) = first_capture_fixture("map_vertex_shader_4e1b73f39c128df0.bin") else {
            eprintln!(
                "mapname_discriminator_matches_live_captured_vertex_shader skipped: set {TERRAIN_CAPTURE_FIXTURE_DIRS_ENV} to a capture run containing map_vertex_shader_4e1b73f39c128df0.bin"
            );
            return;
        };
        let bytes = fs::read(&fixture).expect("read live mapname VS fixture");
        assert_eq!(
            fnv1a64_bytes(&bytes),
            MAPNAME_KNOWN_VERTEX_SHADER_FNV1A64,
            "fixture fnv must be the known live mapname VS hash"
        );
        let metadata =
            reflect_terrain_vertex_shader_metadata(&bytes).expect("reflect live mapname VS");
        assert!(
            terrain_vertex_shader_metadata_matches_mapname(&metadata),
            "live mapname VS 4e1b73f39c128df0 must match the reflected discriminator; inputs={:?} cbs={:?}",
            metadata.input_parameters,
            metadata.constant_buffers,
        );
        // And it must NOT match the terrain discriminator (float3 != int4).
        assert!(!terrain_vertex_shader_metadata_matches_pdxmap(&metadata));
    }

    #[test]
    fn capture_state_tracks_active_mapname_shaders_and_cbs() {
        let mut state = TerrainCaptureState::default();
        // A DiffuseTexture-at-t0 PS WITHOUT the Transp_OffsetX marker (the
        // traderoute/generic-font-overlay shape) must be rejected: the slot
        // shape alone never classifies a mapname PS.
        let markerless_ps = b"Texture2D DiffuseTexture_Texture : register(t0);";
        assert!(!state.observe_mapname_pixel_shader_created(0xBF, markerless_ps));
        assert_eq!(state.mapname_pixel_shader_count(), 0);
        // PS: DiffuseTexture at t0 + the Transp_OffsetX CB read.
        let mapname_ps = b"Texture2D DiffuseTexture_Texture : register(t0); Transp_OffsetX";
        assert!(state.observe_mapname_pixel_shader_created(0xC0, mapname_ps));
        assert_eq!(state.mapname_pixel_shader_count(), 1);
        state.observe_ps_set_shader(0xC0);
        assert_eq!(
            state.mapname_pixel_shader(),
            Some(MapPixelShaderBinding { pixel_shader: 0xC0 })
        );
        assert_eq!(
            state.current_pixel_shader_constant_buffer_name(MAPNAME_TRANSP_OFFSET_CB_BIND_POINT),
            Some(MAPNAME_TRANSP_OFFSET_CB_SOURCE.to_string())
        );
        // VS: float3 POSITION + float2 TEXCOORD + CB1.
        let vs_metadata = mapname_vs_metadata(
            vec![
                mapname_vs_input("POSITION", D3D_REGISTER_COMPONENT_FLOAT32, 0x07),
                mapname_vs_input("TEXCOORD", D3D_REGISTER_COMPONENT_FLOAT32, 0x03),
            ],
            Some(1),
        );
        // Bytecode must carry the Transp_OffsetX marker (the tie-breaker that
        // rejects same-shape colliders whose slot-1 CB is a different struct).
        state.observe_vertex_shader_created(
            0xC1,
            b"DXBC mapname vs Transp_OffsetX bytes",
            Some(vs_metadata.clone()),
        );
        assert_eq!(state.mapname_vertex_shader_count(), 1);
        assert!(state.mapname_vertex_shader_known(0xC1));
        // Same reflected signature but NO Transp_OffsetX marker (a collider like
        // maskedflag/static_button) must NOT be tracked as a mapname VS.
        state.observe_vertex_shader_created(
            0xC2,
            b"DXBC collider vs Offset NextOffset bytes",
            Some(vs_metadata),
        );
        assert_eq!(state.mapname_vertex_shader_count(), 1);
        assert!(!state.mapname_vertex_shader_known(0xC2));
        state.observe_vs_set_shader(0xC1);
        assert_eq!(
            state.mapname_vertex_shader(),
            Some(TerrainVertexShaderBinding {
                vertex_shader: 0xC1
            })
        );
    }

    #[test]
    fn captured_pixel_shader_prefilter_covers_full_family_classifiers() {
        let capture_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../map_viewer/out/captures/session_20260702d");
        if !capture_dir.is_dir() {
            eprintln!(
                "captured_pixel_shader_prefilter_covers_full_family_classifiers skipped: {} absent",
                capture_dir.display()
            );
            return;
        }

        let mut shader_files = 0usize;
        let mut prefilter_passes = 0usize;
        let mut terrain_id_matches = 0usize;
        let mut pdxmap_matches = 0usize;
        let mut pdxwater_matches = 0usize;
        let mut river_matches = 0usize;
        let mut border_matches = 0usize;
        let mut tree_matches = 0usize;
        let mut restore_matches = 0usize;
        let mut false_negatives = Vec::new();

        for entry in fs::read_dir(&capture_dir).expect("read capture shader directory") {
            let path = entry.expect("read capture shader entry").path();
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if !name.contains("_pixel_shader_") || !name.ends_with(".bin") {
                continue;
            }
            shader_files += 1;
            let bytes = fs::read(&path).expect("read captured pixel shader bytecode");
            let passed_prefilter = pixel_shader_bytecode_could_be_capture_family(&bytes);
            if passed_prefilter {
                prefilter_passes += 1;
            }
            let terrain_id_match = terrain_id_slot_from_shader_bytecode(&bytes).is_some();
            let pdxmap_match = pdxmap_resource_slots_from_shader_bytecode(&bytes).is_some();
            let pdxwater_match = pdxwater_resource_slots_from_shader_bytecode(&bytes).is_some();
            let river_match = river_resource_slots_from_shader_bytecode(&bytes).is_some();
            let border_match = border_resource_slots_from_shader_bytecode(&bytes).is_some();
            let tree_match = tree_resource_slots_from_shader_bytecode(&bytes).is_some();
            let restore_match = restore_resource_slots_from_shader_bytecode(&bytes).is_some();

            terrain_id_matches += usize::from(terrain_id_match);
            pdxmap_matches += usize::from(pdxmap_match);
            pdxwater_matches += usize::from(pdxwater_match);
            river_matches += usize::from(river_match);
            border_matches += usize::from(border_match);
            tree_matches += usize::from(tree_match);
            restore_matches += usize::from(restore_match);

            if (terrain_id_match
                || pdxmap_match
                || pdxwater_match
                || river_match
                || border_match
                || tree_match
                || restore_match)
                && !passed_prefilter
            {
                false_negatives.push(name.to_string());
            }
        }

        eprintln!(
            "captured_pixel_shader_prefilter_sweep dir={} pixel_shader_files={} prefilter_passes={} terrain_id_matches={} pdxmap_matches={} pdxwater_matches={} river_matches={} border_matches={} tree_matches={} restore_matches={} false_negatives={}",
            capture_dir.display(),
            shader_files,
            prefilter_passes,
            terrain_id_matches,
            pdxmap_matches,
            pdxwater_matches,
            river_matches,
            border_matches,
            tree_matches,
            restore_matches,
            false_negatives.len()
        );
        assert!(
            false_negatives.is_empty(),
            "capture-family prefilter false-negatives: {false_negatives:?}"
        );
    }

    #[test]
    fn capture_state_tracks_active_non_terrain_map_layer_shaders() {
        let mut state = TerrainCaptureState::default();
        // River shaders declare SnowMudTexture (river.shader:85), unlike
        // pdxmap/border/tree which declare SnowMudData.
        let river_shader = b"RiverData_Texture : register(t3); LeanTexture1_Texture : register(t4); LeanTexture2_Texture : register(t5); SnowMudTexture_Texture : register(t8); GradientBorderChannel1_Texture : register(t12); GradientBorderChannel2_Texture : register(t13);";
        // The REAL shipped border.shader:border PS variant declares ONLY
        // BorderDiffuse + ShadowMap (BORD5: the old 4-symbol signature also
        // demanded SnowMudData/IntelMap and could never match it).
        let border_shader =
            b"BorderDiffuse_Texture : register(t0); ShadowMap_Texture : register(t3);";
        let tree_shader = b"DiffuseMap_Texture : register(t0); NormalMap_Texture : register(t1); TintMap_Texture : register(t2); SeasonMap_Texture : register(t3); TreeMaskTexture_Texture : register(t14); GradientBorderChannel1_Texture : register(t12); GradientBorderChannel2_Texture : register(t13);";

        assert!(state.observe_river_pixel_shader_created(0xA0, river_shader));
        assert!(state.observe_border_pixel_shader_created(0xB0, border_shader));
        assert!(state.observe_tree_pixel_shader_created(0xC0, tree_shader));
        assert_eq!(state.river_pixel_shader_count(), 1);
        assert_eq!(state.border_pixel_shader_count(), 1);
        assert_eq!(state.tree_pixel_shader_count(), 1);

        state.observe_ps_set_shader(0xA0);
        assert_eq!(
            state.river_pixel_shader(),
            Some(MapPixelShaderBinding { pixel_shader: 0xA0 })
        );
        assert!(state.pdxmap_pixel_shader().is_none());
        assert!(state.pdxwater_pixel_shader().is_none());
        assert_eq!(
            state
                .river_pixel_shader_metadata(0xA0)
                .expect("river metadata")
                .resource_slots
                .get(RIVER_DATA_RESOURCE_SYMBOL),
            Some(&3)
        );

        state.observe_ps_set_shader(0xB0);
        assert_eq!(
            state.border_pixel_shader(),
            Some(MapPixelShaderBinding { pixel_shader: 0xB0 })
        );
        assert_eq!(
            state
                .border_pixel_shader_metadata(0xB0)
                .expect("border metadata")
                .resource_slots
                .get(BORDER_DIFFUSE_RESOURCE_SYMBOL),
            Some(&0)
        );

        state.observe_ps_set_shader(0xC0);
        assert_eq!(
            state.tree_pixel_shader(),
            Some(MapPixelShaderBinding { pixel_shader: 0xC0 })
        );
        assert_eq!(
            state
                .tree_pixel_shader_metadata(0xC0)
                .expect("tree metadata")
                .resource_slots
                .get(TREE_MASK_RESOURCE_SYMBOL),
            Some(&14)
        );
    }

    #[test]
    fn capture_state_names_current_constant_buffers_from_reflection() {
        let mut state = TerrainCaptureState::default();
        state.pdxwater_pixel_shaders.insert(
            0x90,
            PdxmapShaderMetadata {
                resource_slots: BTreeMap::new(),
                constant_buffers: BTreeMap::from([("TerrainSceneCB0".to_string(), 0)]),
                byte_len: 4,
                fnv1a64: "pixel".to_string(),
            },
        );
        state.map_vertex_shaders.insert(
            0x91,
            TerrainVertexShaderMetadata {
                input_parameters: Vec::new(),
                constant_buffers: BTreeMap::from([(
                    "gfx/FX/pdxwater.shader:WaterDrawConstants".to_string(),
                    3,
                )]),
                byte_len: 4,
                fnv1a64: "vertex".to_string(),
            },
        );

        assert!(state.current_pixel_shader_constant_buffer_name(0).is_none());
        state.observe_ps_set_shader(0x90);
        state.observe_vs_set_shader(0x91);

        assert_eq!(
            state.current_pixel_shader_constant_buffer_name(0),
            Some("TerrainSceneCB0".to_string())
        );
        assert_eq!(
            state.current_vertex_shader_constant_buffer_name(3),
            Some("gfx/FX/pdxwater.shader:WaterDrawConstants".to_string())
        );
        assert!(state.current_pixel_shader_constant_buffer_name(1).is_none());
    }

    #[test]
    fn capture_state_tracks_generic_vertex_shader_metadata_for_water_passes() {
        let mut state = TerrainCaptureState::default();
        let metadata = TerrainVertexShaderMetadata {
            input_parameters: vec![ShaderInputParameter {
                semantic_name: "POSITION".to_string(),
                semantic_index: 0,
                register: 0,
                component_type: D3D_REGISTER_COMPONENT_SINT32,
                mask: 0x03,
                read_write_mask: 0x03,
            }],
            constant_buffers: BTreeMap::from([(
                "gfx/FX/pdxwater.shader:ConstantBuffer(3,48)".to_string(),
                3,
            )]),
            byte_len: 0,
            fnv1a64: String::new(),
        };
        assert!(!state.observe_vertex_shader_created(
            0x91,
            b"DXBC water vertex shader",
            Some(metadata)
        ));
        assert_eq!(state.terrain_vertex_shader_count(), 0);
        assert_eq!(state.map_vertex_shader_count(), 1);
        state.observe_vs_set_shader(0x91);
        assert_eq!(
            state.map_vertex_shader(),
            Some(TerrainVertexShaderBinding {
                vertex_shader: 0x91
            })
        );
        assert!(state.map_vertex_shader_metadata(0x91).is_some());
    }

    #[test]
    fn capture_state_tracks_multiple_shader_variants_by_declared_slot() {
        let mut state = TerrainCaptureState::default();
        assert!(state.observe_pixel_shader_created(
            0x30,
            b"TerrainIDMap_Texture : register(t3); TerrainDiffuse TerrainColorTint"
        ));
        assert!(state.observe_pixel_shader_created(
            0x50,
            b"TerrainIDMap_Texture : register(t5); TerrainDiffuse TerrainColorTint"
        ));
        assert_eq!(state.pdxmap_pixel_shader_count(), 2);
        state.observe_ps_set_shader(0x30);
        assert!(state.observe_ps_set_shader_resources(3, &[0x33]));
        assert_eq!(
            state.terrain_id_srv(),
            Some(TerrainSrvBinding { srv: 0x33, slot: 3 })
        );
        state.observe_ps_set_shader(0x50);
        assert!(state.observe_ps_set_shader_resources(4, &[0x44, 0x55]));
        assert_eq!(
            state.terrain_id_srv(),
            Some(TerrainSrvBinding { srv: 0x55, slot: 5 })
        );
    }

    #[test]
    fn capture_state_tracks_named_snow_mud_resource_from_active_shader() {
        let mut state = TerrainCaptureState::default();
        assert!(state.observe_pixel_shader_created(
            0x80,
            b"TerrainIDMap_Texture : register(t5); SnowMudData_Texture : register(t7); TerrainDiffuse TerrainColorTint"
        ));
        state.observe_ps_set_shader(0x80);
        assert_eq!(
            state.observe_ps_set_named_shader_resource(
                SNOW_MUD_DATA_RESOURCE_SYMBOL,
                5,
                &[0x55, 0x66, 0x77]
            ),
            Some(TerrainSrvBinding { srv: 0x77, slot: 7 })
        );
        assert_eq!(
            state.shader_resource_srv(SNOW_MUD_DATA_RESOURCE_SYMBOL),
            Some(TerrainSrvBinding { srv: 0x77, slot: 7 })
        );
        assert_eq!(
            state.observe_ps_set_named_shader_resource("InventedRuntimeMap", 5, &[0x55, 0x66]),
            None
        );
    }

    #[test]
    fn capture_state_tracks_input_layout_descriptors_by_bound_pointer() {
        let mut state = TerrainCaptureState::default();
        assert!(!state.observe_input_layout_created(0, terrain_input_layout()));
        assert!(!state.observe_input_layout_created(0x40, Vec::new()));
        assert!(state.observe_input_layout_created(0x40, terrain_input_layout()));
        assert_eq!(state.input_layout_count(), 1);
        assert!(state.current_input_layout().is_none());
        state.observe_ia_set_input_layout(0x40);
        let (layout_ptr, elements) = state.current_input_layout().expect("current layout");
        assert_eq!(layout_ptr, 0x40);
        assert_eq!(elements.len(), 2);
        state.observe_ia_set_input_layout(0);
        assert!(state.current_input_layout().is_none());
    }

    #[test]
    fn source_texture_desc_must_be_shader_resource_supported_8bit_id_texture() {
        let desc = terrain_desc();
        assert_eq!(validate_terrain_id_source_desc(&desc), Ok(()));
        assert_eq!(
            validate_terrain_id_source_desc(&Texture2DDesc {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                ..desc.clone()
            }),
            Ok(())
        );
        assert_eq!(
            validate_terrain_id_source_desc(&Texture2DDesc {
                format: 77,
                ..desc.clone()
            }),
            Err(TerrainCaptureError::UnsupportedFormat(77))
        );
        assert_eq!(
            validate_terrain_id_source_desc(&Texture2DDesc {
                bind_flags: 0,
                ..desc
            }),
            Err(TerrainCaptureError::MissingShaderResourceBind)
        );
    }

    #[test]
    fn light_data_desc_must_be_shader_resource_supported_float_texture() {
        let desc = light_data_desc();
        assert_eq!(
            validate_r32g32b32a32_float_shader_resource_source_desc(&desc),
            Ok(())
        );
        assert_eq!(
            validate_r32g32b32a32_float_shader_resource_source_desc(&Texture2DDesc {
                format: DXGI_FORMAT_R8G8B8A8_UNORM,
                ..desc.clone()
            }),
            Err(TerrainCaptureError::UnsupportedFormat(
                DXGI_FORMAT_R8G8B8A8_UNORM
            ))
        );
        assert_eq!(
            validate_r32g32b32a32_float_shader_resource_source_desc(&Texture2DDesc {
                bind_flags: 0,
                ..desc
            }),
            Err(TerrainCaptureError::MissingShaderResourceBind)
        );
    }

    #[test]
    fn readback_plan_uses_staging_cpu_read_texture() {
        let plan = readback_plan(&terrain_desc()).expect("valid readback plan");
        assert_eq!(plan.source_srv_slot, TERRAIN_ID_SRV_SLOT);
        assert_eq!(plan.map_type, D3D11_MAP_READ);
        assert_eq!(plan.row_bytes, 5632 * 4);
        assert_eq!(plan.total_bytes, 5632 * 2048 * 4);
        assert_eq!(plan.staging_desc.usage, D3D11_USAGE_STAGING);
        assert_eq!(plan.staging_desc.bind_flags, 0);
        assert_eq!(plan.staging_desc.cpu_access_flags, D3D11_CPU_ACCESS_READ);
    }

    #[test]
    fn terrain_vertex_buffer_plan_matches_shader_input_contract() {
        let source = terrain_vertex_buffer_desc();
        let plan = terrain_vertex_buffer_readback_plan(&source, 8, 0)
            .expect("terrain vertex buffer readback plan");
        assert_eq!(plan.vertex_stride, 8);
        assert_eq!(plan.vertex_offset, 0);
        assert_eq!(plan.captured_byte_len, source.byte_width as usize);
        assert_eq!(plan.staging_desc.usage, D3D11_USAGE_STAGING);
        assert_eq!(plan.staging_desc.bind_flags, 0);
        assert_eq!(plan.staging_desc.cpu_access_flags, D3D11_CPU_ACCESS_READ);
    }

    #[test]
    fn terrain_vertex_buffer_plan_rejects_unproven_layouts() {
        let mut source = terrain_vertex_buffer_desc();
        assert_eq!(
            terrain_vertex_buffer_readback_plan(&source, 0, 0),
            Err(TerrainVertexBufferError::StrideTooSmall {
                stride: 0,
                minimum: 1,
            })
        );
        assert_eq!(
            terrain_vertex_buffer_readback_plan(&source, 26, 0),
            Err(TerrainVertexBufferError::StrideNotU32Aligned(26))
        );
        assert_eq!(
            terrain_vertex_buffer_readback_plan(&source, 8, 60),
            Err(TerrainVertexBufferError::FirstVertexPastEnd {
                offset: 60,
                stride: 8,
                byte_width: 64,
            })
        );
        source.bind_flags = 0;
        assert_eq!(
            terrain_vertex_buffer_readback_plan(&source, 8, 0),
            Err(TerrainVertexBufferError::MissingVertexBufferBind)
        );
    }

    #[test]
    fn terrain_input_layout_matches_pdxmap_shader_contract() {
        let layout = terrain_input_layout();
        assert_eq!(validate_terrain_input_layout(&layout, 8), Ok(()));
        assert_eq!(validate_terrain_input_layout(&layout, 32), Ok(()));

        let mut wrong_height_format = layout.clone();
        wrong_height_format[1].format = DXGI_FORMAT_R8G8B8A8_UNORM;
        assert_eq!(
            validate_terrain_input_layout(&wrong_height_format, 8),
            Err(TerrainInputLayoutError::WrongFormat {
                semantic_name: "TEXCOORD",
                semantic_index: 0,
                expected: "signed integer DXGI format with 2 components for HLSL int2",
                actual: DXGI_FORMAT_R8G8B8A8_UNORM,
            })
        );

        let mut append_offset = layout.clone();
        append_offset[1].aligned_byte_offset = D3D11_APPEND_ALIGNED_ELEMENT;
        assert_eq!(
            validate_terrain_input_layout(&append_offset, 8),
            Err(TerrainInputLayoutError::AppendAlignedOffset {
                semantic_name: "TEXCOORD",
                semantic_index: 0,
            })
        );

        let mut overrun_height = layout.clone();
        overrun_height[1].aligned_byte_offset = 6;
        assert_eq!(
            validate_terrain_input_layout(&overrun_height, 8),
            Err(TerrainInputLayoutError::ElementOverrunsStride {
                semantic_name: "TEXCOORD",
                semantic_index: 0,
                offset: 6,
                byte_width: 4,
                stride: 8,
            })
        );
    }

    #[test]
    fn terrain_input_layout_rejects_missing_or_instanced_shader_inputs() {
        let mut missing_height = terrain_input_layout();
        missing_height.pop();
        assert_eq!(
            validate_terrain_input_layout(&missing_height, 8),
            Err(TerrainInputLayoutError::MissingElement {
                semantic_name: "TEXCOORD",
                semantic_index: 0,
            })
        );

        let mut instanced_position = terrain_input_layout();
        instanced_position[0].input_slot_class = 1;
        assert_eq!(
            validate_terrain_input_layout(&instanced_position, 8),
            Err(TerrainInputLayoutError::WrongInputSlotClass {
                semantic_name: "POSITION",
                semantic_index: 0,
                expected: D3D11_INPUT_PER_VERTEX_DATA,
                actual: 1,
            })
        );

        let mut wrong_height_component_count = terrain_input_layout();
        wrong_height_component_count[1].format = DXGI_FORMAT_R8G8B8A8_SINT;
        assert_eq!(
            validate_terrain_input_layout(&wrong_height_component_count, 8),
            Err(TerrainInputLayoutError::WrongFormat {
                semantic_name: "TEXCOORD",
                semantic_index: 0,
                expected: "signed integer DXGI format with 2 components for HLSL int2",
                actual: DXGI_FORMAT_R8G8B8A8_SINT,
            })
        );
    }

    #[test]
    fn mapped_row_pitch_must_fit_rgba_rows() {
        assert_eq!(validate_mapped_row_pitch(5632 * 4, 5632), Ok(()));
        assert_eq!(validate_mapped_row_pitch(5632 * 4 + 256, 5632), Ok(()));
        assert_eq!(
            validate_mapped_row_pitch(100, 5632),
            Err(TerrainCaptureError::InvalidMappedPitch {
                row_pitch: 100,
                row_bytes: 5632 * 4,
            })
        );
    }

    #[test]
    fn pitched_row_copy_strips_gpu_padding() {
        let rows = [
            1, 2, 3, 4, 5, 6, 7, 8, 90, 91, 92, 93, 10, 11, 12, 13, 14, 15, 16, 17, 80, 81, 82, 83,
        ];
        let copied = copy_pitched_rgba_rows(&rows, 12, 2, 2).expect("copy pitched rows");
        assert_eq!(
            copied,
            vec![1, 2, 3, 4, 5, 6, 7, 8, 10, 11, 12, 13, 14, 15, 16, 17]
        );
        assert_eq!(
            copy_pitched_rgba_rows(&rows[..20], 12, 2, 2),
            Err(TerrainCaptureError::MappedRowsTooShort {
                available: 20,
                required: 24,
            })
        );
    }

    #[test]
    fn pitched_bgra_row_copy_converts_to_canonical_rgba() {
        let rows = [
            3, 2, 1, 4, 7, 6, 5, 8, 90, 91, 92, 93, 13, 12, 11, 14, 17, 16, 15, 18, 80, 81, 82, 83,
        ];
        let copied = copy_pitched_terrain_id_rows(&rows, 12, 2, 2, DXGI_FORMAT_B8G8R8A8_UNORM)
            .expect("copy bgra terrain rows");
        assert_eq!(
            copied,
            vec![1, 2, 3, 4, 5, 6, 7, 8, 11, 12, 13, 14, 15, 16, 17, 18]
        );
    }

    #[test]
    fn pitched_bgra_srgb_row_copy_converts_to_canonical_rgba() {
        let rows = [
            3, 2, 1, 4, 7, 6, 5, 8, 90, 91, 92, 93, 13, 12, 11, 14, 17, 16, 15, 18, 80, 81, 82, 83,
        ];
        let copied = copy_pitched_terrain_id_rows(&rows, 12, 2, 2, DXGI_FORMAT_B8G8R8A8_UNORM_SRGB)
            .expect("copy bgra srgb terrain rows");
        assert_eq!(
            copied,
            vec![1, 2, 3, 4, 5, 6, 7, 8, 11, 12, 13, 14, 15, 16, 17, 18]
        );
    }

    #[test]
    fn pitched_r32g32b32a32_float_row_copy_preserves_typed_bytes() {
        let rows = [
            1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 90, 91, 92, 93, 17, 18, 19, 20,
            21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 80, 81, 82, 83,
        ];
        let copied =
            copy_pitched_r32g32b32a32_float_rows(&rows, 20, 1, 2, DXGI_FORMAT_R32G32B32A32_FLOAT)
                .expect("copy float rows");
        assert_eq!(
            copied,
            vec![
                1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23,
                24, 25, 26, 27, 28, 29, 30, 31, 32
            ]
        );
        assert_eq!(
            copy_pitched_r32g32b32a32_float_rows(&rows, 12, 1, 2, DXGI_FORMAT_R32G32B32A32_FLOAT),
            Err(TerrainCaptureError::InvalidMappedPitch {
                row_pitch: 12,
                row_bytes: 16,
            })
        );
        assert_eq!(
            copy_pitched_r32g32b32a32_float_rows(&rows, 20, 1, 2, DXGI_FORMAT_R8G8B8A8_UNORM),
            Err(TerrainCaptureError::UnsupportedFormat(
                DXGI_FORMAT_R8G8B8A8_UNORM
            ))
        );
    }

    fn terrain_desc() -> Texture2DDesc {
        Texture2DDesc {
            width: 5632,
            height: 2048,
            mip_levels: 1,
            array_size: 1,
            format: DXGI_FORMAT_R8G8B8A8_UNORM,
            sample_count: 1,
            sample_quality: 0,
            usage: D3D11_USAGE_DEFAULT,
            bind_flags: D3D11_BIND_SHADER_RESOURCE,
            cpu_access_flags: 0,
            misc_flags: 0,
        }
    }

    fn light_data_desc() -> Texture2DDesc {
        Texture2DDesc {
            width: 128,
            height: 1,
            mip_levels: 1,
            array_size: 1,
            format: DXGI_FORMAT_R32G32B32A32_FLOAT,
            sample_count: 1,
            sample_quality: 0,
            usage: D3D11_USAGE_DEFAULT,
            bind_flags: D3D11_BIND_SHADER_RESOURCE,
            cpu_access_flags: 0,
            misc_flags: 0,
        }
    }

    fn temp_test_dir(name: &str) -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time before unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "hoi4_terrain_capture_{}_{}_{}",
            std::process::id(),
            name,
            nonce
        ));
        fs::create_dir_all(&path).expect("create temp test directory");
        path
    }

    fn has_constant_buffer_naming_complaint(message: &str) -> bool {
        message.contains("expected reflected constant buffer")
            || message.contains("has no named TerrainSceneCB0 PS constant-buffer capture")
            || message.contains("constant-buffer slot") && message.contains("is captured as")
    }

    fn write_stale_constant_buffer_name_contract_fixture(root: &Path) -> std::path::PathBuf {
        let scene_cb0 = vec![0u8; 416];
        let draw_cb1 = vec![0u8; 112];
        write_file(root, TERRAIN_SCENE_CB0_FILE, &scene_cb0);
        write_json_value(
            &root.join(TERRAIN_SCENE_CB0_META_FILE),
            serde_json::json!({
                "symbol": TERRAIN_SCENE_CB0_SYMBOL,
                "file": TERRAIN_SCENE_CB0_FILE,
                "byte_len": scene_cb0.len(),
                "fnv1a64": fnv1a64_bytes(&scene_cb0),
                "shader_constant_buffer_slot": TERRAIN_SCENE_CB0_BIND_POINT,
                "source": "unit_test"
            }),
        );
        for draw_index in 0..2 {
            write_file(
                root,
                &format!("map_frame_draw{draw_index:03}_ia_vertex_buffer_slot0.bin"),
                &le_i16_bytes(&[0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 1, 1]),
            );
            write_file(
                root,
                &format!("map_frame_draw{draw_index:03}_ia_vertex_buffer_slot1.bin"),
                &le_i16_bytes(&[1000, 2000, 1100, 2100, 1200, 2200]),
            );
            write_file(
                root,
                &format!("map_frame_draw{draw_index:03}_ia_index_buffer.bin"),
                &le_u16_bytes(&[0, 1, 2]),
            );
            write_file(
                root,
                &format!("map_frame_draw{draw_index:03}_vs_constant_buffer_slot0.bin"),
                &scene_cb0,
            );
            write_file(
                root,
                &format!("map_frame_draw{draw_index:03}_vs_constant_buffer_slot1.bin"),
                &draw_cb1,
            );
            write_file(
                root,
                &format!("map_frame_draw{draw_index:03}_ps_constant_buffer_slot0.bin"),
                &scene_cb0,
            );
        }
        write_file(root, "terrain_id_map.rgba", &[4u8; 16]);
        write_file(
            root,
            "pdxmap_pixel_shader_0123456789abcdef.bin",
            &[9u8; 128],
        );
        write_file(
            root,
            "pdxmap_vertex_shader_fedcba9876543210.bin",
            &[7u8; 96],
        );
        write_vanilla_source_fixture(root, "gfx/FX/pdxmap.shader", "// gfx/FX/pdxmap.shader\n");
        write_vanilla_source_fixture(root, PDXMAP_FXH_SOURCE, minimal_pdxmap_fxh_source());
        write_vanilla_source_fixture(root, "gfx/FX/constants.fxh", "// gfx/FX/constants.fxh\n");
        write_vanilla_source_fixture(
            root,
            "gfx/FX/standardfuncsgfx.fxh",
            minimal_standard_functions_source(),
        );
        write_vanilla_source_fixture(root, "gfx/FX/tiled_pointlights.fxh", "// tiled\n");
        write_vanilla_source_fixture(root, "gfx/FX/shadow.fxh", "// shadow\n");
        write_vanilla_source_fixture(root, "gfx/FX/fow.fxh", "// fow\n");
        write_vanilla_source_fixture(
            root,
            "common/defines/00_graphics.lua",
            minimal_graphics_defines_source(),
        );

        let draw0 = stale_contract_draw(root, 0);
        let draw1 = stale_contract_draw(root, 1);
        let frame = serde_json::json!({
            "schema": "battle-plan-map-frame-bundle-v1",
            "provenance": "unit_test",
            "capture_moment": TERRAIN_CAPTURE_MOMENT,
            "session_manifest": "map_capture_session.json",
            "shader_cache_constants": {
                "map_size_x": 2.0,
                "map_size_y": 2.0,
                "map_pow2_x": 1.0,
                "map_pow2_y": 1.0,
                "fow_pow2_x": 1.0,
                "fow_pow2_y": 1.0,
                "terrain_id_srv_slot": 5,
                "cache_files": ["dx11_win32/ps_5_0/map_t5.scache"],
                "provenance": "unit_test_shader_cache_scan",
                "extraction": "unit_test_shader_cache_control"
            },
            "active_draw": draw1,
            "draws": [draw0, draw1],
            "post_pass_output_merger": null,
            "vanilla_sources": vanilla_sources_evidence(root),
            "runtime_files": runtime_files_evidence(root),
            "blockers": []
        });
        let session = serde_json::json!({
            "schema": "battle-plan-map-capture-session-v1",
            "provenance": "unit_test",
            "required_first_moment": TERRAIN_CAPTURE_MOMENT,
            "moments": [{
                "name": TERRAIN_CAPTURE_MOMENT,
                "status": "captured_pending_renderer_validation",
                "bundle_file": MAP_FRAME_BUNDLE_FILE,
                "required_payload": [
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
                    "provenance"
                ],
                "blockers": []
            }],
            "deferred_moments": []
        });
        write_json_value(&root.join("map_capture_session.json"), session);
        let path = root.join(MAP_FRAME_BUNDLE_FILE);
        write_json_value(&path, frame);
        path
    }

    fn stale_contract_draw(root: &Path, draw_index: u32) -> Value {
        let scene_cb0_file = format!("map_frame_draw{draw_index:03}_ps_constant_buffer_slot0.bin");
        serde_json::json!({
            "draw_index": draw_index,
            "draw_call": "DrawIndexed",
            "draw_count": 3,
            "draw_start": 0,
            "base_vertex_location": 0,
            "topology": 4,
            "shader": {
                "stage": "PS",
                "pointer": "0x1",
                "bytecode_file": "pdxmap_pixel_shader_0123456789abcdef.bin",
                "byte_len": 128,
                "fnv1a64": file_hash(root, "pdxmap_pixel_shader_0123456789abcdef.bin"),
                "reflection_resources": { "TerrainIDMap": 5 },
                "input_parameters": [],
                "constant_buffers": { "dx11_cb0": 0 },
                "source_evidence": map_frame_shader_sources()
            },
            "vertex_shader": {
                "stage": "VS",
                "pointer": "0x10",
                "bytecode_file": "pdxmap_vertex_shader_fedcba9876543210.bin",
                "byte_len": 96,
                "fnv1a64": file_hash(root, "pdxmap_vertex_shader_fedcba9876543210.bin"),
                "reflection_resources": {},
                "input_parameters": [
                    {
                        "semantic_name": "POSITION",
                        "semantic_index": 0,
                        "register": 0,
                        "component_type": D3D_REGISTER_COMPONENT_SINT32,
                        "mask": 15,
                        "read_write_mask": 15
                    },
                    {
                        "semantic_name": "TEXCOORD",
                        "semantic_index": 0,
                        "register": 1,
                        "component_type": D3D_REGISTER_COMPONENT_SINT32,
                        "mask": 3,
                        "read_write_mask": 3
                    }
                ],
                "constant_buffers": {
                    "dx11_cb0": 0,
                    "dx11_cb1": 1,
                    "gfx/FX/pdxmap.fxh:ConstantBuffer(1,32)": 1
                },
                "source_evidence": map_frame_shader_sources()
            },
            "input_layout": {
                "pointer": "0x2",
                "elements": [
                    {
                        "semantic_name": "POSITION",
                        "semantic_index": 0,
                        "format": DXGI_FORMAT_R16G16B16A16_SINT,
                        "input_slot": 0,
                        "aligned_byte_offset": 0,
                        "input_slot_class": D3D11_INPUT_PER_VERTEX_DATA,
                        "instance_data_step_rate": 0
                    },
                    {
                        "semantic_name": "TEXCOORD",
                        "semantic_index": 0,
                        "format": DXGI_FORMAT_R16G16_SINT,
                        "input_slot": 1,
                        "aligned_byte_offset": 0,
                        "input_slot_class": D3D11_INPUT_PER_VERTEX_DATA,
                        "instance_data_step_rate": 0
                    }
                ]
            },
            "vertex_buffers": {
                "0": buffer_binding(root, draw_index, "IA", 0, None, &format!("map_frame_draw{draw_index:03}_ia_vertex_buffer_slot0.bin"), 1, Some(8), Some(0)),
                "1": buffer_binding(root, draw_index, "IA", 1, None, &format!("map_frame_draw{draw_index:03}_ia_vertex_buffer_slot1.bin"), 1, Some(4), Some(0))
            },
            "index_buffer": {
                "file": format!("map_frame_draw{draw_index:03}_ia_index_buffer.bin"),
                "byte_len": file_len(root, &format!("map_frame_draw{draw_index:03}_ia_index_buffer.bin")),
                "fnv1a64": file_hash(root, &format!("map_frame_draw{draw_index:03}_ia_index_buffer.bin")),
                "dxgi_format": 57,
                "offset": 0,
                "bind_flags": 2,
                "usage": 0,
                "cpu_access_flags": 0
            },
            "constant_buffers": [
                buffer_binding(root, draw_index, "VS", 0, Some("dx11_cb0"), &format!("map_frame_draw{draw_index:03}_vs_constant_buffer_slot0.bin"), D3D11_BIND_CONSTANT_BUFFER, None, None),
                buffer_binding(root, draw_index, "VS", 1, Some("dx11_cb1"), &format!("map_frame_draw{draw_index:03}_vs_constant_buffer_slot1.bin"), D3D11_BIND_CONSTANT_BUFFER, None, None),
                buffer_binding(root, draw_index, "PS", 0, Some("dx11_cb0"), &scene_cb0_file, D3D11_BIND_CONSTANT_BUFFER, None, None)
            ],
            "shader_resources": [{
                "stage": "PS",
                "slot": 5,
                "declared_name": "TerrainIDMap",
                "is_null": false,
                "view_desc": {
                    "format": DXGI_FORMAT_B8G8R8A8_UNORM,
                    "view_dimension": 4,
                    "most_detailed_mip": 0,
                    "mip_levels": 1,
                    "first_array_slice": 0,
                    "array_size": 1,
                    "num_cubes": 0
                },
                "file": "terrain_id_map.rgba",
                "byte_len": file_len(root, "terrain_id_map.rgba"),
                "fnv1a64": file_hash(root, "terrain_id_map.rgba"),
                "dxgi_format": DXGI_FORMAT_B8G8R8A8_UNORM,
                "width": 2,
                "height": 2,
                "mip_count": 1,
                "array_size": 1,
                "sample_count": 1,
                "sample_quality": 0,
                "payload_format": "d3d11_mapped_subresources",
                "subresources": [{
                    "mip_slice": 0,
                    "array_slice": 0,
                    "file_offset": 0,
                    "byte_len": 16,
                    "row_pitch": 8,
                    "depth_pitch": 16,
                    "width": 2,
                    "height": 2
                }],
                "bind_flags": D3D11_BIND_SHADER_RESOURCE,
                "usage": 0,
                "cpu_access_flags": 0,
                "misc_flags": 0
            }],
            "samplers": [{
                "stage": "PS",
                "slot": 5,
                "filter": 0,
                "address_u": 3,
                "address_v": 3,
                "address_w": 3,
                "mip_lod_bias": 0.0,
                "max_anisotropy": 1,
                "comparison_func": 0,
                "border_color": [0.0, 0.0, 0.0, 0.0],
                "min_lod": 0.0,
                "max_lod": 340282346638528859811704183484516925440.0
            }],
            "draw_state": null,
            "post_draw_output_merger": null
        })
    }

    fn buffer_binding(
        root: &Path,
        _draw_index: u32,
        stage: &str,
        slot: u32,
        declared_name: Option<&str>,
        file: &str,
        bind_flags: u32,
        stride: Option<u32>,
        offset: Option<u32>,
    ) -> Value {
        serde_json::json!({
            "stage": stage,
            "slot": slot,
            "declared_name": declared_name,
            "file": file,
            "byte_len": file_len(root, file),
            "fnv1a64": file_hash(root, file),
            "stride": stride,
            "offset": offset,
            "dxgi_format": null,
            "bind_flags": bind_flags,
            "usage": 0,
            "cpu_access_flags": 0
        })
    }

    fn runtime_files_evidence(root: &Path) -> Value {
        let files = [
            "terrain_id_map.rgba",
            TERRAIN_SCENE_CB0_FILE,
            "pdxmap_pixel_shader_0123456789abcdef.bin",
            "pdxmap_vertex_shader_fedcba9876543210.bin",
        ];
        let mut object = serde_json::Map::new();
        for file in files {
            object.insert(file.to_string(), file_evidence(root, file, "unit_test"));
        }
        Value::Object(object)
    }

    fn vanilla_sources_evidence(root: &Path) -> Value {
        let mut object = serde_json::Map::new();
        for source in map_frame_shader_sources()
            .into_iter()
            .chain(std::iter::once("common/defines/00_graphics.lua"))
        {
            let file = format!("vanilla/{source}");
            object.insert(source.to_string(), file_evidence(root, &file, "unit_test"));
        }
        Value::Object(object)
    }

    fn file_evidence(root: &Path, file: &str, provenance: &str) -> Value {
        serde_json::json!({
            "file": file,
            "byte_len": file_len(root, file),
            "fnv1a64": file_hash(root, file),
            "provenance": provenance
        })
    }

    fn write_vanilla_source_fixture(root: &Path, source: &str, contents: impl AsRef<[u8]>) {
        write_file(root, &format!("vanilla/{source}"), contents.as_ref());
    }

    fn write_file(root: &Path, relative: &str, bytes: &[u8]) {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent directory");
        }
        fs::write(path, bytes).expect("write fixture file");
    }

    fn write_json_value(path: &Path, value: Value) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent directory");
        }
        fs::write(
            path,
            serde_json::to_vec_pretty(&value).expect("serialize json"),
        )
        .expect("write json fixture");
    }

    fn file_len(root: &Path, file: &str) -> usize {
        fs::metadata(root.join(file))
            .expect("fixture file metadata")
            .len() as usize
    }

    fn file_hash(root: &Path, file: &str) -> String {
        fnv1a64_bytes(&fs::read(root.join(file)).expect("read fixture file"))
    }

    fn map_frame_shader_sources() -> Vec<&'static str> {
        vec![
            "gfx/FX/pdxmap.shader",
            PDXMAP_FXH_SOURCE,
            "gfx/FX/constants.fxh",
            "gfx/FX/standardfuncsgfx.fxh",
            "gfx/FX/tiled_pointlights.fxh",
            "gfx/FX/shadow.fxh",
            "gfx/FX/fow.fxh",
        ]
    }

    fn minimal_standard_functions_source() -> &'static str {
        r#"
            ConstantBuffer( 0, 0 )
            {
                float4x4    ViewProjectionMatrix;
                float4x4    InvViewProjMatrix;
                float4      vGBCamDistOverride_GBOutlineCutoff;
                float4      vVirtualSunPos;
                float4      vVirtualMoonPos;
                float4      vSecondVirtualSunPos;
                float4      vSecondVirtualMoonPos;
                float3      vCamPos;
                float       HdrRange;
                float3      vCamLookAtDir;
                float       vGlobalTime;
                float4      vFoWOpacity_FoWTime_SnowMudFade_MaxGameSpeed;
                float4      DayNight_Hour_SunDir;
                float3      AmbientPosX;
                float       ShadowFadeFactor;
                float3      AmbientNegX;
                float       FOWFadeFactor;
                float3      AmbientPosY;
                float       MinMeshAlpha;
                float3      AmbientNegY;
                float       NegFogMultiplier;
                float3      AmbientPosZ;
                float3      AmbientNegZ;
                float       CubemapIntensity;
                float4      SunDiffuseIntensity;
                float4      MoonDiffuseIntensity;
                float       GB_TextureHeight;
                float       SunSpecularIntensity;
            };
        "#
    }

    fn minimal_pdxmap_fxh_source() -> &'static str {
        r#"
            ConstantBuffer( 1, 32 )
            {
                float4x4 ShadowMapTextureMatrix;
                float4 QuadOffset_Scale_IsDetail;
                float4 vSnap;
                float4 vBorderLookup_HeightScale_UseMultisample_SeasonLerp;
            }
        "#
    }

    fn minimal_graphics_defines_source() -> &'static str {
        r#"
            NDefines_Graphics = {
                NGraphics = {
                    CAMERA_OUTSIDE_MAP_DISTANCE_TOP = 200.0,
                    CAMERA_OUTSIDE_MAP_DISTANCE_BOTTOM = 200.0,
                    CAMERA_ZOOM_SPEED = 50,
                    CAMERA_ZOOM_KEY_SCALE = 0.02,
                    CAMERA_ZOOM_SPEED_DISTANCE_MULT = 6.0,
                },
                NFrontend = {
                    CAMERA_MIN_HEIGHT = 50.0,
                    CAMERA_MAX_HEIGHT = 3000.0,
                },
            }
        "#
    }

    fn le_i16_bytes(values: &[i16]) -> Vec<u8> {
        values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect()
    }

    fn le_u16_bytes(values: &[u16]) -> Vec<u8> {
        values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect()
    }

    fn terrain_vertex_buffer_desc() -> BufferDesc {
        BufferDesc {
            byte_width: 64,
            usage: D3D11_USAGE_DEFAULT,
            bind_flags: D3D11_BIND_VERTEX_BUFFER,
            cpu_access_flags: 0,
            misc_flags: 0,
            structure_byte_stride: 0,
        }
    }

    fn terrain_input_layout() -> Vec<TerrainInputElementDesc> {
        vec![
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
                semantic_name: "TEXCOORD".to_string(),
                semantic_index: 0,
                format: DXGI_FORMAT_R16G16_SINT,
                input_slot: 1,
                aligned_byte_offset: 0,
                input_slot_class: D3D11_INPUT_PER_VERTEX_DATA,
                instance_data_step_rate: 0,
            },
        ]
    }
}
