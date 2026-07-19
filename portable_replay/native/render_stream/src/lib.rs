//! Exact non-UI render-submit transport shared by recorder and standalone replay.
//!
//! This crate contains no extractor or renderer dependency.  It transports the
//! already-observed XN12/XN13/XN14/XN15 packet surface; it never derives a
//! particle packet, skinning palette, light texture, weather membership, or
//! political fill from gameplay state.

pub use portable_content_manifest::UsedContentKey;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::collections::BTreeSet;

pub const SCHEMA: &str = "hoi4.production-render-submit.v3";
pub const INCOMPATIBLE_ENTITY_SELECTOR_SCHEMA: &str = "hoi4.production-render-submit.v2";
const COMPACT_GAME_HOUR_RAW_EPOCH: i64 = 43_799_999;

fn compact_update_index_from_raw(raw: i32) -> Option<u64> {
    let raw = i64::from(raw);
    let min = 1850i64 * 365 * 24 + COMPACT_GAME_HOUR_RAW_EPOCH;
    let max = 2200i64 * 365 * 24 + COMPACT_GAME_HOUR_RAW_EPOCH;
    if raw < min || raw >= max {
        return None;
    }
    u64::try_from(raw - COMPACT_GAME_HOUR_RAW_EPOCH).ok()
}

mod m02_combat;
pub use m02_combat::{
    ExactM02Field, ExactM02FieldStatus, M02CombatDraw, M02CombatDrawRef, M02CombatFrame,
    M02CombatIndicator, M02CombatKind, M02Direction, M02FogState, M02LawState, M02ObserverRelation,
    M02OverlapState, M02Participants, M02ResultPresentation, M02ResultState, M02WidgetState,
    M02_COMBAT_SCHEMA,
};

mod m03_arrows;
pub use m03_arrows::{
    M03ArrowDraw, M03ArrowDrawMember, M03ArrowDrawMemberRef, M03ArrowDrawRef, M03ArrowFrame,
    M03ArrowRow, M03PlanSubtype, M03StableKey, M03Visibility, M03_A729_EXECUTABLE_SHA256,
    M03_ARROW_SCHEMA, M03_STATIC_PROOF_SHA256,
};

mod o04_cities;
pub use o04_cities::{
    O04CityFrame, O04CitySemantic, O04CitylightState, O04_A729_EXECUTABLE_SHA256, O04_CITY_SCHEMA,
    O04_EVIDENCE_MANIFEST_SHA256, O04_LIVE_CITY_SUBMIT_BINDING_SHA256,
    O04_LIVE_CITY_SUBMIT_VERIFICATION_SHA256,
};

mod ui_pass;
pub use ui_pass::{
    ActiveMapmodeState, AuthenticatedUiPass, FontBinding, HudCommand, HudCommandExecution,
    HudCommandIdentity, HudPaintCommand, HudPlacement, InlineIconBinding, MapmodeProvenance,
    SpriteLaw, TextHorizontalAlignment, TextLayoutInputs, TextRectOffsets, UiExactPipelineDraw,
    UiPassClip, UiPassOperation, UiPassRect, UI_A729_EXECUTABLE_SHA256, UI_PASS_SCHEMA,
};

