mod static_program;
pub use static_program::*;
mod live_authority;
pub use live_authority::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawProcessEpoch {
    pub process_id: u32,
    pub process_start_nonce: u64,
    pub executable_sha256: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum RawShaderStage {
    Vertex,
    Pixel,
    Geometry,
    Hull,
    Domain,
    Compute,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RawDrawFamily {
    Particle,
    Entity,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum RawDrawIdentity {
    Particle {
        effect_instance_id: u64,
        registration_token: String,
        definition_source_token: String,
        builder_call_ordinal: u32,
        packet_ordinal: u32,
        draw_order: u32,
        shader_program_ptr: usize,
    },
    Entity {
        submit_index: u64,
        pass_ordinal: u32,
        entity_instance_id: u64,
        draw_order: u32,
        entity_object_ptr: usize,
        entity_definition_ptr: usize,
        renderer_ptr: usize,
        render_context_ptr: usize,
        mesh_draw_ptr: usize,
        material_binding_ptr: usize,
        shader_program_ptr: usize,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawVfsContentIdentity {
    pub stored_game_relative_path: String,
    pub logical_path: String,
    pub winning_layer_ordinal: u32,
    pub layer_kind: String,
    pub layer_id: String,
    pub source_sha256: String,
    pub payload_sha256: String,
    pub payload_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawPayloadSnapshot {
    pub captured_bytes: Vec<u8>,
    pub captured_sha256: String,
    pub source: RawPayloadSource,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RawPayloadSource {
    VfsRange {
        content: RawVfsContentIdentity,
        source_byte_offset: u64,
        byte_count: u64,
        range_sha256: String,
    },
    RuntimeGenerated {
        reason: String,
    },
    ParticleInstanceRecords,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawShaderDefine {
    pub name: String,
    pub value: String,
    pub scope: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawShaderCompilerInvocation {
    pub source_name: String,
    pub source_payload_sha256: String,
    pub source_payload_bytes: u64,
    pub entry_token: String,
    pub ordered_defines: Vec<RawShaderDefine>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawProgramIdentity {
    pub observation_sequence: u64,
    pub shader_program_ptr: usize,
    pub registration_root_rva: u32,
    pub source: RawVfsContentIdentity,
    pub effect_token: String,
    pub vertex_entry_token: String,
    pub pixel_entry_token: String,
    pub ordered_defines: Vec<RawShaderDefine>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawShaderCreation {
    pub creation_sequence: u64,
    pub shader_ptr: usize,
    pub stage: RawShaderStage,
    pub compiler_identity: String,
    pub target_profile: String,
    pub compiler_flags_u64: u64,
    pub compiler_invocation: Option<RawShaderCompilerInvocation>,
    /// Runtime-generated compiler output required for exact playback.
    pub bytecode: RawPayloadSnapshot,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawInputElement {
    pub semantic_token: String,
    pub semantic_index: u32,
    pub dxgi_format_u32: u32,
    pub input_slot: u32,
    pub aligned_byte_offset: u32,
    pub input_slot_class_u32: u32,
    pub instance_step_rate: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawInputLayoutCreation {
    pub creation_sequence: u64,
    pub input_layout_ptr: usize,
    pub ordered_elements: Vec<RawInputElement>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawRenderTargetBlendDesc {
    pub blend_enable_u32: u32,
    pub source_color_u32: u32,
    pub destination_color_u32: u32,
    pub color_operation_u32: u32,
    pub source_alpha_u32: u32,
    pub destination_alpha_u32: u32,
    pub alpha_operation_u32: u32,
    pub write_mask_rgba_u8: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawBlendStateCreation {
    pub creation_sequence: u64,
    pub blend_state_ptr: usize,
    pub alpha_to_coverage_enable_u32: u32,
    pub independent_blend_enable_u32: u32,
    pub ordered_targets: Vec<RawRenderTargetBlendDesc>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawStencilFaceDesc {
    pub fail_operation_u32: u32,
    pub depth_fail_operation_u32: u32,
    pub pass_operation_u32: u32,
    pub comparison_u32: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawDepthStencilStateCreation {
    pub creation_sequence: u64,
    pub depth_stencil_state_ptr: usize,
    pub depth_enable_u32: u32,
    pub depth_write_mask_u32: u32,
    pub depth_comparison_u32: u32,
    pub stencil_enable_u32: u32,
    pub stencil_read_mask_u8: u8,
    pub stencil_write_mask_u8: u8,
    pub front_face: RawStencilFaceDesc,
    pub back_face: RawStencilFaceDesc,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawRasterizerStateCreation {
    pub creation_sequence: u64,
    pub rasterizer_state_ptr: usize,
    pub fill_mode_u32: u32,
    pub cull_mode_u32: u32,
    pub front_counter_clockwise_u32: u32,
    pub depth_bias_i32: i32,
    pub depth_bias_clamp_f32_bits: u32,
    pub slope_scaled_depth_bias_f32_bits: u32,
    pub depth_clip_enable_u32: u32,
    pub scissor_enable_u32: u32,
    pub multisample_enable_u32: u32,
    pub antialiased_line_enable_u32: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawSamplerStateCreation {
    pub creation_sequence: u64,
    pub sampler_state_ptr: usize,
    pub filter_u32: u32,
    pub address_u_u32: u32,
    pub address_v_u32: u32,
    pub address_w_u32: u32,
    pub mip_lod_bias_f32_bits: u32,
    pub max_anisotropy: u32,
    pub comparison_u32: u32,
    pub border_color_rgba_f32_bits: [u32; 4],
    pub min_lod_f32_bits: u32,
    pub max_lod_f32_bits: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawBufferSnapshot {
    pub snapshot_sequence: u64,
    pub buffer_ptr: usize,
    pub byte_width: u32,
    pub usage_u32: u32,
    pub bind_flags_u32: u32,
    pub cpu_access_flags_u32: u32,
    pub misc_flags_u32: u32,
    pub structure_byte_stride: u32,
    pub payload: RawPayloadSnapshot,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawTextureSubresourceSnapshot {
    pub subresource_index: u32,
    pub mip_level: u32,
    pub array_slice: u32,
    pub row_pitch: u32,
    pub slice_pitch: u32,
    pub payload: RawPayloadSnapshot,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawTextureResourceSnapshot {
    pub snapshot_sequence: u64,
    pub resource_ptr: usize,
    pub dimension_u32: u32,
    pub width: u32,
    pub height: u32,
    pub depth: u32,
    pub mip_levels: u32,
    pub array_size: u32,
    pub dxgi_format_u32: u32,
    pub sample_count: u32,
    pub sample_quality: u32,
    pub usage_u32: u32,
    pub bind_flags_u32: u32,
    pub cpu_access_flags_u32: u32,
    pub misc_flags_u32: u32,
    pub content: Option<RawVfsContentIdentity>,
    pub ordered_subresources: Vec<RawTextureSubresourceSnapshot>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawResourceViewDesc {
    pub dxgi_format_u32: u32,
    pub view_dimension_u32: u32,
    pub flags_u32: u32,
    pub union_words_u32: [u32; 4],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawTexture2dDesc {
    pub width: u32,
    pub height: u32,
    pub mip_levels: u32,
    pub array_size: u32,
    pub dxgi_format_u32: u32,
    pub sample_count: u32,
    pub sample_quality: u32,
    pub usage_u32: u32,
    pub bind_flags_u32: u32,
    pub cpu_access_flags_u32: u32,
    pub misc_flags_u32: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawShaderResourceViewSnapshot {
    pub snapshot_sequence: u64,
    pub shader_resource_view_ptr: usize,
    pub resource_ptr: usize,
    pub desc: RawResourceViewDesc,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawStageSlotObject {
    pub stage: RawShaderStage,
    pub slot: u8,
    pub object_ptr: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawTextureObject {
    pub stage: RawShaderStage,
    pub slot: u8,
    pub shader_resource_view_ptr: usize,
    pub resource_ptr: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawVertexBufferObject {
    pub input_slot: u32,
    pub buffer_ptr: usize,
    pub stride_bytes: u32,
    pub byte_offset: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawIndexBufferObject {
    pub buffer_ptr: usize,
    pub dxgi_format_u32: u32,
    pub byte_offset: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RawDrawCall {
    Draw {
        vertex_count: u32,
        start_vertex: u32,
    },
    DrawIndexed {
        index_count: u32,
        start_index: u32,
        base_vertex: i32,
    },
    DrawInstanced {
        vertex_count_per_instance: u32,
        instance_count: u32,
        start_vertex: u32,
        start_instance: u32,
    },
    DrawIndexedInstanced {
        index_count_per_instance: u32,
        instance_count: u32,
        start_index: u32,
        base_vertex: i32,
        start_instance: u32,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawD3dDrawState {
    pub final_draw_sequence: u64,
    pub identity: RawDrawIdentity,
    pub family: RawDrawFamily,
    pub submit_index: u64,
    pub draw_order: u32,
    pub render_context_ptr: usize,
    pub device_context_ptr: usize,
    pub shader_program_ptr: usize,
    pub vertex_shader_ptr: usize,
    pub pixel_shader_ptr: usize,
    pub geometry_shader_ptr: Option<usize>,
    pub hull_shader_ptr: Option<usize>,
    pub domain_shader_ptr: Option<usize>,
    pub compute_shader_ptr: Option<usize>,
    pub dynamic_shader_linkage_present: bool,
    pub input_layout_ptr: Option<usize>,
    pub blend_state_ptr: Option<usize>,
    pub blend_factor_rgba_f32_bits: [u32; 4],
    pub sample_mask_u32: u32,
    pub depth_stencil_state_ptr: Option<usize>,
    pub stencil_reference_u32: u32,
    pub rasterizer_state_ptr: Option<usize>,
    pub primitive_topology_u32: u32,
    pub ordered_texture_objects: Vec<RawTextureObject>,
    pub ordered_sampler_objects: Vec<RawStageSlotObject>,
    pub ordered_constant_buffer_objects: Vec<RawStageSlotObject>,
    pub ordered_vertex_buffer_objects: Vec<RawVertexBufferObject>,
    pub index_buffer: Option<RawIndexBufferObject>,
    pub ordered_stream_output_buffer_ptrs: Vec<usize>,
    pub ordered_color_target_view_ptrs: Vec<usize>,
    pub depth_stencil_view_ptr: Option<usize>,
    pub ordered_viewports_f32_bits: Vec<[u32; 6]>,
    pub ordered_scissor_rects_i32: Vec<[i32; 4]>,
    pub draw_call: RawDrawCall,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawTextureSemantic {
    pub stage: RawShaderStage,
    pub slot: u8,
    pub semantic_token: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawSamplerSemantic {
    pub stage: RawShaderStage,
    pub slot: u8,
    pub semantic_token: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawConstantBufferSemantic {
    pub stage_mask_u8: u8,
    pub slot: u8,
    pub byte_offset: u32,
    pub byte_count: u32,
    pub semantic_layout: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RawPreparedTarget {
    SwapchainBackBuffer,
    OpaqueRenderTarget { attachment_ordinal: u32 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawOutputTargetSemantic {
    pub view_ptr: usize,
    pub prepared_target: RawPreparedTarget,
    pub resource_token: Option<String>,
    pub producer_pass_token: Option<String>,
    pub persistent_content: Option<RawVfsContentIdentity>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RawDrawContentIdentity {
    Particle {
        registration_token: String,
        definition_source_token: String,
        subsystem_selector_u32: u32,
        definition_content: RawVfsContentIdentity,
        texture_content: RawVfsContentIdentity,
        authored_shader_token: String,
        engine_effect_token: String,
        atlas_columns_u32: u32,
        atlas_rows_u32: u32,
        atlas_half_pixel_width_f32_bits: u32,
        atlas_half_pixel_height_f32_bits: u32,
    },
    Entity {
        resolved_entity_asset: RawVfsContentIdentity,
        mesh: RawVfsContentIdentity,
        entity_registration_token: String,
        entity_group_index: u32,
        mesh_draw_list_index: u32,
        entity_variant_u8: u8,
        mesh_draw_variant_u8: u8,
        pass_selector_i32: i32,
        alternate_list_u8: u8,
        bind_renderer_target_u8: u8,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawDrawSemantics {
    pub observation_sequence: u64,
    pub identity: RawDrawIdentity,
    pub content_identity: RawDrawContentIdentity,
    pub ordered_texture_semantics: Vec<RawTextureSemantic>,
    pub ordered_sampler_semantics: Vec<RawSamplerSemantic>,
    pub ordered_constant_buffer_semantics: Vec<RawConstantBufferSemantic>,
    pub ordered_color_targets: Vec<RawOutputTargetSemantic>,
    pub depth_target: Option<RawOutputTargetSemantic>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawRenderTargetViewSnapshot {
    pub snapshot_sequence: u64,
    pub view_ptr: usize,
    pub resource_ptr: usize,
    pub view_kind: RawOutputViewKind,
    pub desc: RawResourceViewDesc,
    pub resource_dimension_u32: u32,
    pub resource_width: u32,
    pub resource_height: u32,
    pub resource_depth: u32,
    pub resource_mip_levels: u32,
    pub resource_array_size: u32,
    pub resource_sample_count: u32,
    pub resource_sample_quality: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RawOutputViewKind {
    Color,
    DepthStencil,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawRenderCatalogCapture {
    pub process: RawProcessEpoch,
    pub launch_epoch: RawCaptureLaunchEpoch,
    pub submit_index: u64,
    pub frame_sequence: u64,
    pub draw_states: Vec<RawD3dDrawState>,
    pub draw_semantics: Vec<RawDrawSemantics>,
    pub program_identities: Vec<RawProgramIdentity>,
    pub shader_creations: Vec<RawShaderCreation>,
    pub input_layout_creations: Vec<RawInputLayoutCreation>,
    pub blend_state_creations: Vec<RawBlendStateCreation>,
    pub depth_stencil_state_creations: Vec<RawDepthStencilStateCreation>,
    pub rasterizer_state_creations: Vec<RawRasterizerStateCreation>,
    pub sampler_state_creations: Vec<RawSamplerStateCreation>,
    pub buffer_snapshots: Vec<RawBufferSnapshot>,
    pub texture_resource_snapshots: Vec<RawTextureResourceSnapshot>,
    pub shader_resource_view_snapshots: Vec<RawShaderResourceViewSnapshot>,
    pub render_target_view_snapshots: Vec<RawRenderTargetViewSnapshot>,
}

impl RawRenderCatalogCapture {
    pub fn validate_launch_binding(&self) -> Result<(), String> {
        let receipt = &self.launch_epoch.hook_set_receipt_sha256;
        let capture_receipt = &self.launch_epoch.capture_receipt_sha256;
        if self.process.process_id == 0
            || self.process.process_start_nonce == 0
            || self.launch_epoch.process_id != self.process.process_id
            || self.launch_epoch.process_start_nonce != self.process.process_start_nonce
            || self.launch_epoch.observer_installed_qpc == 0
            || self.launch_epoch.d3d_device_created_qpc == 0
            || self.launch_epoch.observer_installed_qpc > self.launch_epoch.d3d_device_created_qpc
            || self.launch_epoch.first_creation_sequence == 0
            || self.launch_epoch.hook_epoch_nonce == 0
            || receipt.len() != 64
            || !receipt
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            || capture_receipt.len() != 64
            || !capture_receipt
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err(
                "catalog lacks an authenticated hook receipt installed before D3D creation".into(),
            );
        }
        Ok(())
    }
}
