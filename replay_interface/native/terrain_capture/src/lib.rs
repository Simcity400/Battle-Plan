#![deny(unsafe_op_in_unsafe_fn)]

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

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
pub const RIVER_REQUIRED_RESOURCE_SYMBOLS: [&str; 6] = [
    RIVER_DATA_RESOURCE_SYMBOL,
    WATER_LEAN_TEXTURE1_SYMBOL,
    WATER_LEAN_TEXTURE2_SYMBOL,
    SNOW_MUD_DATA_RESOURCE_SYMBOL,
    "GradientBorderChannel1",
    "GradientBorderChannel2",
];
pub const BORDER_DIFFUSE_RESOURCE_SYMBOL: &str = "BorderDiffuse";
pub const BORDER_REQUIRED_RESOURCE_SYMBOLS: [&str; 4] = [
    BORDER_DIFFUSE_RESOURCE_SYMBOL,
    SNOW_MUD_DATA_RESOURCE_SYMBOL,
    "IntelMap",
    "ShadowMap",
];
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
pub const D3D11_BIND_SHADER_RESOURCE: u32 = 0x8;
pub const D3D11_CPU_ACCESS_READ: u32 = 0x20000;
pub const D3D11_MAP_READ: u32 = 1;
pub const D3D11_INPUT_PER_VERTEX_DATA: u32 = 0;
pub const D3D11_APPEND_ALIGNED_ELEMENT: u32 = 0xFFFF_FFFF;
pub const D3D11_IA_VERTEX_INPUT_STRUCTURE_ELEMENT_COUNT: u32 = 32;
pub const D3D_REGISTER_COMPONENT_SINT32: u32 = 2;
pub const TERRAIN_VERTEX_CAPTURE_SAMPLE_BYTES: usize = 64 * 1024;
pub const TERRAIN_INPUT_LAYOUT_SHADER_SOURCE: &str =
    "gfx/FX/pdxmap.shader:VS_INPUT_TERRAIN_NOTEXTURE:int4 position + int2 height";
pub const TERRAIN_DRAW_CB1_LAYOUT_SOURCE: &str = "gfx/FX/pdxmap.fxh:ConstantBuffer(1,32)";