mod decimal_u64 {
    use serde::{de::Error, Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(value: &u64, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&value.to_string())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
        let text = String::deserialize(deserializer)?;
        if text.is_empty() || (text.len() > 1 && text.starts_with('0')) {
            return Err(D::Error::custom("u64 text is not canonical decimal"));
        }
        text.parse::<u64>().map_err(D::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProductionRenderFrame {
    pub schema: String,
    pub authority: RenderAuthority,
    #[serde(with = "decimal_u64")]
    pub update_index: u64,
    #[serde(with = "decimal_u64")]
    pub first_sequence: u64,
    #[serde(with = "decimal_u64")]
    pub last_sequence: u64,
    #[serde(with = "decimal_u64")]
    pub presentation_qpc_frequency_hz: u64,
    /// The exact submit whose fully-evaluated output is the hour checkpoint's
    /// displayed state. No consumer may choose first/last heuristically.
    pub display_submit_index: Option<String>,
    /// Pointer-free proof of the exact completed Present selected at the
    /// authenticated hourly cutoff. Production retains only that one Present;
    /// earlier same-hour candidates are debug-forensics data.
    pub selected_present: SelectedPresentReceipt,
    pub submits: Vec<RenderSubmit>,
    /// Exact combat-indicator UI-overlay carrier. This is deliberately
    /// separate from static-map programs and particle/entity ordered draws.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub m02_combat: Option<M02CombatFrame>,
    /// Exact order/movement-arrow semantic rows and draw packets. This map
    /// pass is independent of the swapchain UI-overlay M02 carrier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub m03_arrows: Option<M03ArrowFrame>,
    /// Optional exact semantic join between O04 city-generator rows and the
    /// existing entity draws. It carries no duplicate draw payloads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub o04_cities: Option<O04CityFrame>,
    /// Optional exact selected-Present UI-pass operation stream. Older v3
    /// frames omit it; when present it is complete and fail-closed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authenticated_ui_pass: Option<AuthenticatedUiPass>,
    pub checkpoint: RenderCheckpoint,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectedPresentReceipt {
    pub candidate_count: u32,
    pub candidate_set_sha256: String,
    #[serde(with = "decimal_u64")]
    pub selected_candidate_id: u64,
    pub selected_ordinal: u32,
    pub selected_game_hour_raw: i32,
    #[serde(with = "decimal_u64")]
    pub selected_qpc_ticks: u64,
    #[serde(with = "decimal_u64")]
    pub selection_cutoff_qpc_ticks: u64,
}

impl SelectedPresentReceipt {
    pub fn sealed(
        candidate_count: u32,
        candidate_set_sha256: String,
        selected_candidate_id: u64,
        selected_ordinal: u32,
        selected_game_hour_raw: i32,
        selected_qpc_ticks: u64,
        selection_cutoff_qpc_ticks: u64,
    ) -> Self {
        Self {
            candidate_count,
            candidate_set_sha256,
            selected_candidate_id,
            selected_ordinal,
            selected_game_hour_raw,
            selected_qpc_ticks,
            selection_cutoff_qpc_ticks,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenderAuthority {
    pub process_id: u32,
    #[serde(with = "decimal_u64")]
    pub process_start_nonce: u64,
    #[serde(with = "decimal_u64")]
    pub observer_installed_qpc: u64,
    #[serde(with = "decimal_u64")]
    pub d3d_device_created_qpc: u64,
    #[serde(with = "decimal_u64")]
    pub first_creation_sequence: u64,
    #[serde(with = "decimal_u64")]
    pub hook_epoch_nonce: u64,
    pub hook_set_receipt_sha256: String,
    pub capture_receipt_sha256: String,
    pub launch_epoch_sha256: String,
    pub effective_vfs_manifest_sha256: String,
    pub effect_catalog_sha256: String,
    pub entity_animation_catalog_sha256: String,
    /// Authenticated session input family. Absence means product UI that
    /// depends on input-family selection is omitted; playback never defaults.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recorded_input_variant: Option<RecordedInputVariant>,
    pub particle_runtime_mode: ParticleRuntimeMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordedInputVariant {
    MouseKeyboard,
    Keyboard,
    Controller,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParticleRuntimeMode {
    SubmittedPacketReplay,
}

pub fn compute_render_launch_epoch_sha256(
    process_id: u32,
    process_start_nonce: u64,
    observer_installed_qpc: u64,
    d3d_device_created_qpc: u64,
    first_creation_sequence: u64,
    hook_epoch_nonce: u64,
    hook_set_receipt_sha256: &str,
    capture_receipt_sha256: &str,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"hoi4.production-render-launch-epoch.v1\0");
    hasher.update(process_id.to_le_bytes());
    for value in [
        process_start_nonce,
        observer_installed_qpc,
        d3d_device_created_qpc,
        first_creation_sequence,
        hook_epoch_nonce,
    ] {
        hasher.update(value.to_le_bytes());
    }
    hasher.update(hook_set_receipt_sha256.as_bytes());
    hasher.update(capture_receipt_sha256.as_bytes());
    format!("{:x}", hasher.finalize())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenderSubmit {
    #[serde(with = "decimal_u64")]
    pub submit_index: u64,
    pub global_order: u32,
    #[serde(with = "decimal_u64")]
    pub presentation_qpc_ticks: u64,
    pub interpolation_phase_f32_bits: u32,
    pub ordered_scene_constant_buffers: Vec<ConstantBufferBinding>,
    /// Complete engine-observed static map program for this selected submit.
    /// Empty means the static map is unavailable and must be omitted. A
    /// non-empty carrier is accepted only when every present consumer program
    /// is complete and follows canonical pass/global operation order. Missing
    /// consumers are feature-local omissions, never inferred programs.
    #[serde(default)]
    pub ordered_static_map_programs: Vec<StaticMapPassProgram>,
    pub effect_lifecycle_events: Vec<EffectLifecycleEvent>,
    pub particles: Vec<ParticlePacket>,
    pub entity_presentations: Vec<EntityPresentationEvent>,
    pub particle_packet_set_sha256: String,
    pub entities: Vec<EntitySubmission>,
    /// Authoritative cross-family draw interleave for this submit. Particle
    /// bucket order and entity pass order remain local keys only; the renderer
    /// must execute this list and may not concatenate the family vectors.
    pub ordered_draws: Vec<RenderOutputRef>,
    /// Sole cross-family execution order when static map programs are present.
    /// It spans static clears/draws, runtime particle/entity draws, and
    /// post-process/presentation. Playback may not concatenate those families.
    #[serde(default)]
    pub ordered_render_operations: Vec<RenderExecutionRef>,
    pub direct_visuals: Vec<DirectVisualProducer>,
    pub light_lifecycle_events: Vec<LightLifecycleEvent>,
    pub tiled_lights: Vec<TiledLightSubmission>,
    pub weather_membership: Vec<WeatherMembership>,
    #[serde(default)]
    pub political_textures: Vec<PoliticalTexturePacket>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StaticMapPassProgram {
    pub program_ordinal: u32,
    pub pass_token: String,
    /// Exact ContentManifest keys in engine-observed lookup order.
    pub ordered_content_keys: Vec<String>,
    pub ordered_operations: Vec<StaticMapOperation>,
    pub payload_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "family", rename_all = "snake_case", deny_unknown_fields)]
pub enum RenderExecutionRef {
    Static {
        operation_ordinal: u32,
    },
    Runtime {
        output: RenderOutputRef,
    },
    /// Exact swapchain UI-overlay draw. These references form a suffix after
    /// map/post-process execution and must exactly cover the frame-level M02
    /// carrier. The embedded ordinal remains the complete engine UI-pass
    /// ordinal; it is not renumbered into map/runtime space.
    M02 {
        output: M02CombatDrawRef,
    },
    /// Exact map order/movement-arrow draw in global engine draw order.
    M03 {
        output: M03ArrowDrawRef,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StaticMapOperation {
    /// Global across every static program in this submit.
    pub operation_ordinal: u32,
    pub payload_sha256: String,
    pub command: StaticMapCommand,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum StaticMapCommand {
    ClearColor {
        target: RenderTargetDesc,
        rgba_f32_bits: [u32; 4],
    },
    ClearDepthStencil {
        target: RenderTargetDesc,
        clear_depth: bool,
        clear_stencil: bool,
        depth_f32_bits: u32,
        stencil_u8: u8,
    },
    Draw {
        draw: StaticMapDraw,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StaticMapDraw {
    pub draw_token: String,
    pub shader_program: ShaderProgram,
    pub ordered_shader_resources: Vec<StaticShaderResourceBinding>,
    pub pipeline: PipelineDesc,
    pub geometry: StaticGeometrySource,
    /// Draw-local CBs only. Scene CB association is the containing submit's
    /// `ordered_scene_constant_buffers` and is never reconstructed by name.
    pub ordered_constant_buffers: Vec<ConstantBufferBinding>,
    pub output_state: RenderOutputState,
    pub payload_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StaticShaderResourceBinding {
    pub stage: ShaderStage,
    pub slot: u8,
    pub semantic_token: String,
    pub source: StaticShaderResourceSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum StaticShaderResourceSource {
    UsedContent {
        content: UsedContentKey,
        resource: TextureResourceDesc,
        view: ShaderResourceViewDesc,
    },
    PreparedTarget {
        target: PreparedRenderTarget,
        resource: TextureResourceDesc,
        view: ShaderResourceViewDesc,
        /// Exact last writer of this render-graph resource at the read point.
        producer: RenderExecutionRef,
    },
    PoliticalTexture {
        texture_kind: PoliticalTextureKind,
        target_binding_identity: String,
        upload_order: u32,
        payload_sha256: String,
        resource: TextureResourceDesc,
        view: ShaderResourceViewDesc,
    },
    TiledLightIndex {
        #[serde(with = "decimal_u64")]
        writer_index: u64,
        payload_sha256: String,
        resource: TextureResourceDesc,
        view: ShaderResourceViewDesc,
    },
    TiledLightData {
        #[serde(with = "decimal_u64")]
        writer_index: u64,
        payload_sha256: String,
        resource: TextureResourceDesc,
        view: ShaderResourceViewDesc,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum StaticGeometrySource {
    Exact {
        geometry: GeometryBinding,
    },
    InterpreterOutput {
        interpreter_token: String,
        ordered_input_content_keys: Vec<String>,
        ordered_vertex_buffers: Vec<InterpreterVertexBufferDesc>,
        index_buffer: Option<InterpreterIndexBufferDesc>,
        stream_output_targets: Vec<StreamOutputTargetBinding>,
        draw: DrawCommand,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InterpreterVertexBufferDesc {
    pub input_slot: u32,
    pub stride_bytes: u32,
    pub byte_offset: u32,
    pub vertex_count: u32,
    pub payload_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InterpreterIndexBufferDesc {
    pub format: IndexFormat,
    pub byte_offset: u32,
    pub index_count: u32,
    pub payload_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParticlePacket {
    #[serde(with = "decimal_u64")]
    pub effect_instance_id: u64,
    pub registration_token: String,
    pub resolved_definition_sha256: String,
    pub content_identity: ParticleContentIdentity,
    /// SHA-256 of the canonical serde JSON bytes of `content_identity`.
    pub content_identity_sha256: String,
    pub bucket: u32,
    pub packet_ordinal: u32,
    pub draw_order: u32,
    pub ribbon_tail_order: Option<u32>,
    /// Exact 0x70 submit packet with the process-local pointer at +0x48
    /// canonicalized to zero. Portable replay must never persist or reuse the
    /// captured address; `ordered_gpu_instance_records` is its replacement.
    pub canonical_packet_112_bytes: Vec<u8>,
    pub canonical_packet_sha256: String,
    pub packet_matrix_4x4_f32_bits: [u32; 16],
    pub live_count: u32,
    pub record_stride: u32,
    pub max_count: u32,
    pub ordered_gpu_instance_records: Vec<u8>,
    pub gpu_instance_records_sha256: String,
    pub elapsed_f32_bits: u32,
    pub shader_program: ShaderProgram,
    pub ordered_texture_bindings: Vec<TextureBinding>,
    pub pipeline: PipelineDesc,
    /// Exact IA buffer bindings and draw call. This is intentionally separate
    /// from the pipeline: authored ribbons, static quads, and the captured
    /// instance stream are not interchangeable sources.
    pub geometry: GeometryBinding,
    pub ordered_constant_buffers: Vec<ConstantBufferBinding>,
    pub output_state: RenderOutputState,
    pub atlas_rows: u32,
    pub atlas_columns: u32,
    pub scale_f32_bits: u32,
    pub flags_u16: u16,
}

/// Exact stable identity of the selected particle subsystem. Process-local
/// definition/program/texture pointers and runtime indices are deliberately
/// absent; the extractor must validate those before emitting this carrier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParticleContentIdentity {
    pub definition_content: UsedContentKey,
    /// Exact packet+0x60 selector, also carried by `ParticlePacket::packet_ordinal`.
    pub subsystem_selector_u32: u32,
    pub texture_content: UsedContentKey,
    pub authored_shader_token: String,
    pub engine_effect_token: String,
    pub atlas_columns_u32: u32,
    pub atlas_rows_u32: u32,
    pub atlas_half_pixel_width_f32_bits: u32,
    pub atlas_half_pixel_height_f32_bits: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShaderProgram {
    pub source: UsedContentKey,
    pub effect_token: String,
    pub vertex_entry_token: String,
    pub pixel_entry_token: String,
    pub ordered_defines: Vec<ShaderDefine>,
    pub compiled_vertex_shader: CompiledShaderPayload,
    pub compiled_pixel_shader: CompiledShaderPayload,
    /// Optional programmable stages are explicit carrier state. `None` means
    /// the authenticated draw bound a null shader for that stage; consumers
    /// must not infer null merely because an older VS/PS-only schema omitted it.
    pub geometry_shader: Option<AdditionalShaderStage>,
    pub hull_shader: Option<AdditionalShaderStage>,
    pub domain_shader: Option<AdditionalShaderStage>,
    pub compute_shader: Option<AdditionalShaderStage>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdditionalShaderStage {
    pub entry_token: String,
    pub compiled: CompiledShaderPayload,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShaderDefine {
    pub name: String,
    pub value: String,
    pub scope: ShaderDefineScope,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShaderDefineScope {
    EngineGlobal,
    Effect,
    EntryPoint,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompiledShaderPayload {
    pub compiler_identity: String,
    pub target_profile: String,
    pub compiler_flags_u64: u64,
    pub bytecode: Vec<u8>,
    pub sha256: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShaderStage {
    Vertex,
    Pixel,
    Geometry,
    Hull,
    Domain,
    Compute,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextureBinding {
    pub stage: ShaderStage,
    pub slot: u8,
    pub semantic_token: String,
    pub content: UsedContentKey,
    /// Exact underlying D3D resource and ordered initial subresources. This
    /// prevents DDS/non-DDS decoder defaults and typeless/view-format choices
    /// from becoming renderer policy.
    pub resource: TextureResourceDesc,
    /// Exact SRV descriptor. The underlying DDS format is insufficient because
    /// the live view may select (for example) UNORM_SRGB over UNORM and may
    /// expose only a mip/array subrange.
    pub view: ShaderResourceViewDesc,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextureResourceDesc {
    pub dimension: TextureResourceDimension,
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
    pub ordered_subresources: Vec<TextureSubresourcePayload>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextureResourceDimension {
    Texture1d,
    Texture2d,
    Texture3d,
    TextureCube,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextureSubresourcePayload {
    pub subresource_index: u32,
    pub mip_level: u32,
    pub array_slice: u32,
    pub row_pitch: u32,
    pub slice_pitch: u32,
    pub source: BufferSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "dimension", rename_all = "snake_case", deny_unknown_fields)]
pub enum ShaderResourceViewDesc {
    Buffer {
        dxgi_format_u32: u32,
        first_element: u32,
        element_count: u32,
        flags_u32: u32,
    },
    Texture1d {
        dxgi_format_u32: u32,
        most_detailed_mip: u32,
        mip_levels: u32,
    },
    Texture1dArray {
        dxgi_format_u32: u32,
        most_detailed_mip: u32,
        mip_levels: u32,
        first_array_slice: u32,
        array_size: u32,
    },
    Texture2d {
        dxgi_format_u32: u32,
        most_detailed_mip: u32,
        mip_levels: u32,
    },
    Texture2dArray {
        dxgi_format_u32: u32,
        most_detailed_mip: u32,
        mip_levels: u32,
        first_array_slice: u32,
        array_size: u32,
    },
    Texture2dMultisampled {
        dxgi_format_u32: u32,
    },
    Texture2dMultisampledArray {
        dxgi_format_u32: u32,
        first_array_slice: u32,
        array_size: u32,
    },
    Texture3d {
        dxgi_format_u32: u32,
        most_detailed_mip: u32,
        mip_levels: u32,
    },
    TextureCube {
        dxgi_format_u32: u32,
        most_detailed_mip: u32,
        mip_levels: u32,
    },
    TextureCubeArray {
        dxgi_format_u32: u32,
        most_detailed_mip: u32,
        mip_levels: u32,
        first_face: u32,
        cube_count: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConstantBufferBinding {
    /// Bit 0 VS, bit 1 PS, bit 2 GS, bit 3 HS, bit 4 DS, bit 5 CS.
    pub stage_mask_u8: u8,
    pub slot: u8,
    pub byte_offset: u32,
    pub byte_count: u32,
    pub raw_bytes: Vec<u8>,
    pub sha256: String,
    pub semantic_layout: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PipelineDesc {
    pub topology: PrimitiveTopology,
    pub blend: BlendDesc,
    pub depth: DepthDesc,
    pub rasterizer: RasterizerDesc,
    pub samplers: Vec<SamplerDesc>,
    pub input_layout: Vec<InputElementDesc>,
    pub index_format: IndexFormat,
    pub static_indices_u16: Vec<u16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrimitiveTopology {
    TriangleList,
    TriangleStrip,
    LineList,
    LineStrip,
    PointList,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlendDesc {
    pub alpha_to_coverage_enabled: bool,
    pub independent_blend_enabled: bool,
    pub enabled: bool,
    pub source_color: BlendFactor,
    pub destination_color: BlendFactor,
    pub color_operation: BlendOperation,
    pub source_alpha: BlendFactor,
    pub destination_alpha: BlendFactor,
    pub alpha_operation: BlendOperation,
    pub write_mask_rgba_u8: u8,
    /// Targets 1..N when independent blending is enabled. Target zero uses
    /// the fields above.
    pub additional_targets: Vec<RenderTargetBlendDesc>,
    pub blend_factor_rgba_f32_bits: [u32; 4],
    pub sample_mask_u32: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenderTargetBlendDesc {
    pub enabled: bool,
    pub source_color: BlendFactor,
    pub destination_color: BlendFactor,
    pub color_operation: BlendOperation,
    pub source_alpha: BlendFactor,
    pub destination_alpha: BlendFactor,
    pub alpha_operation: BlendOperation,
    pub write_mask_rgba_u8: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlendFactor {
    Zero,
    One,
    SourceAlpha,
    InverseSourceAlpha,
    DestinationAlpha,
    InverseDestinationAlpha,
    SourceColor,
    InverseSourceColor,
    DestinationColor,
    InverseDestinationColor,
    SourceAlphaSaturate,
    BlendFactor,
    InverseBlendFactor,
    Source1Color,
    InverseSource1Color,
    Source1Alpha,
    InverseSource1Alpha,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlendOperation {
    Add,
    Subtract,
    ReverseSubtract,
    Minimum,
    Maximum,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DepthDesc {
    pub enabled: bool,
    pub write_enabled: bool,
    pub comparison: Comparison,
    pub stencil_enabled: bool,
    pub stencil_read_mask_u8: u8,
    pub stencil_write_mask_u8: u8,
    pub front_face: StencilFaceDesc,
    pub back_face: StencilFaceDesc,
    pub stencil_reference_u32: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StencilFaceDesc {
    pub fail_operation: StencilOperation,
    pub depth_fail_operation: StencilOperation,
    pub pass_operation: StencilOperation,
    pub comparison: Comparison,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StencilOperation {
    Keep,
    Zero,
    Replace,
    IncrementSaturate,
    DecrementSaturate,
    Invert,
    Increment,
    Decrement,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Comparison {
    Never,
    Less,
    Equal,
    LessEqual,
    Greater,
    NotEqual,
    GreaterEqual,
    Always,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RasterizerDesc {
    pub fill: FillMode,
    pub cull: CullMode,
    pub front_counter_clockwise: bool,
    pub depth_clip_enabled: bool,
    pub scissor_enabled: bool,
    pub depth_bias_i32: i32,
    pub depth_bias_clamp_f32_bits: u32,
    pub slope_scaled_depth_bias_f32_bits: u32,
    pub multisample_enabled: bool,
    pub antialiased_line_enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FillMode {
    Solid,
    Wireframe,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CullMode {
    None,
    Front,
    Back,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SamplerDesc {
    pub stage: ShaderStage,
    pub slot: u8,
    pub semantic_token: String,
    pub min_filter: Filter,
    pub mag_filter: Filter,
    pub mip_filter: Filter,
    pub address_u: AddressMode,
    pub address_v: AddressMode,
    pub address_w: AddressMode,
    pub comparison: Option<Comparison>,
    pub mip_lod_bias_f32_bits: u32,
    pub max_anisotropy: u32,
    pub border_color_rgba_f32_bits: [u32; 4],
    pub min_lod_f32_bits: u32,
    pub max_lod_f32_bits: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Filter {
    Point,
    Linear,
    Anisotropic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AddressMode {
    Wrap,
    Mirror,
    Clamp,
    Border,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputElementDesc {
    pub semantic_token: String,
    pub semantic_index: u32,
    pub dxgi_format_u32: u32,
    pub input_slot: u32,
    pub aligned_byte_offset: u32,
    pub per_instance: bool,
    pub instance_step_rate: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IndexFormat {
    R16Uint,
    R32Uint,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeometryBinding {
    pub ordered_vertex_buffers: Vec<VertexBufferBinding>,
    pub index_buffer: Option<IndexBufferBinding>,
    /// Exact D3D11 stream-output bindings at draw time. Empty is an explicit
    /// authenticated null binding. `byte_offset == u32::MAX` preserves the
    /// D3D11 append sentinel rather than inventing an offset.
    pub stream_output_targets: Vec<StreamOutputTargetBinding>,
    pub draw: DrawCommand,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StreamOutputTargetBinding {
    pub slot: u32,
    pub buffer_identity: String,
    pub buffer_byte_width: u32,
    pub byte_offset: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VertexBufferBinding {
    pub input_slot: u32,
    pub stride_bytes: u32,
    pub byte_offset: u32,
    pub source: BufferSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IndexBufferBinding {
    pub format: IndexFormat,
    pub byte_offset: u32,
    pub source: BufferSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BufferSource {
    Inline {
        bytes: Vec<u8>,
        sha256: String,
    },
    UsedContentRange {
        content: UsedContentKey,
        source_byte_offset: u64,
        byte_count: u64,
        range_sha256: String,
    },
    /// References `ParticlePacket::ordered_gpu_instance_records` exactly.
    ParticleInstanceRecords,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DrawCommand {
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenderOutputState {
    pub ordered_color_targets: Vec<RenderTargetDesc>,
    pub depth_target: Option<RenderTargetDesc>,
    pub viewport: ViewportDesc,
    pub scissor: Option<ScissorDesc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenderTargetDesc {
    /// Exact registration identity. The a729 registration seam proves only
    /// swapchain ownership or an opaque attachment ordinal; semantic scene,
    /// depth, post-process, and named-resource aliases are forbidden.
    pub prepared_target: PreparedRenderTarget,
    pub width: u32,
    pub height: u32,
    pub dxgi_format_u32: u32,
    pub mip_level: u32,
    pub sample_count: u32,
    pub sample_quality: u32,
    pub persistent_content: Option<UsedContentKey>,
    /// Exact resource, views, and initialization needed when the selected
    /// attachment cannot be recreated from the legacy mip-zero transient
    /// descriptor. Absence remains valid only for that legacy subset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exact_resource: Option<RenderTargetExactResource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenderTargetExactResource {
    pub resource: RenderTargetResourceDesc,
    pub ordered_views: Vec<RenderTargetViewDesc>,
    pub ordered_initial_subresources: Vec<RenderTargetInitialSubresource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenderTargetResourceDesc {
    pub dimension: TextureResourceDimension,
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RenderTargetViewDesc {
    RenderTarget {
        dxgi_format_u32: u32,
        view: RenderTargetAttachmentView,
    },
    DepthStencil {
        dxgi_format_u32: u32,
        flags_u32: u32,
        view: DepthStencilAttachmentView,
    },
    ShaderResource {
        view: ShaderResourceViewDesc,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "dimension", rename_all = "snake_case", deny_unknown_fields)]
pub enum RenderTargetAttachmentView {
    Texture1d {
        mip_slice: u32,
    },
    Texture1dArray {
        mip_slice: u32,
        first_array_slice: u32,
        array_size: u32,
    },
    Texture2d {
        mip_slice: u32,
    },
    Texture2dArray {
        mip_slice: u32,
        first_array_slice: u32,
        array_size: u32,
    },
    Texture2dMultisampled,
    Texture2dMultisampledArray {
        first_array_slice: u32,
        array_size: u32,
    },
    Texture3d {
        mip_slice: u32,
        first_w_slice: u32,
        w_size: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "dimension", rename_all = "snake_case", deny_unknown_fields)]
pub enum DepthStencilAttachmentView {
    Texture1d {
        mip_slice: u32,
    },
    Texture1dArray {
        mip_slice: u32,
        first_array_slice: u32,
        array_size: u32,
    },
    Texture2d {
        mip_slice: u32,
    },
    Texture2dArray {
        mip_slice: u32,
        first_array_slice: u32,
        array_size: u32,
    },
    Texture2dMultisampled,
    Texture2dMultisampledArray {
        first_array_slice: u32,
        array_size: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenderTargetInitialSubresource {
    pub subresource_ordinal: u32,
    pub row_pitch: u32,
    pub slice_pitch: u32,
    pub source: RenderTargetInitialSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RenderTargetInitialSource {
    Inline {
        bytes: Vec<u8>,
        sha256: String,
    },
    UsedContentRange {
        content: UsedContentKey,
        source_byte_offset: u64,
        byte_count: u64,
        range_sha256: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PreparedRenderTarget {
    SwapchainBackBuffer,
    OpaqueRenderTarget { attachment_ordinal: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ViewportDesc {
    pub top_left_x_f32_bits: u32,
    pub top_left_y_f32_bits: u32,
    pub width_f32_bits: u32,
    pub height_f32_bits: u32,
    pub min_depth_f32_bits: u32,
    pub max_depth_f32_bits: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScissorDesc {
    pub left_i32: i32,
    pub top_i32: i32,
    pub right_i32: i32,
    pub bottom_i32: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectLifecycleEvent {
    #[serde(with = "decimal_u64")]
    pub sequence: u64,
    #[serde(with = "decimal_u64")]
    pub update_index: u64,
    #[serde(with = "decimal_u64")]
    pub effect_instance_id: u64,
    pub operation: EffectOperation,
    pub registration_token: String,
    pub registration_hash_u32: u32,
    pub resolved_definition_sha256: String,
    pub producer_kind: String,
    pub producer_instance_id: String,
    pub authored_event_index: u32,
    pub parent_anchor: ParentAnchor,
    pub initial_rng_seed_u32: u32,
    pub initial_rng_counter_u32: u32,
    pub elapsed_f32_bits: u32,
    pub time_scale_f32_bits: u32,
    pub active_flags_u32: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectOperation {
    Spawn,
    Activate,
    Deactivate,
    StopEmission,
    Destroy,
    Reparent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ParentAnchor {
    Entity {
        #[serde(with = "decimal_u64")]
        entity_instance_id: u64,
        node_token: String,
    },
    World {
        world_transform_4x4_f32_bits: [u32; 16],
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntityPresentationEvent {
    #[serde(with = "decimal_u64")]
    pub sequence: u64,
    #[serde(with = "decimal_u64")]
    pub update_index: u64,
    #[serde(with = "decimal_u64")]
    pub entity_instance_id: u64,
    pub resolved_entity_sha256: String,
    pub state_token: String,
    #[serde(with = "decimal_u64")]
    pub state_enter_update: u64,
    pub animation_token: String,
    pub animation_phase_f32_bits: u32,
    pub animation_speed_f32_bits: u32,
    pub loop_ordinal_u32: u32,
    pub transition_from_state: Option<String>,
    pub transition_phase_f32_bits: u32,
    pub chance_resolution_u32: u32,
    pub world_transform_4x4_f32_bits: [u32; 16],
    pub visibility_flags_u32: u32,
    pub selection_flags_u32: u32,
    pub damage_state_u32: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntitySubmission {
    pub pass_ordinal: u32,
    pub draw_order: u32,
    #[serde(with = "decimal_u64")]
    pub entity_instance_id: u64,
    pub resolved_entity_sha256: String,
    pub content_identity: EntityContentIdentity,
    pub shader_program: ShaderProgram,
    pub ordered_texture_bindings: Vec<TextureBinding>,
    pub pipeline: PipelineDesc,
    pub geometry: GeometryBinding,
    pub ordered_constant_buffers: Vec<ConstantBufferBinding>,
    pub output_state: RenderOutputState,
    pub world_transform_4x4_f32_bits: [u32; 16],
    pub state_flags: u32,
    pub skinning: Skinning,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntityContentIdentity {
    pub resolved_entity_asset: UsedContentKey,
    pub mesh: UsedContentKey,
    pub native_selection: EntityNativeSelection,
    /// SHA-256 of the canonical serde JSON bytes of `native_selection`.
    pub native_selection_sha256: String,
}

/// Raw selector-native tuple consumed by 0x1422644c0, in the exact declared
/// evidence order. None of these values are semantic LOD, submesh, material,
/// damage, or selection labels.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntityNativeSelection {
    /// Exact case-sensitive named-factory token observed at 0x142278710.
    pub entity_registration_token: String,
    pub entity_group_index: u32,
    pub mesh_draw_list_index: u32,
    pub entity_variant_u8: u8,
    pub mesh_draw_variant_u8: u8,
    pub pass_selector_i32: i32,
    pub alternate_list_u8: u8,
    pub bind_renderer_target_u8: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Skinning {
    NotSkinned,
    Palette {
        palette_sha256: String,
        palette_3200_bytes: Vec<u8>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectVisualProducer {
    #[serde(with = "decimal_u64")]
    pub sequence: u64,
    #[serde(with = "decimal_u64")]
    pub update_index: u64,
    pub producer_kind: DirectVisualKind,
    #[serde(with = "decimal_u64")]
    pub producer_instance_id: u64,
    #[serde(with = "decimal_u64")]
    pub entity_instance_id: u64,
    pub effect_instance_id: Option<String>,
    pub operation: DirectVisualOperation,
    pub entity_token: String,
    pub resolved_entity_source_sha256: String,
    pub semantic_state: String,
    pub authored_state_index: u32,
    pub authored_event_index: u32,
    #[serde(with = "decimal_u64")]
    pub state_enter_sequence: u64,
    pub live_phase_f32_bits: u32,
    pub world_transform_4x4_f32_bits: [u32; 16],
    pub node_or_locator_token: Option<String>,
    pub visibility_flags: u32,
    pub direct_object_list_ordinal: u32,
    pub direct_object_count: u32,
    pub rng_counter: u32,
    pub rng_seed: u32,
    pub effect_expiry_update: Option<String>,
    pub submitted_outputs: Vec<RenderOutputRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LightLifecycleEvent {
    #[serde(with = "decimal_u64")]
    pub sequence: u64,
    #[serde(with = "decimal_u64")]
    pub update_index: u64,
    #[serde(with = "decimal_u64")]
    pub light_instance_id: u64,
    pub operation: LightOperation,
    pub producer_instance_id: String,
    pub authored_state_index_u32: u32,
    pub authored_event_index_u32: u32,
    pub authored_payload_sha256: String,
    pub light_resolution: LightResolution,
    pub light_token: Option<String>,
    pub resolved_definition_sha256: Option<String>,
    pub parent_anchor: ParentAnchor,
    #[serde(with = "decimal_u64")]
    pub actual_trigger_tick: u64,
    pub animation_phase_f32_bits: u32,
    pub active_flags_u32: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LightOperation {
    Spawn,
    Activate,
    Deactivate,
    Destroy,
    Reparent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LightResolution {
    ResolvedDefinition,
    ExplicitNull,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DirectVisualKind {
    StrategicAirScene,
    NavalCombatScene,
    TradeRouteConvoy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DirectVisualOperation {
    Create,
    Show,
    Hide,
    StateChange,
    EffectSpawn,
    EffectExpire,
    Destroy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TiledLightSubmission {
    #[serde(with = "decimal_u64")]
    pub update_index: u64,
    #[serde(with = "decimal_u64")]
    pub writer_index: u64,
    #[serde(with = "decimal_u64")]
    pub first_sequence: u64,
    #[serde(with = "decimal_u64")]
    pub last_sequence: u64,
    pub grid_start_inv_cell_size_float4_bits: [u32; 4],
    pub light_index_map_64x64_bgra8: Vec<u8>,
    pub light_index_map_sha256: String,
    pub light_data_map_128x1_rgba32f: Vec<u8>,
    pub light_data_map_sha256: String,
    pub ordered_slot_semantic_joins: Vec<LightSlotJoin>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LightSlotJoin {
    pub light_slot: u8,
    #[serde(with = "decimal_u64")]
    pub light_instance_id: u64,
    pub light_token: String,
    pub resolved_definition_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeatherMembership {
    #[serde(with = "decimal_u64")]
    pub sequence: u64,
    #[serde(with = "decimal_u64")]
    pub update_index: u64,
    pub strategic_region_id: u32,
    pub rain_light_active: bool,
    pub rain_heavy_active: bool,
    pub snow_active: bool,
    pub blizzard_active: bool,
    pub sandstorm_active: bool,
    pub arctic_water_active: bool,
    pub region_value_i64_fixed_1e5: i64,
    pub transition_expiry_game_hour_i32: i32,
    pub airborne_runtime_code: u8,
    pub weather_effect_token: Option<String>,
    pub ordered_positions: Vec<WeatherPosition>,
    pub ordered_weather_instances: Vec<WeatherInstance>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenderCheckpoint {
    #[serde(with = "decimal_u64")]
    pub last_sequence: u64,
    #[serde(with = "decimal_u64")]
    pub last_update_index: u64,
    pub last_submit_index: Option<String>,
    pub next_stable_ids: BTreeMap<String, String>,
    pub active_effect_lifecycle: Vec<EffectLifecycleEvent>,
    pub active_entity_presentation: Vec<EntityPresentationEvent>,
    pub active_direct_visual_producers: Vec<DirectVisualProducer>,
    pub active_light_lifecycle: Vec<LightLifecycleEvent>,
    pub latest_tiled_light_payload: Option<TiledLightSubmission>,
    pub weather_state_and_ordered_membership: Vec<WeatherMembership>,
    pub active_effect_set_sha256: String,
    pub ordered_particle_packets_sha256: String,
    pub ordered_entity_submissions_sha256: String,
    pub ordered_direct_visual_producers_sha256: String,
    pub ordered_light_submissions_sha256: String,
    pub weather_membership_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeatherInstance {
    #[serde(with = "decimal_u64")]
    pub effect_instance_id: u64,
    pub position_ordinal: u16,
    pub class: WeatherPositionClass,
    pub resolved_variant_token: String,
    pub submitted_outputs: Vec<RenderOutputRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RenderOutputRef {
    Particle {
        #[serde(with = "decimal_u64")]
        effect_instance_id: u64,
        bucket: u32,
        packet_ordinal: u32,
        draw_order: u32,
    },
    Entity {
        pass_ordinal: u32,
        #[serde(with = "decimal_u64")]
        entity_instance_id: u64,
        draw_order: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeatherPosition {
    pub position_ordinal: u16,
    pub class: WeatherPositionClass,
    pub resolution: WeatherResolution,
    pub selected_authored_effect_index: Option<u16>,
    pub primary_entity_instance_id: Option<String>,
    pub fading_entity_instance_id: Option<String>,
    pub ordered_effect_instance_ids: Vec<String>,
    pub resolved_variant_token: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WeatherPositionClass {
    Big,
    Small,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WeatherResolution {
    Instantiated,
    ExplicitEmpty,
}

/// Optional exact texture upload.  This is deliberately byte-native: gameplay
/// ownership or diplomatic state is never accepted as a substitute.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PoliticalTexturePacket {
    pub kind: PoliticalTextureKind,
    /// Exact captured CB/SRV/RT write identity. `Primary` is inadmissible with
    /// an empty/generic target because the retained pdxmap pass has no named
    /// primary-colour SRV.
    pub target_binding_identity: String,
    pub upload_order: u32,
    pub width: u32,
    pub height: u32,
    pub row_pitch: u32,
    pub format: PoliticalTextureFormat,
    pub bytes: Vec<u8>,
    pub sha256: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PoliticalTextureKind {
    Primary,
    Secondary,
    GradientBorderChannel1,
    GradientBorderChannel2,
    GradientBorderChannel3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PoliticalTextureFormat {
    #[serde(rename = "DXGI_FORMAT_B8G8R8A8_UNORM_SRGB")]
    B8g8r8a8UnormSrgb,
}

impl ProductionRenderFrame {
    pub fn decode_json(bytes: &[u8]) -> Result<Self, String> {
        let value: serde_json::Value = serde_json::from_slice(bytes)
            .map_err(|error| format!("production render JSON: {error}"))?;
        let schema = value
            .get("schema")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| "production render JSON has no string schema".to_string())?;
        if schema == INCOMPATIBLE_ENTITY_SELECTOR_SCHEMA {
            return Err(format!(
                "production render schema {schema} is incompatible with {SCHEMA}: exact conversion is unavailable because the older carrier does not preserve selector-native entity and particle authority"
            ));
        }
        if schema != SCHEMA {
            return Err(format!(
                "unsupported production render schema {schema}; expected {SCHEMA}"
            ));
        }
        let frame: Self = serde_json::from_slice(bytes)
            .map_err(|error| format!("production render JSON: {error}"))?;
        frame.validate()?;
        Ok(frame)
    }

    pub fn encode_json(&self) -> Result<Vec<u8>, String> {
        self.validate()?;
        serde_json::to_vec(self).map_err(|error| error.to_string())
    }

    pub fn submit(&self, submit_index: u64) -> Option<&RenderSubmit> {
        self.submits
            .iter()
            .find(|row| row.submit_index == submit_index)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema != SCHEMA {
            return Err(format!(
                "unsupported production render schema {}",
                self.schema
            ));
        }
        check_sha256(&self.authority.effective_vfs_manifest_sha256, None)?;
        check_sha256(&self.authority.effect_catalog_sha256, None)?;
        check_sha256(&self.authority.entity_animation_catalog_sha256, None)?;
        check_sha256(&self.authority.hook_set_receipt_sha256, None)?;
        check_sha256(&self.authority.capture_receipt_sha256, None)?;
        check_sha256(&self.authority.launch_epoch_sha256, None)?;
        if self.authority.process_id == 0
            || self.authority.process_start_nonce == 0
            || self.authority.observer_installed_qpc == 0
            || self.authority.d3d_device_created_qpc == 0
            || self.authority.observer_installed_qpc > self.authority.d3d_device_created_qpc
            || self.authority.first_creation_sequence == 0
            || self.authority.hook_epoch_nonce == 0
        {
            return Err(
                "production render authority lacks an authenticated pre-device capture epoch"
                    .into(),
            );
        }
        let expected_launch_epoch_sha256 = compute_render_launch_epoch_sha256(
            self.authority.process_id,
            self.authority.process_start_nonce,
            self.authority.observer_installed_qpc,
            self.authority.d3d_device_created_qpc,
            self.authority.first_creation_sequence,
            self.authority.hook_epoch_nonce,
            &self.authority.hook_set_receipt_sha256,
            &self.authority.capture_receipt_sha256,
        );
        if self.authority.launch_epoch_sha256 != expected_launch_epoch_sha256 {
            return Err("production render launch-epoch SHA-256 mismatch".into());
        }
        if self.first_sequence > self.last_sequence {
            return Err("production render sequence range is inverted".into());
        }
        if self.submits.len() != 1 {
            return Err("production render must retain exactly one selected Present".into());
        }
        let selected = &self.selected_present;
        let submit = &self.submits[0];
        if selected.candidate_count == 0
            || selected.selected_ordinal.checked_add(1) != Some(selected.candidate_count)
            || selected.selected_candidate_id != submit.submit_index
            || compact_update_index_from_raw(selected.selected_game_hour_raw)
                != Some(self.update_index)
            || selected.selected_qpc_ticks != submit.presentation_qpc_ticks
            || selected.selection_cutoff_qpc_ticks == 0
            || selected.selected_qpc_ticks > selected.selection_cutoff_qpc_ticks
        {
            return Err(
                "selected-Present receipt differs from the sole retained submit/hour/cutoff".into(),
            );
        }
        check_sha256(&selected.candidate_set_sha256, None)?;
        let selected_candidate_id = selected.selected_candidate_id.to_string();
        if self.display_submit_index.as_deref() != Some(&selected_candidate_id)
            || self.checkpoint.last_submit_index.as_deref() != Some(&selected_candidate_id)
        {
            return Err("display/checkpoint submit identity differs from selected Present".into());
        }
        let mut last_submit = None;
        let mut last_global = None;
        let mut last_qpc = None;
        if self.presentation_qpc_frequency_hz == 0 {
            return Err("presentation QPC frequency is zero".into());
        }
        for submit in &self.submits {
            if last_submit.is_some_and(|v| submit.submit_index <= v) {
                return Err("submit_index is not strictly increasing".into());
            }
            if last_global.is_some_and(|v| submit.global_order <= v) {
                return Err("global_order is not strictly increasing".into());
            }
            if last_qpc.is_some_and(|v| submit.presentation_qpc_ticks <= v) {
                return Err("presentation QPC ticks are not strictly increasing".into());
            }
            last_submit = Some(submit.submit_index);
            last_global = Some(submit.global_order);
            last_qpc = Some(submit.presentation_qpc_ticks);
            submit.validate(self.first_sequence, self.last_sequence)?;
        }
        if let Some(m02) = &self.m02_combat {
            m02.validate()?;
            if m02.update_index != self.update_index
                || m02.first_sequence != self.first_sequence
                || m02.last_sequence != self.last_sequence
            {
                return Err("M02 update/sequence frontier does not match production frame".into());
            }
            for indicator in &m02.indicators {
                for content in &indicator.used_content {
                    if content.effective_vfs_manifest_sha256
                        != self.authority.effective_vfs_manifest_sha256
                    {
                        return Err("M02 content authority differs from production frame".into());
                    }
                }
                for draw in &indicator.draws {
                    validate_static_draw_vfs_authority(
                        &draw.draw,
                        &self.authority.effective_vfs_manifest_sha256,
                        "M02",
                    )?;
                }
            }
            let display_submit = self
                .display_submit_index
                .as_deref()
                .and_then(|value| value.parse::<u64>().ok())
                .and_then(|submit_index| self.submit(submit_index))
                .ok_or("M02 carrier has no authenticated display submit")?;
            let expected = m02.ordered_draws.iter().cloned().collect::<Vec<_>>();
            let actual = display_submit
                .ordered_render_operations
                .iter()
                .filter_map(|operation| match operation {
                    RenderExecutionRef::M02 { output } => Some(output.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>();
            if actual != expected {
                return Err(
                    "display submit M02 operations do not exactly cover carrier order".into(),
                );
            }
            if self.submits.iter().any(|submit| {
                submit.submit_index != display_submit.submit_index
                    && submit
                        .ordered_render_operations
                        .iter()
                        .any(|operation| matches!(operation, RenderExecutionRef::M02 { .. }))
            }) {
                return Err("non-display submit contains an M02 overlay operation".into());
            }
        } else if self.submits.iter().any(|submit| {
            submit
                .ordered_render_operations
                .iter()
                .any(|operation| matches!(operation, RenderExecutionRef::M02 { .. }))
        }) {
            return Err("unified render order references absent M02 carrier".into());
        }
        if let Some(ui_pass) = &self.authenticated_ui_pass {
            if self.authority.recorded_input_variant.is_none() {
                return Err(
                    "selected-Present UI pass lacks authenticated recorded input variant".into(),
                );
            }
            ui_pass.validate()?;
            if ui_pass.update_index != self.update_index
                || ui_pass.first_sequence != self.first_sequence
                || ui_pass.last_sequence != self.last_sequence
            {
                return Err(
                    "selected-Present UI-pass frontier does not match production frame".into(),
                );
            }
            validate_render_target_vfs_authority(
                &ui_pass.target,
                &self.authority.effective_vfs_manifest_sha256,
                "selected-Present UI pass",
            )?;
            let m02_operations = ui_pass
                .ui_pass_operations
                .iter()
                .filter_map(|operation| match operation {
                    UiPassOperation::M02 { output, .. } => Some(output.clone()),
                    UiPassOperation::HudCommand { .. } => None,
                })
                .collect::<Vec<_>>();
            match &self.m02_combat {
                Some(m02) => {
                    if ui_pass.ui_pass_draw_count != m02.ui_pass_draw_count
                        || m02.ordered_draws.len() != 1
                        || m02_operations.len() != 1
                        || m02_operations != m02.ordered_draws
                    {
                        return Err(
                            "selected-Present UI pass does not exactly interleave the M02 carrier"
                                .into(),
                        );
                    }
                    let expected = &m02.ordered_draws[0];
                    let Some(operation) = ui_pass
                        .ui_pass_operations
                        .get(expected.draw_ordinal as usize)
                    else {
                        return Err(
                            "selected-Present UI pass M02 ordinal exceeds dynamic frontier".into(),
                        );
                    };
                    if !matches!(
                        operation,
                        UiPassOperation::M02 { ui_pass_ordinal, output }
                            if *ui_pass_ordinal == expected.draw_ordinal && output == expected
                    ) {
                        return Err(
                            "selected-Present UI pass lacks exact dynamically recorded M02 ordinal"
                                .into(),
                        );
                    }
                }
                None if !m02_operations.is_empty() => {
                    return Err("selected-Present UI pass references absent M02 carrier".into())
                }
                None => {}
            }
            if let Some(ActiveMapmodeState {
                provenance: MapmodeProvenance::EffectiveVfs { content },
                ..
            }) = &ui_pass.active_mapmode_state
            {
                if content.effective_vfs_manifest_sha256
                    != self.authority.effective_vfs_manifest_sha256
                {
                    return Err("active mapmode VFS authority differs from production frame".into());
                }
            }
            for operation in &ui_pass.ui_pass_operations {
                match operation {
                    UiPassOperation::M02 { output, .. } => {
                        let draw = self
                            .m02_combat
                            .as_ref()
                            .and_then(|m02| m02.draw(output))
                            .ok_or("selected-Present UI pass has a dangling M02 draw reference")?;
                        if draw.draw.output_state.ordered_color_targets.as_slice()
                            != [ui_pass.target.clone()]
                            || draw.draw.output_state.depth_target.is_some()
                        {
                            return Err(
                                "M02 UI draw does not join selected-Present pass target".into()
                            );
                        }
                    }
                    UiPassOperation::HudCommand { command } => {
                        if command
                            .ordered_effective_vfs_used_content
                            .iter()
                            .chain(std::iter::once(&command.identity.selected_gui_definition))
                            .any(|content| {
                                content.effective_vfs_manifest_sha256
                                    != self.authority.effective_vfs_manifest_sha256
                            })
                        {
                            return Err(
                                "UI-pass HUD content authority differs from production frame"
                                    .into(),
                            );
                        }
                        if let HudCommandExecution::ExactPipeline { draw } = &command.execution {
                            validate_static_draw_vfs_authority(
                                &draw.exact_draw,
                                &self.authority.effective_vfs_manifest_sha256,
                                "UI-pass HUD",
                            )?;
                        }
                    }
                }
            }
        }
        if let Some(m03) = &self.m03_arrows {
            m03.validate()?;
            if m03.update_index != self.update_index
                || m03.first_sequence != self.first_sequence
                || m03.last_sequence != self.last_sequence
            {
                return Err("M03 update/sequence frontier does not match production frame".into());
            }
            for content in &m03.used_content {
                if content.effective_vfs_manifest_sha256
                    != self.authority.effective_vfs_manifest_sha256
                {
                    return Err("M03 content authority differs from production frame".into());
                }
            }
            for draw in &m03.draws {
                validate_static_draw_vfs_authority(
                    &draw.draw,
                    &self.authority.effective_vfs_manifest_sha256,
                    "M03",
                )?;
            }
            let display_submit = self
                .display_submit_index
                .as_deref()
                .and_then(|value| value.parse::<u64>().ok())
                .and_then(|submit_index| self.submit(submit_index))
                .ok_or("M03 carrier has no authenticated display submit")?;
            let expected = m03.ordered_draws.iter().cloned().collect::<Vec<_>>();
            let actual = display_submit
                .ordered_render_operations
                .iter()
                .filter_map(|operation| match operation {
                    RenderExecutionRef::M03 { output } => Some(output.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>();
            if actual != expected {
                return Err(
                    "display submit M03 operations do not exactly cover carrier order".into(),
                );
            }
        } else if self.submits.iter().any(|submit| {
            submit
                .ordered_render_operations
                .iter()
                .any(|operation| matches!(operation, RenderExecutionRef::M03 { .. }))
        }) {
            return Err("unified render order references absent M03 carrier".into());
        }
        if let Some(o04) = &self.o04_cities {
            o04.validate()?;
            if o04.update_index != self.update_index
                || o04.first_sequence != self.first_sequence
                || o04.last_sequence != self.last_sequence
            {
                return Err("O04 update/sequence frontier does not match production frame".into());
            }
            let display_submit = self
                .display_submit_index
                .as_deref()
                .and_then(|value| value.parse::<u64>().ok())
                .and_then(|submit_index| self.submit(submit_index))
                .ok_or("O04 carrier has no authenticated display submit")?;
            let referenced = o04.ordered_outputs.iter().cloned().collect::<BTreeSet<_>>();
            let actual_order = display_submit
                .ordered_draws
                .iter()
                .filter(|output| referenced.contains(*output))
                .cloned()
                .collect::<Vec<_>>();
            if actual_order != o04.ordered_outputs {
                return Err("O04 output order differs from authenticated display submit".into());
            }
            for city in &o04.cities {
                if city.entity_asset.effective_vfs_manifest_sha256
                    != self.authority.effective_vfs_manifest_sha256
                    || city.mesh.effective_vfs_manifest_sha256
                        != self.authority.effective_vfs_manifest_sha256
                {
                    return Err("O04 content authority differs from production frame".into());
                }
                let entity_instance_id = match city.referenced_outputs.first() {
                    Some(RenderOutputRef::Entity {
                        entity_instance_id, ..
                    }) => *entity_instance_id,
                    _ => return Err("O04 city has no entity output identity".into()),
                };
                let complete_display_outputs = display_submit
                    .ordered_draws
                    .iter()
                    .filter(|output| {
                        matches!(
                            output,
                            RenderOutputRef::Entity {
                                entity_instance_id: candidate,
                                ..
                            } if *candidate == entity_instance_id
                        )
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                if complete_display_outputs != city.referenced_outputs {
                    return Err(
                        "O04 city output refs are not the complete display-submit entity cover"
                            .into(),
                    );
                }
                for output in &city.referenced_outputs {
                    let entity = match output {
                        RenderOutputRef::Entity {
                            pass_ordinal,
                            entity_instance_id,
                            draw_order,
                        } => display_submit.entities.iter().find(|entity| {
                            entity.pass_ordinal == *pass_ordinal
                                && entity.entity_instance_id == *entity_instance_id
                                && entity.draw_order == *draw_order
                        }),
                        RenderOutputRef::Particle { .. } => None,
                    }
                    .ok_or("O04 city references an absent entity submission")?;
                    if entity.content_identity.resolved_entity_asset != city.entity_asset
                        || entity.content_identity.mesh != city.mesh
                        || entity.world_transform_4x4_f32_bits != city.world_transform_4x4_f32_bits
                    {
                        return Err(
                            "O04 semantic identity/transform differs from entity submission".into(),
                        );
                    }
                }
            }
        }
        match (&self.display_submit_index, self.submits.is_empty()) {
            (None, true) => {}
            (Some(text), false) => {
                if text.is_empty() || (text.len() > 1 && text.starts_with('0')) {
                    return Err("display_submit_index is not canonical decimal".into());
                }
                let id = text
                    .parse::<u64>()
                    .map_err(|_| "display_submit_index is invalid")?;
                if self.submit(id).is_none() {
                    return Err("display_submit_index does not name a retained submit".into());
                }
            }
            _ => return Err("display submit presence does not match submit population".into()),
        }
        if self.checkpoint.last_sequence != self.last_sequence
            || self.checkpoint.last_update_index != self.update_index
        {
            return Err("checkpoint sequence/update frontier does not match frame".into());
        }
        match (&self.checkpoint.last_submit_index, self.submits.last()) {
            (None, None) => {}
            (Some(text), Some(last)) => {
                check_decimal_u64(text, "checkpoint last_submit_index")?;
                if text.parse::<u64>().ok() != Some(last.submit_index) {
                    return Err("checkpoint last_submit_index does not match final submit".into());
                }
            }
            _ => return Err("checkpoint submit frontier does not match frame".into()),
        }
        for hash in [
            &self.checkpoint.active_effect_set_sha256,
            &self.checkpoint.ordered_particle_packets_sha256,
            &self.checkpoint.ordered_entity_submissions_sha256,
            &self.checkpoint.ordered_direct_visual_producers_sha256,
            &self.checkpoint.ordered_light_submissions_sha256,
            &self.checkpoint.weather_membership_sha256,
        ] {
            check_sha256(hash, None)?;
        }
        for (kind, id) in &self.checkpoint.next_stable_ids {
            if kind.trim().is_empty() {
                return Err("checkpoint stable-id kind is empty".into());
            }
            check_decimal_u64(id, "checkpoint next stable id")?;
        }
        Ok(())
    }
}

impl RenderSubmit {
    fn validate(&self, first_sequence: u64, last_sequence: u64) -> Result<(), String> {
        check_sha256(&self.particle_packet_set_sha256, None)?;
        validate_constant_buffers(&self.ordered_scene_constant_buffers)?;
        validate_static_map_programs(
            &self.ordered_static_map_programs,
            self.ordered_scene_constant_buffers.as_slice(),
        )?;
        let mut particle_keys = BTreeSet::new();
        let mut particle_outputs = BTreeSet::new();
        for packet in &self.particles {
            if !particle_keys.insert((packet.bucket, packet.packet_ordinal, packet.draw_order)) {
                return Err("duplicate particle packet order key".into());
            }
            if packet.canonical_packet_112_bytes.len() != 112 {
                return Err("particle canonical packet is not 112 bytes".into());
            }
            if packet.canonical_packet_112_bytes[0x48..0x50] != [0; 8] {
                return Err("particle canonical packet retained process pointer".into());
            }
            check_sha256(
                &packet.canonical_packet_sha256,
                Some(&packet.canonical_packet_112_bytes),
            )?;
            let canonical = &packet.canonical_packet_112_bytes;
            for (word, bits) in canonical[..0x40]
                .chunks_exact(4)
                .zip(packet.packet_matrix_4x4_f32_bits)
            {
                if word != bits.to_le_bytes() {
                    return Err(
                        "particle canonical packet matrix disagrees with typed matrix".into(),
                    );
                }
            }
            for (offset, bits, field) in [
                (0x40, packet.live_count, "live count"),
                (0x50, packet.record_stride, "record stride"),
                (0x54, packet.max_count, "max count"),
                (0x58, packet.elapsed_f32_bits, "elapsed bits"),
                (0x60, packet.packet_ordinal, "packet ordinal"),
            ] {
                if canonical[offset..offset + 4] != bits.to_le_bytes() {
                    return Err(format!(
                        "particle canonical packet disagrees with typed {field}"
                    ));
                }
            }
            if canonical[0x5c..0x5e] != packet.flags_u16.to_le_bytes() {
                return Err("particle canonical packet disagrees with typed flags".into());
            }
            if packet.record_stride != 48 {
                return Err("particle record stride is not 48".into());
            }
            let expected = usize::try_from(packet.live_count)
                .ok()
                .and_then(|count| count.checked_mul(48))
                .ok_or_else(|| "particle instance byte count overflow".to_string())?;
            if packet.ordered_gpu_instance_records.len() != expected
                || packet.live_count > packet.max_count
            {
                return Err("particle live count/instance bytes/max count mismatch".into());
            }
            check_sha256(
                &packet.gpu_instance_records_sha256,
                Some(&packet.ordered_gpu_instance_records),
            )?;
            check_sha256(&packet.resolved_definition_sha256, None)?;
            packet.content_identity.validate()?;
            check_sha256(&packet.content_identity_sha256, None)?;
            if packet.content_identity.computed_sha256()? != packet.content_identity_sha256 {
                return Err("particle content identity SHA-256 mismatch".into());
            }
            if packet.resolved_definition_sha256
                != packet.content_identity.definition_content.payload_sha256
            {
                return Err(
                    "particle resolved hash does not match admitted definition payload hash".into(),
                );
            }
            if packet.packet_ordinal != packet.content_identity.subsystem_selector_u32 {
                return Err("particle packet ordinal does not match subsystem selector".into());
            }
            packet.shader_program.validate()?;
            validate_texture_bindings(&packet.ordered_texture_bindings)?;
            if packet.shader_program.effect_token != packet.content_identity.engine_effect_token {
                return Err("particle engine effect token does not match program receipt".into());
            }
            let selected_texture = packet
                .ordered_texture_bindings
                .iter()
                .find(|binding| binding.content == packet.content_identity.texture_content)
                .ok_or_else(|| {
                    "particle selected texture winner is absent from exact bindings".to_string()
                })?;
            let expected_half_width = (0.5f32 / selected_texture.resource.width as f32).to_bits();
            let expected_half_height = (0.5f32 / selected_texture.resource.height as f32).to_bits();
            if packet.content_identity.atlas_half_pixel_width_f32_bits != expected_half_width
                || packet.content_identity.atlas_half_pixel_height_f32_bits != expected_half_height
            {
                return Err(
                    "particle atlas half-pixel bits do not match selected texture dimensions"
                        .into(),
                );
            }
            packet.pipeline.validate()?;
            packet.geometry.validate(&packet.pipeline, true)?;
            validate_constant_buffers(&packet.ordered_constant_buffers)?;
            packet.output_state.validate()?;
            if packet.atlas_rows == 0 || packet.atlas_columns == 0 {
                return Err("particle atlas dimensions are zero".into());
            }
            if packet.atlas_columns != packet.content_identity.atlas_columns_u32
                || packet.atlas_rows != packet.content_identity.atlas_rows_u32
            {
                return Err("particle atlas dimensions do not match authored identity".into());
            }
            particle_outputs.insert(RenderOutputRef::Particle {
                effect_instance_id: packet.effect_instance_id,
                bucket: packet.bucket,
                packet_ordinal: packet.packet_ordinal,
                draw_order: packet.draw_order,
            });
        }

        for event in &self.effect_lifecycle_events {
            check_sequence(
                event.sequence,
                first_sequence,
                last_sequence,
                "effect lifecycle",
            )?;
            check_sha256(&event.resolved_definition_sha256, None)?;
        }

        for event in &self.entity_presentations {
            check_sequence(
                event.sequence,
                first_sequence,
                last_sequence,
                "entity presentation",
            )?;
            check_sha256(&event.resolved_entity_sha256, None)?;
        }

        let mut entity_keys = BTreeSet::new();
        let mut entity_outputs = BTreeSet::new();
        for entity in &self.entities {
            if !entity_keys.insert((entity.pass_ordinal, entity.entity_instance_id)) {
                return Err("duplicate entity (pass, instance) key".into());
            }
            check_sha256(&entity.resolved_entity_sha256, None)?;
            entity.content_identity.validate()?;
            if entity.resolved_entity_sha256
                != entity.content_identity.resolved_entity_asset.payload_sha256
            {
                return Err(
                    "entity resolved hash does not match admitted asset payload hash".into(),
                );
            }
            if entity.state_flags
                != entity
                    .content_identity
                    .native_selection
                    .packed_state_flags()
            {
                return Err(
                    "entity state_flags do not equal the selector-native packed tuple".into(),
                );
            }
            entity.shader_program.validate()?;
            validate_texture_bindings(&entity.ordered_texture_bindings)?;
            entity.pipeline.validate()?;
            entity.geometry.validate(&entity.pipeline, false)?;
            validate_constant_buffers(&entity.ordered_constant_buffers)?;
            entity.output_state.validate()?;
            if let Skinning::Palette {
                palette_sha256,
                palette_3200_bytes,
            } = &entity.skinning
            {
                if palette_3200_bytes.len() != 3200 {
                    return Err("skinning palette is not 3200 bytes".into());
                }
                check_sha256(palette_sha256, Some(palette_3200_bytes))?;
            }
            entity_outputs.insert(RenderOutputRef::Entity {
                pass_ordinal: entity.pass_ordinal,
                entity_instance_id: entity.entity_instance_id,
                draw_order: entity.draw_order,
            });
        }

        let all_outputs: BTreeSet<_> = particle_outputs
            .iter()
            .chain(entity_outputs.iter())
            .cloned()
            .collect();
        let mut seen_draws = BTreeSet::new();
        for output in &self.ordered_draws {
            if !all_outputs.contains(output) || !seen_draws.insert(output.clone()) {
                return Err("ordered draw list has an unknown or duplicate output".into());
            }
        }
        if seen_draws != all_outputs {
            return Err(
                "ordered draw list does not cover every particle/entity draw exactly once".into(),
            );
        }
        validate_unified_render_operations(self)?;

        for direct in &self.direct_visuals {
            if direct.sequence < first_sequence || direct.sequence > last_sequence {
                return Err("direct visual sequence is outside frame range".into());
            }
            check_sha256(&direct.resolved_entity_source_sha256, None)?;
            if let Some(id) = &direct.effect_instance_id {
                check_decimal_u64(id, "direct visual effect_instance_id")?;
            }
            if let Some(expiry) = &direct.effect_expiry_update {
                check_decimal_u64(expiry, "direct visual effect_expiry_update")?;
            }
            validate_output_refs(
                &direct.submitted_outputs,
                &particle_outputs,
                &entity_outputs,
                "direct visual",
            )?;
        }
        for light in &self.light_lifecycle_events {
            check_sequence(
                light.sequence,
                first_sequence,
                last_sequence,
                "light lifecycle",
            )?;
            check_sha256(&light.authored_payload_sha256, None)?;
            match (light.light_resolution, &light.resolved_definition_sha256) {
                (LightResolution::ResolvedDefinition, Some(hash)) => check_sha256(hash, None)?,
                (LightResolution::ExplicitNull, None) => {}
                _ => return Err("light resolution/hash nullability mismatch".into()),
            }
        }
        for light in &self.tiled_lights {
            if light.first_sequence > light.last_sequence
                || light.first_sequence < first_sequence
                || light.last_sequence > last_sequence
            {
                return Err("tiled-light sequence range is invalid".into());
            }
            if light.light_index_map_64x64_bgra8.len() != 16_384
                || light.light_data_map_128x1_rgba32f.len() != 2_048
            {
                return Err("tiled-light payload dimensions are incomplete".into());
            }
            check_sha256(
                &light.light_index_map_sha256,
                Some(&light.light_index_map_64x64_bgra8),
            )?;
            check_sha256(
                &light.light_data_map_sha256,
                Some(&light.light_data_map_128x1_rgba32f),
            )?;
            let mut slots = BTreeSet::new();
            for join in &light.ordered_slot_semantic_joins {
                if join.light_slot >= 64 || !slots.insert(join.light_slot) {
                    return Err("tiled-light semantic slot is invalid or duplicated".into());
                }
                check_sha256(&join.resolved_definition_sha256, None)?;
            }
        }
        for weather in &self.weather_membership {
            if weather.sequence < first_sequence || weather.sequence > last_sequence {
                return Err("weather sequence is outside frame range".into());
            }
            if weather.airborne_runtime_code == 0 {
                if weather.weather_effect_token.is_some()
                    || weather.ordered_positions.iter().any(|position| {
                        position.resolution != WeatherResolution::ExplicitEmpty
                            || position.selected_authored_effect_index.is_some()
                            || position.primary_entity_instance_id.is_some()
                            || position.fading_entity_instance_id.is_some()
                            || !position.ordered_effect_instance_ids.is_empty()
                            || position.resolved_variant_token.is_some()
                    })
                {
                    return Err("code-zero weather is not explicit-empty".into());
                }
            } else if weather.weather_effect_token.is_none() {
                return Err("nonzero weather code has no token".into());
            }
            let mut instance_keys = BTreeSet::new();
            for instance in &weather.ordered_weather_instances {
                if !instance_keys.insert((instance.effect_instance_id, instance.position_ordinal)) {
                    return Err("duplicate weather instance identity/position".into());
                }
                if instance.submitted_outputs.is_empty() {
                    return Err(
                        "weather instance has no authoritative submitted output join".into(),
                    );
                }
                validate_output_refs(
                    &instance.submitted_outputs,
                    &particle_outputs,
                    &entity_outputs,
                    "weather instance",
                )?;
            }
        }
        let mut texture_orders = BTreeSet::new();
        for texture in &self.political_textures {
            if !texture_orders.insert(texture.upload_order) {
                return Err("duplicate political texture upload order".into());
            }
            let expected = usize::try_from(texture.row_pitch)
                .ok()
                .and_then(|pitch| pitch.checked_mul(texture.height as usize))
                .ok_or_else(|| "political texture byte count overflow".to_string())?;
            if texture.target_binding_identity.trim().is_empty() {
                return Err("political texture has no exact target binding identity".into());
            }
            if texture.width == 0 || texture.height == 0 || texture.bytes.len() != expected {
                return Err("political texture descriptor/byte count mismatch".into());
            }
            check_sha256(&texture.sha256, Some(&texture.bytes))?;
        }
        Ok(())
    }
}

impl StaticMapPassProgram {
    /// Seal the nested draw, operation, and whole-program hashes in canonical
    /// serde field order. Producers call this only after every observed field
    /// has been populated; playback independently recomputes every hash.
    pub fn seal_payload_hashes(&mut self) -> Result<(), String> {
        for operation in &mut self.ordered_operations {
            if let StaticMapCommand::Draw { draw } = &mut operation.command {
                draw.payload_sha256 = draw.computed_payload_sha256()?;
            }
            operation.payload_sha256 = operation.computed_payload_sha256()?;
        }
        self.payload_sha256 = self.computed_payload_sha256()?;
        Ok(())
    }

    fn computed_payload_sha256(&self) -> Result<String, String> {
        let mut canonical = self.clone();
        canonical.payload_sha256.clear();
        canonical_json_sha256(&canonical)
    }
}

impl StaticMapOperation {
    fn computed_payload_sha256(&self) -> Result<String, String> {
        let mut canonical = self.clone();
        canonical.payload_sha256.clear();
        canonical_json_sha256(&canonical)
    }
}

impl StaticMapDraw {
    fn computed_payload_sha256(&self) -> Result<String, String> {
        let mut canonical = self.clone();
        canonical.payload_sha256.clear();
        canonical_json_sha256(&canonical)
    }
}

fn canonical_json_sha256(value: &impl Serialize) -> Result<String, String> {
    let bytes = serde_json::to_vec(value).map_err(|error| error.to_string())?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn validate_static_map_programs(
    programs: &[StaticMapPassProgram],
    scene_constant_buffers: &[ConstantBufferBinding],
) -> Result<(), String> {
    if programs.is_empty() {
        return Ok(());
    }
    // A non-empty program set is tied to the exact selected-submit scene state.
    if scene_constant_buffers.is_empty() {
        return Err("static map programs have no selected-submit scene constant buffers".into());
    }

    let mut previous_program = None;
    let mut previous_operation = None;
    let mut produced_targets = BTreeMap::<PreparedRenderTarget, u32>::new();
    let mut swapchain_output_present = false;
    let mut prepared_target_present = false;
    for program in programs {
        // The token is opaque captured provenance only. It never selects a
        // renderer, pass role, asset, or ordering rule; the sealed program
        // hash authenticates it together with the exact graph and operations.
        if program.pass_token.trim().is_empty() {
            return Err("static map program has a blank opaque pass identity".into());
        }
        if previous_program.is_some_and(|previous| program.program_ordinal <= previous) {
            return Err("static map program ordinals are not strictly increasing".into());
        }
        previous_program = Some(program.program_ordinal);
        if program.ordered_operations.is_empty() {
            return Err("static map program contains no authenticated operation".into());
        }
        let mut content_keys = BTreeSet::new();
        for key in &program.ordered_content_keys {
            if key.trim().is_empty() || !content_keys.insert(key) {
                return Err("static map program content-key order is blank or duplicated".into());
            }
        }
        check_sha256(&program.payload_sha256, None)?;
        if program.computed_payload_sha256()? != program.payload_sha256 {
            return Err("static map program payload SHA-256 mismatch".into());
        }

        for operation in &program.ordered_operations {
            if previous_operation.is_some_and(|previous| operation.operation_ordinal <= previous) {
                return Err("static map operation ordinals are not globally increasing".into());
            }
            previous_operation = Some(operation.operation_ordinal);
            check_sha256(&operation.payload_sha256, None)?;
            if operation.computed_payload_sha256()? != operation.payload_sha256 {
                return Err("static map operation payload SHA-256 mismatch".into());
            }

            match &operation.command {
                StaticMapCommand::ClearColor {
                    target,
                    rgba_f32_bits,
                } => {
                    validate_static_target(target, false)?;
                    if rgba_f32_bits
                        .iter()
                        .any(|bits| !f32::from_bits(*bits).is_finite())
                    {
                        return Err("static color clear carries a non-finite component".into());
                    }
                    produced_targets
                        .insert(target.prepared_target.clone(), operation.operation_ordinal);
                }
                StaticMapCommand::ClearDepthStencil {
                    target,
                    clear_depth,
                    clear_stencil,
                    depth_f32_bits,
                    ..
                } => {
                    if !clear_depth && !clear_stencil {
                        return Err("depth/stencil clear selects neither plane".into());
                    }
                    validate_static_target(target, true)?;
                    let depth = f32::from_bits(*depth_f32_bits);
                    if *clear_depth && (!depth.is_finite() || !(0.0..=1.0).contains(&depth)) {
                        return Err("static depth clear is non-finite or outside [0,1]".into());
                    }
                    produced_targets
                        .insert(target.prepared_target.clone(), operation.operation_ordinal);
                }
                StaticMapCommand::Draw { draw } => {
                    validate_static_map_draw(draw)?;
                    if draw
                        .output_state
                        .ordered_color_targets
                        .iter()
                        .any(|target| {
                            target.prepared_target == PreparedRenderTarget::SwapchainBackBuffer
                        })
                        && draw.ordered_shader_resources.iter().any(|binding| {
                            matches!(
                                &binding.source,
                                StaticShaderResourceSource::PreparedTarget { .. }
                            )
                        })
                    {
                        prepared_target_present = true;
                    }
                    swapchain_output_present |=
                        draw.output_state
                            .ordered_color_targets
                            .iter()
                            .any(|target| {
                                target.prepared_target == PreparedRenderTarget::SwapchainBackBuffer
                            });
                    for target in draw
                        .output_state
                        .ordered_color_targets
                        .iter()
                        .chain(draw.output_state.depth_target.iter())
                    {
                        produced_targets
                            .insert(target.prepared_target.clone(), operation.operation_ordinal);
                    }
                }
            }
        }
    }
    if swapchain_output_present && !prepared_target_present {
        return Err("swapchain output has no authenticated draw from a prepared target".into());
    }
    Ok(())
}

fn validate_unified_render_operations(submit: &RenderSubmit) -> Result<(), String> {
    let static_operations = submit
        .ordered_static_map_programs
        .iter()
        .flat_map(|program| program.ordered_operations.iter())
        .map(|operation| (operation.operation_ordinal, operation))
        .collect::<BTreeMap<_, _>>();
    if !static_operations.is_empty() && submit.ordered_render_operations.is_empty() {
        return Err("static map programs have no unified render-operation order".into());
    }
    if submit.ordered_render_operations.is_empty() {
        return Ok(());
    }
    let m02_count = submit
        .ordered_render_operations
        .iter()
        .filter(|operation| matches!(operation, RenderExecutionRef::M02 { .. }))
        .count();
    let m03_count = submit
        .ordered_render_operations
        .iter()
        .filter(|operation| matches!(operation, RenderExecutionRef::M03 { .. }))
        .count();
    if submit.ordered_render_operations.len()
        != static_operations.len() + submit.ordered_draws.len() + m02_count + m03_count
    {
        return Err(
            "unified render-operation order does not cover every static/runtime operation".into(),
        );
    }

    let mut seen = BTreeSet::new();
    let mut previous_static_ordinal = None;
    let mut runtime_index = 0usize;
    let mut m02_suffix = false;
    let mut last_writer = BTreeMap::<PreparedRenderTarget, RenderExecutionRef>::new();
    for execution in &submit.ordered_render_operations {
        if !seen.insert(execution.clone()) {
            return Err("unified render-operation order contains a duplicate".into());
        }
        match execution {
            RenderExecutionRef::Static { operation_ordinal } => {
                if m02_suffix {
                    return Err("M02 UI-overlay operations are not a render-order suffix".into());
                }
                if previous_static_ordinal.is_some_and(|previous| *operation_ordinal <= previous) {
                    return Err("unified render order changes static operation order".into());
                }
                previous_static_ordinal = Some(*operation_ordinal);
                let operation = static_operations
                    .get(operation_ordinal)
                    .ok_or("unified render order references an unknown static operation")?;
                if let StaticMapCommand::Draw { draw } = &operation.command {
                    validate_static_submit_sources(draw, execution, &last_writer, submit)?;
                }
                for target in static_operation_outputs(operation) {
                    last_writer.insert(target.prepared_target.clone(), execution.clone());
                }
            }
            RenderExecutionRef::Runtime { output } => {
                if m02_suffix {
                    return Err("M02 UI-overlay operations are not a render-order suffix".into());
                }
                if submit.ordered_draws.get(runtime_index) != Some(output) {
                    return Err(
                        "unified render order changes authenticated runtime draw order".into(),
                    );
                }
                runtime_index += 1;
                let output_state = runtime_output_state(submit, output)
                    .ok_or("unified render order references an unknown runtime draw")?;
                for target in output_state
                    .ordered_color_targets
                    .iter()
                    .chain(output_state.depth_target.iter())
                {
                    last_writer.insert(target.prepared_target.clone(), execution.clone());
                }
            }
            RenderExecutionRef::M02 { .. } => {
                m02_suffix = true;
            }
            RenderExecutionRef::M03 { .. } => {
                if m02_suffix {
                    return Err("M03 map-arrow operation occurs after M02 UI-overlay suffix".into());
                }
            }
        }
    }
    if runtime_index != submit.ordered_draws.len()
        || seen
            .iter()
            .filter(|value| matches!(value, RenderExecutionRef::Static { .. }))
            .count()
            != static_operations.len()
    {
        return Err("unified render-operation order has incomplete one-to-one coverage".into());
    }
    Ok(())
}

fn validate_static_submit_sources(
    draw: &StaticMapDraw,
    current: &RenderExecutionRef,
    last_writer: &BTreeMap<PreparedRenderTarget, RenderExecutionRef>,
    submit: &RenderSubmit,
) -> Result<(), String> {
    for binding in &draw.ordered_shader_resources {
        match &binding.source {
            StaticShaderResourceSource::PreparedTarget {
                target, producer, ..
            } => {
                if producer == current || last_writer.get(target) != Some(producer) {
                    return Err(
                        "prepared-target SRV does not name its exact preceding last writer".into(),
                    );
                }
            }
            StaticShaderResourceSource::PoliticalTexture {
                texture_kind,
                target_binding_identity,
                upload_order,
                payload_sha256,
                resource,
                ..
            } => {
                let mut matches = submit.political_textures.iter().filter(|packet| {
                    packet.kind == *texture_kind
                        && packet.target_binding_identity == *target_binding_identity
                        && packet.upload_order == *upload_order
                });
                let packet = matches
                    .next()
                    .ok_or("static draw references an absent political texture packet")?;
                if matches.next().is_some()
                    || packet.sha256 != *payload_sha256
                    || resource.width != packet.width
                    || resource.height != packet.height
                    || resource.dxgi_format_u32 != 91
                    || resource.sample_count != 1
                {
                    return Err("political texture source identity/descriptor differs from selected submit payload".into());
                }
            }
            StaticShaderResourceSource::TiledLightIndex {
                writer_index,
                payload_sha256,
                resource,
                ..
            } => {
                let packet = unique_tiled_light(submit, *writer_index)?;
                if packet.light_index_map_sha256 != *payload_sha256
                    || resource.width != 64
                    || resource.height != 64
                    || resource.dxgi_format_u32 != 87
                    || resource.sample_count != 1
                {
                    return Err("tiled-light index texture identity/descriptor mismatch".into());
                }
            }
            StaticShaderResourceSource::TiledLightData {
                writer_index,
                payload_sha256,
                resource,
                ..
            } => {
                let packet = unique_tiled_light(submit, *writer_index)?;
                if packet.light_data_map_sha256 != *payload_sha256
                    || resource.width != 128
                    || resource.height != 1
                    || resource.dxgi_format_u32 != 2
                    || resource.sample_count != 1
                {
                    return Err("tiled-light data texture identity/descriptor mismatch".into());
                }
            }
            StaticShaderResourceSource::UsedContent { .. } => {}
        }
    }
    Ok(())
}

fn unique_tiled_light(
    submit: &RenderSubmit,
    writer_index: u64,
) -> Result<&TiledLightSubmission, String> {
    let mut matches = submit
        .tiled_lights
        .iter()
        .filter(|packet| packet.writer_index == writer_index);
    let packet = matches
        .next()
        .ok_or("static draw references an absent tiled-light payload")?;
    if matches.next().is_some() {
        return Err("static draw tiled-light writer identity is not unique".into());
    }
    Ok(packet)
}

fn static_operation_outputs(operation: &StaticMapOperation) -> Vec<&RenderTargetDesc> {
    match &operation.command {
        StaticMapCommand::ClearColor { target, .. }
        | StaticMapCommand::ClearDepthStencil { target, .. } => vec![target],
        StaticMapCommand::Draw { draw } => draw
            .output_state
            .ordered_color_targets
            .iter()
            .chain(draw.output_state.depth_target.iter())
            .collect(),
    }
}

fn runtime_output_state<'a>(
    submit: &'a RenderSubmit,
    output: &RenderOutputRef,
) -> Option<&'a RenderOutputState> {
    match output {
        RenderOutputRef::Particle {
            effect_instance_id,
            bucket,
            packet_ordinal,
            draw_order,
        } => submit
            .particles
            .iter()
            .find(|packet| {
                packet.effect_instance_id == *effect_instance_id
                    && packet.bucket == *bucket
                    && packet.packet_ordinal == *packet_ordinal
                    && packet.draw_order == *draw_order
            })
            .map(|packet| &packet.output_state),
        RenderOutputRef::Entity {
            pass_ordinal,
            entity_instance_id,
            draw_order,
        } => submit
            .entities
            .iter()
            .find(|entity| {
                entity.pass_ordinal == *pass_ordinal
                    && entity.entity_instance_id == *entity_instance_id
                    && entity.draw_order == *draw_order
            })
            .map(|entity| &entity.output_state),
    }
}

fn validate_static_map_draw(draw: &StaticMapDraw) -> Result<(), String> {
    if draw.draw_token.trim().is_empty() {
        return Err("static map draw token is blank".into());
    }
    check_sha256(&draw.payload_sha256, None)?;
    if draw.computed_payload_sha256()? != draw.payload_sha256 {
        return Err("static map draw payload SHA-256 mismatch".into());
    }
    draw.shader_program.validate()?;
    draw.pipeline.validate()?;
    validate_constant_buffers(&draw.ordered_constant_buffers)?;
    draw.output_state.validate()?;
    validate_static_output_state(&draw.output_state)?;

    let mut resource_slots = BTreeSet::new();
    for binding in &draw.ordered_shader_resources {
        if binding.semantic_token.trim().is_empty()
            || !resource_slots.insert((binding.stage, binding.slot))
        {
            return Err("static shader resource is unnamed or duplicates a stage/slot".into());
        }
        match &binding.source {
            StaticShaderResourceSource::UsedContent {
                content,
                resource,
                view,
            } => {
                content.validate()?;
                resource.validate()?;
                view.validate()?;
                view.validate_against(resource)?;
            }
            StaticShaderResourceSource::PreparedTarget {
                target,
                resource,
                view,
                ..
            } => {
                target.validate()?;
                validate_prepared_texture_resource(resource)?;
                view.validate()?;
                view.validate_against(resource)?;
            }
            StaticShaderResourceSource::PoliticalTexture {
                target_binding_identity,
                payload_sha256,
                resource,
                view,
                ..
            } => {
                if target_binding_identity.trim().is_empty() {
                    return Err("political texture source has no target binding identity".into());
                }
                check_sha256(payload_sha256, None)?;
                validate_submit_payload_texture_resource(resource)?;
                view.validate()?;
                view.validate_against(resource)?;
            }
            StaticShaderResourceSource::TiledLightIndex {
                payload_sha256,
                resource,
                view,
                ..
            }
            | StaticShaderResourceSource::TiledLightData {
                payload_sha256,
                resource,
                view,
                ..
            } => {
                check_sha256(payload_sha256, None)?;
                validate_submit_payload_texture_resource(resource)?;
                view.validate()?;
                view.validate_against(resource)?;
            }
        }
    }
    draw.geometry.validate(&draw.pipeline)?;
    Ok(())
}

fn validate_static_draw_vfs_authority(
    draw: &StaticMapDraw,
    expected_manifest_sha256: &str,
    owner: &str,
) -> Result<(), String> {
    let check = |content: &UsedContentKey| {
        if content.effective_vfs_manifest_sha256 == expected_manifest_sha256 {
            Ok(())
        } else {
            Err(format!(
                "{owner} draw content authority differs from production frame"
            ))
        }
    };
    check(&draw.shader_program.source)?;
    for binding in &draw.ordered_shader_resources {
        match &binding.source {
            StaticShaderResourceSource::UsedContent {
                content, resource, ..
            } => {
                check(content)?;
                validate_texture_resource_vfs_authority(resource, &check)?;
            }
            StaticShaderResourceSource::PreparedTarget { resource, .. } => {
                validate_texture_resource_vfs_authority(resource, &check)?;
            }
            StaticShaderResourceSource::PoliticalTexture { resource, .. }
            | StaticShaderResourceSource::TiledLightIndex { resource, .. }
            | StaticShaderResourceSource::TiledLightData { resource, .. } => {
                validate_texture_resource_vfs_authority(resource, &check)?;
            }
        }
    }
    if let StaticGeometrySource::Exact { geometry } = &draw.geometry {
        for binding in &geometry.ordered_vertex_buffers {
            validate_buffer_source_vfs_authority(&binding.source, &check)?;
        }
        if let Some(binding) = &geometry.index_buffer {
            validate_buffer_source_vfs_authority(&binding.source, &check)?;
        }
    }
    for target in draw
        .output_state
        .ordered_color_targets
        .iter()
        .chain(draw.output_state.depth_target.iter())
    {
        if let Some(content) = &target.persistent_content {
            check(content)?;
        }
        if let Some(exact) = &target.exact_resource {
            for initial in &exact.ordered_initial_subresources {
                if let RenderTargetInitialSource::UsedContentRange { content, .. } = &initial.source
                {
                    check(content)?;
                }
            }
        }
    }
    Ok(())
}

fn validate_render_target_vfs_authority(
    target: &RenderTargetDesc,
    expected_manifest_sha256: &str,
    owner: &str,
) -> Result<(), String> {
    let check = |content: &UsedContentKey| {
        if content.effective_vfs_manifest_sha256 == expected_manifest_sha256 {
            Ok(())
        } else {
            Err(format!(
                "{owner} render-target content authority differs from production frame"
            ))
        }
    };
    if let Some(content) = &target.persistent_content {
        check(content)?;
    }
    if let Some(exact) = &target.exact_resource {
        for initial in &exact.ordered_initial_subresources {
            if let RenderTargetInitialSource::UsedContentRange { content, .. } = &initial.source {
                check(content)?;
            }
        }
    }
    Ok(())
}

fn collect_static_draw_used_content(draw: &StaticMapDraw) -> Vec<UsedContentKey> {
    fn push_unique(out: &mut Vec<UsedContentKey>, content: &UsedContentKey) {
        if !out.contains(content) {
            out.push(content.clone());
        }
    }
    fn collect_source(out: &mut Vec<UsedContentKey>, source: &BufferSource) {
        if let BufferSource::UsedContentRange { content, .. } = source {
            push_unique(out, content);
        }
    }
    fn collect_resource(out: &mut Vec<UsedContentKey>, resource: &TextureResourceDesc) {
        for subresource in &resource.ordered_subresources {
            collect_source(out, &subresource.source);
        }
    }

    let mut out = Vec::new();
    push_unique(&mut out, &draw.shader_program.source);
    for binding in &draw.ordered_shader_resources {
        match &binding.source {
            StaticShaderResourceSource::UsedContent {
                content, resource, ..
            } => {
                push_unique(&mut out, content);
                collect_resource(&mut out, resource);
            }
            StaticShaderResourceSource::PreparedTarget { resource, .. }
            | StaticShaderResourceSource::PoliticalTexture { resource, .. }
            | StaticShaderResourceSource::TiledLightIndex { resource, .. }
            | StaticShaderResourceSource::TiledLightData { resource, .. } => {
                collect_resource(&mut out, resource);
            }
        }
    }
    if let StaticGeometrySource::Exact { geometry } = &draw.geometry {
        for binding in &geometry.ordered_vertex_buffers {
            collect_source(&mut out, &binding.source);
        }
        if let Some(binding) = &geometry.index_buffer {
            collect_source(&mut out, &binding.source);
        }
    }
    for target in draw
        .output_state
        .ordered_color_targets
        .iter()
        .chain(draw.output_state.depth_target.iter())
    {
        if let Some(content) = &target.persistent_content {
            push_unique(&mut out, content);
        }
        if let Some(exact) = &target.exact_resource {
            for initial in &exact.ordered_initial_subresources {
                if let RenderTargetInitialSource::UsedContentRange { content, .. } = &initial.source
                {
                    push_unique(&mut out, content);
                }
            }
        }
    }
    out
}

fn validate_texture_resource_vfs_authority<F>(
    resource: &TextureResourceDesc,
    check: &F,
) -> Result<(), String>
where
    F: Fn(&UsedContentKey) -> Result<(), String>,
{
    for subresource in &resource.ordered_subresources {
        validate_buffer_source_vfs_authority(&subresource.source, check)?;
    }
    Ok(())
}

fn validate_buffer_source_vfs_authority<F>(source: &BufferSource, check: &F) -> Result<(), String>
where
    F: Fn(&UsedContentKey) -> Result<(), String>,
{
    if let BufferSource::UsedContentRange { content, .. } = source {
        check(content)?;
    }
    Ok(())
}

impl StaticGeometrySource {
    fn validate(&self, pipeline: &PipelineDesc) -> Result<(), String> {
        match self {
            Self::Exact { geometry } => geometry.validate(pipeline, false),
            Self::InterpreterOutput {
                interpreter_token,
                ordered_input_content_keys,
                ordered_vertex_buffers,
                index_buffer,
                stream_output_targets,
                draw,
            } => {
                if interpreter_token.trim().is_empty() || ordered_vertex_buffers.is_empty() {
                    return Err(
                        "static geometry interpreter/token or vertex output is empty".into(),
                    );
                }
                let mut inputs = BTreeSet::new();
                for key in ordered_input_content_keys {
                    if key.trim().is_empty() || !inputs.insert(key) {
                        return Err(
                            "static geometry interpreter input key is blank or duplicated".into(),
                        );
                    }
                }
                let mut slots = BTreeSet::new();
                for vertex in ordered_vertex_buffers {
                    if vertex.stride_bytes == 0
                        || vertex.vertex_count == 0
                        || !slots.insert(vertex.input_slot)
                    {
                        return Err(
                            "interpreter vertex output has an invalid/duplicate slot or count"
                                .into(),
                        );
                    }
                    check_sha256(&vertex.payload_sha256, None)?;
                }
                let layout_slots = pipeline
                    .input_layout
                    .iter()
                    .map(|element| element.input_slot)
                    .collect::<BTreeSet<_>>();
                if layout_slots != slots {
                    return Err("interpreter vertex slots do not exactly match input layout".into());
                }
                match (index_buffer, pipeline.index_format) {
                    (None, IndexFormat::None) => {}
                    (Some(index), format)
                        if format != IndexFormat::None
                            && index.format == format
                            && index.index_count != 0 =>
                    {
                        check_sha256(&index.payload_sha256, None)?;
                    }
                    _ => {
                        return Err(
                            "interpreter index output disagrees with pipeline index format".into(),
                        )
                    }
                }
                validate_stream_output_targets(stream_output_targets)?;
                validate_draw_nonzero(draw)?;
                let indexed = matches!(
                    draw,
                    DrawCommand::DrawIndexed { .. } | DrawCommand::DrawIndexedInstanced { .. }
                );
                if indexed != index_buffer.is_some() {
                    return Err("interpreter draw/index-output binding mismatch".into());
                }
                Ok(())
            }
        }
    }
}

fn validate_stream_output_targets(targets: &[StreamOutputTargetBinding]) -> Result<(), String> {
    let mut slots = BTreeSet::new();
    let mut buffers = BTreeSet::new();
    for target in targets {
        if target.buffer_identity.trim().is_empty()
            || target.buffer_byte_width == 0
            || !slots.insert(target.slot)
            || !buffers.insert(target.buffer_identity.as_str())
            || (target.byte_offset != u32::MAX && target.byte_offset > target.buffer_byte_width)
        {
            return Err(
                "stream-output target has invalid/duplicate identity, slot, size, or offset".into(),
            );
        }
    }
    Ok(())
}

fn validate_draw_nonzero(draw: &DrawCommand) -> Result<(), String> {
    let nonzero = match draw {
        DrawCommand::Draw { vertex_count, .. } => *vertex_count != 0,
        DrawCommand::DrawIndexed { index_count, .. } => *index_count != 0,
        DrawCommand::DrawInstanced {
            vertex_count_per_instance,
            instance_count,
            ..
        } => *vertex_count_per_instance != 0 && *instance_count != 0,
        DrawCommand::DrawIndexedInstanced {
            index_count_per_instance,
            instance_count,
            ..
        } => *index_count_per_instance != 0 && *instance_count != 0,
    };
    if nonzero {
        Ok(())
    } else {
        Err("draw command has a zero vertex/index/instance count".into())
    }
}

fn validate_static_target(target: &RenderTargetDesc, depth: bool) -> Result<(), String> {
    target.prepared_target.validate()?;
    if target.width == 0 || target.height == 0 || target.sample_count == 0 {
        return Err("static render target is zero-sized".into());
    }
    if let Some(content) = &target.persistent_content {
        content.validate()?;
    }
    if depth && target.prepared_target == PreparedRenderTarget::SwapchainBackBuffer {
        return Err("swapchain backbuffer cannot be a depth target".into());
    }
    match &target.exact_resource {
        Some(exact) => exact.validate_against(target, depth)?,
        None => {
            if target.mip_level != 0 || target.persistent_content.is_some() {
                return Err(
                    "persistent/nonzero-mip render target lacks exact resource/view/init descriptor"
                        .into(),
                );
            }
        }
    }
    Ok(())
}

impl RenderTargetExactResource {
    fn validate_against(&self, selected: &RenderTargetDesc, depth: bool) -> Result<(), String> {
        self.resource.validate()?;
        if selected.mip_level >= self.resource.mip_levels
            || selected.sample_count != self.resource.sample_count
            || selected.sample_quality != self.resource.sample_quality
            || selected.width != mip_extent(self.resource.width, selected.mip_level)
            || selected.height != mip_extent(self.resource.height, selected.mip_level)
        {
            return Err(
                "selected render-target mip/extent/sample differs from exact resource".into(),
            );
        }

        let mut render_target_view = None;
        let mut depth_stencil_view = None;
        let mut shader_resource_view_count = 0usize;
        for view in &self.ordered_views {
            match view {
                RenderTargetViewDesc::RenderTarget {
                    dxgi_format_u32,
                    view,
                } => {
                    if *dxgi_format_u32 == 0
                        || self.resource.bind_flags_u32 & 0x20 == 0
                        || render_target_view.is_some()
                    {
                        return Err("exact render target repeats RTV view".into());
                    }
                    view.validate_against(&self.resource)?;
                    render_target_view = Some((*dxgi_format_u32, view.selected_mip()));
                }
                RenderTargetViewDesc::DepthStencil {
                    dxgi_format_u32,
                    flags_u32,
                    view,
                } => {
                    if *dxgi_format_u32 == 0
                        || *flags_u32 & !0x3 != 0
                        || self.resource.bind_flags_u32 & 0x40 == 0
                        || depth_stencil_view.is_some()
                    {
                        return Err("exact render target repeats DSV view".into());
                    }
                    view.validate_against(&self.resource)?;
                    depth_stencil_view = Some((*dxgi_format_u32, view.selected_mip()));
                }
                RenderTargetViewDesc::ShaderResource { view } => {
                    if self.resource.bind_flags_u32 & 0x8 == 0 {
                        return Err(
                            "exact render-target SRV lacks shader-resource bind flag".into()
                        );
                    }
                    shader_resource_view_count += 1;
                    view.validate()?;
                    view.validate_against(&self.resource.as_texture_resource_desc())?;
                }
            }
        }
        let selected_view = if depth {
            if render_target_view.is_some() || depth_stencil_view.is_none() {
                return Err("depth output exact descriptor lacks sole DSV view".into());
            }
            depth_stencil_view
        } else {
            if depth_stencil_view.is_some() || render_target_view.is_none() {
                return Err("color output exact descriptor lacks sole RTV view".into());
            }
            render_target_view
        }
        .expect("required view checked");
        if selected_view.0 == 0
            || selected_view.0 != selected.dxgi_format_u32
            || selected_view.1 != selected.mip_level
        {
            return Err(
                "selected render-target format/mip differs from exact attachment view".into(),
            );
        }
        if shader_resource_view_count > 1 {
            return Err("exact render target repeats SRV view".into());
        }

        let subresource_count = self
            .resource
            .mip_levels
            .checked_mul(self.resource.array_size)
            .ok_or("render-target subresource count overflows")?;
        let mut previous = None;
        for initial in &self.ordered_initial_subresources {
            if initial.subresource_ordinal >= subresource_count
                || previous.is_some_and(|ordinal| initial.subresource_ordinal <= ordinal)
            {
                return Err("render-target initial subresources are out of range/order".into());
            }
            previous = Some(initial.subresource_ordinal);
            initial.validate()?;
        }
        if self.resource.usage_u32 == 1
            && self.ordered_initial_subresources.len() != subresource_count as usize
        {
            return Err("immutable render target lacks complete initial subresources".into());
        }
        if let Some(persistent) = &selected.persistent_content {
            let joined = self.ordered_initial_subresources.iter().any(|initial| {
                matches!(
                    &initial.source,
                    RenderTargetInitialSource::UsedContentRange { content, .. }
                        if content == persistent
                )
            });
            if !joined {
                return Err(
                    "persistent render target is not joined to exact initial content".into(),
                );
            }
        }
        Ok(())
    }
}

impl RenderTargetResourceDesc {
    fn validate(&self) -> Result<(), String> {
        if self.width == 0
            || self.height == 0
            || self.depth == 0
            || self.mip_levels == 0
            || self.array_size == 0
            || self.dxgi_format_u32 == 0
            || self.sample_count == 0
            || self.usage_u32 > 3
        {
            return Err("exact render-target resource descriptor is zero/unknown".into());
        }
        match self.dimension {
            TextureResourceDimension::Texture1d
                if self.height != 1 || self.depth != 1 || self.sample_count != 1 =>
            {
                return Err("Texture1D render-target resource dimensions are invalid".into())
            }
            TextureResourceDimension::Texture2d | TextureResourceDimension::TextureCube
                if self.depth != 1 =>
            {
                return Err("Texture2D/cube render-target depth is not one".into())
            }
            TextureResourceDimension::Texture3d
                if self.array_size != 1 || self.sample_count != 1 =>
            {
                return Err("Texture3D render-target array/sample descriptor is invalid".into())
            }
            _ => {}
        }
        if self.dimension == TextureResourceDimension::TextureCube
            && (self.array_size < 6 || self.array_size % 6 != 0)
        {
            return Err("cube render-target array size is not a face multiple".into());
        }
        if self.sample_count > 1 && self.mip_levels != 1 {
            return Err("multisampled render target has more than one mip".into());
        }
        match self.usage_u32 {
            0 | 1 if self.cpu_access_flags_u32 != 0 => {
                return Err("default/immutable render target has CPU access flags".into())
            }
            2 if self.cpu_access_flags_u32 & 0x1_0000 == 0 => {
                return Err("dynamic render target lacks D3D11_CPU_ACCESS_WRITE".into())
            }
            3 if self.bind_flags_u32 != 0 => {
                return Err("staging render target has bind flags".into())
            }
            _ => {}
        }
        Ok(())
    }

    fn as_texture_resource_desc(&self) -> TextureResourceDesc {
        TextureResourceDesc {
            dimension: self.dimension,
            width: self.width,
            height: self.height,
            depth: self.depth,
            mip_levels: self.mip_levels,
            array_size: self.array_size,
            dxgi_format_u32: self.dxgi_format_u32,
            sample_count: self.sample_count,
            sample_quality: self.sample_quality,
            usage_u32: self.usage_u32,
            bind_flags_u32: self.bind_flags_u32,
            cpu_access_flags_u32: self.cpu_access_flags_u32,
            misc_flags_u32: self.misc_flags_u32,
            ordered_subresources: vec![],
        }
    }
}

impl RenderTargetAttachmentView {
    fn selected_mip(&self) -> u32 {
        match self {
            Self::Texture1d { mip_slice }
            | Self::Texture1dArray { mip_slice, .. }
            | Self::Texture2d { mip_slice }
            | Self::Texture2dArray { mip_slice, .. }
            | Self::Texture3d { mip_slice, .. } => *mip_slice,
            Self::Texture2dMultisampled | Self::Texture2dMultisampledArray { .. } => 0,
        }
    }

    fn validate_against(&self, resource: &RenderTargetResourceDesc) -> Result<(), String> {
        validate_attachment_view(
            self.selected_mip(),
            attachment_array_range(self),
            matches!(
                self,
                Self::Texture2dMultisampled | Self::Texture2dMultisampledArray { .. }
            ),
            resource,
            matches!(self, Self::Texture1d { .. } | Self::Texture1dArray { .. }),
            matches!(self, Self::Texture3d { .. }),
        )?;
        if let Self::Texture3d {
            first_w_slice,
            w_size,
            mip_slice,
        } = self
        {
            let depth = mip_extent(resource.depth, *mip_slice);
            if *w_size == 0
                || first_w_slice
                    .checked_add(*w_size)
                    .is_none_or(|end| end > depth)
            {
                return Err("Texture3D RTV W-slice range is invalid".into());
            }
        }
        Ok(())
    }
}

impl DepthStencilAttachmentView {
    fn selected_mip(&self) -> u32 {
        match self {
            Self::Texture1d { mip_slice }
            | Self::Texture1dArray { mip_slice, .. }
            | Self::Texture2d { mip_slice }
            | Self::Texture2dArray { mip_slice, .. } => *mip_slice,
            Self::Texture2dMultisampled | Self::Texture2dMultisampledArray { .. } => 0,
        }
    }

    fn validate_against(&self, resource: &RenderTargetResourceDesc) -> Result<(), String> {
        validate_attachment_view(
            self.selected_mip(),
            depth_attachment_array_range(self),
            matches!(
                self,
                Self::Texture2dMultisampled | Self::Texture2dMultisampledArray { .. }
            ),
            resource,
            matches!(self, Self::Texture1d { .. } | Self::Texture1dArray { .. }),
            false,
        )
    }
}

fn validate_attachment_view(
    mip: u32,
    array_range: Option<(u32, u32)>,
    multisampled: bool,
    resource: &RenderTargetResourceDesc,
    one_dimensional: bool,
    three_dimensional: bool,
) -> Result<(), String> {
    if mip >= resource.mip_levels || multisampled != (resource.sample_count > 1) {
        return Err("render-target attachment view mip/sample is invalid".into());
    }
    if one_dimensional != (resource.dimension == TextureResourceDimension::Texture1d)
        || three_dimensional != (resource.dimension == TextureResourceDimension::Texture3d)
        || (!one_dimensional
            && !three_dimensional
            && !matches!(
                resource.dimension,
                TextureResourceDimension::Texture2d | TextureResourceDimension::TextureCube
            ))
    {
        return Err("render-target attachment view dimension differs from resource".into());
    }
    if let Some((first, count)) = array_range {
        if count == 0
            || first
                .checked_add(count)
                .is_none_or(|end| end > resource.array_size)
        {
            return Err("render-target attachment array range is invalid".into());
        }
    } else if resource.array_size != 1 && !three_dimensional {
        return Err("array render target uses a non-array attachment view".into());
    }
    Ok(())
}

fn attachment_array_range(view: &RenderTargetAttachmentView) -> Option<(u32, u32)> {
    match view {
        RenderTargetAttachmentView::Texture1dArray {
            first_array_slice,
            array_size,
            ..
        }
        | RenderTargetAttachmentView::Texture2dArray {
            first_array_slice,
            array_size,
            ..
        }
        | RenderTargetAttachmentView::Texture2dMultisampledArray {
            first_array_slice,
            array_size,
        } => Some((*first_array_slice, *array_size)),
        _ => None,
    }
}

fn depth_attachment_array_range(view: &DepthStencilAttachmentView) -> Option<(u32, u32)> {
    match view {
        DepthStencilAttachmentView::Texture1dArray {
            first_array_slice,
            array_size,
            ..
        }
        | DepthStencilAttachmentView::Texture2dArray {
            first_array_slice,
            array_size,
            ..
        }
        | DepthStencilAttachmentView::Texture2dMultisampledArray {
            first_array_slice,
            array_size,
        } => Some((*first_array_slice, *array_size)),
        _ => None,
    }
}

impl RenderTargetInitialSubresource {
    fn validate(&self) -> Result<(), String> {
        if self.row_pitch == 0 || self.slice_pitch < self.row_pitch {
            return Err("render-target initial subresource pitch is invalid".into());
        }
        match &self.source {
            RenderTargetInitialSource::Inline { bytes, sha256 } => {
                if bytes.len() < self.slice_pitch as usize {
                    return Err(
                        "render-target inline initialization is shorter than slice pitch".into(),
                    );
                }
                check_sha256(sha256, Some(bytes))
            }
            RenderTargetInitialSource::UsedContentRange {
                content,
                source_byte_offset,
                byte_count,
                range_sha256,
            } => {
                content.validate()?;
                check_sha256(range_sha256, None)?;
                if *byte_count < self.slice_pitch as u64
                    || source_byte_offset
                        .checked_add(*byte_count)
                        .is_none_or(|end| end > content.payload_bytes)
                {
                    return Err("render-target initial content range is invalid".into());
                }
                Ok(())
            }
        }
    }
}

fn mip_extent(base: u32, mip: u32) -> u32 {
    base.checked_shr(mip).unwrap_or(0).max(1)
}

fn validate_static_output_state(output: &RenderOutputState) -> Result<(), String> {
    let viewport = &output.viewport;
    let values = [
        viewport.top_left_x_f32_bits,
        viewport.top_left_y_f32_bits,
        viewport.width_f32_bits,
        viewport.height_f32_bits,
        viewport.min_depth_f32_bits,
        viewport.max_depth_f32_bits,
    ]
    .map(f32::from_bits);
    if values.iter().any(|value| !value.is_finite())
        || values[2] <= 0.0
        || values[3] <= 0.0
        || values[4] < 0.0
        || values[5] > 1.0
        || values[4] > values[5]
    {
        return Err("static viewport is non-finite, empty, or has an invalid depth range".into());
    }
    Ok(())
}

fn validate_prepared_texture_resource(resource: &TextureResourceDesc) -> Result<(), String> {
    if resource.dimension != TextureResourceDimension::Texture2d
        || resource.width == 0
        || resource.height == 0
        || resource.depth != 1
        || resource.mip_levels == 0
        || resource.array_size != 1
        || resource.sample_count == 0
        || !resource.ordered_subresources.is_empty()
    {
        return Err("prepared render-graph texture descriptor is invalid or carries invented initialization bytes".into());
    }
    Ok(())
}

fn validate_submit_payload_texture_resource(resource: &TextureResourceDesc) -> Result<(), String> {
    validate_prepared_texture_resource(resource)?;
    if resource.sample_count != 1 {
        return Err("submit-carried dynamic texture is unexpectedly multisampled".into());
    }
    Ok(())
}

impl ShaderProgram {
    fn validate(&self) -> Result<(), String> {
        self.source.validate()?;
        if self.effect_token.trim().is_empty()
            || self.vertex_entry_token.trim().is_empty()
            || self.pixel_entry_token.trim().is_empty()
            || self
                .ordered_defines
                .iter()
                .any(|define| define.name.trim().is_empty())
        {
            return Err("shader program has a blank token/define".into());
        }
        self.compiled_vertex_shader.validate("vertex")?;
        self.compiled_pixel_shader.validate("pixel")?;
        for (name, stage) in [
            ("geometry", self.geometry_shader.as_ref()),
            ("hull", self.hull_shader.as_ref()),
            ("domain", self.domain_shader.as_ref()),
            ("compute", self.compute_shader.as_ref()),
        ] {
            if let Some(stage) = stage {
                if stage.entry_token.trim().is_empty() {
                    return Err(format!("compiled {name} shader has a blank entry token"));
                }
                stage.compiled.validate(name)?;
            }
        }
        Ok(())
    }
}

impl CompiledShaderPayload {
    fn validate(&self, stage: &str) -> Result<(), String> {
        if self.compiler_identity.trim().is_empty()
            || self.target_profile.trim().is_empty()
            || self.bytecode.is_empty()
        {
            return Err(format!(
                "compiled {stage} shader has blank provenance or bytecode"
            ));
        }
        check_sha256(&self.sha256, Some(&self.bytecode))
    }
}

impl EntityContentIdentity {
    fn validate(&self) -> Result<(), String> {
        self.resolved_entity_asset.validate()?;
        self.mesh.validate()?;
        self.native_selection.validate()?;
        check_sha256(&self.native_selection_sha256, None)?;
        if self.native_selection.computed_sha256()? != self.native_selection_sha256 {
            return Err("entity selector-native identity SHA-256 mismatch".into());
        }
        Ok(())
    }
}

impl ParticleContentIdentity {
    pub fn computed_sha256(&self) -> Result<String, String> {
        canonical_json_sha256(self)
    }

    fn validate(&self) -> Result<(), String> {
        self.definition_content.validate()?;
        self.texture_content.validate()?;
        if self.authored_shader_token.trim().is_empty()
            || self.engine_effect_token.trim().is_empty()
        {
            return Err("particle content identity has a blank shader/effect token".into());
        }
        if self.atlas_columns_u32 == 0 || self.atlas_rows_u32 == 0 {
            return Err("particle content identity has zero authored atlas dimensions".into());
        }
        Ok(())
    }
}

impl EntityNativeSelection {
    pub fn computed_sha256(&self) -> Result<String, String> {
        canonical_json_sha256(self)
    }

    pub fn packed_state_flags(&self) -> u32 {
        u32::from(self.entity_variant_u8)
            | (u32::from(self.mesh_draw_variant_u8) << 8)
            | (u32::from(self.bind_renderer_target_u8) << 16)
            | (u32::from(self.alternate_list_u8) << 24)
    }

    fn validate(&self) -> Result<(), String> {
        if self.entity_registration_token.trim().is_empty() {
            return Err("entity selector-native identity has a blank registration token".into());
        }
        Ok(())
    }
}

impl PipelineDesc {
    fn validate(&self) -> Result<(), String> {
        if self.blend.write_mask_rgba_u8 & !0x0f != 0
            || self.blend.additional_targets.len() > 7
            || (!self.blend.independent_blend_enabled && !self.blend.additional_targets.is_empty())
            || self
                .blend
                .additional_targets
                .iter()
                .any(|target| target.write_mask_rgba_u8 & !0x0f != 0)
            || self
                .blend
                .blend_factor_rgba_f32_bits
                .iter()
                .any(|bits| !f32::from_bits(*bits).is_finite())
        {
            return Err("pipeline blend write mask has unknown bits".into());
        }
        for bits in [
            self.rasterizer.depth_bias_clamp_f32_bits,
            self.rasterizer.slope_scaled_depth_bias_f32_bits,
        ] {
            if !f32::from_bits(bits).is_finite() {
                return Err("pipeline rasterizer bias is not finite".into());
            }
        }
        let mut sampler_slots = BTreeSet::new();
        for sampler in &self.samplers {
            if sampler.semantic_token.trim().is_empty()
                || !sampler_slots.insert((sampler.stage, sampler.slot))
                || !(1..=16).contains(&sampler.max_anisotropy)
                || !f32::from_bits(sampler.mip_lod_bias_f32_bits).is_finite()
                || !f32::from_bits(sampler.min_lod_f32_bits).is_finite()
                || !f32::from_bits(sampler.max_lod_f32_bits).is_finite()
                || f32::from_bits(sampler.min_lod_f32_bits)
                    > f32::from_bits(sampler.max_lod_f32_bits)
                || sampler
                    .border_color_rgba_f32_bits
                    .iter()
                    .any(|bits| !f32::from_bits(*bits).is_finite())
            {
                return Err("pipeline sampler slot is duplicate or unnamed".into());
            }
        }
        let mut elements = BTreeSet::new();
        for element in &self.input_layout {
            if element.semantic_token.trim().is_empty()
                || !elements.insert((
                    element.input_slot,
                    element.semantic_token.as_str(),
                    element.semantic_index,
                ))
            {
                return Err("pipeline input element is duplicate or unnamed".into());
            }
            if element.per_instance != (element.instance_step_rate != 0) {
                return Err("pipeline input element instance step-rate mismatch".into());
            }
        }
        match self.index_format {
            IndexFormat::R16Uint if self.static_indices_u16.is_empty() => {
                Err("R16 pipeline has no static indices".into())
            }
            IndexFormat::R32Uint if !self.static_indices_u16.is_empty() => {
                Err("R32 pipeline cannot carry u16 static indices".into())
            }
            IndexFormat::None if !self.static_indices_u16.is_empty() => {
                Err("non-indexed pipeline carries static indices".into())
            }
            _ => Ok(()),
        }
    }
}

impl RenderOutputState {
    fn validate(&self) -> Result<(), String> {
        if self.ordered_color_targets.is_empty() {
            return Err("render output state has no color target".into());
        }
        let mut bindings = BTreeSet::new();
        for target in &self.ordered_color_targets {
            validate_static_target(target, false)?;
            if !bindings.insert(&target.prepared_target) {
                return Err("render target descriptor is duplicated".into());
            }
        }
        if let Some(target) = &self.depth_target {
            validate_static_target(target, true)?;
            if !bindings.insert(&target.prepared_target) {
                return Err("render target descriptor is duplicated".into());
            }
        }
        if let Some(scissor) = &self.scissor {
            if scissor.right_i32 <= scissor.left_i32 || scissor.bottom_i32 <= scissor.top_i32 {
                return Err("render scissor is empty or inverted".into());
            }
        }
        Ok(())
    }
}

fn validate_texture_bindings(bindings: &[TextureBinding]) -> Result<(), String> {
    let mut slots = BTreeSet::new();
    for binding in bindings {
        if binding.semantic_token.trim().is_empty() || !slots.insert((binding.stage, binding.slot))
        {
            return Err("texture binding slot is duplicate or unnamed".into());
        }
        binding.content.validate()?;
        binding.resource.validate()?;
        binding.view.validate()?;
        binding.view.validate_against(&binding.resource)?;
    }
    Ok(())
}

impl TextureResourceDesc {
    fn validate(&self) -> Result<(), String> {
        if self.width == 0
            || self.height == 0
            || self.depth == 0
            || self.mip_levels == 0
            || self.array_size == 0
            || self.sample_count != 1
        {
            return Err("texture resource has invalid dimensions/mips/array/samples".into());
        }
        match self.dimension {
            TextureResourceDimension::Texture1d if self.height != 1 || self.depth != 1 => {
                return Err("Texture1D resource has non-unit height/depth".into())
            }
            TextureResourceDimension::Texture2d if self.depth != 1 => {
                return Err("Texture2D resource has non-unit depth".into())
            }
            TextureResourceDimension::Texture3d if self.array_size != 1 => {
                return Err("Texture3D resource has an array size other than one".into())
            }
            TextureResourceDimension::TextureCube
                if self.depth != 1 || self.array_size % 6 != 0 =>
            {
                return Err("texture cube does not contain complete six-face arrays".into())
            }
            _ => {}
        }
        let expected = match self.dimension {
            TextureResourceDimension::Texture3d => self.mip_levels,
            _ => self
                .mip_levels
                .checked_mul(self.array_size)
                .ok_or_else(|| "texture subresource count overflows".to_string())?,
        };
        if usize::try_from(expected).ok() != Some(self.ordered_subresources.len()) {
            return Err("texture resource does not carry every ordered subresource".into());
        }
        for (ordinal, subresource) in self.ordered_subresources.iter().enumerate() {
            let ordinal = u32::try_from(ordinal)
                .map_err(|_| "texture subresource ordinal overflows u32".to_string())?;
            let expected_mip = ordinal % self.mip_levels;
            let expected_array = if self.dimension == TextureResourceDimension::Texture3d {
                0
            } else {
                ordinal / self.mip_levels
            };
            if subresource.subresource_index != ordinal
                || subresource.mip_level != expected_mip
                || subresource.array_slice != expected_array
                || subresource.row_pitch == 0
                || subresource.slice_pitch == 0
            {
                return Err("texture subresource order/index/pitch mismatch".into());
            }
            subresource.source.validate(false)?;
        }
        Ok(())
    }
}

impl ShaderResourceViewDesc {
    fn validate(&self) -> Result<(), String> {
        let valid = match self {
            Self::Buffer { element_count, .. } => *element_count != 0,
            Self::Texture1d { mip_levels, .. }
            | Self::Texture2d { mip_levels, .. }
            | Self::Texture3d { mip_levels, .. }
            | Self::TextureCube { mip_levels, .. } => *mip_levels != 0,
            Self::Texture1dArray {
                mip_levels,
                array_size,
                ..
            }
            | Self::Texture2dArray {
                mip_levels,
                array_size,
                ..
            } => *mip_levels != 0 && *array_size != 0,
            Self::Texture2dMultisampled { .. } => true,
            Self::Texture2dMultisampledArray { array_size, .. } => *array_size != 0,
            Self::TextureCubeArray {
                mip_levels,
                cube_count,
                ..
            } => *mip_levels != 0 && *cube_count != 0,
        };
        if valid {
            Ok(())
        } else {
            Err("shader-resource view exposes an empty element/mip/array range".into())
        }
    }

    fn validate_against(&self, resource: &TextureResourceDesc) -> Result<(), String> {
        let range_ok = |first: u32, count: u32, total: u32| {
            count != 0 && first.checked_add(count).is_some_and(|end| end <= total)
        };
        let compatible = match self {
            Self::Buffer { .. } => false,
            Self::Texture1d {
                most_detailed_mip,
                mip_levels,
                ..
            } => {
                resource.dimension == TextureResourceDimension::Texture1d
                    && resource.array_size == 1
                    && range_ok(*most_detailed_mip, *mip_levels, resource.mip_levels)
            }
            Self::Texture1dArray {
                most_detailed_mip,
                mip_levels,
                first_array_slice,
                array_size,
                ..
            } => {
                resource.dimension == TextureResourceDimension::Texture1d
                    && range_ok(*most_detailed_mip, *mip_levels, resource.mip_levels)
                    && range_ok(*first_array_slice, *array_size, resource.array_size)
            }
            Self::Texture2d {
                most_detailed_mip,
                mip_levels,
                ..
            } => {
                resource.dimension == TextureResourceDimension::Texture2d
                    && resource.array_size == 1
                    && range_ok(*most_detailed_mip, *mip_levels, resource.mip_levels)
            }
            Self::Texture2dArray {
                most_detailed_mip,
                mip_levels,
                first_array_slice,
                array_size,
                ..
            } => {
                resource.dimension == TextureResourceDimension::Texture2d
                    && range_ok(*most_detailed_mip, *mip_levels, resource.mip_levels)
                    && range_ok(*first_array_slice, *array_size, resource.array_size)
            }
            Self::Texture2dMultisampled { .. } | Self::Texture2dMultisampledArray { .. } => false,
            Self::Texture3d {
                most_detailed_mip,
                mip_levels,
                ..
            } => {
                resource.dimension == TextureResourceDimension::Texture3d
                    && range_ok(*most_detailed_mip, *mip_levels, resource.mip_levels)
            }
            Self::TextureCube {
                most_detailed_mip,
                mip_levels,
                ..
            } => {
                resource.dimension == TextureResourceDimension::TextureCube
                    && resource.array_size == 6
                    && range_ok(*most_detailed_mip, *mip_levels, resource.mip_levels)
            }
            Self::TextureCubeArray {
                most_detailed_mip,
                mip_levels,
                first_face,
                cube_count,
                ..
            } => {
                resource.dimension == TextureResourceDimension::TextureCube
                    && range_ok(*most_detailed_mip, *mip_levels, resource.mip_levels)
                    && first_face % 6 == 0
                    && cube_count
                        .checked_mul(6)
                        .is_some_and(|faces| range_ok(*first_face, faces, resource.array_size))
            }
        };
        if compatible {
            Ok(())
        } else {
            Err("shader-resource view is incompatible with its exact texture resource".into())
        }
    }
}

impl PreparedRenderTarget {
    fn validate(&self) -> Result<(), String> {
        Ok(())
    }
}

impl GeometryBinding {
    fn validate(
        &self,
        pipeline: &PipelineDesc,
        allow_particle_records: bool,
    ) -> Result<(), String> {
        if self.ordered_vertex_buffers.is_empty() {
            return Err("geometry has no vertex buffers".into());
        }
        let mut buffer_slots = BTreeSet::new();
        for buffer in &self.ordered_vertex_buffers {
            if buffer.stride_bytes == 0 || !buffer_slots.insert(buffer.input_slot) {
                return Err("geometry vertex-buffer slot is duplicate or has zero stride".into());
            }
            buffer.source.validate(allow_particle_records)?;
            let slot_is_instance = pipeline
                .input_layout
                .iter()
                .filter(|element| element.input_slot == buffer.input_slot)
                .all(|element| element.per_instance);
            if matches!(buffer.source, BufferSource::ParticleInstanceRecords) && !slot_is_instance {
                return Err(
                    "particle instance records are bound to a per-vertex input slot".into(),
                );
            }
        }
        let layout_slots: BTreeSet<_> = pipeline
            .input_layout
            .iter()
            .map(|element| element.input_slot)
            .collect();
        if layout_slots != buffer_slots {
            return Err(
                "geometry vertex-buffer slots do not exactly match the input layout".into(),
            );
        }
        match (&self.index_buffer, pipeline.index_format) {
            (None, IndexFormat::None) => {}
            (Some(index), format) if format != IndexFormat::None && index.format == format => {
                index.source.validate(false)?;
            }
            _ => return Err("geometry index binding disagrees with pipeline index format".into()),
        }
        let mut stream_output_slots = BTreeSet::new();
        let mut stream_output_buffers = BTreeSet::new();
        for target in &self.stream_output_targets {
            if target.buffer_identity.trim().is_empty()
                || target.buffer_byte_width == 0
                || !stream_output_slots.insert(target.slot)
                || !stream_output_buffers.insert(target.buffer_identity.as_str())
                || (target.byte_offset != u32::MAX && target.byte_offset > target.buffer_byte_width)
            {
                return Err(
                    "stream-output target has invalid/duplicate identity, slot, size, or offset"
                        .into(),
                );
            }
        }
        let indexed_draw = matches!(
            self.draw,
            DrawCommand::DrawIndexed { .. } | DrawCommand::DrawIndexedInstanced { .. }
        );
        if indexed_draw != self.index_buffer.is_some() {
            return Err("draw command/index-buffer binding mismatch".into());
        }
        let nonzero = match self.draw {
            DrawCommand::Draw { vertex_count, .. } => vertex_count != 0,
            DrawCommand::DrawIndexed { index_count, .. } => index_count != 0,
            DrawCommand::DrawInstanced {
                vertex_count_per_instance,
                instance_count,
                ..
            } => vertex_count_per_instance != 0 && instance_count != 0,
            DrawCommand::DrawIndexedInstanced {
                index_count_per_instance,
                instance_count,
                ..
            } => index_count_per_instance != 0 && instance_count != 0,
        };
        if !nonzero {
            return Err("draw command has a zero vertex/index/instance count".into());
        }
        Ok(())
    }
}

impl BufferSource {
    fn validate(&self, allow_particle_records: bool) -> Result<(), String> {
        match self {
            Self::Inline { bytes, sha256 } => {
                if bytes.is_empty() {
                    return Err("inline GPU buffer source is empty".into());
                }
                check_sha256(sha256, Some(bytes))
            }
            Self::UsedContentRange {
                content,
                source_byte_offset,
                byte_count,
                range_sha256,
            } => {
                content.validate()?;
                check_sha256(range_sha256, None)?;
                let end = source_byte_offset
                    .checked_add(*byte_count)
                    .ok_or_else(|| "content buffer range overflows".to_string())?;
                if *byte_count == 0 || end > content.payload_bytes {
                    return Err("content buffer range is empty or outside its payload".into());
                }
                Ok(())
            }
            Self::ParticleInstanceRecords if allow_particle_records => Ok(()),
            Self::ParticleInstanceRecords => {
                Err("entity geometry references particle packet instance bytes".into())
            }
        }
    }
}

fn validate_constant_buffers(bindings: &[ConstantBufferBinding]) -> Result<(), String> {
    let mut slots = BTreeSet::new();
    for binding in bindings {
        if binding.stage_mask_u8 == 0
            || binding.stage_mask_u8 & !0x3f != 0
            || binding.semantic_layout.trim().is_empty()
            || !slots.insert((binding.stage_mask_u8, binding.slot))
        {
            return Err(
                "constant-buffer binding is duplicate/unnamed/has invalid stage mask".into(),
            );
        }
        let end = usize::try_from(binding.byte_offset)
            .ok()
            .and_then(|offset| {
                usize::try_from(binding.byte_count)
                    .ok()
                    .and_then(|count| offset.checked_add(count))
            })
            .ok_or_else(|| "constant-buffer byte range overflow".to_string())?;
        if binding.byte_count == 0 || end > binding.raw_bytes.len() {
            return Err("constant-buffer byte range exceeds captured bytes".into());
        }
        check_sha256(&binding.sha256, Some(&binding.raw_bytes))?;
    }
    Ok(())
}

fn validate_output_refs(
    refs: &[RenderOutputRef],
    particle_outputs: &BTreeSet<RenderOutputRef>,
    entity_outputs: &BTreeSet<RenderOutputRef>,
    owner: &str,
) -> Result<(), String> {
    let mut unique = BTreeSet::new();
    for output in refs {
        if !unique.insert(output.clone()) {
            return Err(format!("{owner} repeats a submitted output join"));
        }
        let exists = match output {
            RenderOutputRef::Particle { .. } => particle_outputs.contains(output),
            RenderOutputRef::Entity { .. } => entity_outputs.contains(output),
        };
        if !exists {
            return Err(format!("{owner} references an absent submitted output"));
        }
    }
    Ok(())
}

fn check_sha256(text: &str, bytes: Option<&[u8]>) -> Result<(), String> {
    if text.len() != 64
        || !text
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Err("SHA-256 is not 64 lowercase hexadecimal characters".into());
    }
    if let Some(bytes) = bytes {
        let actual = format!("{:x}", Sha256::digest(bytes));
        if actual != text {
            return Err("SHA-256 payload mismatch".into());
        }
    }
    Ok(())
}

fn check_decimal_u64(text: &str, field: &str) -> Result<(), String> {
    if text.is_empty() || (text.len() > 1 && text.starts_with('0')) || text.parse::<u64>().is_err()
    {
        return Err(format!("{field} is not canonical decimal u64"));
    }
    Ok(())
}

fn check_sequence(value: u64, first: u64, last: u64, field: &str) -> Result<(), String> {
    if value < first || value > last {
        return Err(format!("{field} sequence is outside frame range"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use portable_content_manifest::ContentLayerKind;

    fn hash(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }

    fn content(path: &str) -> UsedContentKey {
        UsedContentKey {
            effective_vfs_manifest_sha256: "2".repeat(64),
            normalized_game_relative_path: path.into(),
            winning_layer_ordinal: 0,
            layer_kind: ContentLayerKind::Base,
            layer_id: "base".into(),
            source_sha256: "c".repeat(64),
            payload_sha256: "c".repeat(64),
            payload_bytes: 1,
        }
    }

    fn entity_content_identity() -> EntityContentIdentity {
        let native_selection = EntityNativeSelection {
            entity_registration_token: "unit".into(),
            entity_group_index: 1,
            mesh_draw_list_index: 2,
            entity_variant_u8: 3,
            mesh_draw_variant_u8: 4,
            pass_selector_i32: -5,
            alternate_list_u8: 6,
            bind_renderer_target_u8: 7,
        };
        let native_selection_sha256 = native_selection.computed_sha256().unwrap();
        EntityContentIdentity {
            resolved_entity_asset: content("gfx/entities/unit.asset"),
            mesh: content("gfx/models/unit.mesh"),
            native_selection,
            native_selection_sha256,
        }
    }

    fn particle_content_identity(selector: u32) -> ParticleContentIdentity {
        ParticleContentIdentity {
            definition_content: content("gfx/particles/smoke.asset"),
            subsystem_selector_u32: selector,
            texture_content: content("gfx/textures/particle.dds"),
            authored_shader_token: "ParticleAdditive".into(),
            engine_effect_token: "ParticleAdditive".into(),
            atlas_columns_u32: 1,
            atlas_rows_u32: 1,
            atlas_half_pixel_width_f32_bits: 0.5f32.to_bits(),
            atlas_half_pixel_height_f32_bits: 0.5f32.to_bits(),
        }
    }

    fn exact_m02_carrier() -> (M02CombatFrame, M02CombatDrawRef) {
        let shader = shader();
        let texture = texture();
        let mut output = output_state();
        output.ordered_color_targets[0].prepared_target = PreparedRenderTarget::SwapchainBackBuffer;
        let mut draw = StaticMapDraw {
            draw_token: "m02.nav_battle_progress_small".into(),
            shader_program: shader.clone(),
            ordered_shader_resources: vec![StaticShaderResourceBinding {
                stage: texture.stage,
                slot: texture.slot,
                semantic_token: texture.semantic_token,
                source: StaticShaderResourceSource::UsedContent {
                    content: texture.content.clone(),
                    resource: texture.resource,
                    view: texture.view,
                },
            }],
            pipeline: pipeline(),
            geometry: StaticGeometrySource::Exact {
                geometry: entity_geometry(),
            },
            ordered_constant_buffers: vec![constant_buffer()],
            output_state: output,
            payload_sha256: String::new(),
        };
        draw.payload_sha256 = draw.computed_payload_sha256().unwrap();
        let widget = vec![1, 2, 3, 4];
        let reference = M02CombatDrawRef {
            stable_combat_handle_le_hex: "69120000be500200".into(),
            draw_ordinal: 859,
        };
        (
            M02CombatFrame {
                schema: M02_COMBAT_SCHEMA.into(),
                update_index: 16_959_360,
                first_sequence: 40,
                last_sequence: 42,
                ui_pass_draw_count: 1167,
                validated_laws: vec![
                    M02LawState::AActiveBaseline,
                    M02LawState::BZoomBoundary,
                    M02LawState::CResultLifecycle,
                    M02LawState::DPortStrikeDiscriminator,
                    M02LawState::EVisibilityControl,
                ],
                indicators: vec![M02CombatIndicator {
                    stable_combat_handle_le_hex: reference.stable_combat_handle_le_hex.clone(),
                    kind: M02CombatKind::LandActive,
                    lifecycle_sequence: 42,
                    location_province_id: ExactM02Field::recorded(8819),
                    participants: ExactM02Field::recorded(M02Participants {
                        attacker_country_id: 1,
                        defender_country_id: 2,
                        attacker_source_id: "army:1".into(),
                        defender_source_id: "army:2".into(),
                        attacker_selector_byte_0xba: 0,
                        defender_selector_byte_0xba: 1,
                    }),
                    progress_percent: ExactM02Field::recorded(92),
                    direction: ExactM02Field::recorded(M02Direction::AttackerToDefender),
                    result: ExactM02Field::not_applicable(),
                    observer_country_id: ExactM02Field::recorded(1),
                    fog_of_war: ExactM02Field::recorded(M02FogState::Visible),
                    visible: ExactM02Field::recorded(true),
                    overlap: ExactM02Field::recorded(M02OverlapState {
                        group_id: "land:8819".into(),
                        depth_ordinal: 0,
                        anchor_world_xyz_f32_bits: [0; 3],
                    }),
                    scale_f32_bits: ExactM02Field::not_applicable(),
                    small_scale: ExactM02Field::not_applicable(),
                    raw_widget_states: vec![M02WidgetState {
                        semantic_role: "progress_small".into(),
                        owner_offset: 0x120,
                        canonical_sha256: hash(&widget),
                        canonical_bytes: widget,
                    }],
                    used_content: vec![shader.source, texture.content],
                    draws: vec![M02CombatDraw {
                        draw_ordinal: reference.draw_ordinal,
                        semantic_role: "progress_small".into(),
                        draw,
                    }],
                }],
                ordered_draws: vec![reference.clone()],
            },
            reference,
        )
    }

    fn production_frame_with_m02() -> ProductionRenderFrame {
        let (m02, reference) = exact_m02_carrier();
        ProductionRenderFrame {
            schema: SCHEMA.into(),
            authority: RenderAuthority {
                process_id: 7,
                process_start_nonce: 8,
                observer_installed_qpc: 9,
                d3d_device_created_qpc: 10,
                first_creation_sequence: 1,
                hook_epoch_nonce: 11,
                hook_set_receipt_sha256: "a".repeat(64),
                capture_receipt_sha256: "b".repeat(64),
                launch_epoch_sha256: compute_render_launch_epoch_sha256(
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
                particle_runtime_mode: ParticleRuntimeMode::SubmittedPacketReplay,
            },
            update_index: 16_959_360,
            first_sequence: 40,
            last_sequence: 42,
            presentation_qpc_frequency_hz: 10_000_000,
            display_submit_index: Some("11".into()),
            selected_present: SelectedPresentReceipt::sealed(
                1,
                "c".repeat(64),
                11,
                0,
                60_759_359,
                500,
                600,
            ),
            submits: vec![RenderSubmit {
                submit_index: 11,
                global_order: 1,
                presentation_qpc_ticks: 500,
                interpolation_phase_f32_bits: 0,
                ordered_scene_constant_buffers: vec![],
                ordered_static_map_programs: vec![],
                effect_lifecycle_events: vec![],
                particles: vec![],
                entity_presentations: vec![],
                particle_packet_set_sha256: "5".repeat(64),
                entities: vec![],
                ordered_draws: vec![],
                ordered_render_operations: vec![RenderExecutionRef::M02 { output: reference }],
                direct_visuals: vec![],
                light_lifecycle_events: vec![],
                tiled_lights: vec![],
                weather_membership: vec![],
                political_textures: vec![],
            }],
            m02_combat: Some(m02),
            m03_arrows: None,
            o04_cities: None,
            authenticated_ui_pass: None,
            checkpoint: empty_checkpoint(42, 16_959_360, Some("11")),
        }
    }

    #[test]
    fn m02_unified_overlay_ref_is_exact_display_submit_suffix() {
        let frame = production_frame_with_m02();
        let encoded = frame.encode_json().unwrap();
        assert_eq!(ProductionRenderFrame::decode_json(&encoded).unwrap(), frame);

        let mut wrong = production_frame_with_m02();
        if let RenderExecutionRef::M02 { output } =
            &mut wrong.submits[0].ordered_render_operations[0]
        {
            output.stable_combat_handle_le_hex = "1111111111111111".into();
        }
        assert!(wrong.validate().unwrap_err().contains("exactly cover"));

        let mut non_display = production_frame_with_m02();
        let duplicate = non_display.submits[0].ordered_render_operations[0].clone();
        non_display.submits.push(RenderSubmit {
            submit_index: 12,
            global_order: 2,
            presentation_qpc_ticks: 501,
            interpolation_phase_f32_bits: 0,
            ordered_scene_constant_buffers: vec![],
            ordered_static_map_programs: vec![],
            effect_lifecycle_events: vec![],
            particles: vec![],
            entity_presentations: vec![],
            particle_packet_set_sha256: "6".repeat(64),
            entities: vec![],
            ordered_draws: vec![],
            ordered_render_operations: vec![duplicate],
            direct_visuals: vec![],
            light_lifecycle_events: vec![],
            tiled_lights: vec![],
            weather_membership: vec![],
            political_textures: vec![],
        });
        non_display.checkpoint.last_submit_index = Some("12".into());
        assert!(non_display
            .validate()
            .unwrap_err()
            .contains("exactly one selected Present"));

        let mut non_suffix = production_frame_with_m02();
        let mut program = StaticMapPassProgram {
            program_ordinal: 0,
            pass_token: format!("opaque-pass:{}", "a".repeat(64)),
            ordered_content_keys: vec![],
            ordered_operations: vec![StaticMapOperation {
                operation_ordinal: 7,
                payload_sha256: String::new(),
                command: StaticMapCommand::ClearColor {
                    target: output_state().ordered_color_targets[0].clone(),
                    rgba_f32_bits: [0; 4],
                },
            }],
            payload_sha256: String::new(),
        };
        program.seal_payload_hashes().unwrap();
        non_suffix.submits[0].ordered_scene_constant_buffers = vec![constant_buffer()];
        non_suffix.submits[0].ordered_static_map_programs = vec![program];
        non_suffix.submits[0]
            .ordered_render_operations
            .push(RenderExecutionRef::Static {
                operation_ordinal: 7,
            });
        let error = non_suffix.validate().unwrap_err();
        assert!(error.contains("not a render-order suffix"), "{error}");
    }

    #[test]
    fn selected_present_receipt_is_required_exact_and_tamper_evident() {
        let frame = production_frame_with_m02();
        frame.validate().unwrap();

        let mut zero_candidates = frame.clone();
        zero_candidates.selected_present.candidate_count = 0;
        assert!(zero_candidates
            .validate()
            .unwrap_err()
            .contains("differs from the sole retained submit/hour/cutoff"));

        let mut retargeted = frame.clone();
        retargeted.selected_present.selected_candidate_id = 12;
        assert!(retargeted
            .validate()
            .unwrap_err()
            .contains("differs from the sole retained submit/hour/cutoff"));

        let mut multi = frame.clone();
        multi.selected_present.candidate_count = 2;
        multi.selected_present.selected_ordinal = 1;
        multi.selected_present.candidate_set_sha256 = "d".repeat(64);
        multi.validate().unwrap();
        multi.selected_present.selected_ordinal = 0;
        assert!(multi
            .validate()
            .unwrap_err()
            .contains("differs from the sole retained submit/hour/cutoff"));

        let mut wrong_raw = frame.clone();
        wrong_raw.selected_present.selected_game_hour_raw += 1;
        assert!(wrong_raw
            .validate()
            .unwrap_err()
            .contains("differs from the sole retained submit/hour/cutoff"));

        let mut malformed_present = serde_json::to_value(&frame).unwrap();
        malformed_present["selected_present"]["selected_candidate_id"] = serde_json::json!("011");
        assert!(serde_json::from_value::<ProductionRenderFrame>(malformed_present).is_err());

        let mut unknown_present_field = serde_json::to_value(&frame).unwrap();
        unknown_present_field["selected_present"]["guessed_present_kind"] =
            serde_json::json!("display");
        assert!(serde_json::from_value::<ProductionRenderFrame>(unknown_present_field).is_err());

        let mut bad_receipt = frame;
        bad_receipt.selected_present.candidate_set_sha256 = "z".repeat(64);
        assert!(bad_receipt.validate().unwrap_err().contains("SHA-256"));
    }

    #[test]
    fn older_v3_without_optional_input_or_ui_pass_remains_compatible() {
        let frame = production_frame_with_m02();
        let encoded = frame.encode_json().unwrap();
        let value: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
        assert!(value["authority"].get("recorded_input_variant").is_none());
        assert!(value.get("authenticated_ui_pass").is_none());
        assert_eq!(ProductionRenderFrame::decode_json(&encoded).unwrap(), frame);
    }

    fn shader() -> ShaderProgram {
        let vs = vec![1, 2, 3, 4];
        let ps = vec![5, 6, 7, 8];
        ShaderProgram {
            source: content("gfx/fx/particle.shader"),
            effect_token: "ParticleAdditive".into(),
            vertex_entry_token: "VertexSimpleParticle".into(),
            pixel_entry_token: "PixelSimpleParticle".into(),
            ordered_defines: vec![ShaderDefine {
                name: "PARTICLE".into(),
                value: "1".into(),
                scope: ShaderDefineScope::Effect,
            }],
            compiled_vertex_shader: CompiledShaderPayload {
                compiler_identity: "fxc-10.1".into(),
                target_profile: "vs_5_0".into(),
                compiler_flags_u64: 0,
                sha256: hash(&vs),
                bytecode: vs,
            },
            compiled_pixel_shader: CompiledShaderPayload {
                compiler_identity: "fxc-10.1".into(),
                target_profile: "ps_5_0".into(),
                compiler_flags_u64: 0,
                sha256: hash(&ps),
                bytecode: ps,
            },
            geometry_shader: None,
            hull_shader: None,
            domain_shader: None,
            compute_shader: None,
        }
    }

    fn constant_buffer() -> ConstantBufferBinding {
        ConstantBufferBinding {
            stage_mask_u8: 1,
            slot: 0,
            byte_offset: 0,
            byte_count: 16,
            raw_bytes: vec![0; 16],
            sha256: hash(&vec![0; 16]),
            semantic_layout: "ProjectionMatrix".into(),
        }
    }

    fn texture() -> TextureBinding {
        let texel = vec![255u8; 4];
        TextureBinding {
            stage: ShaderStage::Pixel,
            slot: 0,
            semantic_token: "DiffuseMap".into(),
            content: content("gfx/textures/particle.dds"),
            resource: TextureResourceDesc {
                dimension: TextureResourceDimension::Texture2d,
                width: 1,
                height: 1,
                depth: 1,
                mip_levels: 1,
                array_size: 1,
                dxgi_format_u32: 28,
                sample_count: 1,
                sample_quality: 0,
                usage_u32: 0,
                bind_flags_u32: 0x8,
                cpu_access_flags_u32: 0,
                misc_flags_u32: 0,
                ordered_subresources: vec![TextureSubresourcePayload {
                    subresource_index: 0,
                    mip_level: 0,
                    array_slice: 0,
                    row_pitch: 4,
                    slice_pitch: 4,
                    source: BufferSource::Inline {
                        sha256: hash(&texel),
                        bytes: texel,
                    },
                }],
            },
            view: ShaderResourceViewDesc::Texture2d {
                dxgi_format_u32: 29,
                most_detailed_mip: 0,
                mip_levels: 1,
            },
        }
    }

    fn particle_geometry(instance_count: u32) -> GeometryBinding {
        let indices = vec![0u8, 0, 1, 0, 2, 0, 2, 0, 1, 0, 3, 0];
        GeometryBinding {
            ordered_vertex_buffers: vec![VertexBufferBinding {
                input_slot: 0,
                stride_bytes: 48,
                byte_offset: 0,
                source: BufferSource::ParticleInstanceRecords,
            }],
            index_buffer: Some(IndexBufferBinding {
                format: IndexFormat::R16Uint,
                byte_offset: 0,
                source: BufferSource::Inline {
                    sha256: hash(&indices),
                    bytes: indices,
                },
            }),
            stream_output_targets: Vec::new(),
            draw: DrawCommand::DrawIndexedInstanced {
                index_count_per_instance: 6,
                instance_count,
                start_index: 0,
                base_vertex: 0,
                start_instance: 0,
            },
        }
    }

    fn entity_geometry() -> GeometryBinding {
        let indices = vec![0u8, 0, 1, 0, 2, 0, 2, 0, 1, 0, 3, 0];
        let vertices = vec![0u8; 48];
        GeometryBinding {
            ordered_vertex_buffers: vec![VertexBufferBinding {
                input_slot: 0,
                stride_bytes: 48,
                byte_offset: 0,
                source: BufferSource::Inline {
                    sha256: hash(&vertices),
                    bytes: vertices,
                },
            }],
            index_buffer: Some(IndexBufferBinding {
                format: IndexFormat::R16Uint,
                byte_offset: 0,
                source: BufferSource::Inline {
                    sha256: hash(&indices),
                    bytes: indices,
                },
            }),
            stream_output_targets: Vec::new(),
            draw: DrawCommand::DrawIndexed {
                index_count: 6,
                start_index: 0,
                base_vertex: 0,
            },
        }
    }

    fn pipeline() -> PipelineDesc {
        PipelineDesc {
            topology: PrimitiveTopology::TriangleList,
            blend: BlendDesc {
                alpha_to_coverage_enabled: false,
                independent_blend_enabled: false,
                enabled: true,
                source_color: BlendFactor::SourceAlpha,
                destination_color: BlendFactor::One,
                color_operation: BlendOperation::Add,
                source_alpha: BlendFactor::Zero,
                destination_alpha: BlendFactor::One,
                alpha_operation: BlendOperation::Add,
                write_mask_rgba_u8: 0x7,
                additional_targets: vec![],
                blend_factor_rgba_f32_bits: [1f32.to_bits(); 4],
                sample_mask_u32: u32::MAX,
            },
            depth: DepthDesc {
                enabled: true,
                write_enabled: false,
                comparison: Comparison::LessEqual,
                stencil_enabled: false,
                stencil_read_mask_u8: 0xff,
                stencil_write_mask_u8: 0xff,
                front_face: StencilFaceDesc {
                    fail_operation: StencilOperation::Keep,
                    depth_fail_operation: StencilOperation::Keep,
                    pass_operation: StencilOperation::Keep,
                    comparison: Comparison::Always,
                },
                back_face: StencilFaceDesc {
                    fail_operation: StencilOperation::Keep,
                    depth_fail_operation: StencilOperation::Keep,
                    pass_operation: StencilOperation::Keep,
                    comparison: Comparison::Always,
                },
                stencil_reference_u32: 0,
            },
            rasterizer: RasterizerDesc {
                fill: FillMode::Solid,
                cull: CullMode::Back,
                front_counter_clockwise: false,
                depth_clip_enabled: true,
                scissor_enabled: false,
                depth_bias_i32: 0,
                depth_bias_clamp_f32_bits: 0f32.to_bits(),
                slope_scaled_depth_bias_f32_bits: 0f32.to_bits(),
                multisample_enabled: false,
                antialiased_line_enabled: false,
            },
            samplers: vec![SamplerDesc {
                stage: ShaderStage::Pixel,
                slot: 0,
                semantic_token: "DiffuseMap".into(),
                min_filter: Filter::Linear,
                mag_filter: Filter::Linear,
                mip_filter: Filter::Linear,
                address_u: AddressMode::Wrap,
                address_v: AddressMode::Wrap,
                address_w: AddressMode::Wrap,
                comparison: None,
                mip_lod_bias_f32_bits: 0f32.to_bits(),
                max_anisotropy: 1,
                border_color_rgba_f32_bits: [0f32.to_bits(); 4],
                min_lod_f32_bits: 0f32.to_bits(),
                max_lod_f32_bits: f32::MAX.to_bits(),
            }],
            input_layout: vec![InputElementDesc {
                semantic_token: "POSITION".into(),
                semantic_index: 0,
                dxgi_format_u32: 6,
                input_slot: 0,
                aligned_byte_offset: 0,
                per_instance: true,
                instance_step_rate: 1,
            }],
            index_format: IndexFormat::R16Uint,
            static_indices_u16: vec![0, 1, 2, 2, 1, 3],
        }
    }

    fn output_state() -> RenderOutputState {
        RenderOutputState {
            ordered_color_targets: vec![RenderTargetDesc {
                prepared_target: PreparedRenderTarget::OpaqueRenderTarget {
                    attachment_ordinal: 0,
                },
                width: 32,
                height: 32,
                dxgi_format_u32: 10,
                mip_level: 0,
                sample_count: 1,
                sample_quality: 0,
                persistent_content: None,
                exact_resource: None,
            }],
            depth_target: None,
            viewport: ViewportDesc {
                top_left_x_f32_bits: 0,
                top_left_y_f32_bits: 0,
                width_f32_bits: 32f32.to_bits(),
                height_f32_bits: 32f32.to_bits(),
                min_depth_f32_bits: 0,
                max_depth_f32_bits: 1f32.to_bits(),
            },
            scissor: None,
        }
    }

    fn identity_transform() -> [u32; 16] {
        [
            1.0f32.to_bits(),
            0,
            0,
            0,
            0,
            1.0f32.to_bits(),
            0,
            0,
            0,
            0,
            1.0f32.to_bits(),
            0,
            10.0f32.to_bits(),
            20.0f32.to_bits(),
            30.0f32.to_bits(),
            1.0f32.to_bits(),
        ]
    }

    fn production_frame_with_o04() -> ProductionRenderFrame {
        let mut frame = production_frame_with_m02();
        frame.m02_combat = None;
        frame.submits[0].ordered_render_operations.clear();
        let output = RenderOutputRef::Entity {
            pass_ordinal: 1,
            entity_instance_id: 8,
            draw_order: 2,
        };
        frame.submits[0].entities = vec![EntitySubmission {
            pass_ordinal: 1,
            draw_order: 2,
            entity_instance_id: 8,
            resolved_entity_sha256: "c".repeat(64),
            content_identity: entity_content_identity(),
            shader_program: shader(),
            ordered_texture_bindings: vec![texture()],
            pipeline: pipeline(),
            geometry: entity_geometry(),
            ordered_constant_buffers: vec![constant_buffer()],
            output_state: output_state(),
            world_transform_4x4_f32_bits: identity_transform(),
            state_flags: 3 | (4 << 8) | (7 << 16) | (6 << 24),
            skinning: Skinning::NotSkinned,
        }];
        frame.submits[0].ordered_draws = vec![output.clone()];
        frame.o04_cities = Some(O04CityFrame {
            schema: O04_CITY_SCHEMA.into(),
            executable_sha256: O04_A729_EXECUTABLE_SHA256.into(),
            evidence_manifest_sha256: O04_EVIDENCE_MANIFEST_SHA256.into(),
            live_city_submit_binding_sha256: O04_LIVE_CITY_SUBMIT_BINDING_SHA256.into(),
            live_city_submit_verification_sha256: O04_LIVE_CITY_SUBMIT_VERIFICATION_SHA256.into(),
            update_index: frame.update_index,
            first_sequence: frame.first_sequence,
            last_sequence: frame.last_sequence,
            cities: vec![O04CitySemantic {
                event_sequence: frame.last_sequence,
                generator_ordinal: 73,
                candidate_xy: [12, 34],
                province_id: 8819,
                color_index: 7,
                density_f32_bits: 0.75f32.to_bits(),
                entity_scale_f32_bits: 1.25f32.to_bits(),
                entity_asset: entity_content_identity().resolved_entity_asset,
                mesh: entity_content_identity().mesh,
                world_transform_4x4_f32_bits: identity_transform(),
                citylight_state: O04CitylightState::RegisteredCitylight,
                referenced_outputs: vec![output.clone()],
            }],
            ordered_outputs: vec![output],
        });
        frame
    }

    fn production_frame_with_exact_ui_pass() -> ProductionRenderFrame {
        let mut frame = production_frame_with_m02();
        frame.authority.recorded_input_variant = Some(RecordedInputVariant::MouseKeyboard);
        let m02 = frame.m02_combat.as_ref().unwrap();
        let m02_ref = m02.ordered_draws[0].clone();
        let target = m02
            .draw(&m02_ref)
            .unwrap()
            .draw
            .output_state
            .ordered_color_targets[0]
            .clone();
        let mut ui_pass_operations = Vec::new();
        let gui_definition = content("interface/modded_selected_hud.gui");
        for ui_pass_ordinal in 0..m02.ui_pass_draw_count {
            if ui_pass_ordinal == 859 {
                ui_pass_operations.push(UiPassOperation::M02 {
                    ui_pass_ordinal,
                    output: m02_ref.clone(),
                });
            } else {
                let identity = HudCommandIdentity {
                    authored_surface_identity: "modded_selected_present_hud".into(),
                    runtime_root: "modded_selected_present_root".into(),
                    selected_gui_definition: gui_definition.clone(),
                    child_path: vec![ui_pass_ordinal],
                    element_type: "modded_paint_element".into(),
                    name: format!("element-{ui_pass_ordinal}"),
                    paint_ordinal: ui_pass_ordinal,
                };
                let placement = HudPlacement {
                    destination_rect: if ui_pass_ordinal == 0 {
                        UiPassRect {
                            left_i32: 0,
                            top_i32: 0,
                            right_i32: 32,
                            bottom_i32: 32,
                        }
                    } else {
                        UiPassRect {
                            left_i32: 0,
                            top_i32: 0,
                            right_i32: 1,
                            bottom_i32: 1,
                        }
                    },
                    clip_state: UiPassClip::Disabled,
                    surface_paint_ordinal: ui_pass_ordinal,
                };
                let mut ordered_content_for_paint = Vec::new();
                let execution = if ui_pass_ordinal == 0 {
                    HudCommandExecution::ExactPipeline {
                        draw: UiExactPipelineDraw {
                            exact_draw: m02.draw(&m02_ref).unwrap().draw.clone(),
                        },
                    }
                } else {
                    let (paint, extra_content) = match ui_pass_ordinal % 3 {
                        0 => {
                            let law = match ui_pass_ordinal % 5 {
                                0 => SpriteLaw::Plain,
                                1 => SpriteLaw::Scaled,
                                2 => SpriteLaw::NineSlice {
                                    border_x: 2,
                                    border_y: 3,
                                },
                                3 => SpriteLaw::ProgressLeftCrop {
                                    state_bits: 0.5f32.to_bits(),
                                },
                                _ => SpriteLaw::ProgressMinMax {
                                    state_bits: 0.75f32.to_bits(),
                                },
                            };
                            let is_minmax = matches!(law, SpriteLaw::ProgressMinMax { .. });
                            let mut content_rows = vec![
                                content("interface/modded_selected_hud.gfx"),
                                content("gfx/interface/modded_selected_hud.dds"),
                            ];
                            if is_minmax {
                                content_rows
                                    .push(content("gfx/interface/modded_selected_hud_back.dds"));
                            }
                            (
                                HudPaintCommand::Sprite {
                                    sprite_id: format!("sprite-{ui_pass_ordinal}"),
                                    resolved_frame: ui_pass_ordinal % 4,
                                    law,
                                    sprite_definition_content_index: 1,
                                    texture_content_index: 2,
                                    back_texture_content_index: is_minmax.then_some(3),
                                },
                                content_rows,
                            )
                        }
                        1 => (
                            HudPaintCommand::Text {
                                runtime_text_context_id: format!("runtime-text-{ui_pass_ordinal}"),
                                runtime_text_frame_hour: frame.update_index as i64,
                                runtime_text_pov_country_or_global: "MOD".into(),
                                runtime_text_view_id: "modded_selected_present_root".into(),
                                runtime_text_element_id: format!("element-{ui_pass_ordinal}"),
                                rgba: [248, 249, 250, 255],
                                horizontal_alignment: TextHorizontalAlignment::Center,
                                layout_inputs: TextLayoutInputs::AuthoredReproducible {
                                    authored_rect_offsets: TextRectOffsets {
                                        left_i32: 0,
                                        top_i32: 0,
                                        right_i32: 160,
                                        bottom_i32: 24,
                                    },
                                    max_width: Some(160),
                                    wrap: true,
                                    clip_rect_offsets: Some(TextRectOffsets {
                                        left_i32: 0,
                                        top_i32: 0,
                                        right_i32: 160,
                                        bottom_i32: 24,
                                    }),
                                },
                                font_bindings: vec![
                                    FontBinding {
                                        font_identity: "modded_selected_hud".into(),
                                        definition_content_index: 1,
                                        atlas_texture_content_indices: vec![2],
                                    },
                                    FontBinding {
                                        font_identity: "modded_selected_hud_fallback".into(),
                                        definition_content_index: 3,
                                        atlas_texture_content_indices: vec![4, 5],
                                    },
                                ],
                                inline_icon_bindings: vec![InlineIconBinding {
                                    sprite_identity: "GFX_modded_selected_hud_inline".into(),
                                    definition_content_index: 6,
                                    texture_content_indices: vec![7],
                                }],
                            },
                            vec![
                                content("interface/fonts/modded_selected_hud.font"),
                                content("gfx/fonts/modded_selected_hud.dds"),
                                content("interface/fonts/modded_selected_hud_fallback.font"),
                                content("gfx/fonts/modded_selected_hud_fallback_0.dds"),
                                content("gfx/fonts/modded_selected_hud_fallback_1.dds"),
                                content("interface/modded_selected_hud_inline.gfx"),
                                content("gfx/interface/modded_selected_hud_inline.dds"),
                            ],
                        ),
                        _ => (
                            HudPaintCommand::MaskedSprite {
                                sprite_id: format!("masked-sprite-{ui_pass_ordinal}"),
                                mask_slot: 1,
                                overlay_slot: 2,
                                sprite_definition_content_index: 1,
                                texture_content_index: 2,
                                mask_texture_content_index: 3,
                                overlay_texture_content_index: 4,
                            },
                            vec![
                                content("interface/modded_masked_hud.gfx"),
                                content("gfx/interface/modded_masked_hud.dds"),
                                content("gfx/interface/modded_mask.dds"),
                                content("gfx/interface/modded_overlay.dds"),
                            ],
                        ),
                    };
                    ordered_content_for_paint = extra_content;
                    HudCommandExecution::InterpretedPaint { paint }
                };
                let mut ordered_content = vec![gui_definition.clone()];
                if let HudCommandExecution::ExactPipeline { draw } = &execution {
                    for content in collect_static_draw_used_content(&draw.exact_draw) {
                        if !ordered_content.contains(&content) {
                            ordered_content.push(content);
                        }
                    }
                } else {
                    ordered_content.extend(ordered_content_for_paint);
                }
                let mut command = HudCommand {
                    ordinal: ui_pass_ordinal,
                    identity,
                    placement,
                    execution,
                    ordered_effective_vfs_used_content: ordered_content,
                    command_identity_sha256: String::new(),
                };
                command.command_identity_sha256 =
                    command.computed_identity_sha256(&target).unwrap();
                ui_pass_operations.push(UiPassOperation::HudCommand { command });
            }
        }
        let mut authenticated_ui_pass = AuthenticatedUiPass {
            schema: UI_PASS_SCHEMA.into(),
            update_index: frame.update_index,
            first_sequence: frame.first_sequence,
            last_sequence: frame.last_sequence,
            ui_pass_draw_count: m02.ui_pass_draw_count,
            target,
            active_mapmode_state: Some(ActiveMapmodeState {
                mode_token: "modded_weather_mapmode".into(),
                button_ordinal: 7,
                authored_button_identity: "modded_weather_button".into(),
                layout_root_token: "modded_8_button_mapmode".into(),
                provenance: MapmodeProvenance::EffectiveVfs {
                    content: gui_definition,
                },
            }),
            ui_pass_operations,
            pass_identity_sha256: String::new(),
        };
        authenticated_ui_pass.pass_identity_sha256 =
            authenticated_ui_pass.computed_identity_sha256().unwrap();
        frame.authenticated_ui_pass = Some(authenticated_ui_pass);
        frame
    }

    fn reshape_exact_ui_m02_interleave(
        frame: &mut ProductionRenderFrame,
        ui_pass_draw_count: u32,
        m02_draw_ordinal: u32,
    ) {
        assert!(ui_pass_draw_count > 0);
        assert!(m02_draw_ordinal < ui_pass_draw_count);
        let m02 = frame.m02_combat.as_mut().unwrap();
        m02.ui_pass_draw_count = ui_pass_draw_count;
        m02.indicators[0].draws[0].draw_ordinal = m02_draw_ordinal;
        m02.ordered_draws[0].draw_ordinal = m02_draw_ordinal;
        let m02_ref = m02.ordered_draws[0].clone();
        frame.submits[0].ordered_render_operations = vec![RenderExecutionRef::M02 {
            output: m02_ref.clone(),
        }];

        let pass = frame.authenticated_ui_pass.as_mut().unwrap();
        pass.ui_pass_draw_count = ui_pass_draw_count;
        pass.ui_pass_operations
            .truncate(ui_pass_draw_count as usize);
        pass.ui_pass_operations[m02_draw_ordinal as usize] = UiPassOperation::M02 {
            ui_pass_ordinal: m02_draw_ordinal,
            output: m02_ref,
        };
        pass.pass_identity_sha256 = pass.computed_identity_sha256().unwrap();
    }

    fn exact_ui_command_mut(frame: &mut ProductionRenderFrame, ordinal: usize) -> &mut HudCommand {
        match &mut frame
            .authenticated_ui_pass
            .as_mut()
            .unwrap()
            .ui_pass_operations[ordinal]
        {
            UiPassOperation::HudCommand { command } => command,
            UiPassOperation::M02 { .. } => panic!("expected HUD command at ordinal {ordinal}"),
        }
    }

    fn rehash_exact_ui_command_and_pass(frame: &mut ProductionRenderFrame, ordinal: usize) {
        let target = frame.authenticated_ui_pass.as_ref().unwrap().target.clone();
        let command = exact_ui_command_mut(frame, ordinal);
        command.command_identity_sha256 = command.computed_identity_sha256(&target).unwrap();
        let ui_pass = frame.authenticated_ui_pass.as_mut().unwrap();
        ui_pass.pass_identity_sha256 = ui_pass.computed_identity_sha256().unwrap();
    }

    fn production_frame_with_m03_a48_row() -> ProductionRenderFrame {
        let mut frame = production_frame_with_m02();
        frame.m02_combat = None;
        let mut draw = exact_m02_carrier().0.indicators[0].draws[0].draw.clone();
        draw.draw_token = "m03.exact-a48-movement".into();
        draw.payload_sha256 = draw.computed_payload_sha256().unwrap();
        let stable_key = M03StableKey::Movement {
            owner_country_id: 1,
            division_id: 42,
            movement_generation: 3,
        };
        let draw_ref = M03ArrowDrawRef {
            engine_draw_ordinal: 23,
        };
        frame.m03_arrows = Some(M03ArrowFrame {
            schema: M03_ARROW_SCHEMA.into(),
            executable_sha256: M03_A729_EXECUTABLE_SHA256.into(),
            static_semantic_proof_sha256: M03_STATIC_PROOF_SHA256.into(),
            update_index: frame.update_index,
            first_sequence: frame.first_sequence,
            last_sequence: frame.last_sequence,
            used_content: vec![
                content("gfx/fx/particle.shader"),
                content("gfx/textures/particle.dds"),
            ],
            rows: vec![M03ArrowRow {
                event_sequence: frame.last_sequence,
                stable_key: stable_key.clone(),
                definition_name: "moveArrow".into(),
                plan_subtype: None,
                ordered_division_ids: vec![],
                ordered_fixed_point_input_points_1e5: vec![[0, 0], [100_000, 100_000]],
                ordered_engine_working_points_1e5: Some(vec![[0, 0], [100_000, 100_000]]),
                assigned: true,
                active: true,
                selected: false,
                progress_raw_u32: 5,
                fade_in_f32_bits: 0.2f32.to_bits(),
                fade_out_f32_bits: 0.0f32.to_bits(),
                culling_flags_u16: 0,
                visibility: M03Visibility::Submitted,
                manager_insertion_ordinal: 4,
                definition_group_ordinal: 2,
                comparator_definition_a0_f32_bits: 2.0f32.to_bits(),
                comparator_definition_a4_f32_bits: 3.0f32.to_bits(),
                comparator_definition_c8_f32_bits: 0.5f32.to_bits(),
                comparator_arrow_178_f32_bits: 4.0f32.to_bits(),
                comparator_primary_f32_bits: 5.0f32.to_bits(),
                comparator_tie_17c_f32_bits: 1.0f32.to_bits(),
                draw_members: vec![M03ArrowDrawMemberRef {
                    engine_draw_ordinal: 23,
                    member_ordinal: 0,
                }],
            }],
            draws: vec![M03ArrowDraw {
                engine_draw_ordinal: 23,
                draw,
                members: vec![M03ArrowDrawMember {
                    stable_key,
                    member_ordinal: 0,
                    first_vertex: 0,
                    vertex_count: 6,
                }],
            }],
            ordered_draws: vec![draw_ref.clone()],
        });
        frame.submits[0].ordered_render_operations =
            vec![RenderExecutionRef::M03 { output: draw_ref }];
        frame
    }

    #[test]
    fn o04_semantic_carrier_round_trips_and_rejects_join_vfs_and_evidence_tamper() {
        let frame = production_frame_with_o04();
        let encoded = frame.encode_json().unwrap();
        assert_eq!(ProductionRenderFrame::decode_json(&encoded).unwrap(), frame);

        let mut dangling = frame.clone();
        if let Some(RenderOutputRef::Entity { draw_order, .. }) =
            dangling.o04_cities.as_mut().unwrap().cities[0]
                .referenced_outputs
                .first_mut()
        {
            *draw_order = 99;
        }
        assert!(dangling.validate().unwrap_err().contains("ordered outputs"));

        let mut mismatched_transform = frame.clone();
        mismatched_transform.o04_cities.as_mut().unwrap().cities[0].world_transform_4x4_f32_bits
            [12] = 11.0f32.to_bits();
        assert!(mismatched_transform
            .validate()
            .unwrap_err()
            .contains("differs from entity submission"));

        let mut wrong_vfs = frame.clone();
        wrong_vfs.o04_cities.as_mut().unwrap().cities[0]
            .entity_asset
            .effective_vfs_manifest_sha256 = "f".repeat(64);
        assert!(wrong_vfs
            .validate()
            .unwrap_err()
            .contains("content authority"));

        let mut wrong_evidence = frame;
        wrong_evidence
            .o04_cities
            .as_mut()
            .unwrap()
            .evidence_manifest_sha256 = "e".repeat(64);
        assert!(wrong_evidence
            .validate()
            .unwrap_err()
            .contains("evidence authority"));
    }

    #[test]
    fn exact_ui_pass_round_trips_and_rejects_gap_ref_ordinal_and_target_tamper() {
        let frame = production_frame_with_exact_ui_pass();
        let encoded = frame.encode_json().unwrap();
        assert_eq!(ProductionRenderFrame::decode_json(&encoded).unwrap(), frame);

        let mut gap = frame.clone();
        gap.authenticated_ui_pass
            .as_mut()
            .unwrap()
            .ui_pass_operations
            .pop();
        assert!(gap.validate().unwrap_err().contains("draw frontier"));

        let mut wrong_ref = frame.clone();
        if let UiPassOperation::M02 { output, .. } = &mut wrong_ref
            .authenticated_ui_pass
            .as_mut()
            .unwrap()
            .ui_pass_operations[859]
        {
            output.draw_ordinal = 858;
        }
        assert!(wrong_ref
            .validate()
            .unwrap_err()
            .contains("ordinal mismatch"));

        let mut wrong_target = frame.clone();
        if let UiPassOperation::HudCommand { command } = &mut wrong_target
            .authenticated_ui_pass
            .as_mut()
            .unwrap()
            .ui_pass_operations[0]
        {
            command.placement.destination_rect.right_i32 = 31;
        }
        assert!(wrong_target
            .validate()
            .unwrap_err()
            .contains("identity SHA-256 mismatch"));

        let mut duplicate_ref = frame;
        let mut duplicate = match &duplicate_ref
            .authenticated_ui_pass
            .as_ref()
            .unwrap()
            .ui_pass_operations[1]
        {
            UiPassOperation::HudCommand { command } => command.clone(),
            UiPassOperation::M02 { .. } => unreachable!(),
        };
        let duplicate_target = duplicate_ref
            .authenticated_ui_pass
            .as_ref()
            .unwrap()
            .target
            .clone();
        duplicate.ordinal = 0;
        duplicate.command_identity_sha256 = duplicate
            .computed_identity_sha256(&duplicate_target)
            .unwrap();
        duplicate_ref
            .authenticated_ui_pass
            .as_mut()
            .unwrap()
            .ui_pass_operations[0] = UiPassOperation::HudCommand { command: duplicate };
        assert!(duplicate_ref
            .validate()
            .unwrap_err()
            .contains("identity is duplicated"));

        let mut incomplete_content_join = production_frame_with_exact_ui_pass();
        let target = incomplete_content_join
            .authenticated_ui_pass
            .as_ref()
            .unwrap()
            .target
            .clone();
        let ui_pass = incomplete_content_join
            .authenticated_ui_pass
            .as_mut()
            .unwrap();
        if let UiPassOperation::HudCommand { command } = &mut ui_pass.ui_pass_operations[1] {
            command.ordered_effective_vfs_used_content.pop();
            command.command_identity_sha256 = command.computed_identity_sha256(&target).unwrap();
        }
        ui_pass.pass_identity_sha256 = ui_pass.computed_identity_sha256().unwrap();
        assert!(incomplete_content_join
            .validate()
            .unwrap_err()
            .contains("content index is invalid"));

        let mut missing_minmax_back = production_frame_with_exact_ui_pass();
        let target = missing_minmax_back
            .authenticated_ui_pass
            .as_ref()
            .unwrap()
            .target
            .clone();
        let ui_pass = missing_minmax_back.authenticated_ui_pass.as_mut().unwrap();
        if let UiPassOperation::HudCommand { command } = &mut ui_pass.ui_pass_operations[9] {
            if let HudCommandExecution::InterpretedPaint {
                paint:
                    HudPaintCommand::Sprite {
                        back_texture_content_index,
                        ..
                    },
            } = &mut command.execution
            {
                *back_texture_content_index = None;
            }
            command.command_identity_sha256 = command.computed_identity_sha256(&target).unwrap();
        }
        ui_pass.pass_identity_sha256 = ui_pass.computed_identity_sha256().unwrap();
        assert!(missing_minmax_back
            .validate()
            .unwrap_err()
            .contains("lacks exact back-texture"));

        let mut duplicate_minmax_index = production_frame_with_exact_ui_pass();
        let target = duplicate_minmax_index
            .authenticated_ui_pass
            .as_ref()
            .unwrap()
            .target
            .clone();
        let ui_pass = duplicate_minmax_index
            .authenticated_ui_pass
            .as_mut()
            .unwrap();
        if let UiPassOperation::HudCommand { command } = &mut ui_pass.ui_pass_operations[9] {
            if let HudCommandExecution::InterpretedPaint {
                paint:
                    HudPaintCommand::Sprite {
                        texture_content_index,
                        back_texture_content_index,
                        ..
                    },
            } = &mut command.execution
            {
                *back_texture_content_index = Some(*texture_content_index);
            }
            command.command_identity_sha256 = command.computed_identity_sha256(&target).unwrap();
        }
        ui_pass.pass_identity_sha256 = ui_pass.computed_identity_sha256().unwrap();
        assert!(duplicate_minmax_index
            .validate()
            .unwrap_err()
            .contains("indices are not distinct"));

        let mut unproven_back = production_frame_with_exact_ui_pass();
        let target = unproven_back
            .authenticated_ui_pass
            .as_ref()
            .unwrap()
            .target
            .clone();
        let ui_pass = unproven_back.authenticated_ui_pass.as_mut().unwrap();
        if let UiPassOperation::HudCommand { command } = &mut ui_pass.ui_pass_operations[3] {
            if let HudCommandExecution::InterpretedPaint {
                paint:
                    HudPaintCommand::Sprite {
                        back_texture_content_index,
                        ..
                    },
            } = &mut command.execution
            {
                *back_texture_content_index = Some(2);
            }
            command.command_identity_sha256 = command.computed_identity_sha256(&target).unwrap();
        }
        ui_pass.pass_identity_sha256 = ui_pass.computed_identity_sha256().unwrap();
        assert!(unproven_back
            .validate()
            .unwrap_err()
            .contains("unproven back texture"));

        let mut absent_input = production_frame_with_exact_ui_pass();
        absent_input.authority.recorded_input_variant = None;
        assert!(absent_input
            .validate()
            .unwrap_err()
            .contains("recorded input variant"));

        let mut wrong_mapmode_vfs = production_frame_with_exact_ui_pass();
        if let Some(ActiveMapmodeState {
            provenance: MapmodeProvenance::EffectiveVfs { content },
            ..
        }) = &mut wrong_mapmode_vfs
            .authenticated_ui_pass
            .as_mut()
            .unwrap()
            .active_mapmode_state
        {
            content.effective_vfs_manifest_sha256 = "f".repeat(64);
        }
        let ui_pass = wrong_mapmode_vfs.authenticated_ui_pass.as_mut().unwrap();
        ui_pass.pass_identity_sha256 = ui_pass.computed_identity_sha256().unwrap();
        assert!(wrong_mapmode_vfs
            .validate()
            .unwrap_err()
            .contains("mapmode VFS authority"));
    }

    #[test]
    fn exact_ui_pass_accepts_modded_dynamic_frontier_and_m02_ordinal() {
        let mut frame = production_frame_with_exact_ui_pass();
        reshape_exact_ui_m02_interleave(&mut frame, 23, 17);
        frame.validate().unwrap();
        let encoded = frame.encode_json().unwrap();
        assert_eq!(ProductionRenderFrame::decode_json(&encoded).unwrap(), frame);

        let mut wrong_frontier = frame.clone();
        wrong_frontier
            .m02_combat
            .as_mut()
            .unwrap()
            .ui_pass_draw_count = 24;
        assert!(wrong_frontier
            .validate()
            .unwrap_err()
            .contains("exactly interleave"));

        let mut wrong_dynamic_ordinal = frame;
        let pass = wrong_dynamic_ordinal
            .authenticated_ui_pass
            .as_mut()
            .unwrap();
        pass.ui_pass_operations.swap(16, 17);
        pass.pass_identity_sha256 = pass.computed_identity_sha256().unwrap();
        assert!(wrong_dynamic_ordinal
            .validate()
            .unwrap_err()
            .contains("0..N-1"));
    }

    #[test]
    fn exact_ui_text_round_trips_fallback_fonts_icons_and_captured_owner_layout() {
        let frame = production_frame_with_exact_ui_pass();
        let command = match &frame
            .authenticated_ui_pass
            .as_ref()
            .unwrap()
            .ui_pass_operations[1]
        {
            UiPassOperation::HudCommand { command } => command,
            UiPassOperation::M02 { .. } => unreachable!(),
        };
        let HudCommandExecution::InterpretedPaint {
            paint:
                HudPaintCommand::Text {
                    font_bindings,
                    inline_icon_bindings,
                    ..
                },
        } = &command.execution
        else {
            panic!("expected text paint")
        };
        assert_eq!(font_bindings.len(), 2);
        assert_eq!(font_bindings[1].atlas_texture_content_indices, vec![4, 5]);
        assert_eq!(inline_icon_bindings.len(), 1);
        assert_eq!(inline_icon_bindings[0].texture_content_indices, vec![7]);
        let encoded = frame.encode_json().unwrap();
        assert_eq!(ProductionRenderFrame::decode_json(&encoded).unwrap(), frame);

        let mut captured = production_frame_with_exact_ui_pass();
        let command = exact_ui_command_mut(&mut captured, 1);
        let HudCommandExecution::InterpretedPaint {
            paint: HudPaintCommand::Text { layout_inputs, .. },
        } = &mut command.execution
        else {
            unreachable!()
        };
        *layout_inputs = TextLayoutInputs::CapturedRuntime;
        rehash_exact_ui_command_and_pass(&mut captured, 1);
        captured.validate().unwrap();
        let encoded = captured.encode_json().unwrap();
        assert_eq!(
            ProductionRenderFrame::decode_json(&encoded).unwrap(),
            captured
        );
    }

    #[test]
    fn exact_ui_text_hashes_all_inputs_and_rejects_invalid_bindings() {
        for identity_axis in ["hour", "pov", "view", "element"] {
            let mut tampered = production_frame_with_exact_ui_pass();
            let command = exact_ui_command_mut(&mut tampered, 1);
            let HudCommandExecution::InterpretedPaint {
                paint:
                    HudPaintCommand::Text {
                        runtime_text_frame_hour,
                        runtime_text_pov_country_or_global,
                        runtime_text_view_id,
                        runtime_text_element_id,
                        ..
                    },
            } = &mut command.execution
            else {
                unreachable!()
            };
            match identity_axis {
                "hour" => *runtime_text_frame_hour += 1,
                "pov" => runtime_text_pov_country_or_global.push_str("_tampered"),
                "view" => runtime_text_view_id.push_str("_tampered"),
                "element" => runtime_text_element_id.push_str("_tampered"),
                _ => unreachable!(),
            }
            assert!(tampered
                .validate()
                .unwrap_err()
                .contains("command identity SHA-256 mismatch"));
        }

        let mut rgba_tamper = production_frame_with_exact_ui_pass();
        let command = exact_ui_command_mut(&mut rgba_tamper, 1);
        let HudCommandExecution::InterpretedPaint {
            paint: HudPaintCommand::Text { rgba, .. },
        } = &mut command.execution
        else {
            unreachable!()
        };
        rgba[0] ^= 1;
        assert!(rgba_tamper
            .validate()
            .unwrap_err()
            .contains("command identity SHA-256 mismatch"));

        let mut layout_tamper = production_frame_with_exact_ui_pass();
        let command = exact_ui_command_mut(&mut layout_tamper, 1);
        let HudCommandExecution::InterpretedPaint {
            paint: HudPaintCommand::Text { layout_inputs, .. },
        } = &mut command.execution
        else {
            unreachable!()
        };
        let TextLayoutInputs::AuthoredReproducible { wrap, .. } = layout_inputs else {
            unreachable!()
        };
        *wrap = !*wrap;
        assert!(layout_tamper
            .validate()
            .unwrap_err()
            .contains("command identity SHA-256 mismatch"));

        let mut pass_hash_tamper = production_frame_with_exact_ui_pass();
        let target = pass_hash_tamper
            .authenticated_ui_pass
            .as_ref()
            .unwrap()
            .target
            .clone();
        let command = exact_ui_command_mut(&mut pass_hash_tamper, 1);
        let HudCommandExecution::InterpretedPaint {
            paint: HudPaintCommand::Text { font_bindings, .. },
        } = &mut command.execution
        else {
            unreachable!()
        };
        font_bindings[0].font_identity.push_str("-tampered");
        command.command_identity_sha256 = command.computed_identity_sha256(&target).unwrap();
        assert!(pass_hash_tamper
            .validate()
            .unwrap_err()
            .contains("UI-pass identity SHA-256 mismatch"));

        let mut duplicate_font = production_frame_with_exact_ui_pass();
        let command = exact_ui_command_mut(&mut duplicate_font, 1);
        let HudCommandExecution::InterpretedPaint {
            paint: HudPaintCommand::Text { font_bindings, .. },
        } = &mut command.execution
        else {
            unreachable!()
        };
        font_bindings[1].font_identity = font_bindings[0].font_identity.clone();
        rehash_exact_ui_command_and_pass(&mut duplicate_font, 1);
        assert!(duplicate_font
            .validate()
            .unwrap_err()
            .contains("font binding identity/atlas list is invalid"));

        let mut duplicate_index = production_frame_with_exact_ui_pass();
        let command = exact_ui_command_mut(&mut duplicate_index, 1);
        let HudCommandExecution::InterpretedPaint {
            paint: HudPaintCommand::Text { font_bindings, .. },
        } = &mut command.execution
        else {
            unreachable!()
        };
        font_bindings[1].definition_content_index = 2;
        rehash_exact_ui_command_and_pass(&mut duplicate_index, 1);
        assert!(duplicate_index
            .validate()
            .unwrap_err()
            .contains("content indices are not distinct"));

        let mut missing_font = production_frame_with_exact_ui_pass();
        let command = exact_ui_command_mut(&mut missing_font, 1);
        let HudCommandExecution::InterpretedPaint {
            paint: HudPaintCommand::Text { font_bindings, .. },
        } = &mut command.execution
        else {
            unreachable!()
        };
        font_bindings.clear();
        rehash_exact_ui_command_and_pass(&mut missing_font, 1);
        assert!(missing_font
            .validate()
            .unwrap_err()
            .contains("no exact font binding"));

        let mut missing_icon = production_frame_with_exact_ui_pass();
        let command = exact_ui_command_mut(&mut missing_icon, 1);
        let HudCommandExecution::InterpretedPaint {
            paint:
                HudPaintCommand::Text {
                    inline_icon_bindings,
                    ..
                },
        } = &mut command.execution
        else {
            unreachable!()
        };
        inline_icon_bindings.clear();
        rehash_exact_ui_command_and_pass(&mut missing_icon, 1);
        assert!(missing_icon
            .validate()
            .unwrap_err()
            .contains("content order/cover is not exact"));
    }

    #[test]
    fn m03_a48_portable_row_round_trips_and_rejects_join_proof_and_vfs_tamper() {
        let frame = production_frame_with_m03_a48_row();
        let encoded = frame.encode_json().unwrap();
        assert_eq!(ProductionRenderFrame::decode_json(&encoded).unwrap(), frame);

        let mut wrong_join = frame.clone();
        wrong_join.m03_arrows.as_mut().unwrap().draws[0].members[0].member_ordinal = 1;
        assert!(wrong_join
            .validate()
            .unwrap_err()
            .contains("draw-member order/range/key"));

        let mut wrong_proof = frame.clone();
        wrong_proof
            .m03_arrows
            .as_mut()
            .unwrap()
            .static_semantic_proof_sha256 = "e".repeat(64);
        assert!(wrong_proof
            .validate()
            .unwrap_err()
            .contains("promoted a729 static proof"));

        let mut wrong_vfs = frame.clone();
        wrong_vfs.m03_arrows.as_mut().unwrap().used_content[0].effective_vfs_manifest_sha256 =
            "f".repeat(64);
        assert!(wrong_vfs
            .validate()
            .unwrap_err()
            .contains("content authority"));

        let mut nested_wrong_vfs = frame;
        let draw = &mut nested_wrong_vfs.m03_arrows.as_mut().unwrap().draws[0].draw;
        draw.shader_program.source.effective_vfs_manifest_sha256 = "f".repeat(64);
        draw.payload_sha256 = draw.computed_payload_sha256().unwrap();
        assert!(nested_wrong_vfs
            .validate()
            .unwrap_err()
            .contains("draw content authority"));
    }

    fn canonical_particle_packet(
        matrix: [u32; 16],
        live_count: u32,
        max_count: u32,
        elapsed_f32_bits: u32,
        flags_u16: u16,
        packet_ordinal: u32,
    ) -> Vec<u8> {
        let mut packet = vec![0; 112];
        for (dst, bits) in packet[..0x40].chunks_exact_mut(4).zip(matrix) {
            dst.copy_from_slice(&bits.to_le_bytes());
        }
        packet[0x40..0x44].copy_from_slice(&live_count.to_le_bytes());
        packet[0x50..0x54].copy_from_slice(&48u32.to_le_bytes());
        packet[0x54..0x58].copy_from_slice(&max_count.to_le_bytes());
        packet[0x58..0x5c].copy_from_slice(&elapsed_f32_bits.to_le_bytes());
        packet[0x5c..0x5e].copy_from_slice(&flags_u16.to_le_bytes());
        packet[0x60..0x64].copy_from_slice(&packet_ordinal.to_le_bytes());
        packet
    }

    #[test]
    fn exact_packet_and_palette_round_trip() {
        let palette = vec![7; 3200];
        let canonical_packet = canonical_particle_packet([0; 16], 2, 4, 0, 0, 4);
        let mut frame = ProductionRenderFrame {
            schema: SCHEMA.into(),
            authority: RenderAuthority {
                process_id: 7,
                process_start_nonce: 8,
                observer_installed_qpc: 9,
                d3d_device_created_qpc: 10,
                first_creation_sequence: 1,
                hook_epoch_nonce: 11,
                hook_set_receipt_sha256: "a".repeat(64),
                capture_receipt_sha256: "b".repeat(64),
                launch_epoch_sha256: compute_render_launch_epoch_sha256(
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
                particle_runtime_mode: ParticleRuntimeMode::SubmittedPacketReplay,
            },
            update_index: 16_959_360,
            first_sequence: 40,
            last_sequence: 42,
            presentation_qpc_frequency_hz: 10_000_000,
            display_submit_index: Some("11".into()),
            selected_present: SelectedPresentReceipt::sealed(
                1,
                "c".repeat(64),
                11,
                0,
                60_759_359,
                500,
                600,
            ),
            submits: vec![RenderSubmit {
                submit_index: 11,
                global_order: 1,
                presentation_qpc_ticks: 500,
                interpolation_phase_f32_bits: 0,
                ordered_scene_constant_buffers: vec![constant_buffer()],
                ordered_static_map_programs: vec![],
                particle_packet_set_sha256: "5".repeat(64),
                particles: vec![ParticlePacket {
                    effect_instance_id: 3,
                    registration_token: "smoke".into(),
                    resolved_definition_sha256: "c".repeat(64),
                    content_identity: particle_content_identity(4),
                    content_identity_sha256: particle_content_identity(4)
                        .computed_sha256()
                        .unwrap(),
                    bucket: 2,
                    packet_ordinal: 4,
                    draw_order: 5,
                    ribbon_tail_order: None,
                    canonical_packet_112_bytes: canonical_packet.clone(),
                    canonical_packet_sha256: hash(&canonical_packet),
                    packet_matrix_4x4_f32_bits: [0; 16],
                    live_count: 2,
                    record_stride: 48,
                    max_count: 4,
                    ordered_gpu_instance_records: vec![0; 96],
                    gpu_instance_records_sha256: hash(&vec![0; 96]),
                    elapsed_f32_bits: 0,
                    shader_program: shader(),
                    ordered_texture_bindings: vec![texture()],
                    pipeline: pipeline(),
                    geometry: particle_geometry(2),
                    ordered_constant_buffers: vec![constant_buffer()],
                    output_state: output_state(),
                    atlas_rows: 1,
                    atlas_columns: 1,
                    scale_f32_bits: 0,
                    flags_u16: 0,
                }],
                effect_lifecycle_events: vec![],
                entity_presentations: vec![],
                light_lifecycle_events: vec![],
                entities: vec![EntitySubmission {
                    pass_ordinal: 1,
                    draw_order: 2,
                    entity_instance_id: 8,
                    resolved_entity_sha256: "c".repeat(64),
                    content_identity: entity_content_identity(),
                    shader_program: shader(),
                    ordered_texture_bindings: vec![texture()],
                    pipeline: pipeline(),
                    geometry: entity_geometry(),
                    ordered_constant_buffers: vec![constant_buffer()],
                    output_state: output_state(),
                    world_transform_4x4_f32_bits: [0; 16],
                    state_flags: 3 | (4 << 8) | (7 << 16) | (6 << 24),
                    skinning: Skinning::Palette {
                        palette_sha256: hash(&palette),
                        palette_3200_bytes: palette,
                    },
                }],
                ordered_draws: vec![
                    RenderOutputRef::Entity {
                        pass_ordinal: 1,
                        entity_instance_id: 8,
                        draw_order: 2,
                    },
                    RenderOutputRef::Particle {
                        effect_instance_id: 3,
                        bucket: 2,
                        packet_ordinal: 4,
                        draw_order: 5,
                    },
                ],
                ordered_render_operations: Vec::new(),
                direct_visuals: vec![],
                tiled_lights: vec![],
                weather_membership: vec![],
                political_textures: vec![],
            }],
            m02_combat: None,
            m03_arrows: None,
            o04_cities: None,
            authenticated_ui_pass: None,
            checkpoint: empty_checkpoint(42, 16_959_360, Some("11")),
        };
        let gs = vec![9, 10, 11, 12];
        frame.submits[0].particles[0].shader_program.geometry_shader =
            Some(AdditionalShaderStage {
                entry_token: "GeometryParticle".into(),
                compiled: CompiledShaderPayload {
                    compiler_identity: "fxc-10.1".into(),
                    target_profile: "gs_5_0".into(),
                    compiler_flags_u64: 0,
                    sha256: hash(&gs),
                    bytecode: gs,
                },
            });
        frame.submits[0].particles[0].geometry.stream_output_targets =
            vec![StreamOutputTargetBinding {
                slot: 2,
                buffer_identity: "particle-stream-output-2".into(),
                buffer_byte_width: 4096,
                byte_offset: u32::MAX,
            }];
        let static_texture = texture();
        let mut static_program = StaticMapPassProgram {
            program_ordinal: 7,
            pass_token: format!("opaque-pass:{}", "b".repeat(64)),
            ordered_content_keys: vec![
                "gfx/fx/particle.shader".into(),
                "gfx/textures/particle.dds".into(),
            ],
            ordered_operations: vec![
                StaticMapOperation {
                    operation_ordinal: 100,
                    payload_sha256: String::new(),
                    command: StaticMapCommand::ClearColor {
                        target: output_state().ordered_color_targets[0].clone(),
                        rgba_f32_bits: [0; 4],
                    },
                },
                StaticMapOperation {
                    operation_ordinal: 101,
                    payload_sha256: String::new(),
                    command: StaticMapCommand::Draw {
                        draw: StaticMapDraw {
                            draw_token: "terrain-chunk-0".into(),
                            shader_program: shader(),
                            ordered_shader_resources: vec![StaticShaderResourceBinding {
                                stage: static_texture.stage,
                                slot: static_texture.slot,
                                semantic_token: static_texture.semantic_token,
                                source: StaticShaderResourceSource::UsedContent {
                                    content: static_texture.content,
                                    resource: static_texture.resource,
                                    view: static_texture.view,
                                },
                            }],
                            pipeline: pipeline(),
                            geometry: StaticGeometrySource::Exact {
                                geometry: entity_geometry(),
                            },
                            ordered_constant_buffers: vec![constant_buffer()],
                            output_state: output_state(),
                            payload_sha256: String::new(),
                        },
                    },
                },
            ],
            payload_sha256: String::new(),
        };
        static_program.seal_payload_hashes().unwrap();
        frame.submits[0].ordered_static_map_programs = vec![static_program];
        frame.submits[0].ordered_render_operations = vec![
            RenderExecutionRef::Static {
                operation_ordinal: 100,
            },
            RenderExecutionRef::Static {
                operation_ordinal: 101,
            },
            RenderExecutionRef::Runtime {
                output: frame.submits[0].ordered_draws[0].clone(),
            },
            RenderExecutionRef::Runtime {
                output: frame.submits[0].ordered_draws[1].clone(),
            },
        ];
        let encoded = frame.encode_json().unwrap();
        let decoded = ProductionRenderFrame::decode_json(&encoded).unwrap();
        assert_eq!(decoded, frame);
        assert_eq!(
            decoded.submits[0].entities[0]
                .content_identity
                .native_selection
                .pass_selector_i32,
            -5
        );
        assert!(decoded.submits[0].ordered_static_map_programs[0]
            .pass_token
            .starts_with("opaque-pass:"));
        assert_eq!(
            decoded.submits[0].particles[0]
                .geometry
                .stream_output_targets[0]
                .byte_offset,
            u32::MAX
        );
        let mut authority_tampered = decoded.clone();
        authority_tampered.authority.process_start_nonce += 1;
        assert!(authority_tampered
            .validate()
            .unwrap_err()
            .contains("launch-epoch SHA-256 mismatch"));
        let mut identity_tampered = decoded.clone();
        identity_tampered.submits[0].ordered_static_map_programs[0].pass_token =
            "opaque-pass:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff".into();
        assert!(identity_tampered
            .validate()
            .unwrap_err()
            .contains("program payload SHA-256 mismatch"));
        let mut selector_tampered = decoded.clone();
        selector_tampered.submits[0].entities[0]
            .content_identity
            .native_selection
            .mesh_draw_list_index += 1;
        assert!(selector_tampered
            .validate()
            .unwrap_err()
            .contains("selector-native identity SHA-256 mismatch"));
        let mut asset_substituted = decoded.clone();
        asset_substituted.submits[0].entities[0].resolved_entity_sha256 = "1".repeat(64);
        assert!(asset_substituted
            .validate()
            .unwrap_err()
            .contains("does not match admitted asset payload hash"));
        let mut flags_substituted = decoded.clone();
        flags_substituted.submits[0].entities[0].state_flags ^= 1 << 16;
        assert!(flags_substituted
            .validate()
            .unwrap_err()
            .contains("do not equal the selector-native packed tuple"));
        let mut particle_identity_tampered = decoded.clone();
        particle_identity_tampered.submits[0].particles[0]
            .content_identity
            .subsystem_selector_u32 += 1;
        assert!(particle_identity_tampered
            .validate()
            .unwrap_err()
            .contains("particle content identity SHA-256 mismatch"));
        let mut texture_substituted = decoded.clone();
        texture_substituted.submits[0].particles[0]
            .content_identity
            .texture_content = content("gfx/textures/substitute.dds");
        texture_substituted.submits[0].particles[0].content_identity_sha256 =
            texture_substituted.submits[0].particles[0]
                .content_identity
                .computed_sha256()
                .unwrap();
        assert!(texture_substituted
            .validate()
            .unwrap_err()
            .contains("selected texture winner is absent"));
        let mut half_pixel_substituted = decoded.clone();
        half_pixel_substituted.submits[0].particles[0]
            .content_identity
            .atlas_half_pixel_width_f32_bits ^= 1;
        half_pixel_substituted.submits[0].particles[0].content_identity_sha256 =
            half_pixel_substituted.submits[0].particles[0]
                .content_identity
                .computed_sha256()
                .unwrap();
        assert!(half_pixel_substituted
            .validate()
            .unwrap_err()
            .contains("half-pixel bits do not match"));
        let mut tampered = decoded;
        tampered.submits[0].ordered_static_map_programs[0].ordered_operations[0].payload_sha256 =
            "0".repeat(64);
        tampered.submits[0].ordered_static_map_programs[0].payload_sha256 = tampered.submits[0]
            .ordered_static_map_programs[0]
            .computed_payload_sha256()
            .unwrap();
        assert!(tampered
            .validate()
            .unwrap_err()
            .contains("operation payload SHA-256 mismatch"));
        assert!(String::from_utf8(encoded)
            .unwrap()
            .contains("\"submit_index\":\"11\""));
    }

    #[test]
    fn v2_carrier_fails_with_explicit_no_exact_conversion_error() {
        let error =
            ProductionRenderFrame::decode_json(br#"{"schema":"hoi4.production-render-submit.v2"}"#)
                .unwrap_err();
        assert!(error.contains("incompatible"));
        assert!(error.contains("exact conversion is unavailable"));
        assert!(error.contains("selector-native entity and particle authority"));
    }

    #[test]
    fn particle_identity_order_and_duplicates_are_preserved_and_hashed() {
        let first = particle_content_identity(4);
        let second = particle_content_identity(9);
        let ordered = vec![first.clone(), first.clone(), second.clone()];
        let encoded = serde_json::to_vec(&ordered).unwrap();
        let decoded: Vec<ParticleContentIdentity> = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(decoded, ordered);
        assert_eq!(decoded[0], decoded[1]);
        let reordered = vec![second, first.clone(), first];
        assert_ne!(
            canonical_json_sha256(&ordered).unwrap(),
            canonical_json_sha256(&reordered).unwrap()
        );
    }

    #[test]
    fn semantic_entity_selector_labels_are_rejected_by_v3() {
        let mut value = serde_json::to_value(entity_content_identity()).unwrap();
        value.as_object_mut().unwrap().insert(
            "material_variant_token".into(),
            serde_json::json!("invented"),
        );
        let error = serde_json::from_value::<EntityContentIdentity>(value).unwrap_err();
        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn semantic_render_target_aliases_are_rejected_by_v3() {
        for legacy in [
            r#"{"kind":"main_scene_color"}"#,
            r#"{"kind":"main_scene_depth"}"#,
            r#"{"kind":"post_process_color","producer_pass_token":"x","attachment_ordinal":0}"#,
            r#"{"kind":"named_render_graph_resource","resource_token":"x","producer_pass_token":"y","attachment_ordinal":0}"#,
        ] {
            assert!(serde_json::from_str::<PreparedRenderTarget>(legacy).is_err());
        }
        assert_eq!(
            serde_json::from_str::<PreparedRenderTarget>(
                r#"{"kind":"opaque_render_target","attachment_ordinal":7}"#,
            )
            .unwrap(),
            PreparedRenderTarget::OpaqueRenderTarget {
                attachment_ordinal: 7
            }
        );
    }

    #[test]
    fn rejects_semantic_or_truncated_substitutes() {
        let mut truncated_packet = canonical_particle_packet([0; 16], 1, 1, 0, 0, 0);
        truncated_packet.pop();
        let frame = ProductionRenderFrame {
            schema: SCHEMA.into(),
            authority: RenderAuthority {
                process_id: 7,
                process_start_nonce: 8,
                observer_installed_qpc: 9,
                d3d_device_created_qpc: 10,
                first_creation_sequence: 1,
                hook_epoch_nonce: 11,
                hook_set_receipt_sha256: "a".repeat(64),
                capture_receipt_sha256: "b".repeat(64),
                launch_epoch_sha256: compute_render_launch_epoch_sha256(
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
                particle_runtime_mode: ParticleRuntimeMode::SubmittedPacketReplay,
            },
            update_index: 16_959_360,
            first_sequence: 0,
            last_sequence: 0,
            presentation_qpc_frequency_hz: 10_000_000,
            display_submit_index: Some("1".into()),
            selected_present: SelectedPresentReceipt::sealed(
                1,
                "c".repeat(64),
                1,
                0,
                60_759_359,
                1,
                2,
            ),
            submits: vec![RenderSubmit {
                submit_index: 1,
                global_order: 1,
                presentation_qpc_ticks: 1,
                interpolation_phase_f32_bits: 0,
                ordered_scene_constant_buffers: vec![],
                ordered_static_map_programs: vec![],
                particle_packet_set_sha256: "5".repeat(64),
                particles: vec![ParticlePacket {
                    effect_instance_id: 1,
                    registration_token: "x".into(),
                    resolved_definition_sha256: "c".repeat(64),
                    content_identity: particle_content_identity(0),
                    content_identity_sha256: particle_content_identity(0)
                        .computed_sha256()
                        .unwrap(),
                    bucket: 0,
                    packet_ordinal: 0,
                    draw_order: 0,
                    ribbon_tail_order: None,
                    canonical_packet_112_bytes: truncated_packet.clone(),
                    canonical_packet_sha256: hash(&truncated_packet),
                    packet_matrix_4x4_f32_bits: [0; 16],
                    live_count: 1,
                    record_stride: 48,
                    max_count: 1,
                    ordered_gpu_instance_records: vec![0; 48],
                    gpu_instance_records_sha256: hash(&vec![0; 48]),
                    elapsed_f32_bits: 0,
                    shader_program: shader(),
                    ordered_texture_bindings: vec![texture()],
                    pipeline: pipeline(),
                    geometry: particle_geometry(1),
                    ordered_constant_buffers: vec![constant_buffer()],
                    output_state: output_state(),
                    atlas_rows: 1,
                    atlas_columns: 1,
                    scale_f32_bits: 0,
                    flags_u16: 0,
                }],
                effect_lifecycle_events: vec![],
                entity_presentations: vec![],
                light_lifecycle_events: vec![],
                entities: vec![],
                ordered_draws: vec![RenderOutputRef::Particle {
                    effect_instance_id: 1,
                    bucket: 0,
                    packet_ordinal: 0,
                    draw_order: 0,
                }],
                ordered_render_operations: Vec::new(),
                direct_visuals: vec![],
                tiled_lights: vec![],
                weather_membership: vec![],
                political_textures: vec![],
            }],
            m02_combat: None,
            m03_arrows: None,
            o04_cities: None,
            authenticated_ui_pass: None,
            checkpoint: empty_checkpoint(0, 16_959_360, Some("1")),
        };
        assert!(frame.validate().unwrap_err().contains("112"));
    }

    #[test]
    fn exact_render_target_extension_covers_persistent_nonzero_mip_and_mrt() {
        fn exact_target(attachment_ordinal: u32, persistent: bool) -> RenderTargetDesc {
            let mut persistent_key = content(&format!(
                "gfx/modded/runtime_target_{attachment_ordinal}.dds"
            ));
            persistent_key.payload_bytes = 16;
            RenderTargetDesc {
                prepared_target: PreparedRenderTarget::OpaqueRenderTarget { attachment_ordinal },
                width: 2,
                height: 2,
                dxgi_format_u32: 28,
                mip_level: 1,
                sample_count: 1,
                sample_quality: 0,
                persistent_content: persistent.then(|| persistent_key.clone()),
                exact_resource: Some(RenderTargetExactResource {
                    resource: RenderTargetResourceDesc {
                        dimension: TextureResourceDimension::Texture2d,
                        width: 4,
                        height: 4,
                        depth: 1,
                        mip_levels: 3,
                        array_size: 1,
                        dxgi_format_u32: 28,
                        sample_count: 1,
                        sample_quality: 0,
                        usage_u32: 0,
                        bind_flags_u32: 0x20,
                        cpu_access_flags_u32: 0,
                        misc_flags_u32: 0,
                    },
                    ordered_views: vec![RenderTargetViewDesc::RenderTarget {
                        dxgi_format_u32: 28,
                        view: RenderTargetAttachmentView::Texture2d { mip_slice: 1 },
                    }],
                    ordered_initial_subresources: persistent
                        .then(|| RenderTargetInitialSubresource {
                            subresource_ordinal: 1,
                            row_pitch: 8,
                            slice_pitch: 16,
                            source: RenderTargetInitialSource::UsedContentRange {
                                content: persistent_key,
                                source_byte_offset: 0,
                                byte_count: 16,
                                range_sha256: "d".repeat(64),
                            },
                        })
                        .into_iter()
                        .collect(),
                }),
            }
        }

        let output = RenderOutputState {
            ordered_color_targets: vec![exact_target(3, true), exact_target(4, false)],
            depth_target: None,
            viewport: ViewportDesc {
                top_left_x_f32_bits: 0.0f32.to_bits(),
                top_left_y_f32_bits: 0.0f32.to_bits(),
                width_f32_bits: 2.0f32.to_bits(),
                height_f32_bits: 2.0f32.to_bits(),
                min_depth_f32_bits: 0.0f32.to_bits(),
                max_depth_f32_bits: 1.0f32.to_bits(),
            },
            scissor: None,
        };
        output.validate().unwrap();

        let mut missing_exact = output.clone();
        missing_exact.ordered_color_targets[0].exact_resource = None;
        assert!(missing_exact
            .validate()
            .unwrap_err()
            .contains("persistent/nonzero-mip"));

        let mut wrong_view = output;
        if let Some(exact) = &mut wrong_view.ordered_color_targets[0].exact_resource {
            exact.ordered_views[0] = RenderTargetViewDesc::RenderTarget {
                dxgi_format_u32: 28,
                view: RenderTargetAttachmentView::Texture2d { mip_slice: 0 },
            };
        }
        assert!(wrong_view
            .validate()
            .unwrap_err()
            .contains("selected render-target format/mip"));
    }

    fn empty_checkpoint(
        last_sequence: u64,
        last_update: u64,
        last_submit: Option<&str>,
    ) -> RenderCheckpoint {
        RenderCheckpoint {
            last_sequence,
            last_update_index: last_update,
            last_submit_index: last_submit.map(str::to_string),
            next_stable_ids: BTreeMap::new(),
            active_effect_lifecycle: vec![],
            active_entity_presentation: vec![],
            active_direct_visual_producers: vec![],
            active_light_lifecycle: vec![],
            latest_tiled_light_payload: None,
            weather_state_and_ordered_membership: vec![],
            active_effect_set_sha256: "6".repeat(64),
            ordered_particle_packets_sha256: "7".repeat(64),
            ordered_entity_submissions_sha256: "8".repeat(64),
            ordered_direct_visual_producers_sha256: "9".repeat(64),
            ordered_light_submissions_sha256: "a".repeat(64),
            weather_membership_sha256: "b".repeat(64),
        }
    }
}