pub const IUNKNOWN_QUERY_INTERFACE_INDEX: usize = 0;
pub const IUNKNOWN_ADD_REF_INDEX: usize = 1;
pub const IUNKNOWN_RELEASE_INDEX: usize = 2;
pub const SWAP_CHAIN_GET_DEVICE_INDEX: usize = 7;
pub const SWAP_CHAIN_PRESENT_INDEX: usize = 8;
pub const SWAP_CHAIN_RESIZE_BUFFERS_INDEX: usize = 13;
pub const DEVICE_CREATE_TEXTURE2D_INDEX: usize = 5;
pub const DEVICE_CREATE_SHADER_RESOURCE_VIEW_INDEX: usize = 7;
pub const DEVICE_CREATE_INPUT_LAYOUT_INDEX: usize = 11;
pub const DEVICE_CREATE_VERTEX_SHADER_INDEX: usize = 12;
pub const DEVICE_CREATE_PIXEL_SHADER_INDEX: usize = 15;
pub const CONTEXT_PS_SET_SHADER_RESOURCES_INDEX: usize = 8;
pub const CONTEXT_PS_SET_SHADER_INDEX: usize = 9;
pub const CONTEXT_VS_SET_SHADER_INDEX: usize = 11;
pub const CONTEXT_DRAW_INDEXED_INDEX: usize = 12;
pub const CONTEXT_DRAW_INDEX: usize = 13;
pub const CONTEXT_MAP_INDEX: usize = 14;
pub const CONTEXT_UNMAP_INDEX: usize = 15;
pub const CONTEXT_IA_SET_INPUT_LAYOUT_INDEX: usize = 17;
pub const CONTEXT_IA_SET_VERTEX_BUFFERS_INDEX: usize = 18;
pub const CONTEXT_DRAW_INDEXED_INSTANCED_INDEX: usize = 20;
pub const CONTEXT_DRAW_INSTANCED_INDEX: usize = 21;
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
    tree_pixel_shaders: BTreeMap<usize, PdxmapShaderMetadata>,
    map_vertex_shaders: BTreeMap<usize, TerrainVertexShaderMetadata>,
    terrain_vertex_shaders: BTreeMap<usize, TerrainVertexShaderMetadata>,
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
                self.map_vertex_shaders.insert(
                    shader_ptr,
                    TerrainVertexShaderMetadata {
                        byte_len: bytecode.len(),
                        fnv1a64: fnv1a64_bytes(bytecode),
                        ..reflection
                    },
                );
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

    pub fn tree_pixel_shader(&self) -> Option<MapPixelShaderBinding> {
        self.current_pixel_shader
            .filter(|pixel_shader| self.tree_pixel_shaders.contains_key(pixel_shader))
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

    pub fn tree_pixel_shader_metadata(&self, pixel_shader: usize) -> Option<&PdxmapShaderMetadata> {
        self.tree_pixel_shaders.get(&pixel_shader)
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
        let pixel_shader = self.current_pixel_shader?;
        self.pdxwater_pixel_shaders
            .get(&pixel_shader)
            .or_else(|| self.pdxmap_pixel_shaders.get(&pixel_shader))
            .or_else(|| self.river_pixel_shaders.get(&pixel_shader))
            .or_else(|| self.border_pixel_shaders.get(&pixel_shader))
            .or_else(|| self.tree_pixel_shaders.get(&pixel_shader))
            .and_then(|shader| {
                shader
                    .constant_buffers
                    .iter()
                    .find(|(_, bind_point)| **bind_point == slot)
                    .map(|(name, _)| name.clone())
            })
    }

    pub fn current_vertex_shader_constant_buffer_name(&self, slot: u32) -> Option<String> {
        let vertex_shader = self.current_vertex_shader?;
        self.terrain_vertex_shaders
            .get(&vertex_shader)
            .or_else(|| self.map_vertex_shaders.get(&vertex_shader))
            .and_then(|shader| {
                shader
                    .constant_buffers
                    .iter()
                    .find(|(_, bind_point)| **bind_point == slot)
                    .map(|(name, _)| name.clone())
            })
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

    pub fn tree_pixel_shader_count(&self) -> usize {
        self.tree_pixel_shaders.len()
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
        }
        1
    }

    pub unsafe fn get_msg_proc(code: i32, wparam: usize, lparam: isize) -> isize {
        unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) }
    }

    unsafe extern "system" fn attach_worker(_param: Lpvoid) -> Dword {
        if !is_hoi4_process() {
            return 0;
        }
        write_probe_frame();
        if let Some(path) = module_path() {
            if let Some(dir) = path.parent() {
                super::d3d11_runtime::run_one_shot_capture(dir, TERRAIN_CAPTURE_TIMEOUT_MS);
            }
        }
        0
    }

    fn write_probe_frame() {
        let Some(mut path) = module_path() else {
            return;
        };
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

    fn is_hoi4_process() -> bool {
        process_path()
            .and_then(|path| path.file_name().map(|name| name.to_os_string()))
            .and_then(|name| name.to_str().map(|name| name.to_string()))
            .is_some_and(|name| name.eq_ignore_ascii_case("hoi4.exe"))
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

#[cfg(test)]
mod tests {
    use super::*;

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
        let river_slots = river_resource_slots_from_shader_bytecode(
            b"RiverData_Texture : register(t3); LeanTexture1_Texture : register(t4); \
              LeanTexture2_Texture : register(t5); SnowMudData_Texture : register(t8); \
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
    fn capture_state_tracks_active_non_terrain_map_layer_shaders() {
        let mut state = TerrainCaptureState::default();
        let river_shader = b"RiverData_Texture : register(t3); LeanTexture1_Texture : register(t4); LeanTexture2_Texture : register(t5); SnowMudData_Texture : register(t8); GradientBorderChannel1_Texture : register(t12); GradientBorderChannel2_Texture : register(t13);";
        let border_shader = b"BorderDiffuse_Texture : register(t0); SnowMudData_Texture : register(t1); IntelMap_Texture : register(t2); ShadowMap_Texture : register(t3);";
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
