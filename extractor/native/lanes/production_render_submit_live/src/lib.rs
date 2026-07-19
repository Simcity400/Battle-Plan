//! Authenticated a729 observer for the XN12/XN15 post-simulation particle seam.
//!
//! Runtime addresses are observation joins only. The emitted packet has its
//! process pointer at `+0x48` zeroed and replaces it with the exact ordered GPU
//! instance bytes. Catalog strings are admitted only through a manifest-pinned
//! resolver; an unresolved definition fails the whole submit.

#[cfg(test)]
use portable_content_manifest::normalize_game_relative_path;
use portable_content_manifest::UsedContentKey;
use portable_render_stream::{
    BufferSource, ConstantBufferBinding, DirectVisualKind, DirectVisualOperation,
    DirectVisualProducer, DrawCommand, EntityContentIdentity, EntitySubmission, GeometryBinding,
    LightSlotJoin, ParticlePacket, PipelineDesc, RenderExecutionRef, RenderOutputRef,
    RenderOutputState, ShaderProgram, Skinning, StaticMapCommand, StaticMapPassProgram,
    TextureBinding, TiledLightSubmission, WeatherInstance, WeatherMembership, WeatherPosition,
    WeatherResolution,
};
pub use production_render_catalog_capture_contract::{
    correlate_content_creation, materialize_d77_draw_authority, materialize_d80_static_authority,
    materialize_program_identity, RawBlendStateCreation, RawBufferSnapshot, RawCaptureLaunchEpoch,
    RawConstantBufferSemantic, RawContentCreationReceipt, RawD3dContentCreationObservation,
    RawD3dCreatedObjectKind, RawD3dDrawState, RawDepthStencilStateCreation, RawDrawCall,
    RawDrawContentIdentity, RawDrawFamily, RawDrawIdentity, RawDrawSemantics,
    RawEffectiveVfsOpenObservation, RawEffectiveVfsWinnerIndexBinding, RawEffectiveVfsWinnerRow,
    RawIndexBufferObject, RawInputElement, RawInputLayoutCreation, RawLiveDrawAuthorityObservation,
    RawOutputTargetSemantic, RawOutputViewKind, RawPayloadSnapshot, RawPayloadSource,
    RawPreparedTarget, RawProcessEpoch, RawProgramIdentity, RawProgramRegistrationObservation,
    RawRasterizerStateCreation, RawReflectedConstantBufferVariable, RawReflectedResourceKind,
    RawReflectedStageSemantics, RawRegisteredRenderTargetKind, RawRenderCatalogCapture,
    RawRenderTargetBlendDesc, RawRenderTargetRegistrationObservation,
    RawRenderTargetRegistrationReceipt, RawRenderTargetViewSnapshot, RawResourceViewDesc,
    RawSamplerSemantic, RawSamplerStateCreation, RawShaderBytecodeReflectionObservation,
    RawShaderCompilerInvocation, RawShaderCreation, RawShaderDefine, RawShaderReflectedResource,
    RawShaderResourceViewSnapshot, RawShaderSignatureParameter, RawShaderStage, RawStageSlotObject,
    RawStaticAuthorityMaterialization, RawStaticAuthorityObservation, RawStaticContentJoin,
    RawStaticD3dDrawState, RawStaticDrawCall, RawStaticOrderOperation, RawStaticProgramCapture,
    RawStaticProgramCaptureSink, RawStaticShaderResourceSemantic, RawStaticShaderResourceSource,
    RawStaticStreamOutputBuffer, RawStaticTextureSourceObservation, RawStencilFaceDesc,
    RawTexture2dDesc, RawTextureObject, RawTextureResourceSnapshot, RawTextureSemantic,
    RawTextureSubresourceSnapshot, RawVertexBufferObject, RawVfsContentIdentity,
    GENERIC_TEXTURE_RTV_PRODUCER_RVA, WINDOW_BACKBUFFER_RTV_PRODUCER_RVA,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Mutex, OnceLock};

#[cfg(windows)]
pub mod a729_hook;
mod authority_producer;
mod observed_live_authority;
mod runtime_content_interpreter;
#[cfg(windows)]
pub mod shader_reflection;
pub use authority_producer::*;
pub use observed_live_authority::*;
pub use runtime_content_interpreter::*;

pub const GLOBAL_RENDER_ENVELOPE_RVA: u32 = 0x0222_8DF0;
pub const PARTICLE_OBJECT_CTOR_RVA: u32 = 0x0227_EA10;
pub const PARTICLE_BUCKET_SUBMIT_RVA: u32 = 0x0227_ECD0;
pub const PARTICLE_RUNTIME_CREATE_RVA: u32 = 0x023E_8190;
pub const PARTICLE_RUNTIME_DESTROY_RVA: u32 = 0x023E_8400;
pub const PARTICLE_PACKET_BUILDER_RVA: u32 = 0x023E_8B90;
pub const PARTICLE_RENDER_SUBMIT_RVA: u32 = 0x0227_F420;
pub const PARTICLE_FINAL_DRAW_RVA: u32 = 0x023F_E970;
pub const ENTITY_FACTORY_RVA: u32 = 0x0227_7DF0;
pub const ENTITY_DESTROY_RVA: u32 = 0x0227_87C0;
pub const ENTITY_RENDER_ROOT_RVA: u32 = 0x0226_44C0;
pub const ENTITY_FINAL_DRAW_RVA: u32 = 0x023F_E9B0;
pub const INDEXED_INSTANCED_FINAL_DRAW_RVA: u32 = 0x023F_E9F0;
pub const INSTANCED_FINAL_DRAW_RVA: u32 = 0x023F_EA40;
/// Combined a729 clear wrapper. Flag bit 0 emits ClearRenderTargetView first;
/// flag bits 1/2 then emit ClearDepthStencilView for depth/stencil.
pub const RENDER_TARGET_CLEAR_RVA: u32 = 0x023F_CA30;
pub const SHADER_PROGRAM_BINDER_RVA: u32 = 0x0240_0520;
/// Engine shader/effect lookup used by the authenticated static-map and
/// post-process registration roots below.
pub const ENGINE_SHADER_EFFECT_LOOKUP_RVA: u32 = 0x023F_4EC0;
pub const MAP_EFFECT_REGISTRATION_ROOT_RVA: u32 = 0x0126_1100;
pub const RAILROAD_EFFECT_REGISTRATION_ROOT_RVA: u32 = 0x0159_B9B0;
pub const MAP_NAME_EFFECT_REGISTRATION_ROOT_RVA: u32 = 0x00B4_0890;
pub const RESTORE_BLOOM_EFFECT_REGISTRATION_ROOT_RVA: u32 = 0x00FF_1410;
pub const LUT_BLEND_EFFECT_REGISTRATION_ROOT_RVA: u32 = 0x0126_F590;
/// Generic PdxMesh effect registration used by both static map objects and
/// dynamic entity models. The shared lookup is disambiguated by the
/// role-specific effective-VFS catalog, never by the path alone.
pub const PDXMESH_EFFECT_REGISTRATION_ROOT_RVA: u32 = 0x0224_BA50;
pub const MAP_SCENE_RENDER_ROOT_RVA: u32 = 0x00B4_81A0;
pub const POST_PROCESS_RENDER_ROOT_RVA: u32 = 0x00FF_3450;
pub const PRESENT_RENDER_ROOT_RVA: u32 = 0x020B_6530;
pub const WEATHER_VISUAL_PRODUCER_RVA: u32 = 0x0166_EC50;
pub const HIGH_RESOLUTION_SEED_RVA: u32 = 0x024D_BE60;

pub const PACKET_BYTES: usize = 0x70;
pub const GPU_INSTANCE_STRIDE: u32 = 0x30;
pub const PACKET_POINTER_OFFSET: usize = 0x48;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RenderOrderBoundary {
    MapSceneBegin,
    MapSceneEnd,
    PostProcessBegin,
    PostProcessEnd,
    PresentBegin,
    PresentEnd,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinalDrawCall {
    Draw,
    DrawIndexed,
    DrawInstanced,
    DrawIndexedInstanced,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinalDrawFamily {
    StaticOrPostProcess,
    Particle,
    Entity,
}

/// One exact call-order event from the a729 map/post-process/present envelope.
/// The four integer arguments retain the native final-draw wrapper inputs;
/// unused fields are zero. Process pointers remain extractor-local evidence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case", deny_unknown_fields)]
pub enum RawRenderOrderEvent {
    Boundary {
        sequence: u64,
        kind: RenderOrderBoundary,
        producer_rva: u32,
    },
    FinalDraw {
        sequence: u64,
        call: FinalDrawCall,
        family: FinalDrawFamily,
        producer_rva: u32,
        render_context_ptr: usize,
        shader_program_ptr: usize,
        arguments: [u32; 4],
    },
    ClearColor {
        sequence: u64,
        /// Dedicated catalog-observation sequence for the exact clear-time
        /// RTV view-to-resource snapshot. This is not inferred from order.
        capture_sequence: u64,
        producer_rva: u32,
        render_target_object_ptr: usize,
        render_target_view_ptr: usize,
        rgba_f32_bits: [u32; 4],
    },
    ClearDepthStencil {
        sequence: u64,
        /// Dedicated catalog-observation sequence for the exact clear-time
        /// DSV view-to-resource snapshot. This is not inferred from order.
        capture_sequence: u64,
        producer_rva: u32,
        render_target_object_ptr: usize,
        depth_stencil_view_ptr: usize,
        clear_depth: bool,
        clear_stencil: bool,
        depth_f32_bits: u32,
        stencil_u8: u8,
    },
}

impl RawRenderOrderEvent {
    pub fn sequence(&self) -> u64 {
        match self {
            Self::Boundary { sequence, .. }
            | Self::FinalDraw { sequence, .. }
            | Self::ClearColor { sequence, .. }
            | Self::ClearDepthStencil { sequence, .. } => *sequence,
        }
    }

    pub fn validate(&self) -> Result<(), PromotionError> {
        if self.sequence() == 0 {
            return invalid("render-order event has zero sequence");
        }
        match self {
            Self::Boundary {
                kind, producer_rva, ..
            } => {
                let expected = match kind {
                    RenderOrderBoundary::MapSceneBegin | RenderOrderBoundary::MapSceneEnd => {
                        MAP_SCENE_RENDER_ROOT_RVA
                    }
                    RenderOrderBoundary::PostProcessBegin | RenderOrderBoundary::PostProcessEnd => {
                        POST_PROCESS_RENDER_ROOT_RVA
                    }
                    RenderOrderBoundary::PresentBegin | RenderOrderBoundary::PresentEnd => {
                        PRESENT_RENDER_ROOT_RVA
                    }
                };
                if *producer_rva != expected {
                    return invalid("render-order boundary has the wrong producer RVA");
                }
            }
            Self::FinalDraw {
                call,
                family,
                producer_rva,
                render_context_ptr,
                shader_program_ptr,
                arguments,
                ..
            } => {
                let expected = match call {
                    FinalDrawCall::Draw => PARTICLE_FINAL_DRAW_RVA,
                    FinalDrawCall::DrawIndexed => ENTITY_FINAL_DRAW_RVA,
                    FinalDrawCall::DrawIndexedInstanced => INDEXED_INSTANCED_FINAL_DRAW_RVA,
                    FinalDrawCall::DrawInstanced => INSTANCED_FINAL_DRAW_RVA,
                };
                if *producer_rva != expected
                    || *render_context_ptr == 0
                    || *shader_program_ptr == 0
                    || arguments[0] == 0
                    || (matches!(family, FinalDrawFamily::Particle)
                        && !matches!(call, FinalDrawCall::Draw))
                    || (matches!(family, FinalDrawFamily::Entity)
                        && !matches!(call, FinalDrawCall::DrawIndexed))
                {
                    return invalid("final-draw order event is incomplete or mismatched");
                }
            }
            Self::ClearColor {
                capture_sequence,
                producer_rva,
                render_target_object_ptr,
                render_target_view_ptr,
                ..
            } => {
                if *capture_sequence == 0
                    || *producer_rva != RENDER_TARGET_CLEAR_RVA
                    || *render_target_object_ptr == 0
                    || *render_target_view_ptr == 0
                {
                    return invalid("color-clear order event is incomplete or mismatched");
                }
            }
            Self::ClearDepthStencil {
                capture_sequence,
                producer_rva,
                render_target_object_ptr,
                depth_stencil_view_ptr,
                clear_depth,
                clear_stencil,
                depth_f32_bits,
                stencil_u8,
                ..
            } => {
                if *capture_sequence == 0
                    || *producer_rva != RENDER_TARGET_CLEAR_RVA
                    || *render_target_object_ptr == 0
                    || *depth_stencil_view_ptr == 0
                    || (!*clear_depth && !*clear_stencil)
                    || (*clear_depth && *depth_f32_bits != 1.0f32.to_bits())
                    || *stencil_u8 != 0
                {
                    return invalid("depth/stencil-clear order event is incomplete or mismatched");
                }
            }
        }
        Ok(())
    }
}

/// Validates a drained event stream as one contiguous native order. This does
/// not fabricate `RenderExecutionRef` rows: the static catalog join consumes
/// these exact events and must account for every final draw between the phase
/// boundaries before it may build the portable unified order.
pub fn validate_render_order_events(raw: &[RawRenderOrderEvent]) -> Result<(), PromotionError> {
    if raw.is_empty() {
        return invalid("render-order event stream is empty");
    }
    let mut prior = None;
    let mut boundary_state = 0u8;
    for event in raw {
        event.validate()?;
        if prior.is_some_and(|value| event.sequence() != value + 1) {
            return invalid("render-order event stream is repeated or gapped");
        }
        prior = Some(event.sequence());
        match event {
            RawRenderOrderEvent::Boundary { kind, .. } => {
                let expected = match boundary_state {
                    0 => RenderOrderBoundary::MapSceneBegin,
                    1 => RenderOrderBoundary::MapSceneEnd,
                    2 => RenderOrderBoundary::PostProcessBegin,
                    3 => RenderOrderBoundary::PostProcessEnd,
                    4 => RenderOrderBoundary::PresentBegin,
                    5 => RenderOrderBoundary::PresentEnd,
                    _ => unreachable!(),
                };
                if *kind != expected {
                    return invalid("render-order phase boundaries are incomplete or out of order");
                }
                boundary_state = (boundary_state + 1) % 6;
            }
            RawRenderOrderEvent::FinalDraw { .. }
            | RawRenderOrderEvent::ClearColor { .. }
            | RawRenderOrderEvent::ClearDepthStencil { .. }
                if !matches!(boundary_state, 1 | 3) =>
            {
                return invalid("render operation is outside the map/post-process phase envelope");
            }
            RawRenderOrderEvent::FinalDraw { .. }
            | RawRenderOrderEvent::ClearColor { .. }
            | RawRenderOrderEvent::ClearDepthStencil { .. } => {}
        }
    }
    if boundary_state != 0 {
        return invalid("render-order event stream ends inside an incomplete frame");
    }
    Ok(())
}

/// Exact process-local join from an observed native operation to the portable
/// static operation catalog. Pointer fields never leave the extractor; the
/// returned ordinal must name the one complete catalog operation represented
/// by the event.
pub trait ExactStaticRenderOperationResolver {
    fn authority_manifest_sha256(&self) -> &str;
    fn resolve_operation_ordinal(&self, event: &RawRenderOrderEvent) -> Option<u32>;
}

/// Produces the sole portable execution order spanning static map draws and
/// clears, runtime particle/entity output, post-process, and the Present
/// boundary. Every static catalog operation and runtime output must be
/// consumed exactly once; unresolved, duplicated, or type-mismatched joins
/// reject the whole frame.
pub fn promote_unified_render_execution_order<R: ExactStaticRenderOperationResolver>(
    raw: &[RawRenderOrderEvent],
    runtime_order: &[RenderOutputRef],
    static_programs: &[StaticMapPassProgram],
    resolver: &R,
) -> Result<Vec<RenderExecutionRef>, PromotionError> {
    validate_render_order_events(raw)?;
    check_sha256(
        resolver.authority_manifest_sha256(),
        "static render operation catalog authority",
    )?;
    let mut static_operations = BTreeMap::new();
    for program in static_programs {
        for operation in &program.ordered_operations {
            if static_operations
                .insert(operation.operation_ordinal, &operation.command)
                .is_some()
            {
                return invalid("static render catalog repeats an operation ordinal");
            }
        }
    }
    if static_operations.is_empty() {
        return invalid("static render catalog is empty");
    }

    let mut output = Vec::new();
    let mut runtime_index = 0usize;
    let mut consumed_static = std::collections::BTreeSet::new();
    for event in raw {
        match event {
            RawRenderOrderEvent::Boundary { .. } => {}
            RawRenderOrderEvent::FinalDraw {
                family: FinalDrawFamily::Particle,
                ..
            } => {
                let Some(value @ RenderOutputRef::Particle { .. }) =
                    runtime_order.get(runtime_index)
                else {
                    return invalid("particle draw has no matching runtime output");
                };
                output.push(RenderExecutionRef::Runtime {
                    output: value.clone(),
                });
                runtime_index += 1;
            }
            RawRenderOrderEvent::FinalDraw {
                family: FinalDrawFamily::Entity,
                ..
            } => {
                let Some(value @ RenderOutputRef::Entity { .. }) = runtime_order.get(runtime_index)
                else {
                    return invalid("entity draw has no matching runtime output");
                };
                output.push(RenderExecutionRef::Runtime {
                    output: value.clone(),
                });
                runtime_index += 1;
            }
            event @ RawRenderOrderEvent::FinalDraw {
                family: FinalDrawFamily::StaticOrPostProcess,
                ..
            } => append_static_execution_ref(
                event,
                resolver,
                &static_operations,
                &mut consumed_static,
                &mut output,
                |command| matches!(command, StaticMapCommand::Draw { .. }),
            )?,
            event @ RawRenderOrderEvent::ClearColor { .. } => append_static_execution_ref(
                event,
                resolver,
                &static_operations,
                &mut consumed_static,
                &mut output,
                |command| matches!(command, StaticMapCommand::ClearColor { .. }),
            )?,
            event @ RawRenderOrderEvent::ClearDepthStencil { .. } => append_static_execution_ref(
                event,
                resolver,
                &static_operations,
                &mut consumed_static,
                &mut output,
                |command| matches!(command, StaticMapCommand::ClearDepthStencil { .. }),
            )?,
        }
    }
    if runtime_index != runtime_order.len() {
        return invalid("runtime execution catalog has unobserved outputs");
    }
    if consumed_static.len() != static_operations.len() {
        return invalid("static render catalog has unobserved operations");
    }
    Ok(output)
}

fn append_static_execution_ref<R: ExactStaticRenderOperationResolver>(
    event: &RawRenderOrderEvent,
    resolver: &R,
    static_operations: &BTreeMap<u32, &StaticMapCommand>,
    consumed_static: &mut std::collections::BTreeSet<u32>,
    output: &mut Vec<RenderExecutionRef>,
    expected_command: impl FnOnce(&StaticMapCommand) -> bool,
) -> Result<(), PromotionError> {
    let ordinal = resolver
        .resolve_operation_ordinal(event)
        .ok_or_else(|| PromotionError::Invalid("native static operation is unresolved".into()))?;
    let command = static_operations.get(&ordinal).ok_or_else(|| {
        PromotionError::Invalid("native static operation names a missing ordinal".into())
    })?;
    if !expected_command(command) || !consumed_static.insert(ordinal) {
        return invalid("native static operation is type-mismatched or repeated");
    }
    output.push(RenderExecutionRef::Static {
        operation_ordinal: ordinal,
    });
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DefinitionIdentity {
    pub registration_token: String,
    pub definition_source_token: String,
    pub registration_hash_u32: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawParticlePacket {
    pub effect_instance_id: u64,
    pub definition: DefinitionIdentity,
    /// Exact first-level render-list selector passed to `0x14227ecd0`.
    pub render_list_class: u32,
    /// Exact second-level bucket read from object type `+0x38`.
    pub render_subbucket: u16,
    pub builder_call_ordinal: u32,
    pub packet_ordinal: u32,
    /// Exact particle type/definition owner read from the runtime at the
    /// packet-builder seam. Extractor-local only.
    pub particle_type_definition_ptr: usize,
    /// Exact packet `+0x60` flattened subsystem selector. It must equal
    /// `packet_ordinal`; the duplicate field makes the native table join
    /// explicit until portable reduction.
    pub subsystem_selector_u32: u32,
    pub shader_program_index_u32: u32,
    pub texture_runtime_index_u32: u32,
    /// Exact final token observed at the 0x1423f4ec0 engine effect lookup,
    /// after engine-owned default/suffix resolution.
    pub engine_effect_token: String,
    pub atlas_half_pixel_width_f32_bits: u32,
    pub atlas_half_pixel_height_f32_bits: u32,
    pub atlas_columns_f32_bits: u32,
    pub atlas_rows_f32_bits: u32,
    pub draw_order: u32,
    /// Exact process-local shader-program join observed by the a729 high-level
    /// program binder before this final draw. It is never serialized.
    pub shader_program_ptr: usize,
    /// True only when the a729 binder/state law and retained draw census have
    /// authenticated null GS/HS/DS/CS state for this exact final draw.
    pub additional_shader_stages_authenticated_null: bool,
    /// True only when the a729 binder/state law and retained draw census have
    /// authenticated an empty D3D11 stream-output binding for this exact draw.
    pub stream_output_targets_authenticated_null: bool,
    pub ribbon_tail_order: Option<u32>,
    pub canonical_packet_112_bytes: Vec<u8>,
    pub packet_matrix_4x4_f32_bits: [u32; 16],
    pub live_count: u32,
    pub max_count: u32,
    pub ordered_gpu_instance_records: Vec<u8>,
    pub elapsed_f32_bits: u32,
    pub scale_f32_bits: u32,
    pub flags_u16: u16,
    /// Stable parent allocated around the entity factory, when this effect was
    /// constructed as an entity child. World effects remain `None`.
    pub parent_entity_instance_id: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawParticleSubmit {
    pub submit_index: u64,
    pub interpolation_phase_f32_bits: u32,
    pub packets: Vec<RawParticlePacket>,
    pub entity_submissions: Vec<RawEntitySubmission>,
}

/// One atomically drained runtime submit and its immutable, process-local D3D
/// catalog observations. The catalog builder must validate the capture and
/// discard every pointer before portable emission.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawRenderSubmitCapture {
    pub submit: RawParticleSubmit,
    pub catalog: RawRenderCatalogCapture,
    /// Exact D3DReflect output correlated to each distinct
    /// `(shader_program_ptr, shader creation)` pair used by this submit.
    pub shader_reflections: Vec<RawShaderBytecodeReflectionObservation>,
}

/// Production authority join for the two semantic rows that cannot be
/// manufactured from D3D11 state. Implementations are backed by the selected
/// install/profile's effective-VFS interpreters and live engine bindings; slot
/// numbers, pointer values, captured vanilla paths, or replay defaults are not
/// acceptable substitutes.
pub trait ExactLiveRenderCatalogSemantics {
    fn resolve_program_identity(
        &self,
        submit: &RawParticleSubmit,
        draw: &RawD3dDrawState,
    ) -> Result<RawProgramIdentity, String>;

    fn resolve_draw_semantics(
        &self,
        submit: &RawParticleSubmit,
        draw: &RawD3dDrawState,
    ) -> Result<RawDrawSemantics, String>;
}

/// Completes the raw hook capture exactly once before the strict catalog
/// builder is allowed to see it. The join is cardinality preserving and keyed
/// by the exact final-draw identity/sequence; a resolver cannot add, drop,
/// reorder, or retarget a draw.
pub fn materialize_live_render_catalog_semantics(
    capture: &mut RawRenderSubmitCapture,
    resolver: &impl ExactLiveRenderCatalogSemantics,
) -> Result<(), PromotionError> {
    if !capture.catalog.program_identities.is_empty() || !capture.catalog.draw_semantics.is_empty()
    {
        return invalid("live render semantics were already materialized");
    }
    let expected = capture
        .submit
        .packets
        .len()
        .checked_add(capture.submit.entity_submissions.len())
        .ok_or_else(|| PromotionError::Invalid("live draw cardinality overflow".into()))?;
    if capture.catalog.draw_states.len() != expected {
        return invalid("live draw-state cardinality differs from the raw submit");
    }

    let mut prior_sequence = 0u64;
    let mut seen = BTreeMap::new();
    let mut programs = Vec::with_capacity(expected);
    let mut semantics = Vec::with_capacity(expected);
    for draw in &capture.catalog.draw_states {
        if draw.final_draw_sequence == 0
            || draw.final_draw_sequence <= prior_sequence
            || draw.submit_index != capture.submit.submit_index
            || seen
                .insert(draw.identity.clone(), draw.final_draw_sequence)
                .is_some()
        {
            return invalid("live draw identities/sequences are stale, repeated, or reordered");
        }
        prior_sequence = draw.final_draw_sequence;

        let program = resolver
            .resolve_program_identity(&capture.submit, draw)
            .map_err(|error| PromotionError::Invalid(format!("live program authority: {error}")))?;
        if program.observation_sequence != draw.final_draw_sequence
            || program.shader_program_ptr != draw.shader_program_ptr
            || program.shader_program_ptr == 0
            || program.effect_token.trim().is_empty()
            || program.vertex_entry_token.trim().is_empty()
            || program.pixel_entry_token.trim().is_empty()
        {
            return invalid("live program authority retargeted or incompletely identified a draw");
        }
        validate_raw_vfs_identity(&program.source, "live shader program source")?;
        if program.ordered_defines.iter().any(|define| {
            define.name.trim().is_empty()
                || !matches!(
                    define.scope.as_str(),
                    "engine_global" | "effect" | "entry_point"
                )
        }) {
            return invalid("live shader program defines are incomplete or have unknown scope");
        }

        let semantic = resolver
            .resolve_draw_semantics(&capture.submit, draw)
            .map_err(|error| PromotionError::Invalid(format!("live draw authority: {error}")))?;
        if semantic.observation_sequence != draw.final_draw_sequence
            || semantic.identity != draw.identity
            || semantic.ordered_texture_semantics.len() != draw.ordered_texture_objects.len()
            || semantic.ordered_sampler_semantics.len() != draw.ordered_sampler_objects.len()
            || semantic.ordered_constant_buffer_semantics.len()
                != draw.ordered_constant_buffer_objects.len()
            || semantic.ordered_color_targets.len() != draw.ordered_color_target_view_ptrs.len()
            || semantic.depth_target.is_some() != draw.depth_stencil_view_ptr.is_some()
        {
            return invalid("live draw authority changed identity or binding cardinality");
        }
        for (object, meaning) in draw
            .ordered_texture_objects
            .iter()
            .zip(&semantic.ordered_texture_semantics)
        {
            if object.stage != meaning.stage
                || object.slot != meaning.slot
                || meaning.semantic_token.trim().is_empty()
            {
                return invalid("live texture semantic disagrees with the observed binding");
            }
        }
        for (object, meaning) in draw
            .ordered_sampler_objects
            .iter()
            .zip(&semantic.ordered_sampler_semantics)
        {
            if object.stage != meaning.stage
                || object.slot != meaning.slot
                || meaning.semantic_token.trim().is_empty()
            {
                return invalid("live sampler semantic disagrees with the observed binding");
            }
        }
        for (view, target) in draw
            .ordered_color_target_view_ptrs
            .iter()
            .zip(&semantic.ordered_color_targets)
        {
            if *view == 0 || target.view_ptr != *view {
                return invalid("live color-target semantic disagrees with the observed view");
            }
        }
        if let (Some(view), Some(target)) = (draw.depth_stencil_view_ptr, &semantic.depth_target) {
            if view == 0 || target.view_ptr != view {
                return invalid("live depth-target semantic disagrees with the observed view");
            }
        }
        match &semantic.content_identity {
            RawDrawContentIdentity::Particle {
                registration_token,
                definition_source_token,
                definition_content,
                texture_content,
                authored_shader_token,
                engine_effect_token,
                atlas_columns_u32,
                atlas_rows_u32,
                ..
            } if draw.family == RawDrawFamily::Particle => {
                validate_raw_vfs_identity(definition_content, "particle definition")?;
                validate_raw_vfs_identity(texture_content, "particle texture")?;
                if registration_token.trim().is_empty()
                    || definition_source_token.trim().is_empty()
                    || authored_shader_token.trim().is_empty()
                    || engine_effect_token.trim().is_empty()
                    || *atlas_rows_u32 == 0
                    || *atlas_columns_u32 == 0
                {
                    return invalid("particle atlas authority supplied zero dimensions");
                }
            }
            RawDrawContentIdentity::Entity {
                resolved_entity_asset,
                mesh,
                entity_registration_token,
                ..
            } if draw.family == RawDrawFamily::Entity => {
                validate_raw_vfs_identity(resolved_entity_asset, "entity asset")?;
                validate_raw_vfs_identity(mesh, "entity mesh")?;
                if entity_registration_token.trim().is_empty() {
                    return invalid("entity registration authority is incomplete");
                }
            }
            _ => return invalid("live content authority assigned the wrong draw family"),
        }
        programs.push(program);
        semantics.push(semantic);
    }
    capture.catalog.program_identities = programs;
    capture.catalog.draw_semantics = semantics;
    Ok(())
}

fn validate_raw_vfs_identity(
    content: &RawVfsContentIdentity,
    label: &str,
) -> Result<(), PromotionError> {
    if content.logical_path.trim().is_empty()
        || content.layer_id.trim().is_empty()
        || !matches!(
            content.layer_kind.as_str(),
            "base" | "integrated_dlc" | "dlc" | "mod"
        )
        || content.payload_bytes == 0
        || !is_lower_sha256(&content.source_sha256)
        || !is_lower_sha256(&content.payload_sha256)
    {
        return Err(PromotionError::Invalid(format!(
            "{label} effective-VFS identity is incomplete"
        )));
    }
    Ok(())
}

fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Non-hook production assembler for one exact d80 static/post-process frame.
/// The injected hook only emits raw rows into this sink; validation, epoch
/// equality, ordering, and immutable catalog closure live here.
#[derive(Clone, Debug)]
pub struct ProductionStaticProgramCaptureSink {
    process: RawProcessEpoch,
    launch_epoch: RawCaptureLaunchEpoch,
    submit_index: u64,
    frame_sequence: u64,
    authority: ProductionStaticCaptureAuthority,
    ordered_operations: Vec<RawStaticOrderOperation>,
    draws: Vec<RawStaticD3dDrawState>,
}

/// Exact non-D3D authority observed beside one captured static draw. These
/// values must come from shader reflection/registration, engine VFS open
/// receipts, and render-graph target registration. The D3D hook cannot infer
/// them from slots, byte hashes, vanilla paths, or a fixed role catalog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObservedStaticDrawAuthority {
    pub capture_sequence: u64,
    pub shader_program_ptr: usize,
    pub program_identity: RawProgramIdentity,
    pub ordered_texture_semantics: Vec<RawStaticShaderResourceSemantic>,
    pub ordered_sampler_semantics: Vec<RawSamplerSemantic>,
    pub ordered_constant_buffer_semantics: Vec<RawConstantBufferSemantic>,
    pub ordered_color_targets: Vec<RawOutputTargetSemantic>,
    pub depth_target: Option<RawOutputTargetSemantic>,
    pub ordered_content: Vec<RawStaticContentJoin>,
}

/// Correlates independent reflection/VFS/render-graph receipts with D3D core
/// observations. Every receipt is single-use and the frame cannot commit
/// while an authority row remains unmatched.
#[derive(Clone, Debug, Default)]
pub struct ProductionStaticCaptureAuthority {
    draws: BTreeMap<u64, ObservedStaticDrawAuthority>,
    output_targets: BTreeMap<(u64, usize), RawOutputTargetSemantic>,
    materialized_program_identities: Vec<RawProgramIdentity>,
}

impl ProductionStaticCaptureAuthority {
    /// Derives opaque render-graph attachment authority from exact OM binding
    /// order and draw-time view/resource snapshots. Color slots retain their
    /// observed order and the optional depth slot follows them; no engine role,
    /// vanilla name, or filename participates.
    pub fn derive_opaque_draw_output_targets(
        core: &RawStaticD3dDrawState,
        catalog: &RawRenderCatalogCapture,
    ) -> Result<
        (
            Vec<RawOutputTargetSemantic>,
            Option<RawOutputTargetSemantic>,
        ),
        String,
    > {
        if core.ordered_color_target_view_ptrs.is_empty() {
            return Err("d80 draw has no observed OM color target".into());
        }
        let mut seen = std::collections::BTreeSet::new();
        let mut colors = Vec::with_capacity(core.ordered_color_target_view_ptrs.len());
        for (attachment_ordinal, view_ptr) in core
            .ordered_color_target_view_ptrs
            .iter()
            .copied()
            .enumerate()
        {
            if !seen.insert(view_ptr) {
                return Err("d80 draw repeats an OM output view".into());
            }
            colors.push(Self::derive_opaque_output_target(
                core.capture_sequence,
                view_ptr,
                RawOutputViewKind::Color,
                u32::try_from(attachment_ordinal)
                    .map_err(|_| "d80 color attachment ordinal overflow".to_owned())?,
                catalog,
            )?);
        }
        let depth = core
            .depth_stencil_view_ptr
            .map(|view_ptr| {
                if !seen.insert(view_ptr) {
                    return Err("d80 draw repeats one view as color and depth".into());
                }
                Self::derive_opaque_output_target(
                    core.capture_sequence,
                    view_ptr,
                    RawOutputViewKind::DepthStencil,
                    u32::try_from(colors.len())
                        .map_err(|_| "d80 depth attachment ordinal overflow".to_owned())?,
                    catalog,
                )
            })
            .transpose()?;
        Ok((colors, depth))
    }

    /// Registers one clear target from a clear-time view/resource snapshot.
    /// The hook must allocate a capture sequence for that snapshot; the global
    /// render-order sequence is not substituted when the two differ.
    pub fn observe_opaque_clear_output_target(
        &mut self,
        capture_sequence: u64,
        target_view_ptr: usize,
        view_kind: RawOutputViewKind,
        catalog: &RawRenderCatalogCapture,
    ) -> Result<(), String> {
        let target = Self::derive_opaque_output_target(
            capture_sequence,
            target_view_ptr,
            view_kind,
            0,
            catalog,
        )?;
        self.observe_output_target(capture_sequence, target)
    }

    fn derive_opaque_output_target(
        _capture_sequence: u64,
        _target_view_ptr: usize,
        _view_kind: RawOutputViewKind,
        _attachment_ordinal: u32,
        _catalog: &RawRenderCatalogCapture,
    ) -> Result<RawOutputTargetSemantic, String> {
        Err("d80 output target requires an exact engine registration receipt".into())
    }

    pub fn observe_draw(&mut self, authority: ObservedStaticDrawAuthority) -> Result<(), String> {
        if authority.capture_sequence == 0
            || authority.shader_program_ptr == 0
            || authority.program_identity.observation_sequence != authority.capture_sequence
            || authority.program_identity.shader_program_ptr != authority.shader_program_ptr
            || authority.ordered_color_targets.is_empty()
            || authority.ordered_content.is_empty()
            || self.draws.contains_key(&authority.capture_sequence)
        {
            return Err("d80 draw authority is incomplete, mismatched, or repeated".into());
        }
        self.draws.insert(authority.capture_sequence, authority);
        Ok(())
    }

    pub fn observe_output_target(
        &mut self,
        capture_sequence: u64,
        target: RawOutputTargetSemantic,
    ) -> Result<(), String> {
        if capture_sequence == 0
            || target.view_ptr == 0
            || self
                .output_targets
                .contains_key(&(capture_sequence, target.view_ptr))
        {
            return Err("d80 output-target authority is incomplete or repeated".into());
        }
        self.output_targets
            .insert((capture_sequence, target.view_ptr), target);
        Ok(())
    }

    /// Joins an observed D3D core row to its exact non-D3D receipts. Semantic
    /// fields on the core must be empty so a hook cannot override authority.
    pub fn materialize_draw(
        &mut self,
        mut core: RawStaticD3dDrawState,
    ) -> Result<RawStaticD3dDrawState, String> {
        if !core.ordered_texture_semantics.is_empty()
            || !core.ordered_sampler_semantics.is_empty()
            || !core.ordered_constant_buffer_semantics.is_empty()
            || !core.ordered_color_targets.is_empty()
            || core.depth_target.is_some()
            || !core.ordered_content.is_empty()
        {
            return Err("d80 D3D core attempted to supply semantic authority".into());
        }
        let authority = self
            .draws
            .remove(&core.capture_sequence)
            .ok_or_else(|| "d80 D3D core has no exact semantic/VFS authority".to_owned())?;
        if authority.shader_program_ptr != core.shader_program_ptr
            || authority.ordered_texture_semantics.len() != core.ordered_texture_objects.len()
            || authority
                .ordered_texture_semantics
                .iter()
                .zip(&core.ordered_texture_objects)
                .any(|(semantic, object)| {
                    semantic.stage != object.stage || semantic.slot != object.slot
                })
            || authority.ordered_sampler_semantics.len() != core.ordered_sampler_objects.len()
            || authority
                .ordered_sampler_semantics
                .iter()
                .zip(&core.ordered_sampler_objects)
                .any(|(semantic, object)| {
                    semantic.stage != object.stage || semantic.slot != object.slot
                })
            || authority.ordered_color_targets.len() != core.ordered_color_target_view_ptrs.len()
            || authority
                .ordered_color_targets
                .iter()
                .zip(&core.ordered_color_target_view_ptrs)
                .any(|(semantic, view_ptr)| semantic.view_ptr != *view_ptr)
            || authority
                .depth_target
                .as_ref()
                .map(|target| target.view_ptr)
                != core.depth_stencil_view_ptr
        {
            return Err("d80 D3D core and semantic/VFS authority disagree".into());
        }
        if self.materialized_program_identities.iter().any(|identity| {
            identity.observation_sequence == authority.capture_sequence
                || identity.shader_program_ptr == authority.shader_program_ptr
                    && identity.observation_sequence
                        == authority.program_identity.observation_sequence
        }) {
            return Err("d80 catalog repeats a static program authority receipt".into());
        }
        self.materialized_program_identities
            .push(authority.program_identity.clone());
        core.ordered_texture_semantics = authority.ordered_texture_semantics;
        core.ordered_sampler_semantics = authority.ordered_sampler_semantics;
        core.ordered_constant_buffer_semantics = authority.ordered_constant_buffer_semantics;
        core.ordered_color_targets = authority.ordered_color_targets;
        core.depth_target = authority.depth_target;
        core.ordered_content = authority.ordered_content;
        Ok(core)
    }

    pub fn materialize_output_target(
        &mut self,
        capture_sequence: u64,
        target_view_ptr: usize,
    ) -> Result<RawOutputTargetSemantic, String> {
        self.output_targets
            .remove(&(capture_sequence, target_view_ptr))
            .ok_or_else(|| "d80 clear has no exact render-graph target authority".into())
    }

    fn validate_consumed(&self) -> Result<(), String> {
        if self.draws.is_empty() && self.output_targets.is_empty() {
            Ok(())
        } else {
            Err("d80 frame contains unmatched semantic/VFS authority receipts".into())
        }
    }

    fn finalize_catalog(self, catalog: &mut RawRenderCatalogCapture) -> Result<(), String> {
        self.validate_consumed()?;
        if self.materialized_program_identities.iter().any(|incoming| {
            catalog.program_identities.iter().any(|existing| {
                existing.observation_sequence == incoming.observation_sequence
                    || existing.shader_program_ptr == incoming.shader_program_ptr
                        && existing.observation_sequence == incoming.observation_sequence
            })
        }) {
            return Err(
                "d80 immutable catalog conflicts with materialized program authority".into(),
            );
        }
        catalog
            .program_identities
            .extend(self.materialized_program_identities);
        Ok(())
    }
}

const MAX_COMPLETED_STATIC_PROGRAM_CAPTURES: usize = 8;
static COMPLETED_STATIC_PROGRAM_CAPTURES: OnceLock<Mutex<VecDeque<RawStaticProgramCapture>>> =
    OnceLock::new();

fn completed_static_program_captures() -> &'static Mutex<VecDeque<RawStaticProgramCapture>> {
    COMPLETED_STATIC_PROGRAM_CAPTURES.get_or_init(|| Mutex::new(VecDeque::new()))
}

impl ProductionStaticProgramCaptureSink {
    pub fn new(
        process: RawProcessEpoch,
        launch_epoch: RawCaptureLaunchEpoch,
        submit_index: u64,
        frame_sequence: u64,
    ) -> Result<Self, String> {
        if process.process_id == 0
            || process.process_start_nonce == 0
            || launch_epoch.process_id != process.process_id
            || launch_epoch.process_start_nonce != process.process_start_nonce
            || submit_index == 0
            || frame_sequence == 0
        {
            return Err("d80 sink has an incomplete or mismatched epoch".into());
        }
        Ok(Self {
            process,
            launch_epoch,
            submit_index,
            frame_sequence,
            authority: ProductionStaticCaptureAuthority::default(),
            ordered_operations: Vec::new(),
            draws: Vec::new(),
        })
    }

    pub fn authority_mut(&mut self) -> &mut ProductionStaticCaptureAuthority {
        &mut self.authority
    }

    pub fn push_order_operation(
        &mut self,
        operation: RawStaticOrderOperation,
    ) -> Result<(), String> {
        if !matches!(operation, RawStaticOrderOperation::Draw { .. }) {
            return Err("d80 clears must pass through the authority materializer".into());
        }
        self.push_materialized_order_operation(operation)
    }

    fn push_materialized_order_operation(
        &mut self,
        operation: RawStaticOrderOperation,
    ) -> Result<(), String> {
        let expected = self
            .ordered_operations
            .last()
            .map(|previous| previous.sequence().saturating_add(1));
        if operation.sequence() == 0
            || expected.is_some_and(|value| operation.sequence() != value)
            || operation.capture_sequence() == 0
            || self
                .ordered_operations
                .iter()
                .any(|row| row.capture_sequence() == operation.capture_sequence())
        {
            return Err("d80 sink operation is gapped, repeated, or capture-ambiguous".into());
        }
        self.ordered_operations.push(operation);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn push_clear_color(
        &mut self,
        sequence: u64,
        capture_sequence: u64,
        target_object_ptr: usize,
        target_view_ptr: usize,
        rgba_f32_bits: [u32; 4],
    ) -> Result<(), String> {
        let target = self
            .authority
            .materialize_output_target(capture_sequence, target_view_ptr)?;
        self.push_materialized_order_operation(RawStaticOrderOperation::ClearColor {
            sequence,
            capture_sequence,
            target_object_ptr,
            target_view_ptr,
            target,
            rgba_f32_bits,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn push_clear_depth_stencil(
        &mut self,
        sequence: u64,
        capture_sequence: u64,
        target_object_ptr: usize,
        target_view_ptr: usize,
        clear_depth: bool,
        clear_stencil: bool,
        depth_f32_bits: u32,
        stencil_u8: u8,
    ) -> Result<(), String> {
        let target = self
            .authority
            .materialize_output_target(capture_sequence, target_view_ptr)?;
        self.push_materialized_order_operation(RawStaticOrderOperation::ClearDepthStencil {
            sequence,
            capture_sequence,
            target_object_ptr,
            target_view_ptr,
            target,
            clear_depth,
            clear_stencil,
            depth_f32_bits,
            stencil_u8,
        })
    }

    pub fn finish(
        self,
        mut catalog: RawRenderCatalogCapture,
    ) -> Result<RawStaticProgramCapture, String> {
        for draw in &self.draws {
            let (colors, depth) =
                ProductionStaticCaptureAuthority::derive_opaque_draw_output_targets(
                    draw, &catalog,
                )?;
            if draw.ordered_color_targets != colors || draw.depth_target != depth {
                return Err(
                    "d80 draw output authority was not derived from exact OM snapshots".into(),
                );
            }
        }
        for operation in &self.ordered_operations {
            let (capture_sequence, target_view_ptr, view_kind, target) = match operation {
                RawStaticOrderOperation::Draw { .. } => continue,
                RawStaticOrderOperation::ClearColor {
                    capture_sequence,
                    target_view_ptr,
                    target,
                    ..
                } => (
                    *capture_sequence,
                    *target_view_ptr,
                    RawOutputViewKind::Color,
                    target,
                ),
                RawStaticOrderOperation::ClearDepthStencil {
                    capture_sequence,
                    target_view_ptr,
                    target,
                    ..
                } => (
                    *capture_sequence,
                    *target_view_ptr,
                    RawOutputViewKind::DepthStencil,
                    target,
                ),
            };
            let derived = ProductionStaticCaptureAuthority::derive_opaque_output_target(
                capture_sequence,
                target_view_ptr,
                view_kind,
                0,
                &catalog,
            )?;
            if *target != derived {
                return Err(
                    "d80 clear output authority was not derived from its exact view snapshot"
                        .into(),
                );
            }
        }
        self.authority.finalize_catalog(&mut catalog)?;
        let capture = RawStaticProgramCapture {
            process: self.process,
            launch_epoch: self.launch_epoch,
            submit_index: self.submit_index,
            frame_sequence: self.frame_sequence,
            ordered_operations: self.ordered_operations,
            draws: self.draws,
            catalog,
        };
        capture.validate_complete()?;
        Ok(capture)
    }
}

impl RawStaticProgramCaptureSink for ProductionStaticProgramCaptureSink {
    fn push_static_draw(&mut self, state: RawStaticD3dDrawState) -> Result<(), String> {
        let state = self.authority.materialize_draw(state)?;
        if state.capture_sequence == 0
            || self
                .draws
                .iter()
                .any(|row| row.capture_sequence == state.capture_sequence)
        {
            return Err("d80 sink repeats or nulls a static draw sequence".into());
        }
        self.draws.push(state);
        Ok(())
    }
}

/// Completes validation before publishing one d80 frame to the normal
/// collector queue. The queue is intentionally small and fail-closed: an
/// attached recorder must keep up, and a full or poisoned queue is reported
/// to the hook instead of silently dropping or replacing evidence.
pub fn commit_static_program_capture(
    sink: ProductionStaticProgramCaptureSink,
    catalog: RawRenderCatalogCapture,
) -> Result<(), String> {
    let capture = sink.finish(catalog)?;
    let mut completed = completed_static_program_captures()
        .lock()
        .map_err(|_| "d80 completed-capture queue is poisoned".to_owned())?;
    if completed.len() >= MAX_COMPLETED_STATIC_PROGRAM_CAPTURES {
        return Err("d80 completed-capture queue is full".into());
    }
    completed.push_back(capture);
    Ok(())
}

/// Atomically drains complete d80 captures for the normal collector path.
/// Incomplete sinks are never visible here and process-local observations are
/// consumed by the authenticated builder before portable serialization.
pub fn drain_complete_static_program_captures() -> Result<Vec<RawStaticProgramCapture>, String> {
    let mut completed = completed_static_program_captures()
        .lock()
        .map_err(|_| "d80 completed-capture queue is poisoned".to_owned())?;
    Ok(completed.drain(..).collect())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedParticleCatalogRow {
    pub registration_token: String,
    pub definition_source_token: String,
    pub resolved_definition_sha256: String,
    pub definition_content: UsedContentKey,
    pub content_identity: portable_render_stream::ParticleContentIdentity,
    pub content_identity_sha256: String,
    pub shader_program: ShaderProgram,
    pub ordered_texture_bindings: Vec<TextureBinding>,
    pub pipeline: PipelineDesc,
    pub geometry: GeometryBinding,
    pub ordered_constant_buffers: Vec<ConstantBufferBinding>,
    pub output_state: RenderOutputState,
    pub atlas_rows: u32,
    pub atlas_columns: u32,
}

pub trait ExactParticleCatalogResolver {
    fn authority_manifest_sha256(&self) -> &str;
    fn resolve(&self, raw: &RawParticlePacket) -> Option<ResolvedParticleCatalogRow>;
}

/// Stable process-local key for one exact particle draw. Definition identity
/// alone is insufficient because constant buffers, output state and draw
/// parameters can change between submissions of the same authored subsystem.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ParticleDrawCatalogKey {
    pub effect_instance_id: u64,
    pub registration_token: String,
    pub definition_source_token: String,
    pub builder_call_ordinal: u32,
    pub packet_ordinal: u32,
    pub draw_order: u32,
    pub shader_program_ptr: usize,
}

impl From<&RawParticlePacket> for ParticleDrawCatalogKey {
    fn from(raw: &RawParticlePacket) -> Self {
        Self {
            effect_instance_id: raw.effect_instance_id,
            registration_token: raw.definition.registration_token.clone(),
            definition_source_token: raw.definition.definition_source_token.clone(),
            builder_call_ordinal: raw.builder_call_ordinal,
            packet_ordinal: raw.packet_ordinal,
            draw_order: raw.draw_order,
            shader_program_ptr: raw.shader_program_ptr,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawWeatherEntityRef {
    pub entity_instance_id: u64,
    /// Process-local join consumed only by the effective-VFS catalog resolver.
    /// It is never serialized into the portable replay.
    pub definition_ptr: usize,
    pub ordered_effect_instance_ids: Vec<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawWeatherEntityPair {
    pub primary: Option<RawWeatherEntityRef>,
    pub fading: Option<RawWeatherEntityRef>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawWeatherFactoryAttempt {
    pub definition_ptr: usize,
    pub allocated_entity_instance_id: u64,
    pub returned_entity_instance_id: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawWeatherObservation {
    pub observation_index: u64,
    pub strategic_region_id: u32,
    pub rain_light_active: bool,
    pub rain_heavy_active: bool,
    pub snow_active: bool,
    pub blizzard_active: bool,
    pub sandstorm_active: bool,
    pub arctic_water_active: bool,
    pub region_value_i64_fixed_1e5: i64,
    pub transition_expiry_game_hour_i32: i32,
    pub previous_airborne_runtime_code: u8,
    pub airborne_runtime_code: u8,
    /// Exact two words written by `0x1424dbe60`, before the producer advances
    /// word zero once per authored weather position.
    pub selection_seed_words: Option<[u32; 2]>,
    pub ordered_factory_attempts: Vec<RawWeatherFactoryAttempt>,
    /// Exact compact live vector at producer `+0x48`, in engine order.
    pub ordered_live_pairs: Vec<RawWeatherEntityPair>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedWeatherCatalogRow {
    pub weather_effect_token: Option<String>,
    /// Includes every authored position, including explicit-empty membership.
    pub ordered_positions: Vec<WeatherPosition>,
    pub ordered_weather_instances: Vec<WeatherInstance>,
}

pub trait ExactWeatherCatalogResolver {
    fn authority_manifest_sha256(&self) -> &str;
    fn resolve(&self, raw: &RawWeatherObservation) -> Option<ResolvedWeatherCatalogRow>;
}

/// Process-local entity draw observed at the a729 renderer boundary. Pointer
/// fields are resolver joins only and must never be serialized.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawEntitySubmission {
    pub submit_index: u64,
    pub pass_ordinal: u32,
    pub draw_order: u32,
    pub entity_instance_id: u64,
    /// Exact case-sensitive token passed to the named a729 entity factory.
    /// `None` is an explicit direct-definition factory path and cannot be
    /// promoted through the game-file entity interpreter.
    pub entity_registration_token: Option<String>,
    pub entity_object_ptr: usize,
    pub entity_definition_ptr: usize,
    pub renderer_ptr: usize,
    pub render_context_ptr: usize,
    pub mesh_draw_ptr: usize,
    pub material_binding_ptr: usize,
    /// Exact renderer list selectors/ordinals retained without assigning
    /// semantic LOD/submesh meaning beyond the proven native traversal.
    pub entity_group_index: u32,
    pub mesh_draw_list_index: u32,
    pub entity_variant_u8: u8,
    pub mesh_draw_variant_u8: u8,
    pub pass_selector_i32: i32,
    pub bind_renderer_target_u8: u8,
    pub alternate_list_u8: u8,
    pub shader_program_ptr: usize,
    pub additional_shader_stages_authenticated_null: bool,
    pub stream_output_targets_authenticated_null: bool,
    pub world_transform_4x4_f32_bits: [u32; 16],
    pub state_flags: u32,
    pub skinning: RawSkinning,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RawSkinning {
    NotSkinned,
    /// Exact 50-matrix, 3,200-byte palette consumed by this pass/draw.
    Palette3200(Vec<u8>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedEntityCatalogRow {
    pub resolved_entity_sha256: String,
    pub content_identity: EntityContentIdentity,
    pub shader_program: ShaderProgram,
    pub ordered_texture_bindings: Vec<TextureBinding>,
    pub pipeline: PipelineDesc,
    pub geometry: GeometryBinding,
    pub ordered_constant_buffers: Vec<ConstantBufferBinding>,
    pub output_state: RenderOutputState,
}

pub trait ExactEntityCatalogResolver {
    fn authority_manifest_sha256(&self) -> &str;
    fn resolve(&self, raw: &RawEntitySubmission) -> Option<ResolvedEntityCatalogRow>;
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct EntityDrawCatalogKey {
    pub submit_index: u64,
    pub pass_ordinal: u32,
    pub entity_instance_id: u64,
    pub draw_order: u32,
}

impl From<&RawEntitySubmission> for EntityDrawCatalogKey {
    fn from(raw: &RawEntitySubmission) -> Self {
        Self {
            submit_index: raw.submit_index,
            pass_ordinal: raw.pass_ordinal,
            entity_instance_id: raw.entity_instance_id,
            draw_order: raw.draw_order,
        }
    }
}

/// Exact per-draw catalog assembled by the extractor-side live wrapper
/// interpreter. It accepts only rows whose every game-content reference is an
/// exact member of the finalized portable content manifest produced from
/// `extractor.live_binding_source_inventory.v3` and
/// `extractor.static_catalog_source_join.v1`.
#[derive(Clone, Debug)]
pub struct AuthenticatedRenderCatalog {
    authority_manifest_sha256: String,
    particle_rows: BTreeMap<ParticleDrawCatalogKey, ResolvedParticleCatalogRow>,
    entity_rows: BTreeMap<EntityDrawCatalogKey, ResolvedEntityCatalogRow>,
}

/// Pointer-free receipt from the exact a729 shader/effect lookup. The process
/// local program index authenticates a successful engine lookup but is never a
/// portable content identity; the later effective-VFS closure join supplies
/// the exact winning files.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawMapShaderLookup {
    pub sequence: u64,
    pub engine_lookup_rva: u32,
    pub registration_root_rva: u32,
    pub logical_shader_path: String,
    pub effect_name: String,
    pub program_index: i32,
}

impl RawMapShaderLookup {
    pub fn validate(&self) -> Result<(), PromotionError> {
        if self.sequence == 0
            || self.engine_lookup_rva != ENGINE_SHADER_EFFECT_LOOKUP_RVA
            || self.program_index < 0
            || self.logical_shader_path.is_empty()
            || self.logical_shader_path.len() > 512
            || self.effect_name.is_empty()
            || self.effect_name.len() > 256
            || self.logical_shader_path.chars().any(char::is_control)
            || self.effect_name.chars().any(char::is_control)
        {
            return invalid("map shader lookup receipt is incomplete or malformed");
        }
        if !is_authenticated_map_registration_root(self.registration_root_rva) {
            return invalid("map shader lookup registration root is not authenticated");
        }
        Ok(())
    }
}

/// Registration producers proven in the pinned a729 executable. Logical paths
/// are deliberately absent: the hook records the live argument and the
/// effective-VFS index supplies its exact semantic-role membership.
pub const AUTHENTICATED_MAP_REGISTRATION_ROOTS: [u32; 6] = [
    MAP_EFFECT_REGISTRATION_ROOT_RVA,
    RAILROAD_EFFECT_REGISTRATION_ROOT_RVA,
    MAP_NAME_EFFECT_REGISTRATION_ROOT_RVA,
    RESTORE_BLOOM_EFFECT_REGISTRATION_ROOT_RVA,
    LUT_BLEND_EFFECT_REGISTRATION_ROOT_RVA,
    PDXMESH_EFFECT_REGISTRATION_ROOT_RVA,
];

pub fn is_authenticated_map_registration_root(registration_root_rva: u32) -> bool {
    AUTHENTICATED_MAP_REGISTRATION_ROOTS.contains(&registration_root_rva)
}

// Test-only archive of the superseded fixed-role d80 classifier. Production
// uses the opaque RawStaticProgramCapture contract and cannot call any symbol
// in this module.
#[cfg(test)]
mod legacy_map_content_classifier {
    use super::*;

    pub const PRODUCTION_MAP_CONTENT_CONSUMERS: [&str; 11] = [
        "map_viewer.production_map.terrain",
        "map_viewer.production_map.water",
        "map_viewer.production_map.rivers",
        "map_viewer.production_map.borders",
        "map_viewer.production_map.trees",
        "map_viewer.production_map.fixed_objects",
        "map_viewer.production_map.rails",
        "map_viewer.production_map.map_names",
        "map_viewer.production_map.unit_models",
        "map_viewer.production_map.lighting",
        "map_viewer.production_map.post_process",
    ];

    #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum MapContentRole {
        Terrain,
        Water,
        Rivers,
        Borders,
        Trees,
        FixedObjects,
        Rails,
        MapNames,
        UnitModels,
        Lighting,
        PostProcess,
    }

    impl MapContentRole {
        pub const ALL: [Self; 11] = [
            Self::Terrain,
            Self::Water,
            Self::Rivers,
            Self::Borders,
            Self::Trees,
            Self::FixedObjects,
            Self::Rails,
            Self::MapNames,
            Self::UnitModels,
            Self::Lighting,
            Self::PostProcess,
        ];
        pub const fn consumer(self) -> &'static str {
            PRODUCTION_MAP_CONTENT_CONSUMERS[self as usize]
        }
    }

    /// Engine-proven semantic families for each registration producer. This is a
    /// producer/consumer law, not a file-name catalog: mods may replace or rename
    /// every logical path while retaining the same observed engine producer.
    pub fn map_registration_root_supports_role(
        registration_root_rva: u32,
        role: MapContentRole,
    ) -> bool {
        match registration_root_rva {
            MAP_EFFECT_REGISTRATION_ROOT_RVA => matches!(
                role,
                MapContentRole::Terrain
                    | MapContentRole::Water
                    | MapContentRole::Rivers
                    | MapContentRole::Borders
                    | MapContentRole::Trees
                    | MapContentRole::Lighting
            ),
            RAILROAD_EFFECT_REGISTRATION_ROOT_RVA => role == MapContentRole::Rails,
            MAP_NAME_EFFECT_REGISTRATION_ROOT_RVA => role == MapContentRole::MapNames,
            RESTORE_BLOOM_EFFECT_REGISTRATION_ROOT_RVA | LUT_BLEND_EFFECT_REGISTRATION_ROOT_RVA => {
                role == MapContentRole::PostProcess
            }
            PDXMESH_EFFECT_REGISTRATION_ROOT_RVA => {
                matches!(
                    role,
                    MapContentRole::FixedObjects | MapContentRole::UnitModels
                )
            }
            _ => false,
        }
    }

    pub const MAP_CONTENT_SNAPSHOT_SCHEMA: &str = "extractor.production-map-content-snapshot.v1";
    pub const MAP_CONTENT_SNAPSHOT_COMPACT_DOMAIN_ID: u16 = 80;
    pub const MAX_MAP_CONTENT_SNAPSHOT_PAYLOAD_BYTES: usize = 16 * 1024 * 1024;

    /// Honest provenance for one d80 semantic-role closure. A live lookup carries
    /// the exact pointer-free receipt fields; an authenticated static program
    /// carries its selected-submit/program identity. Neither variant permits a
    /// late/static builder to manufacture an engine lookup RVA.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
    pub enum MapContentObservationEvidence {
        EngineLookup {
            sequence: u64,
            engine_lookup_rva: u32,
            registration_root_rva: u32,
            logical_shader_path: String,
            effect_name: String,
            program_index: i32,
        },
        StaticProgram {
            submit_index: u64,
            program_ordinal: u32,
            pass_token: String,
            program_payload_sha256: String,
        },
    }

    /// One effective-VFS observation made either at an authenticated engine lookup
    /// or by a completely validated static-map program. The evidence and
    /// corroborating closure digest remain extractor-side provenance; only
    /// `consumer` and `content` enter the portable manifest.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct MapContentObservation {
        pub content_key: String,
        pub content: UsedContentKey,
        pub consumer: String,
        pub evidence: MapContentObservationEvidence,
        pub corroboration_sha256: String,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct OmittedMapContentRole {
        pub consumer: String,
        pub reason: String,
    }

    /// Complete, fail-closed partition of the production-map vocabulary. A role
    /// may resolve to several paths, but every role is either backed by at least
    /// one exact live observation or explicitly omitted with a data-needed reason.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct AuthenticatedMapContentSnapshot {
        pub schema: String,
        pub authority_manifest_sha256: String,
        pub observations: Vec<MapContentObservation>,
        pub omitted_roles: Vec<OmittedMapContentRole>,
    }

    /// Canonical pointer-free d80 payload. A byte sequence is admitted only when
    /// it is the exact compact JSON encoding produced here and the decoded
    /// snapshot passes the full authority/provenance/partition validation.
    pub fn encode_map_content_snapshot_payload(
        snapshot: &AuthenticatedMapContentSnapshot,
    ) -> Result<Vec<u8>, PromotionError> {
        snapshot.validate()?;
        let payload = serde_json::to_vec(snapshot)
            .map_err(|error| PromotionError::Invalid(format!("map snapshot encode: {error}")))?;
        if payload.len() > MAX_MAP_CONTENT_SNAPSHOT_PAYLOAD_BYTES {
            return invalid("map content snapshot exceeds the bounded d80 payload size");
        }
        Ok(payload)
    }

    pub fn decode_map_content_snapshot_payload(
        payload: &[u8],
    ) -> Result<AuthenticatedMapContentSnapshot, PromotionError> {
        if payload.is_empty() || payload.len() > MAX_MAP_CONTENT_SNAPSHOT_PAYLOAD_BYTES {
            return invalid("map content snapshot d80 payload size is invalid");
        }
        let snapshot: AuthenticatedMapContentSnapshot = serde_json::from_slice(payload)
            .map_err(|error| PromotionError::Invalid(format!("map snapshot decode: {error}")))?;
        snapshot.validate()?;
        if serde_json::to_vec(&snapshot)
            .map_err(|error| PromotionError::Invalid(format!("map snapshot re-encode: {error}")))?
            != payload
        {
            return invalid("map content snapshot d80 payload is not canonical");
        }
        Ok(snapshot)
    }

    impl AuthenticatedMapContentSnapshot {
        pub fn new(
            authority_manifest_sha256: String,
            observations: Vec<MapContentObservation>,
            omitted_roles: Vec<OmittedMapContentRole>,
        ) -> Result<Self, PromotionError> {
            let snapshot = Self {
                schema: MAP_CONTENT_SNAPSHOT_SCHEMA.into(),
                authority_manifest_sha256,
                observations,
                omitted_roles,
            };
            snapshot.validate()?;
            Ok(snapshot)
        }

        /// Revalidates even directly constructed snapshots at the finalization
        /// boundary. Public fields are useful to FFI/collector adapters, so no
        /// caller is trusted merely because `new` exists.
        pub fn validate(&self) -> Result<(), PromotionError> {
            if self.schema != MAP_CONTENT_SNAPSHOT_SCHEMA {
                return invalid("map content snapshot has an unsupported schema");
            }
            check_sha256(
                &self.authority_manifest_sha256,
                "map content authority manifest",
            )?;
            let allowed = PRODUCTION_MAP_CONTENT_CONSUMERS
                .into_iter()
                .collect::<std::collections::BTreeSet<_>>();
            let mut observed_roles = std::collections::BTreeSet::new();
            let mut observed_pairs = std::collections::BTreeSet::new();
            for row in &self.observations {
                if !allowed.contains(row.consumer.as_str()) {
                    return invalid("map content observation has an unknown semantic consumer");
                }
                row.content
                    .validate()
                    .map_err(|error| PromotionError::Invalid(format!("map content: {error}")))?;
                if row.content.effective_vfs_manifest_sha256 != self.authority_manifest_sha256 {
                    return invalid("map content observation names a different effective VFS");
                }
                let normalized = normalize_game_relative_path(&row.content_key)
                    .map_err(PromotionError::Invalid)?;
                if normalized != row.content_key
                    || normalized != row.content.normalized_game_relative_path
                {
                    return invalid("map content key/path is not one canonical VFS path");
                }
                let role = MapContentRole::ALL
                    .into_iter()
                    .find(|role| role.consumer() == row.consumer)
                    .ok_or_else(|| {
                        PromotionError::Invalid(
                            "map content observation has no typed semantic role".into(),
                        )
                    })?;
                match &row.evidence {
                    MapContentObservationEvidence::EngineLookup {
                        sequence,
                        engine_lookup_rva,
                        registration_root_rva,
                        logical_shader_path,
                        effect_name,
                        program_index,
                    } => {
                        RawMapShaderLookup {
                            sequence: *sequence,
                            engine_lookup_rva: *engine_lookup_rva,
                            registration_root_rva: *registration_root_rva,
                            logical_shader_path: logical_shader_path.clone(),
                            effect_name: effect_name.clone(),
                            program_index: *program_index,
                        }
                        .validate()?;
                        if !map_registration_root_supports_role(*registration_root_rva, role) {
                            return invalid(
                                "map content lookup evidence does not support its semantic role",
                            );
                        }
                    }
                    MapContentObservationEvidence::StaticProgram {
                        pass_token,
                        program_payload_sha256,
                        ..
                    } => {
                        if pass_token != &row.consumer {
                            return invalid(
                                "map content static-program evidence does not match its consumer",
                            );
                        }
                        check_sha256(program_payload_sha256, "map content static program")?;
                    }
                }
                check_sha256(&row.corroboration_sha256, "map content corroboration")?;
                if !observed_pairs.insert((row.consumer.as_str(), row.content_key.as_str())) {
                    return invalid("map content snapshot repeats a semantic consumer/path pair");
                }
                observed_roles.insert(row.consumer.as_str());
            }
            let mut omitted = std::collections::BTreeSet::new();
            for row in &self.omitted_roles {
                if !allowed.contains(row.consumer.as_str())
                    || observed_roles.contains(row.consumer.as_str())
                    || !omitted.insert(row.consumer.as_str())
                    || row.reason.trim().is_empty()
                {
                    return invalid("map content snapshot has an invalid omitted role");
                }
            }
            let covered = observed_roles
                .iter()
                .copied()
                .chain(omitted.iter().copied())
                .collect::<std::collections::BTreeSet<_>>();
            if covered != allowed {
                return invalid(
                    "map content snapshot does not partition the complete role vocabulary",
                );
            }
            Ok(())
        }
    }

    #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct MapShaderLookupKey {
        pub registration_root_rva: u32,
        pub logical_shader_path: String,
        pub effect_name: String,
    }

    impl From<&RawMapShaderLookup> for MapShaderLookupKey {
        fn from(value: &RawMapShaderLookup) -> Self {
            Self {
                registration_root_rva: value.registration_root_rva,
                logical_shader_path: normalize_game_relative_path(&value.logical_shader_path)
                    .unwrap_or_else(|_| value.logical_shader_path.clone()),
                effect_name: value.effect_name.clone(),
            }
        }
    }

    /// Complete effective-VFS dependency closure for one semantic map role. The
    /// extractor-side static interpreter produces this from the finalized winning
    /// inventory; the live lookup receipts merely prove which closure was active.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct AuthenticatedMapRoleClosure {
        pub role: MapContentRole,
        pub complete_dependency_closure: bool,
        pub dependency_closure_sha256: String,
        pub lookup_keys: Vec<MapShaderLookupKey>,
        pub content: Vec<UsedContentKey>,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct EffectiveVfsMapContentIndexDocument {
        pub schema: String,
        pub authority_manifest_sha256: String,
        pub source_inventory_sha256: String,
        pub winning_content: Vec<UsedContentKey>,
        pub closures: Vec<AuthenticatedMapRoleClosure>,
    }

    #[derive(Clone, Debug)]
    pub struct EffectiveVfsMapContentResolver {
        authority_manifest_sha256: String,
        closures: BTreeMap<MapContentRole, AuthenticatedMapRoleClosure>,
    }

    impl EffectiveVfsMapContentResolver {
        pub fn decode_authenticated_index(bytes: &[u8]) -> Result<Self, PromotionError> {
            let document: EffectiveVfsMapContentIndexDocument = serde_json::from_slice(bytes)
                .map_err(|error| {
                    PromotionError::Invalid(format!("parse map content index: {error}"))
                })?;
            if document.schema != "extractor.effective-vfs-map-content-index.v1"
                || document.source_inventory_sha256 != document.authority_manifest_sha256
            {
                return invalid("map content index schema/source-inventory binding is invalid");
            }
            Self::new(
                document.authority_manifest_sha256,
                document.winning_content,
                document.closures,
            )
        }

        pub fn new(
            authority_manifest_sha256: String,
            winning_content: impl IntoIterator<Item = UsedContentKey>,
            closures: impl IntoIterator<Item = AuthenticatedMapRoleClosure>,
        ) -> Result<Self, PromotionError> {
            check_sha256(&authority_manifest_sha256, "map content resolver authority")?;
            let mut winners = BTreeMap::new();
            for content in winning_content {
                validate_content_key(&content, &authority_manifest_sha256, "map content winner")?;
                let path = content.normalized_game_relative_path.clone();
                if winners.insert(path, content).is_some() {
                    return invalid("map content winning inventory repeats a path");
                }
            }
            if winners.is_empty() {
                return invalid("map content winning inventory is empty");
            }
            let mut resolved = BTreeMap::new();
            for closure in closures {
                if !closure.complete_dependency_closure
                    || closure.lookup_keys.is_empty()
                    || closure.content.is_empty()
                {
                    continue;
                }
                let mut prior_lookup = None;
                for key in &closure.lookup_keys {
                    if key.effect_name.is_empty()
                        || normalize_game_relative_path(&key.logical_shader_path)
                            .ok()
                            .as_deref()
                            != Some(key.logical_shader_path.as_str())
                        || !map_registration_root_supports_role(
                            key.registration_root_rva,
                            closure.role,
                        )
                        || prior_lookup.as_ref().is_some_and(|prior| *prior >= key)
                    {
                        return invalid(
                            "map role closure has an invalid or noncanonical lookup key",
                        );
                    }
                    prior_lookup = Some(key);
                }
                let mut prior_content: Option<&str> = None;
                for content in &closure.content {
                    validate_content_key(content, &authority_manifest_sha256, "map role content")?;
                    let path = content.normalized_game_relative_path.as_str();
                    if winners.get(path) != Some(content)
                        || prior_content.is_some_and(|prior| prior >= path)
                    {
                        return invalid(
                            "map role closure content is stale, repeated, or noncanonical",
                        );
                    }
                    prior_content = Some(path);
                }
                let digest =
                    map_role_closure_sha256(closure.role, &closure.lookup_keys, &closure.content)?;
                if digest != closure.dependency_closure_sha256 {
                    return invalid("map role dependency closure digest is invalid");
                }
                let role = closure.role;
                if resolved.insert(role, closure).is_some() {
                    return invalid("map content index repeats a semantic role");
                }
            }
            Ok(Self {
                authority_manifest_sha256,
                closures: resolved,
            })
        }

        pub fn authority_manifest_sha256(&self) -> &str {
            &self.authority_manifest_sha256
        }

        pub fn resolve_role(&self, role: MapContentRole) -> Option<&AuthenticatedMapRoleClosure> {
            self.closures.get(&role)
        }
    }

    /// A static VFS index alone cannot honestly construct an in-process d80
    /// observation. Callers must join exact live lookup receipts through
    /// `promote_map_content_lookups`, or use the strict authenticated static-program
    /// closure builder.
    pub fn promote_resolved_map_content_index(
        _resolver: &EffectiveVfsMapContentResolver,
    ) -> Result<AuthenticatedMapContentSnapshot, PromotionError> {
        invalid(
            "effective-VFS map index has no authenticated live-lookup or static-program provenance",
        )
    }

    pub fn map_role_closure_sha256(
        role: MapContentRole,
        lookup_keys: &[MapShaderLookupKey],
        content: &[UsedContentKey],
    ) -> Result<String, PromotionError> {
        let bytes = serde_json::to_vec(&(role, lookup_keys, content)).map_err(|error| {
            PromotionError::Invalid(format!("map role closure encode: {error}"))
        })?;
        Ok(hex_sha256(&bytes))
    }

    /// Promotes only roles whose live lookup set is wholly covered by one complete
    /// effective-VFS closure. Unactivated or unresolved roles are recorded as
    /// feature-local omissions; a malformed/failed live lookup rejects the entire
    /// snapshot instead of degrading it to a guessed vanilla route.
    pub fn promote_map_content_lookups(
        raw: &[RawMapShaderLookup],
        resolver: &EffectiveVfsMapContentResolver,
    ) -> Result<AuthenticatedMapContentSnapshot, PromotionError> {
        check_sha256(
            resolver.authority_manifest_sha256(),
            "map content resolver authority",
        )?;
        let mut prior_sequence = 0;
        let mut role_by_key = BTreeMap::<MapShaderLookupKey, Option<MapContentRole>>::new();
        for role in MapContentRole::ALL {
            let Some(closure) = resolver.resolve_role(role) else {
                continue;
            };
            for key in &closure.lookup_keys {
                role_by_key
                    .entry(key.clone())
                    .and_modify(|existing| *existing = None)
                    .or_insert(Some(role));
            }
        }
        let mut by_role =
            BTreeMap::<MapContentRole, BTreeMap<MapShaderLookupKey, RawMapShaderLookup>>::new();
        for row in raw {
            row.validate()?;
            if row.sequence <= prior_sequence {
                return invalid("map shader lookup sequence is repeated or noncanonical");
            }
            prior_sequence = row.sequence;
            let key = MapShaderLookupKey::from(row);
            let Some(Some(role)) = role_by_key.get(&key).copied() else {
                return invalid(
                    "live map shader lookup has no unique effective-VFS semantic-role closure",
                );
            };
            by_role
                .entry(role)
                .or_default()
                .entry(key)
                .or_insert_with(|| row.clone());
        }
        let mut observations = Vec::new();
        let mut omitted_roles = Vec::new();
        for role in MapContentRole::ALL {
            let Some(observed_keys) = by_role.get(&role) else {
                omitted_roles.push(OmittedMapContentRole {
                    consumer: role.consumer().into(),
                    reason: "role had no authenticated engine shader lookup in this capture".into(),
                });
                continue;
            };
            let Some(closure) = resolver.resolve_role(role) else {
                omitted_roles.push(OmittedMapContentRole {
                    consumer: role.consumer().into(),
                    reason: "live role has no complete effective-VFS dependency closure".into(),
                });
                continue;
            };
            let observed_keys = observed_keys
                .keys()
                .cloned()
                .collect::<std::collections::BTreeSet<_>>();
            let closure_keys = closure
                .lookup_keys
                .iter()
                .cloned()
                .collect::<std::collections::BTreeSet<_>>();
            if !observed_keys.is_subset(&closure_keys) {
                return invalid("live map shader lookup is absent from its effective-VFS closure");
            }
            let receipt = by_role
                .get(&role)
                .and_then(|rows| rows.values().next())
                .ok_or_else(|| {
                    PromotionError::Invalid("observed map role has no lookup receipt".into())
                })?;
            for content in &closure.content {
                observations.push(MapContentObservation {
                    content_key: content.normalized_game_relative_path.clone(),
                    content: content.clone(),
                    consumer: role.consumer().into(),
                    evidence: MapContentObservationEvidence::EngineLookup {
                        sequence: receipt.sequence,
                        engine_lookup_rva: receipt.engine_lookup_rva,
                        registration_root_rva: receipt.registration_root_rva,
                        logical_shader_path: receipt.logical_shader_path.clone(),
                        effect_name: receipt.effect_name.clone(),
                        program_index: receipt.program_index,
                    },
                    corroboration_sha256: closure.dependency_closure_sha256.clone(),
                });
            }
        }
        observations.sort_by(|left, right| {
            (&left.consumer, &left.content_key).cmp(&(&right.consumer, &right.content_key))
        });
        AuthenticatedMapContentSnapshot::new(
            resolver.authority_manifest_sha256().into(),
            observations,
            omitted_roles,
        )
    }
}

#[cfg(test)]
pub use legacy_map_content_classifier::*;

impl AuthenticatedRenderCatalog {
    pub fn new(
        manifest: &portable_content_manifest::ContentManifest,
        particle_rows: impl IntoIterator<Item = (ParticleDrawCatalogKey, ResolvedParticleCatalogRow)>,
        entity_rows: impl IntoIterator<Item = (EntityDrawCatalogKey, ResolvedEntityCatalogRow)>,
    ) -> Result<Self, PromotionError> {
        manifest
            .validate()
            .map_err(|error| PromotionError::Invalid(format!("content manifest: {error}")))?;
        if manifest.manifest_digest_sha256.is_none() {
            return invalid("content manifest is not finalized with its digest");
        }
        let authority = manifest.effective_vfs_manifest_sha256.clone();
        let content_by_path = manifest
            .required_content
            .iter()
            .map(|required| {
                (
                    required.content.normalized_game_relative_path.clone(),
                    required.content.clone(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let mut particles = BTreeMap::new();
        for (key, row) in particle_rows {
            validate_particle_catalog_row(
                &row,
                &authority,
                particle_geometry_instance_count(&row.geometry)?,
            )?;
            validate_catalog_content_membership(
                particle_catalog_content(&row),
                &content_by_path,
                "particle catalog",
            )?;
            if particles.insert(key, row).is_some() {
                return invalid("particle catalog repeats an exact draw key");
            }
        }
        let mut entities = BTreeMap::new();
        for (key, row) in entity_rows {
            validate_entity_catalog_row(&row, &authority)?;
            validate_catalog_content_membership(
                entity_catalog_content(&row),
                &content_by_path,
                "entity catalog",
            )?;
            if entities.insert(key, row).is_some() {
                return invalid("entity catalog repeats an exact draw key");
            }
        }
        Ok(Self {
            authority_manifest_sha256: authority,
            particle_rows: particles,
            entity_rows: entities,
        })
    }
}

impl ExactParticleCatalogResolver for AuthenticatedRenderCatalog {
    fn authority_manifest_sha256(&self) -> &str {
        &self.authority_manifest_sha256
    }

    fn resolve(&self, raw: &RawParticlePacket) -> Option<ResolvedParticleCatalogRow> {
        self.particle_rows
            .get(&ParticleDrawCatalogKey::from(raw))
            .cloned()
    }
}

impl ExactEntityCatalogResolver for AuthenticatedRenderCatalog {
    fn authority_manifest_sha256(&self) -> &str {
        &self.authority_manifest_sha256
    }

    fn resolve(&self, raw: &RawEntitySubmission) -> Option<ResolvedEntityCatalogRow> {
        self.entity_rows
            .get(&EntityDrawCatalogKey::from(raw))
            .cloned()
    }
}

fn particle_geometry_instance_count(geometry: &GeometryBinding) -> Result<u32, PromotionError> {
    match geometry.draw {
        DrawCommand::DrawInstanced { instance_count, .. }
        | DrawCommand::DrawIndexedInstanced { instance_count, .. } => Ok(instance_count),
        _ => invalid("particle catalog row does not contain an instanced draw"),
    }
}

fn particle_catalog_content(row: &ResolvedParticleCatalogRow) -> Vec<&UsedContentKey> {
    let mut out = vec![
        &row.content_identity.definition_content,
        &row.content_identity.texture_content,
        &row.shader_program.source,
    ];
    append_render_content(&mut out, &row.ordered_texture_bindings, &row.geometry);
    out
}

fn entity_catalog_content(row: &ResolvedEntityCatalogRow) -> Vec<&UsedContentKey> {
    let mut out = vec![
        &row.content_identity.resolved_entity_asset,
        &row.content_identity.mesh,
        &row.shader_program.source,
    ];
    append_render_content(&mut out, &row.ordered_texture_bindings, &row.geometry);
    out
}

fn append_render_content<'a>(
    out: &mut Vec<&'a UsedContentKey>,
    textures: &'a [TextureBinding],
    geometry: &'a GeometryBinding,
) {
    for texture in textures {
        out.push(&texture.content);
        for subresource in &texture.resource.ordered_subresources {
            append_buffer_content(out, &subresource.source);
        }
    }
    for vertex in &geometry.ordered_vertex_buffers {
        append_buffer_content(out, &vertex.source);
    }
    if let Some(index) = &geometry.index_buffer {
        append_buffer_content(out, &index.source);
    }
}

fn append_buffer_content<'a>(out: &mut Vec<&'a UsedContentKey>, source: &'a BufferSource) {
    if let BufferSource::UsedContentRange { content, .. } = source {
        out.push(content);
    }
}

fn validate_catalog_content_membership(
    keys: Vec<&UsedContentKey>,
    content_by_path: &BTreeMap<String, UsedContentKey>,
    label: &str,
) -> Result<(), PromotionError> {
    for key in keys {
        match content_by_path.get(&key.normalized_game_relative_path) {
            Some(expected) if expected == key => {}
            Some(_) => {
                return invalid(&format!(
                    "{label} content identity disagrees with finalized VFS winner: {}",
                    key.normalized_game_relative_path
                ))
            }
            None => {
                return invalid(&format!(
                    "{label} references content absent from the finalized source join: {}",
                    key.normalized_game_relative_path
                ))
            }
        }
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawDirectVisualObservation {
    pub sequence: u64,
    pub update_index: u64,
    pub producer_kind: DirectVisualKind,
    pub producer_instance_id: u64,
    pub entity_instance_id: u64,
    pub effect_instance_id: Option<u64>,
    pub operation: DirectVisualOperation,
    pub entity_definition_ptr: usize,
    pub semantic_state: String,
    pub authored_state_index: u32,
    pub authored_event_index: u32,
    pub state_enter_sequence: u64,
    pub live_phase_f32_bits: u32,
    pub world_transform_4x4_f32_bits: [u32; 16],
    pub node_or_locator_token: Option<String>,
    pub visibility_flags: u32,
    pub direct_object_list_ordinal: u32,
    pub direct_object_count: u32,
    pub rng_counter: u32,
    pub rng_seed: u32,
    pub effect_expiry_update: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedDirectVisualRow {
    pub entity_token: String,
    pub resolved_entity_source: UsedContentKey,
    pub submitted_outputs: Vec<RenderOutputRef>,
}

pub trait ExactDirectVisualResolver {
    fn authority_manifest_sha256(&self) -> &str;
    fn resolve(&self, raw: &RawDirectVisualObservation) -> Option<ResolvedDirectVisualRow>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawLightSlotJoin {
    pub light_slot: u8,
    pub light_instance_id: u64,
    pub definition_ptr: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawTiledLightObservation {
    pub update_index: u64,
    pub writer_index: u64,
    pub first_sequence: u64,
    pub last_sequence: u64,
    pub grid_start_inv_cell_size_float4_bits: [u32; 4],
    pub light_index_map_64x64_bgra8: Vec<u8>,
    pub light_data_map_128x1_rgba32f: Vec<u8>,
    pub ordered_slot_joins: Vec<RawLightSlotJoin>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedLightSlot {
    pub light_token: String,
    pub resolved_definition: UsedContentKey,
}

pub trait ExactLightCatalogResolver {
    fn authority_manifest_sha256(&self) -> &str;
    fn resolve_slot(
        &self,
        raw: &RawTiledLightObservation,
        join: &RawLightSlotJoin,
    ) -> Option<ResolvedLightSlot>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PromotionError {
    Invalid(String),
    UnresolvedCatalog {
        registration_token: String,
        packet_ordinal: u32,
    },
}

pub fn promote_entity_submission<R: ExactEntityCatalogResolver>(
    raw: &RawEntitySubmission,
    resolver: &R,
) -> Result<EntitySubmission, PromotionError> {
    check_sha256(
        resolver.authority_manifest_sha256(),
        "entity catalog manifest",
    )?;
    if raw.submit_index == 0
        || raw.entity_instance_id == 0
        || raw.entity_object_ptr == 0
        || raw.entity_definition_ptr == 0
        || raw.renderer_ptr == 0
        || raw.render_context_ptr == 0
        || raw.mesh_draw_ptr == 0
        || raw.material_binding_ptr == 0
        || raw.shader_program_ptr == 0
        || !raw.additional_shader_stages_authenticated_null
        || !raw.stream_output_targets_authenticated_null
    {
        return invalid("entity submission has an incomplete runtime join");
    }
    let row = resolver.resolve(raw).ok_or_else(|| {
        PromotionError::Invalid("entity catalog could not resolve the exact draw".into())
    })?;
    validate_entity_catalog_row(&row, resolver.authority_manifest_sha256())?;
    validate_authenticated_null_stages_and_stream_output(
        &row.shader_program,
        &row.geometry,
        "entity",
    )?;
    let skinning = match &raw.skinning {
        RawSkinning::NotSkinned => Skinning::NotSkinned,
        RawSkinning::Palette3200(bytes) => {
            if bytes.len() != 3_200 {
                return invalid("skinned entity submission does not contain 3,200 palette bytes");
            }
            Skinning::Palette {
                palette_sha256: hex_sha256(bytes),
                palette_3200_bytes: bytes.clone(),
            }
        }
    };
    Ok(EntitySubmission {
        pass_ordinal: raw.pass_ordinal,
        draw_order: raw.draw_order,
        entity_instance_id: raw.entity_instance_id,
        resolved_entity_sha256: row.resolved_entity_sha256,
        content_identity: row.content_identity,
        shader_program: row.shader_program,
        ordered_texture_bindings: row.ordered_texture_bindings,
        pipeline: row.pipeline,
        geometry: row.geometry,
        ordered_constant_buffers: row.ordered_constant_buffers,
        output_state: row.output_state,
        world_transform_4x4_f32_bits: raw.world_transform_4x4_f32_bits,
        state_flags: raw.state_flags,
        skinning,
    })
}

pub fn promote_direct_visual<R: ExactDirectVisualResolver>(
    raw: &RawDirectVisualObservation,
    resolver: &R,
) -> Result<DirectVisualProducer, PromotionError> {
    check_sha256(
        resolver.authority_manifest_sha256(),
        "direct-visual catalog manifest",
    )?;
    if raw.sequence == 0
        || raw.update_index == 0
        || raw.producer_instance_id == 0
        || raw.entity_instance_id == 0
        || raw.entity_definition_ptr == 0
        || raw.semantic_state.is_empty()
        || raw.state_enter_sequence == 0
        || raw.direct_object_list_ordinal >= raw.direct_object_count
    {
        return invalid("direct-visual observation is incomplete");
    }
    let effect_operation = matches!(
        raw.operation,
        DirectVisualOperation::EffectSpawn | DirectVisualOperation::EffectExpire
    );
    if effect_operation != raw.effect_instance_id.is_some() {
        return invalid("direct-visual effect identity disagrees with the operation");
    }
    if matches!(raw.operation, DirectVisualOperation::EffectExpire)
        != raw.effect_expiry_update.is_some()
    {
        return invalid("direct-visual expiry update disagrees with the operation");
    }
    if raw
        .node_or_locator_token
        .as_ref()
        .is_some_and(|value| value.is_empty())
    {
        return invalid("direct-visual node/locator token is blank");
    }
    let row = resolver.resolve(raw).ok_or_else(|| {
        PromotionError::Invalid(
            "direct-visual catalog could not resolve the observed entity".into(),
        )
    })?;
    if row.entity_token.is_empty() {
        return invalid("direct-visual resolver returned an empty entity token");
    }
    validate_content_key(
        &row.resolved_entity_source,
        resolver.authority_manifest_sha256(),
        "direct-visual entity source",
    )?;
    Ok(DirectVisualProducer {
        sequence: raw.sequence,
        update_index: raw.update_index,
        producer_kind: raw.producer_kind,
        producer_instance_id: raw.producer_instance_id,
        entity_instance_id: raw.entity_instance_id,
        effect_instance_id: raw.effect_instance_id.map(|value| value.to_string()),
        operation: raw.operation,
        entity_token: row.entity_token,
        resolved_entity_source_sha256: row.resolved_entity_source.payload_sha256,
        semantic_state: raw.semantic_state.clone(),
        authored_state_index: raw.authored_state_index,
        authored_event_index: raw.authored_event_index,
        state_enter_sequence: raw.state_enter_sequence,
        live_phase_f32_bits: raw.live_phase_f32_bits,
        world_transform_4x4_f32_bits: raw.world_transform_4x4_f32_bits,
        node_or_locator_token: raw.node_or_locator_token.clone(),
        visibility_flags: raw.visibility_flags,
        direct_object_list_ordinal: raw.direct_object_list_ordinal,
        direct_object_count: raw.direct_object_count,
        rng_counter: raw.rng_counter,
        rng_seed: raw.rng_seed,
        effect_expiry_update: raw.effect_expiry_update.map(|value| value.to_string()),
        submitted_outputs: row.submitted_outputs,
    })
}

pub fn promote_tiled_light_observation<R: ExactLightCatalogResolver>(
    raw: &RawTiledLightObservation,
    resolver: &R,
) -> Result<TiledLightSubmission, PromotionError> {
    check_sha256(
        resolver.authority_manifest_sha256(),
        "light catalog manifest",
    )?;
    if raw.update_index == 0
        || raw.writer_index == 0
        || raw.first_sequence == 0
        || raw.last_sequence < raw.first_sequence
        || raw.light_index_map_64x64_bgra8.len() != 16_384
        || raw.light_data_map_128x1_rgba32f.len() != 2_048
    {
        return invalid(
            "tiled-light observation has invalid indices, range, or payload dimensions",
        );
    }
    let mut seen_slots = std::collections::BTreeSet::new();
    let mut resolved = Vec::with_capacity(raw.ordered_slot_joins.len());
    for join in &raw.ordered_slot_joins {
        if join.light_slot >= 64
            || join.light_instance_id == 0
            || join.definition_ptr == 0
            || !seen_slots.insert(join.light_slot)
        {
            return invalid("tiled-light slot join is invalid or duplicated");
        }
        let row = resolver.resolve_slot(raw, join).ok_or_else(|| {
            PromotionError::Invalid("light catalog could not resolve an observed slot".into())
        })?;
        if row.light_token.is_empty() {
            return invalid("light resolver returned an empty token");
        }
        validate_content_key(
            &row.resolved_definition,
            resolver.authority_manifest_sha256(),
            "light definition",
        )?;
        resolved.push(LightSlotJoin {
            light_slot: join.light_slot,
            light_instance_id: join.light_instance_id,
            light_token: row.light_token,
            resolved_definition_sha256: row.resolved_definition.payload_sha256,
        });
    }
    if raw
        .light_index_map_64x64_bgra8
        .iter()
        .copied()
        .filter(|value| *value != 255)
        .any(|value| !seen_slots.contains(&value))
    {
        return invalid("tiled-light index map references a slot without a semantic join");
    }
    Ok(TiledLightSubmission {
        update_index: raw.update_index,
        writer_index: raw.writer_index,
        first_sequence: raw.first_sequence,
        last_sequence: raw.last_sequence,
        grid_start_inv_cell_size_float4_bits: raw.grid_start_inv_cell_size_float4_bits,
        light_index_map_sha256: hex_sha256(&raw.light_index_map_64x64_bgra8),
        light_index_map_64x64_bgra8: raw.light_index_map_64x64_bgra8.clone(),
        light_data_map_sha256: hex_sha256(&raw.light_data_map_128x1_rgba32f),
        light_data_map_128x1_rgba32f: raw.light_data_map_128x1_rgba32f.clone(),
        ordered_slot_semantic_joins: resolved,
    })
}

pub fn promote_submit<R: ExactParticleCatalogResolver>(
    raw: &RawParticleSubmit,
    resolver: &R,
) -> Result<Vec<ParticlePacket>, PromotionError> {
    check_sha256(resolver.authority_manifest_sha256(), "catalog manifest")?;
    let mut out = Vec::with_capacity(raw.packets.len());
    for packet in &raw.packets {
        validate_raw_packet(packet)?;
        let row = resolver
            .resolve(packet)
            .ok_or_else(|| PromotionError::UnresolvedCatalog {
                registration_token: packet.definition.registration_token.clone(),
                packet_ordinal: packet.packet_ordinal,
            })?;
        if row.registration_token != packet.definition.registration_token
            || row.definition_source_token != packet.definition.definition_source_token
        {
            return Err(PromotionError::Invalid(
                "catalog row does not match the observed definition identity".into(),
            ));
        }
        validate_particle_catalog_row(
            &row,
            resolver.authority_manifest_sha256(),
            packet.live_count,
        )?;
        validate_authenticated_null_stages_and_stream_output(
            &row.shader_program,
            &row.geometry,
            "particle",
        )?;
        let packet_hash = hex_sha256(&packet.canonical_packet_112_bytes);
        let records_hash = hex_sha256(&packet.ordered_gpu_instance_records);
        out.push(ParticlePacket {
            effect_instance_id: packet.effect_instance_id,
            registration_token: packet.definition.registration_token.clone(),
            resolved_definition_sha256: row.resolved_definition_sha256,
            content_identity: row.content_identity,
            content_identity_sha256: row.content_identity_sha256,
            bucket: ((packet.render_list_class & 0xffff) << 16)
                | u32::from(packet.render_subbucket),
            packet_ordinal: packet.packet_ordinal,
            draw_order: packet.draw_order,
            ribbon_tail_order: packet.ribbon_tail_order,
            canonical_packet_112_bytes: packet.canonical_packet_112_bytes.clone(),
            canonical_packet_sha256: packet_hash,
            packet_matrix_4x4_f32_bits: packet.packet_matrix_4x4_f32_bits,
            live_count: packet.live_count,
            record_stride: GPU_INSTANCE_STRIDE,
            max_count: packet.max_count,
            ordered_gpu_instance_records: packet.ordered_gpu_instance_records.clone(),
            gpu_instance_records_sha256: records_hash,
            elapsed_f32_bits: packet.elapsed_f32_bits,
            shader_program: row.shader_program,
            ordered_texture_bindings: row.ordered_texture_bindings,
            pipeline: row.pipeline,
            geometry: row.geometry,
            ordered_constant_buffers: row.ordered_constant_buffers,
            output_state: row.output_state,
            atlas_rows: row.atlas_rows,
            atlas_columns: row.atlas_columns,
            scale_f32_bits: packet.scale_f32_bits,
            flags_u16: packet.flags_u16,
        });
    }
    Ok(out)
}

/// Emits the exact runtime portion of the neutral cross-family execution
/// sequence. Static-map and post-process operations are merged only by the
/// authenticated static observer; this function never guesses their slots.
pub fn promote_runtime_execution_order(
    raw: &RawParticleSubmit,
) -> Result<Vec<RenderExecutionRef>, PromotionError> {
    let expected = raw
        .packets
        .len()
        .checked_add(raw.entity_submissions.len())
        .ok_or_else(|| PromotionError::Invalid("runtime draw count overflow".into()))?;
    let mut rows = Vec::with_capacity(expected);
    for packet in &raw.packets {
        rows.push((
            packet.draw_order,
            RenderExecutionRef::Runtime {
                output: RenderOutputRef::Particle {
                    effect_instance_id: packet.effect_instance_id,
                    bucket: ((packet.render_list_class & 0xffff) << 16)
                        | u32::from(packet.render_subbucket),
                    packet_ordinal: packet.packet_ordinal,
                    draw_order: packet.draw_order,
                },
            },
        ));
    }
    for entity in &raw.entity_submissions {
        rows.push((
            entity.draw_order,
            RenderExecutionRef::Runtime {
                output: RenderOutputRef::Entity {
                    pass_ordinal: entity.pass_ordinal,
                    entity_instance_id: entity.entity_instance_id,
                    draw_order: entity.draw_order,
                },
            },
        ));
    }
    rows.sort_by_key(|(order, _)| *order);
    for (expected_order, (actual_order, _)) in rows.iter().enumerate() {
        if usize::try_from(*actual_order).ok() != Some(expected_order) {
            return invalid("runtime execution order is duplicated, gapped, or nonzero-based");
        }
    }
    Ok(rows.into_iter().map(|(_, output)| output).collect())
}

fn validate_particle_catalog_row(
    row: &ResolvedParticleCatalogRow,
    authority_manifest_sha256: &str,
    live_count: u32,
) -> Result<(), PromotionError> {
    check_sha256(&row.resolved_definition_sha256, "definition")?;
    validate_content_key(
        &row.definition_content,
        authority_manifest_sha256,
        "particle definition",
    )?;
    if row.definition_content.payload_sha256 != row.resolved_definition_sha256 {
        return invalid("definition content payload hash disagrees with resolved definition hash");
    }
    if row.content_identity.definition_content != row.definition_content
        || row.content_identity.atlas_rows_u32 != row.atlas_rows
        || row.content_identity.atlas_columns_u32 != row.atlas_columns
        || row.content_identity.computed_sha256() != Ok(row.content_identity_sha256.clone())
    {
        return invalid("particle selector-native content identity is inconsistent");
    }
    validate_content_key(
        &row.content_identity.texture_content,
        authority_manifest_sha256,
        "particle subsystem texture",
    )?;
    validate_content_key(
        &row.shader_program.source,
        authority_manifest_sha256,
        "particle shader source",
    )?;
    if row.shader_program.effect_token.is_empty()
        || row.shader_program.vertex_entry_token.is_empty()
        || row.shader_program.pixel_entry_token.is_empty()
    {
        return invalid("particle shader program identity is incomplete");
    }
    if row.shader_program.effect_token != row.content_identity.engine_effect_token {
        return invalid("particle engine effect token disagrees with compiled program");
    }
    validate_compiled_shader(
        &row.shader_program.compiled_vertex_shader,
        "compiled particle VS",
    )?;
    validate_compiled_shader(
        &row.shader_program.compiled_pixel_shader,
        "compiled particle PS",
    )?;
    if row.ordered_texture_bindings.is_empty() {
        return invalid("particle catalog has no ordered texture binding");
    }
    for binding in &row.ordered_texture_bindings {
        if binding.semantic_token.is_empty() {
            return invalid("particle texture binding semantic is empty");
        }
        validate_content_key(
            &binding.content,
            authority_manifest_sha256,
            "particle texture",
        )?;
        validate_texture_resource(
            &binding.resource,
            authority_manifest_sha256,
            "particle texture",
        )?;
    }
    if !row
        .ordered_texture_bindings
        .iter()
        .any(|binding| binding.content == row.content_identity.texture_content)
    {
        return invalid("particle subsystem texture is not live-bound");
    }
    if row.ordered_constant_buffers.is_empty() {
        return invalid("particle catalog has no exact constant-buffer bindings");
    }
    for binding in &row.ordered_constant_buffers {
        if binding.stage_mask_u8 == 0
            || binding.byte_count as usize != binding.raw_bytes.len()
            || binding.semantic_layout.is_empty()
            || hex_sha256(&binding.raw_bytes) != binding.sha256
        {
            return invalid("particle constant-buffer binding is incomplete or hash-invalid");
        }
    }
    if row.atlas_rows == 0 || row.atlas_columns == 0 {
        return invalid("particle atlas dimensions are zero");
    }
    validate_geometry(
        &row.geometry,
        authority_manifest_sha256,
        true,
        Some(live_count),
    )?;
    Ok(())
}

fn validate_entity_catalog_row(
    row: &ResolvedEntityCatalogRow,
    authority_manifest_sha256: &str,
) -> Result<(), PromotionError> {
    check_sha256(&row.resolved_entity_sha256, "resolved entity")?;
    validate_content_key(
        &row.content_identity.resolved_entity_asset,
        authority_manifest_sha256,
        "entity asset",
    )?;
    if row.content_identity.resolved_entity_asset.payload_sha256 != row.resolved_entity_sha256 {
        return invalid("entity asset payload hash disagrees with resolved entity hash");
    }
    validate_content_key(
        &row.content_identity.mesh,
        authority_manifest_sha256,
        "entity mesh",
    )?;
    if row
        .content_identity
        .native_selection
        .entity_registration_token
        .trim()
        .is_empty()
        || row.content_identity.native_selection.computed_sha256()
            != Ok(row.content_identity.native_selection_sha256.clone())
    {
        return invalid("entity selector-native content identity is incomplete");
    }
    validate_render_bindings(
        &row.shader_program,
        &row.ordered_texture_bindings,
        &row.ordered_constant_buffers,
        authority_manifest_sha256,
        "entity",
    )?;
    validate_geometry(&row.geometry, authority_manifest_sha256, false, None)
}

fn validate_authenticated_null_stages_and_stream_output(
    shader: &ShaderProgram,
    geometry: &GeometryBinding,
    label: &str,
) -> Result<(), PromotionError> {
    if shader.geometry_shader.is_some()
        || shader.hull_shader.is_some()
        || shader.domain_shader.is_some()
        || shader.compute_shader.is_some()
        || !geometry.stream_output_targets.is_empty()
    {
        return invalid(&format!(
            "{label} catalog disagrees with authenticated a729 null optional-stage/stream-output state"
        ));
    }
    Ok(())
}

fn validate_geometry(
    geometry: &GeometryBinding,
    authority_manifest_sha256: &str,
    allow_particle_records: bool,
    expected_instance_count: Option<u32>,
) -> Result<(), PromotionError> {
    if geometry.ordered_vertex_buffers.is_empty() {
        return invalid("geometry has no vertex buffers");
    }
    let mut slots = std::collections::BTreeSet::new();
    let mut particle_sources = 0usize;
    for buffer in &geometry.ordered_vertex_buffers {
        if buffer.stride_bytes == 0 || !slots.insert(buffer.input_slot) {
            return invalid("geometry vertex-buffer slot is duplicated or has zero stride");
        }
        validate_buffer_source(
            &buffer.source,
            authority_manifest_sha256,
            allow_particle_records,
        )?;
        if matches!(buffer.source, BufferSource::ParticleInstanceRecords) {
            particle_sources += 1;
            if buffer.stride_bytes != GPU_INSTANCE_STRIDE {
                return invalid("particle instance geometry does not use the exact 48-byte stride");
            }
        }
    }
    if allow_particle_records && particle_sources != 1 {
        return invalid("particle geometry must bind exactly one captured instance-record source");
    }
    if let Some(index) = &geometry.index_buffer {
        validate_buffer_source(&index.source, authority_manifest_sha256, false)?;
    }
    if let Some(expected) = expected_instance_count {
        let actual = match geometry.draw {
            DrawCommand::DrawInstanced { instance_count, .. }
            | DrawCommand::DrawIndexedInstanced { instance_count, .. } => instance_count,
            _ => return invalid("particle geometry does not use an instanced draw command"),
        };
        if actual != expected {
            return invalid("particle draw instance count disagrees with live_count");
        }
    }
    Ok(())
}

fn validate_buffer_source(
    source: &BufferSource,
    authority_manifest_sha256: &str,
    allow_particle_records: bool,
) -> Result<(), PromotionError> {
    match source {
        BufferSource::Inline { bytes, sha256 } => {
            if bytes.is_empty() || hex_sha256(bytes) != *sha256 {
                return invalid("inline geometry bytes are empty or hash-invalid");
            }
        }
        BufferSource::UsedContentRange {
            content,
            source_byte_offset,
            byte_count,
            range_sha256,
        } => {
            validate_content_key(content, authority_manifest_sha256, "geometry content")?;
            check_sha256(range_sha256, "geometry content range")?;
            if *byte_count == 0
                || source_byte_offset
                    .checked_add(*byte_count)
                    .is_none_or(|end| end > content.payload_bytes)
            {
                return invalid("geometry content range is empty, overflowing, or out of bounds");
            }
        }
        BufferSource::ParticleInstanceRecords if !allow_particle_records => {
            return invalid("entity/static geometry references particle instance records");
        }
        BufferSource::ParticleInstanceRecords => {}
    }
    Ok(())
}

fn validate_render_bindings(
    shader: &ShaderProgram,
    textures: &[TextureBinding],
    constant_buffers: &[ConstantBufferBinding],
    authority_manifest_sha256: &str,
    label: &str,
) -> Result<(), PromotionError> {
    validate_content_key(
        &shader.source,
        authority_manifest_sha256,
        &format!("{label} shader source"),
    )?;
    if shader.effect_token.is_empty()
        || shader.vertex_entry_token.is_empty()
        || shader.pixel_entry_token.is_empty()
    {
        return invalid(&format!("{label} shader program identity is incomplete"));
    }
    if shader
        .ordered_defines
        .iter()
        .any(|define| define.name.trim().is_empty())
    {
        return invalid(&format!("{label} shader has a blank typed define name"));
    }
    validate_compiled_shader(
        &shader.compiled_vertex_shader,
        &format!("compiled {label} VS"),
    )?;
    validate_compiled_shader(
        &shader.compiled_pixel_shader,
        &format!("compiled {label} PS"),
    )?;
    if textures.is_empty() {
        return invalid(&format!("{label} catalog has no ordered texture binding"));
    }
    let mut texture_slots = std::collections::BTreeSet::new();
    for binding in textures {
        if binding.semantic_token.is_empty() || !texture_slots.insert((binding.stage, binding.slot))
        {
            return invalid(&format!(
                "{label} texture binding is unnamed or duplicates a stage/slot"
            ));
        }
        validate_content_key(
            &binding.content,
            authority_manifest_sha256,
            &format!("{label} texture"),
        )?;
        validate_texture_resource(
            &binding.resource,
            authority_manifest_sha256,
            &format!("{label} texture"),
        )?;
    }
    if constant_buffers.is_empty() {
        return invalid(&format!(
            "{label} catalog has no exact constant-buffer bindings"
        ));
    }
    let mut cb_slots = std::collections::BTreeSet::new();
    for binding in constant_buffers {
        if binding.stage_mask_u8 == 0
            || binding.byte_count as usize != binding.raw_bytes.len()
            || binding.semantic_layout.is_empty()
            || hex_sha256(&binding.raw_bytes) != binding.sha256
            || !cb_slots.insert((binding.stage_mask_u8, binding.slot, binding.byte_offset))
        {
            return invalid(&format!(
                "{label} constant-buffer binding is incomplete, duplicated, or hash-invalid"
            ));
        }
    }
    Ok(())
}

fn validate_compiled_shader(
    shader: &portable_render_stream::CompiledShaderPayload,
    label: &str,
) -> Result<(), PromotionError> {
    if shader.compiler_identity.trim().is_empty()
        || shader.target_profile.trim().is_empty()
        || shader.bytecode.is_empty()
        || hex_sha256(&shader.bytecode) != shader.sha256
    {
        return invalid(&format!(
            "{label} has blank provenance, empty bytecode, or an invalid hash"
        ));
    }
    Ok(())
}

fn validate_texture_resource(
    resource: &portable_render_stream::TextureResourceDesc,
    authority_manifest_sha256: &str,
    label: &str,
) -> Result<(), PromotionError> {
    if resource.width == 0
        || resource.height == 0
        || resource.depth == 0
        || resource.mip_levels == 0
        || resource.array_size == 0
        || resource.sample_count != 1
        || resource.ordered_subresources.is_empty()
    {
        return invalid(&format!("{label} resource descriptor is incomplete"));
    }
    for (ordinal, subresource) in resource.ordered_subresources.iter().enumerate() {
        if subresource.subresource_index as usize != ordinal
            || subresource.row_pitch == 0
            || subresource.slice_pitch == 0
        {
            return invalid(&format!("{label} subresource order/pitch is invalid"));
        }
        validate_buffer_source(&subresource.source, authority_manifest_sha256, false)?;
    }
    Ok(())
}

fn validate_content_key(
    key: &UsedContentKey,
    authority_manifest_sha256: &str,
    label: &str,
) -> Result<(), PromotionError> {
    key.validate()
        .map_err(|error| PromotionError::Invalid(format!("{label}: {error}")))?;
    if key.effective_vfs_manifest_sha256 != authority_manifest_sha256 {
        return invalid(&format!(
            "{label} belongs to a different effective-VFS manifest"
        ));
    }
    Ok(())
}

pub fn promote_weather_observation<R: ExactWeatherCatalogResolver>(
    raw: &RawWeatherObservation,
    sequence: u64,
    update_index: u64,
    resolver: &R,
) -> Result<WeatherMembership, PromotionError> {
    check_sha256(
        resolver.authority_manifest_sha256(),
        "weather catalog manifest",
    )?;
    if raw.observation_index == 0 || raw.previous_airborne_runtime_code == raw.airborne_runtime_code
    {
        return invalid("weather observation is not an airborne-code change");
    }
    if raw.airborne_runtime_code == 0 {
        if raw.selection_seed_words.is_some()
            || !raw.ordered_factory_attempts.is_empty()
            || !raw.ordered_live_pairs.is_empty()
        {
            return invalid("code-zero weather change contains selection or live membership");
        }
    } else if raw.selection_seed_words.is_none() {
        return invalid("nonzero weather change is missing the exact selection seed");
    }
    let row = resolver.resolve(raw).ok_or_else(|| {
        PromotionError::Invalid("weather catalog could not resolve exact ordered membership".into())
    })?;
    if (raw.airborne_runtime_code == 0) != row.weather_effect_token.is_none() {
        return invalid("weather token nullability disagrees with runtime code");
    }
    validate_weather_resolution(raw, &row)?;
    Ok(WeatherMembership {
        sequence,
        update_index,
        strategic_region_id: raw.strategic_region_id,
        rain_light_active: raw.rain_light_active,
        rain_heavy_active: raw.rain_heavy_active,
        snow_active: raw.snow_active,
        blizzard_active: raw.blizzard_active,
        sandstorm_active: raw.sandstorm_active,
        arctic_water_active: raw.arctic_water_active,
        region_value_i64_fixed_1e5: raw.region_value_i64_fixed_1e5,
        transition_expiry_game_hour_i32: raw.transition_expiry_game_hour_i32,
        airborne_runtime_code: raw.airborne_runtime_code,
        weather_effect_token: row.weather_effect_token,
        ordered_positions: row.ordered_positions,
        ordered_weather_instances: row.ordered_weather_instances,
    })
}

fn validate_weather_resolution(
    raw: &RawWeatherObservation,
    row: &ResolvedWeatherCatalogRow,
) -> Result<(), PromotionError> {
    let mut observed_entity_ids = Vec::new();
    for pair in &raw.ordered_live_pairs {
        if let Some(primary) = &pair.primary {
            observed_entity_ids.push(primary.entity_instance_id);
        }
        if let Some(fading) = &pair.fading {
            observed_entity_ids.push(fading.entity_instance_id);
        }
    }
    let mut successful_factory_ids = Vec::new();
    let mut allocated_factory_ids = std::collections::BTreeSet::new();
    for attempt in &raw.ordered_factory_attempts {
        if attempt.definition_ptr == 0
            || attempt.allocated_entity_instance_id == 0
            || !allocated_factory_ids.insert(attempt.allocated_entity_instance_id)
        {
            return invalid("weather factory attempt identity is invalid or duplicated");
        }
        if let Some(returned) = attempt.returned_entity_instance_id {
            if returned != attempt.allocated_entity_instance_id {
                return invalid("weather factory returned a different stable entity id");
            }
            successful_factory_ids.push(returned);
        }
    }
    if successful_factory_ids != observed_entity_ids {
        return invalid("weather factory successes disagree with the live pair vector");
    }
    let mut expected_pairs = Vec::new();
    let mut expected_effects = Vec::new();
    let mut last_ordinal = None;
    for position in &row.ordered_positions {
        if last_ordinal.is_some_and(|value| position.position_ordinal <= value) {
            return invalid("weather position ordinals are not strictly increasing");
        }
        last_ordinal = Some(position.position_ordinal);
        match position.resolution {
            WeatherResolution::ExplicitEmpty => {
                if position.primary_entity_instance_id.is_some()
                    || position.fading_entity_instance_id.is_some()
                    || !position.ordered_effect_instance_ids.is_empty()
                    || position.resolved_variant_token.is_some()
                {
                    return invalid("explicit-empty weather position contains live identity");
                }
            }
            WeatherResolution::Instantiated => {
                if position.selected_authored_effect_index.is_none()
                    || position.resolved_variant_token.is_none()
                    || (position.primary_entity_instance_id.is_none()
                        && position.fading_entity_instance_id.is_none())
                {
                    return invalid("instantiated weather position is incomplete");
                }
                expected_pairs.push((
                    position.primary_entity_instance_id.clone(),
                    position.fading_entity_instance_id.clone(),
                    position.ordered_effect_instance_ids.clone(),
                ));
                expected_effects.extend(position.ordered_effect_instance_ids.iter().cloned());
            }
        }
    }
    let observed_pairs: Vec<_> = raw
        .ordered_live_pairs
        .iter()
        .map(|pair| {
            let mut effects = Vec::new();
            if let Some(primary) = &pair.primary {
                effects.extend(
                    primary
                        .ordered_effect_instance_ids
                        .iter()
                        .map(u64::to_string),
                );
            }
            if let Some(fading) = &pair.fading {
                effects.extend(
                    fading
                        .ordered_effect_instance_ids
                        .iter()
                        .map(u64::to_string),
                );
            }
            (
                pair.primary
                    .as_ref()
                    .map(|value| value.entity_instance_id.to_string()),
                pair.fading
                    .as_ref()
                    .map(|value| value.entity_instance_id.to_string()),
                effects,
            )
        })
        .collect();
    if expected_pairs != observed_pairs {
        return invalid("resolved weather positions disagree with the exact live pair vector");
    }
    let instance_effects: Vec<_> = row
        .ordered_weather_instances
        .iter()
        .map(|value| value.effect_instance_id.to_string())
        .collect();
    if instance_effects != expected_effects {
        return invalid("weather instance list disagrees with ordered position effects");
    }
    Ok(())
}

/// Reproduces the producer's exact per-position selection word from the two
/// captured seed words. The caller must advance `seed_words[0]` once after
/// each authored position, including positions whose selected vector is empty.
pub fn weather_selection_word(seed_words: [u32; 2]) -> u32 {
    let mut value = seed_words[1].wrapping_add(seed_words[0].wrapping_mul(0xb529_7a4d));
    value = (value >> 8 ^ value).wrapping_add(0x68e3_1da4);
    value = (value.wrapping_mul(0x100) ^ value).wrapping_mul(0x1b56_c4e9);
    value = (value >> 8 ^ value).wrapping_mul(0x92d6_8ca2);
    (value >> 8 ^ value) & 0x7fff_ffff
}

pub fn validate_raw_packet(packet: &RawParticlePacket) -> Result<(), PromotionError> {
    if packet.effect_instance_id == 0
        || packet.definition.registration_token.is_empty()
        || packet.definition.definition_source_token.is_empty()
        || packet.shader_program_ptr == 0
        || !packet.additional_shader_stages_authenticated_null
        || !packet.stream_output_targets_authenticated_null
    {
        return invalid("packet identity is incomplete");
    }
    if fnv1a32(packet.definition.registration_token.as_bytes())
        != packet.definition.registration_hash_u32
    {
        return invalid("registration FNV does not match token");
    }
    if packet.canonical_packet_112_bytes.len() != PACKET_BYTES
        || packet.canonical_packet_112_bytes[PACKET_POINTER_OFFSET..PACKET_POINTER_OFFSET + 8]
            != [0; 8]
    {
        return invalid("packet is not a pointer-zero canonical 0x70 record");
    }
    let expected = usize::try_from(packet.live_count)
        .ok()
        .and_then(|count| count.checked_mul(GPU_INSTANCE_STRIDE as usize))
        .ok_or_else(|| PromotionError::Invalid("GPU instance byte count overflow".into()))?;
    if packet.live_count > packet.max_count || packet.ordered_gpu_instance_records.len() != expected
    {
        return invalid("GPU instance bytes/live/max mismatch");
    }
    if u32::from_le_bytes(
        packet.canonical_packet_112_bytes[0x40..0x44]
            .try_into()
            .unwrap(),
    ) != packet.live_count
        || u32::from_le_bytes(
            packet.canonical_packet_112_bytes[0x50..0x54]
                .try_into()
                .unwrap(),
        ) != GPU_INSTANCE_STRIDE
        || u32::from_le_bytes(
            packet.canonical_packet_112_bytes[0x54..0x58]
                .try_into()
                .unwrap(),
        ) != packet.max_count
        || u32::from_le_bytes(
            packet.canonical_packet_112_bytes[0x58..0x5c]
                .try_into()
                .unwrap(),
        ) != packet.elapsed_f32_bits
    {
        return invalid("typed packet fields disagree with canonical bytes");
    }
    Ok(())
}

pub fn fnv1a32(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0x811c_9dc5u32, |hash, byte| {
        (hash ^ u32::from(*byte)).wrapping_mul(0x0100_0193)
    })
}

pub fn canonicalize_packet(mut packet: [u8; PACKET_BYTES]) -> [u8; PACKET_BYTES] {
    packet[PACKET_POINTER_OFFSET..PACKET_POINTER_OFFSET + 8].fill(0);
    packet
}

pub fn hex_sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn check_sha256(value: &str, label: &str) -> Result<(), PromotionError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return invalid(&format!("{label} SHA-256 is not canonical lowercase hex"));
    }
    Ok(())
}

fn invalid<T>(message: &str) -> Result<T, PromotionError> {
    Err(PromotionError::Invalid(message.into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use portable_content_manifest::ContentLayerKind;
    use portable_render_stream::{
        AddressMode, BlendDesc, BlendFactor, BlendOperation, Comparison, CompiledShaderPayload,
        CullMode, DepthDesc, EntityNativeSelection, FillMode, Filter, IndexBufferBinding,
        IndexFormat, InputElementDesc, ParticleContentIdentity, PreparedRenderTarget,
        PrimitiveTopology, RasterizerDesc, RenderTargetDesc, SamplerDesc, ShaderResourceViewDesc,
        ShaderStage, StaticMapOperation, StencilFaceDesc, StencilOperation, TextureResourceDesc,
        TextureResourceDimension, TextureSubresourcePayload, VertexBufferBinding, ViewportDesc,
    };

    fn content(path: &str, payload_sha256: String) -> UsedContentKey {
        UsedContentKey {
            effective_vfs_manifest_sha256: "1".repeat(64),
            normalized_game_relative_path: path.into(),
            winning_layer_ordinal: 0,
            layer_kind: ContentLayerKind::Base,
            layer_id: "base".into(),
            source_sha256: payload_sha256.clone(),
            payload_sha256,
            payload_bytes: 1,
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
                additional_targets: Vec::new(),
                blend_factor_rgba_f32_bits: [0; 4],
                sample_mask_u32: u32::MAX,
            },
            depth: DepthDesc {
                enabled: true,
                write_enabled: false,
                comparison: Comparison::LessEqual,
                stencil_enabled: false,
                stencil_read_mask_u8: u8::MAX,
                stencil_write_mask_u8: u8::MAX,
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
                depth_bias_clamp_f32_bits: 0,
                slope_scaled_depth_bias_f32_bits: 0,
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
                mip_lod_bias_f32_bits: 0,
                max_anisotropy: 1,
                border_color_rgba_f32_bits: [0; 4],
                min_lod_f32_bits: 0,
                max_lod_f32_bits: f32::INFINITY.to_bits(),
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

    fn index_source() -> BufferSource {
        let bytes = vec![0, 0, 1, 0, 2, 0, 2, 0, 1, 0, 3, 0];
        BufferSource::Inline {
            sha256: hex_sha256(&bytes),
            bytes,
        }
    }

    fn particle_geometry(instance_count: u32) -> GeometryBinding {
        GeometryBinding {
            ordered_vertex_buffers: vec![VertexBufferBinding {
                input_slot: 0,
                stride_bytes: GPU_INSTANCE_STRIDE,
                byte_offset: 0,
                source: BufferSource::ParticleInstanceRecords,
            }],
            index_buffer: Some(IndexBufferBinding {
                format: IndexFormat::R16Uint,
                byte_offset: 0,
                source: index_source(),
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
        let bytes = vec![0; 48];
        GeometryBinding {
            ordered_vertex_buffers: vec![VertexBufferBinding {
                input_slot: 0,
                stride_bytes: 48,
                byte_offset: 0,
                source: BufferSource::Inline {
                    sha256: hex_sha256(&bytes),
                    bytes,
                },
            }],
            index_buffer: Some(IndexBufferBinding {
                format: IndexFormat::R16Uint,
                byte_offset: 0,
                source: index_source(),
            }),
            stream_output_targets: Vec::new(),
            draw: DrawCommand::DrawIndexed {
                index_count: 6,
                start_index: 0,
                base_vertex: 0,
            },
        }
    }

    fn compiled_shader(profile: &str, byte: u8) -> CompiledShaderPayload {
        let bytecode = vec![byte; 8];
        CompiledShaderPayload {
            compiler_identity: "fxc-10.1".into(),
            target_profile: profile.into(),
            compiler_flags_u64: 0,
            sha256: hex_sha256(&bytecode),
            bytecode,
        }
    }

    fn texture_resource() -> TextureResourceDesc {
        let bytes = vec![0; 4];
        TextureResourceDesc {
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
            bind_flags_u32: 8,
            cpu_access_flags_u32: 0,
            misc_flags_u32: 0,
            ordered_subresources: vec![TextureSubresourcePayload {
                subresource_index: 0,
                mip_level: 0,
                array_slice: 0,
                row_pitch: 4,
                slice_pitch: 4,
                source: BufferSource::Inline {
                    sha256: hex_sha256(&bytes),
                    bytes,
                },
            }],
        }
    }

    struct Resolver;
    impl ExactParticleCatalogResolver for Resolver {
        fn authority_manifest_sha256(&self) -> &str {
            "1111111111111111111111111111111111111111111111111111111111111111"
        }
        fn resolve(&self, raw: &RawParticlePacket) -> Option<ResolvedParticleCatalogRow> {
            let definition = &raw.definition;
            let definition_hash = "2f".repeat(32);
            let cb_bytes = vec![0; 16];
            let definition_content = content(
                "gfx/particles/environment/rain_clouds.asset",
                definition_hash.clone(),
            );
            let texture_content = content("gfx/particles/cloud.dds", "4f".repeat(32));
            let content_identity = ParticleContentIdentity {
                definition_content: definition_content.clone(),
                subsystem_selector_u32: raw.subsystem_selector_u32,
                texture_content: texture_content.clone(),
                authored_shader_token: "ParticleAdditive".into(),
                engine_effect_token: raw.engine_effect_token.clone(),
                atlas_columns_u32: 1,
                atlas_rows_u32: 1,
                atlas_half_pixel_width_f32_bits: raw.atlas_half_pixel_width_f32_bits,
                atlas_half_pixel_height_f32_bits: raw.atlas_half_pixel_height_f32_bits,
            };
            let content_identity_sha256 = content_identity.computed_sha256().ok()?;
            Some(ResolvedParticleCatalogRow {
                registration_token: definition.registration_token.clone(),
                definition_source_token: definition.definition_source_token.clone(),
                resolved_definition_sha256: definition_hash.clone(),
                definition_content,
                content_identity,
                content_identity_sha256,
                shader_program: ShaderProgram {
                    source: content("gfx/fx/particle.shader", "3f".repeat(32)),
                    effect_token: "ParticleAdditive".into(),
                    vertex_entry_token: "VertexSimpleParticle".into(),
                    pixel_entry_token: "PixelSimpleParticle".into(),
                    ordered_defines: vec![],
                    compiled_vertex_shader: compiled_shader("vs_5_0", 5),
                    compiled_pixel_shader: compiled_shader("ps_5_0", 6),
                    geometry_shader: None,
                    hull_shader: None,
                    domain_shader: None,
                    compute_shader: None,
                },
                ordered_texture_bindings: vec![TextureBinding {
                    stage: ShaderStage::Pixel,
                    slot: 0,
                    semantic_token: "DiffuseMap".into(),
                    content: texture_content,
                    resource: texture_resource(),
                    view: ShaderResourceViewDesc::Texture2d {
                        dxgi_format_u32: 28,
                        most_detailed_mip: 0,
                        mip_levels: 1,
                    },
                }],
                pipeline: pipeline(),
                geometry: particle_geometry(raw.live_count),
                ordered_constant_buffers: vec![ConstantBufferBinding {
                    stage_mask_u8: 1,
                    slot: 0,
                    byte_offset: 0,
                    byte_count: cb_bytes.len() as u32,
                    sha256: hex_sha256(&cb_bytes),
                    raw_bytes: cb_bytes,
                    semantic_layout: "ProjectionMatrix".into(),
                }],
                output_state: RenderOutputState {
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
                },
                atlas_rows: 1,
                atlas_columns: 1,
            })
        }
    }

    struct EntityResolver;
    impl ExactEntityCatalogResolver for EntityResolver {
        fn authority_manifest_sha256(&self) -> &str {
            "1111111111111111111111111111111111111111111111111111111111111111"
        }

        fn resolve(&self, _: &RawEntitySubmission) -> Option<ResolvedEntityCatalogRow> {
            let mut raw = packet();
            raw.definition = DefinitionIdentity {
                registration_token: "entity_test".into(),
                definition_source_token: "gfx/entities/test.asset".into(),
                registration_hash_u32: fnv1a32(b"entity_test"),
            };
            let particle = Resolver.resolve(&raw)?;
            let entity_hash = "7f".repeat(32);
            Some(ResolvedEntityCatalogRow {
                resolved_entity_sha256: entity_hash.clone(),
                content_identity: {
                    let native_selection = EntityNativeSelection {
                        entity_registration_token: "entity_test".into(),
                        entity_group_index: 2,
                        mesh_draw_list_index: 1,
                        entity_variant_u8: 0,
                        mesh_draw_variant_u8: 0,
                        pass_selector_i32: 0,
                        alternate_list_u8: 0,
                        bind_renderer_target_u8: 0,
                    };
                    EntityContentIdentity {
                        resolved_entity_asset: content("gfx/entities/test.asset", entity_hash),
                        mesh: content("gfx/models/test.mesh", "8f".repeat(32)),
                        native_selection_sha256: native_selection.computed_sha256().ok()?,
                        native_selection,
                    }
                },
                shader_program: particle.shader_program,
                ordered_texture_bindings: particle.ordered_texture_bindings,
                pipeline: particle.pipeline,
                geometry: entity_geometry(),
                ordered_constant_buffers: particle.ordered_constant_buffers,
                output_state: particle.output_state,
            })
        }
    }

    struct DirectResolver;
    impl ExactDirectVisualResolver for DirectResolver {
        fn authority_manifest_sha256(&self) -> &str {
            "1111111111111111111111111111111111111111111111111111111111111111"
        }

        fn resolve(&self, _: &RawDirectVisualObservation) -> Option<ResolvedDirectVisualRow> {
            Some(ResolvedDirectVisualRow {
                entity_token: "fighter_entity".into(),
                resolved_entity_source: content("gfx/entities/air.asset", "9f".repeat(32)),
                submitted_outputs: vec![],
            })
        }
    }

    struct LightResolver;
    impl ExactLightCatalogResolver for LightResolver {
        fn authority_manifest_sha256(&self) -> &str {
            "1111111111111111111111111111111111111111111111111111111111111111"
        }

        fn resolve_slot(
            &self,
            _: &RawTiledLightObservation,
            _: &RawLightSlotJoin,
        ) -> Option<ResolvedLightSlot> {
            Some(ResolvedLightSlot {
                light_token: "alarm_flash".into(),
                resolved_definition: content("gfx/entities/map_lights.asset", "af".repeat(32)),
            })
        }
    }

    fn packet() -> RawParticlePacket {
        let token = "rain_clouds_file";
        let mut raw = [0u8; PACKET_BYTES];
        raw[0x40..0x44].copy_from_slice(&2u32.to_le_bytes());
        raw[0x48..0x50].copy_from_slice(&0x1234_5678usize.to_le_bytes());
        raw[0x50..0x54].copy_from_slice(&48u32.to_le_bytes());
        raw[0x54..0x58].copy_from_slice(&4u32.to_le_bytes());
        raw[0x58..0x5c].copy_from_slice(&0x3f80_0000u32.to_le_bytes());
        RawParticlePacket {
            effect_instance_id: 7,
            definition: DefinitionIdentity {
                registration_token: token.into(),
                definition_source_token: "gfx/particles/environment/rain_clouds.asset".into(),
                registration_hash_u32: fnv1a32(token.as_bytes()),
            },
            render_list_class: 2,
            render_subbucket: 3,
            builder_call_ordinal: 0,
            packet_ordinal: 9,
            particle_type_definition_ptr: 0x3300,
            subsystem_selector_u32: 9,
            shader_program_index_u32: 1,
            texture_runtime_index_u32: 2,
            engine_effect_token: "ParticleAdditive".into(),
            atlas_half_pixel_width_f32_bits: 0.5f32.to_bits(),
            atlas_half_pixel_height_f32_bits: 0.5f32.to_bits(),
            atlas_columns_f32_bits: 1.0f32.to_bits(),
            atlas_rows_f32_bits: 1.0f32.to_bits(),
            draw_order: 0,
            shader_program_ptr: 0x4400,
            additional_shader_stages_authenticated_null: true,
            stream_output_targets_authenticated_null: true,
            ribbon_tail_order: None,
            canonical_packet_112_bytes: canonicalize_packet(raw).to_vec(),
            packet_matrix_4x4_f32_bits: [0; 16],
            live_count: 2,
            max_count: 4,
            ordered_gpu_instance_records: vec![0; 96],
            elapsed_f32_bits: 0x3f80_0000,
            scale_f32_bits: 0x3f80_0000,
            flags_u16: 0,
            parent_entity_instance_id: Some(11),
        }
    }

    fn observed_vfs(path: &str, byte: u8) -> RawVfsContentIdentity {
        RawVfsContentIdentity {
            logical_path: path.into(),
            winning_layer_ordinal: 0,
            layer_kind: "base".into(),
            layer_id: "base".into(),
            source_sha256: format!("{byte:02x}").repeat(32),
            payload_sha256: format!("{byte:02x}").repeat(32),
            payload_bytes: 1,
        }
    }

    struct LiveSemantics;
    impl ExactLiveRenderCatalogSemantics for LiveSemantics {
        fn resolve_program_identity(
            &self,
            _: &RawParticleSubmit,
            draw: &RawD3dDrawState,
        ) -> Result<RawProgramIdentity, String> {
            Ok(RawProgramIdentity {
                observation_sequence: draw.final_draw_sequence,
                shader_program_ptr: draw.shader_program_ptr,
                source: observed_vfs("gfx/fx/mod_particle.shader", 0x31),
                effect_token: "mod_effect".into(),
                vertex_entry_token: "VSMain".into(),
                pixel_entry_token: "PSMain".into(),
                ordered_defines: vec![RawShaderDefine {
                    name: "MOD_DEFINE".into(),
                    value: "1".into(),
                    scope: "effect".into(),
                }],
            })
        }

        fn resolve_draw_semantics(
            &self,
            _: &RawParticleSubmit,
            draw: &RawD3dDrawState,
        ) -> Result<RawDrawSemantics, String> {
            Ok(RawDrawSemantics {
                observation_sequence: draw.final_draw_sequence,
                identity: draw.identity.clone(),
                content_identity: RawDrawContentIdentity::Particle {
                    registration_token: "rain_clouds_file".into(),
                    definition_source_token: "gfx/particles/environment/rain_clouds.asset".into(),
                    subsystem_selector_u32: 9,
                    definition_content: observed_vfs("gfx/particles/mod_particle.asset", 0x32),
                    texture_content: observed_vfs("gfx/particles/cloud.dds", 0x33),
                    authored_shader_token: "ParticleAdditive".into(),
                    engine_effect_token: "ParticleAdditive".into(),
                    atlas_columns_u32: 1,
                    atlas_rows_u32: 1,
                    atlas_half_pixel_width_f32_bits: 0.5f32.to_bits(),
                    atlas_half_pixel_height_f32_bits: 0.5f32.to_bits(),
                },
                ordered_texture_semantics: vec![],
                ordered_sampler_semantics: vec![],
                ordered_constant_buffer_semantics: vec![],
                ordered_color_targets: vec![],
                depth_target: None,
            })
        }
    }

    fn empty_live_capture() -> RawRenderSubmitCapture {
        let packet = packet();
        let identity = RawDrawIdentity::Particle {
            effect_instance_id: packet.effect_instance_id,
            registration_token: packet.definition.registration_token.clone(),
            definition_source_token: packet.definition.definition_source_token.clone(),
            builder_call_ordinal: packet.builder_call_ordinal,
            packet_ordinal: packet.packet_ordinal,
            draw_order: packet.draw_order,
            shader_program_ptr: packet.shader_program_ptr,
        };
        RawRenderSubmitCapture {
            submit: RawParticleSubmit {
                submit_index: 1,
                interpolation_phase_f32_bits: 0,
                packets: vec![packet],
                entity_submissions: vec![],
            },
            shader_reflections: Vec::new(),
            catalog: RawRenderCatalogCapture {
                process: RawProcessEpoch {
                    process_id: 1,
                    process_start_nonce: 2,
                    executable_sha256: "30".repeat(32),
                },
                launch_epoch: RawCaptureLaunchEpoch {
                    process_id: 1,
                    process_start_nonce: 2,
                    observer_installed_qpc: 3,
                    d3d_device_created_qpc: 4,
                    first_creation_sequence: 1,
                    hook_epoch_nonce: 5,
                    hook_set_receipt_sha256: "33".repeat(32),
                    capture_receipt_sha256: "34".repeat(32),
                },
                submit_index: 1,
                frame_sequence: 1,
                draw_states: vec![RawD3dDrawState {
                    final_draw_sequence: 10,
                    identity,
                    family: RawDrawFamily::Particle,
                    submit_index: 1,
                    draw_order: 0,
                    render_context_ptr: 0x1000,
                    device_context_ptr: 0x2000,
                    shader_program_ptr: 0x4400,
                    vertex_shader_ptr: 0x5000,
                    pixel_shader_ptr: 0x6000,
                    geometry_shader_ptr: None,
                    hull_shader_ptr: None,
                    domain_shader_ptr: None,
                    compute_shader_ptr: None,
                    dynamic_shader_linkage_present: false,
                    input_layout_ptr: None,
                    blend_state_ptr: None,
                    blend_factor_rgba_f32_bits: [0; 4],
                    sample_mask_u32: u32::MAX,
                    depth_stencil_state_ptr: None,
                    stencil_reference_u32: 0,
                    rasterizer_state_ptr: None,
                    primitive_topology_u32: 4,
                    ordered_texture_objects: vec![],
                    ordered_sampler_objects: vec![],
                    ordered_constant_buffer_objects: vec![],
                    ordered_vertex_buffer_objects: vec![],
                    index_buffer: None,
                    ordered_stream_output_buffer_ptrs: vec![],
                    ordered_color_target_view_ptrs: vec![],
                    depth_stencil_view_ptr: None,
                    ordered_viewports_f32_bits: vec![],
                    ordered_scissor_rects_i32: vec![],
                    draw_call: RawDrawCall::Draw {
                        vertex_count: 3,
                        start_vertex: 0,
                    },
                }],
                draw_semantics: vec![],
                program_identities: vec![],
                shader_creations: vec![],
                input_layout_creations: vec![],
                blend_state_creations: vec![],
                depth_stencil_state_creations: vec![],
                rasterizer_state_creations: vec![],
                sampler_state_creations: vec![],
                buffer_snapshots: vec![],
                texture_resource_snapshots: vec![],
                shader_resource_view_snapshots: vec![],
                render_target_view_snapshots: vec![],
            },
        }
    }

    #[test]
    fn live_semantics_materialize_once_without_retargeting() {
        let mut capture = empty_live_capture();
        materialize_live_render_catalog_semantics(&mut capture, &LiveSemantics).unwrap();
        assert_eq!(capture.catalog.program_identities.len(), 1);
        assert_eq!(capture.catalog.draw_semantics.len(), 1);
        assert_eq!(
            capture.catalog.program_identities[0].observation_sequence,
            capture.catalog.draw_states[0].final_draw_sequence
        );
        assert!(materialize_live_render_catalog_semantics(&mut capture, &LiveSemantics).is_err());
    }

    #[test]
    fn pointer_is_zero_and_single_gpu_blob_promotes() {
        let raw = RawParticleSubmit {
            submit_index: 1,
            interpolation_phase_f32_bits: 0,
            packets: vec![packet()],
            entity_submissions: vec![],
        };
        let promoted = promote_submit(&raw, &Resolver).unwrap();
        assert_eq!(promoted.len(), 1);
        assert_eq!(&promoted[0].canonical_packet_112_bytes[0x48..0x50], &[0; 8]);
        assert_eq!(promoted[0].ordered_gpu_instance_records.len(), 96);
        assert_eq!(promoted[0].elapsed_f32_bits, 0x3f80_0000);
    }

    #[test]
    fn rejects_second_blob_semantics_disguised_as_stride() {
        let mut value = packet();
        value.canonical_packet_112_bytes[0x58..0x5c].copy_from_slice(&48u32.to_le_bytes());
        assert!(validate_raw_packet(&value).is_err());
    }

    #[test]
    fn particle_promotion_rejects_unauthenticated_optional_stage_nulls() {
        let mut value = packet();
        value.additional_shader_stages_authenticated_null = false;
        assert!(validate_raw_packet(&value).is_err());
        value.additional_shader_stages_authenticated_null = true;
        value.stream_output_targets_authenticated_null = false;
        assert!(validate_raw_packet(&value).is_err());
    }

    #[test]
    fn runtime_execution_order_is_one_cross_family_contiguous_sequence() {
        let mut particle = packet();
        particle.draw_order = 1;
        let entity = RawEntitySubmission {
            submit_index: 3,
            pass_ordinal: 4,
            draw_order: 0,
            entity_instance_id: 9,
            entity_registration_token: Some("test_entity".into()),
            entity_object_ptr: 1,
            entity_definition_ptr: 2,
            renderer_ptr: 3,
            render_context_ptr: 4,
            mesh_draw_ptr: 5,
            material_binding_ptr: 6,
            entity_group_index: 0,
            mesh_draw_list_index: 0,
            entity_variant_u8: 0,
            mesh_draw_variant_u8: 0,
            pass_selector_i32: 0,
            bind_renderer_target_u8: 0,
            alternate_list_u8: 0,
            shader_program_ptr: 7,
            additional_shader_stages_authenticated_null: true,
            stream_output_targets_authenticated_null: true,
            world_transform_4x4_f32_bits: [0; 16],
            state_flags: 0,
            skinning: RawSkinning::NotSkinned,
        };
        let raw = RawParticleSubmit {
            submit_index: 3,
            interpolation_phase_f32_bits: 0,
            packets: vec![particle],
            entity_submissions: vec![entity],
        };
        let order = promote_runtime_execution_order(&raw).unwrap();
        assert!(matches!(
            order[0],
            RenderExecutionRef::Runtime {
                output: RenderOutputRef::Entity { draw_order: 0, .. }
            }
        ));
        assert!(matches!(
            order[1],
            RenderExecutionRef::Runtime {
                output: RenderOutputRef::Particle { draw_order: 1, .. }
            }
        ));
        let mut gapped = raw;
        gapped.packets[0].draw_order = 2;
        assert!(promote_runtime_execution_order(&gapped).is_err());
    }

    #[test]
    fn map_snapshot_validates_complete_partition_and_many_paths_per_role() {
        let consumer = PRODUCTION_MAP_CONTENT_CONSUMERS[0];
        let observations = ["gfx/map/terrain_a.dds", "gfx/map/terrain_b.dds"]
            .into_iter()
            .enumerate()
            .map(|(index, path)| MapContentObservation {
                content_key: path.into(),
                content: content(path, format!("{:064x}", index + 2)),
                consumer: consumer.into(),
                evidence: MapContentObservationEvidence::StaticProgram {
                    submit_index: 1,
                    program_ordinal: 0,
                    pass_token: consumer.into(),
                    program_payload_sha256: "3".repeat(64),
                },
                corroboration_sha256: "3".repeat(64),
            })
            .collect();
        let omitted_roles = PRODUCTION_MAP_CONTENT_CONSUMERS[1..]
            .iter()
            .map(|consumer| OmittedMapContentRole {
                consumer: (*consumer).into(),
                reason: "no authenticated observation in this capture".into(),
            })
            .collect();
        let snapshot =
            AuthenticatedMapContentSnapshot::new("1".repeat(64), observations, omitted_roles)
                .unwrap();
        snapshot.validate().unwrap();
        assert_eq!(snapshot.observations.len(), 2);
        let payload = encode_map_content_snapshot_payload(&snapshot).unwrap();
        assert_eq!(
            decode_map_content_snapshot_payload(&payload).unwrap(),
            snapshot
        );
        let mut noncanonical = payload;
        noncanonical.push(b'\n');
        assert!(decode_map_content_snapshot_payload(&noncanonical).is_err());
    }

    #[test]
    fn direct_map_snapshot_construction_cannot_bypass_provenance_or_partition() {
        let mut snapshot = AuthenticatedMapContentSnapshot {
            schema: MAP_CONTENT_SNAPSHOT_SCHEMA.into(),
            authority_manifest_sha256: "1".repeat(64),
            observations: Vec::new(),
            omitted_roles: Vec::new(),
        };
        assert!(snapshot.validate().is_err());
        snapshot.omitted_roles = PRODUCTION_MAP_CONTENT_CONSUMERS
            .iter()
            .map(|consumer| OmittedMapContentRole {
                consumer: (*consumer).into(),
                reason: "capture omitted role".into(),
            })
            .collect();
        snapshot.validate().unwrap();
        snapshot.omitted_roles[0].reason.clear();
        assert!(snapshot.validate().is_err());
    }

    #[test]
    fn map_shader_lookup_accepts_live_paths_only_under_proven_registration_roots() {
        let receipt = RawMapShaderLookup {
            sequence: 1,
            engine_lookup_rva: ENGINE_SHADER_EFFECT_LOOKUP_RVA,
            registration_root_rva: MAP_EFFECT_REGISTRATION_ROOT_RVA,
            logical_shader_path: "gfx/FX/mod_replaced_map_shader.lua".into(),
            effect_name: "terrain".into(),
            program_index: 7,
        };
        receipt.validate().unwrap();
        let mut forged = receipt;
        forged.registration_root_rva = 0x00ab_cdef;
        assert!(forged.validate().is_err());
    }

    #[test]
    fn live_map_lookup_promotes_only_through_an_authenticated_vfs_closure() {
        let authority = "1".repeat(64);
        let terrain_content = content("gfx/fx/pdxmap.shader", "2".repeat(64));
        let lookup_key = MapShaderLookupKey {
            registration_root_rva: MAP_EFFECT_REGISTRATION_ROOT_RVA,
            logical_shader_path: "gfx/fx/pdxmap.lua".into(),
            effect_name: "terrain".into(),
        };
        let closure = AuthenticatedMapRoleClosure {
            role: MapContentRole::Terrain,
            complete_dependency_closure: true,
            dependency_closure_sha256: map_role_closure_sha256(
                MapContentRole::Terrain,
                std::slice::from_ref(&lookup_key),
                std::slice::from_ref(&terrain_content),
            )
            .unwrap(),
            lookup_keys: vec![lookup_key],
            content: vec![terrain_content.clone()],
        };
        let resolver =
            EffectiveVfsMapContentResolver::new(authority, [terrain_content.clone()], [closure])
                .unwrap();
        let raw = RawMapShaderLookup {
            sequence: 1,
            engine_lookup_rva: ENGINE_SHADER_EFFECT_LOOKUP_RVA,
            registration_root_rva: MAP_EFFECT_REGISTRATION_ROOT_RVA,
            logical_shader_path: "gfx/FX/pdxmap.lua".into(),
            effect_name: "terrain".into(),
            program_index: 7,
        };

        let snapshot = promote_map_content_lookups(std::slice::from_ref(&raw), &resolver).unwrap();
        assert_eq!(snapshot.observations.len(), 1);
        assert_eq!(snapshot.omitted_roles.len(), 10);
        assert_eq!(snapshot.observations[0].content, terrain_content);
        assert_eq!(
            snapshot.observations[0].evidence,
            MapContentObservationEvidence::EngineLookup {
                sequence: raw.sequence,
                engine_lookup_rva: raw.engine_lookup_rva,
                registration_root_rva: raw.registration_root_rva,
                logical_shader_path: raw.logical_shader_path.clone(),
                effect_name: raw.effect_name.clone(),
                program_index: raw.program_index,
            }
        );
        let payload = encode_map_content_snapshot_payload(&snapshot).unwrap();
        assert_eq!(
            decode_map_content_snapshot_payload(&payload).unwrap(),
            snapshot
        );

        let mut failed_lookup = raw;
        failed_lookup.program_index = -1;
        assert!(promote_map_content_lookups(&[failed_lookup], &resolver).is_err());
    }

    #[test]
    fn static_program_evidence_rejects_unknown_or_mismatched_consumer() {
        let consumer = PRODUCTION_MAP_CONTENT_CONSUMERS[0];
        let mut snapshot = AuthenticatedMapContentSnapshot::new(
            "1".repeat(64),
            vec![MapContentObservation {
                content_key: "gfx/map/terrain.dds".into(),
                content: content("gfx/map/terrain.dds", "2".repeat(64)),
                consumer: consumer.into(),
                evidence: MapContentObservationEvidence::StaticProgram {
                    submit_index: 1,
                    program_ordinal: 0,
                    pass_token: consumer.into(),
                    program_payload_sha256: "3".repeat(64),
                },
                corroboration_sha256: "4".repeat(64),
            }],
            PRODUCTION_MAP_CONTENT_CONSUMERS[1..]
                .iter()
                .map(|consumer| OmittedMapContentRole {
                    consumer: (*consumer).into(),
                    reason: "not observed".into(),
                })
                .collect(),
        )
        .unwrap();
        snapshot.observations[0].evidence = MapContentObservationEvidence::StaticProgram {
            submit_index: 1,
            program_ordinal: 0,
            pass_token: "mod.guessed.role".into(),
            program_payload_sha256: "3".repeat(64),
        };
        assert!(snapshot.validate().is_err());
    }

    #[test]
    fn generic_pdxmesh_static_index_cannot_fabricate_observation_provenance() {
        let authority = "1".repeat(64);
        let fixed_content = content("gfx/models/map/fixed.mesh", "3".repeat(64));
        let unit_content = content("gfx/models/units/unit.mesh", "4".repeat(64));
        let lookup_key = MapShaderLookupKey {
            registration_root_rva: PDXMESH_EFFECT_REGISTRATION_ROOT_RVA,
            logical_shader_path: "gfx/fx/pdxmesh.lua".into(),
            effect_name: "PdxMeshColor".into(),
        };
        let closure = |role, content: UsedContentKey| AuthenticatedMapRoleClosure {
            role,
            complete_dependency_closure: true,
            dependency_closure_sha256: map_role_closure_sha256(
                role,
                std::slice::from_ref(&lookup_key),
                std::slice::from_ref(&content),
            )
            .unwrap(),
            lookup_keys: vec![lookup_key.clone()],
            content: vec![content],
        };
        let resolver = EffectiveVfsMapContentResolver::new(
            authority,
            [fixed_content.clone(), unit_content.clone()],
            [
                closure(MapContentRole::FixedObjects, fixed_content),
                closure(MapContentRole::UnitModels, unit_content),
            ],
        )
        .unwrap();

        assert!(promote_resolved_map_content_index(&resolver).is_err());
    }

    #[test]
    fn complete_static_d80_index_still_requires_observed_program_provenance() {
        let authority = "1".repeat(64);
        let lookup = |role| match role {
            MapContentRole::Terrain => (MAP_EFFECT_REGISTRATION_ROOT_RVA, "gfx/fx/pdxmap.lua"),
            MapContentRole::Water => (MAP_EFFECT_REGISTRATION_ROOT_RVA, "gfx/fx/pdxwater.lua"),
            MapContentRole::Rivers => (MAP_EFFECT_REGISTRATION_ROOT_RVA, "gfx/fx/river.lua"),
            MapContentRole::Borders => (MAP_EFFECT_REGISTRATION_ROOT_RVA, "gfx/fx/border.lua"),
            MapContentRole::Trees => (MAP_EFFECT_REGISTRATION_ROOT_RVA, "gfx/fx/tree.lua"),
            MapContentRole::FixedObjects | MapContentRole::UnitModels => {
                (PDXMESH_EFFECT_REGISTRATION_ROOT_RVA, "gfx/fx/pdxmesh.lua")
            }
            MapContentRole::Rails => (
                RAILROAD_EFFECT_REGISTRATION_ROOT_RVA,
                "gfx/models/supply/railroad.shader",
            ),
            MapContentRole::MapNames => {
                (MAP_NAME_EFFECT_REGISTRATION_ROOT_RVA, "gfx/fx/mapname.lua")
            }
            MapContentRole::Lighting => (MAP_EFFECT_REGISTRATION_ROOT_RVA, "gfx/fx/sky.lua"),
            MapContentRole::PostProcess => (
                RESTORE_BLOOM_EFFECT_REGISTRATION_ROOT_RVA,
                "gfx/fx/restorescene.lua",
            ),
        };
        let mut winners = Vec::new();
        let mut closures = Vec::new();
        for (index, role) in MapContentRole::ALL.into_iter().enumerate() {
            let content = content(
                &format!("gfx/test/map_role_{index}.asset"),
                format!("{:064x}", index + 2),
            );
            let (registration_root_rva, logical_shader_path) = lookup(role);
            let lookup_key = MapShaderLookupKey {
                registration_root_rva,
                logical_shader_path: logical_shader_path.into(),
                effect_name: format!("effect_{index:02}"),
            };
            closures.push(AuthenticatedMapRoleClosure {
                role,
                complete_dependency_closure: true,
                dependency_closure_sha256: map_role_closure_sha256(
                    role,
                    std::slice::from_ref(&lookup_key),
                    std::slice::from_ref(&content),
                )
                .unwrap(),
                lookup_keys: vec![lookup_key],
                content: vec![content.clone()],
            });
            winners.push(content);
        }
        let resolver = EffectiveVfsMapContentResolver::new(authority, winners, closures).unwrap();
        assert!(promote_resolved_map_content_index(&resolver).is_err());
    }

    struct StaticOrderResolver;

    impl ExactStaticRenderOperationResolver for StaticOrderResolver {
        fn authority_manifest_sha256(&self) -> &str {
            "1111111111111111111111111111111111111111111111111111111111111111"
        }

        fn resolve_operation_ordinal(&self, event: &RawRenderOrderEvent) -> Option<u32> {
            match event {
                RawRenderOrderEvent::ClearColor { .. } => Some(0),
                RawRenderOrderEvent::ClearDepthStencil { .. } => Some(1),
                _ => None,
            }
        }
    }

    #[test]
    fn unified_order_covers_static_clears_runtime_and_present_frame_exactly_once() {
        let target = RenderTargetDesc {
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
        };
        let static_programs = vec![StaticMapPassProgram {
            program_ordinal: 0,
            pass_token: "map_and_post".into(),
            ordered_content_keys: Vec::new(),
            ordered_operations: vec![
                StaticMapOperation {
                    operation_ordinal: 0,
                    payload_sha256: "2".repeat(64),
                    command: StaticMapCommand::ClearColor {
                        target: target.clone(),
                        rgba_f32_bits: [0, 0, 0, 1.0f32.to_bits()],
                    },
                },
                StaticMapOperation {
                    operation_ordinal: 1,
                    payload_sha256: "3".repeat(64),
                    command: StaticMapCommand::ClearDepthStencil {
                        target,
                        clear_depth: true,
                        clear_stencil: false,
                        depth_f32_bits: 1.0f32.to_bits(),
                        stencil_u8: 0,
                    },
                },
            ],
            payload_sha256: "4".repeat(64),
        }];
        let boundary = |sequence, kind, producer_rva| RawRenderOrderEvent::Boundary {
            sequence,
            kind,
            producer_rva,
        };
        let events = vec![
            boundary(
                1,
                RenderOrderBoundary::MapSceneBegin,
                MAP_SCENE_RENDER_ROOT_RVA,
            ),
            RawRenderOrderEvent::ClearColor {
                sequence: 2,
                capture_sequence: 20,
                producer_rva: RENDER_TARGET_CLEAR_RVA,
                render_target_object_ptr: 1,
                render_target_view_ptr: 2,
                rgba_f32_bits: [0, 0, 0, 1.0f32.to_bits()],
            },
            RawRenderOrderEvent::FinalDraw {
                sequence: 3,
                call: FinalDrawCall::Draw,
                family: FinalDrawFamily::Particle,
                producer_rva: PARTICLE_FINAL_DRAW_RVA,
                render_context_ptr: 3,
                shader_program_ptr: 4,
                arguments: [6, 0, 0, 0],
            },
            boundary(
                4,
                RenderOrderBoundary::MapSceneEnd,
                MAP_SCENE_RENDER_ROOT_RVA,
            ),
            boundary(
                5,
                RenderOrderBoundary::PostProcessBegin,
                POST_PROCESS_RENDER_ROOT_RVA,
            ),
            RawRenderOrderEvent::ClearDepthStencil {
                sequence: 6,
                capture_sequence: 60,
                producer_rva: RENDER_TARGET_CLEAR_RVA,
                render_target_object_ptr: 1,
                depth_stencil_view_ptr: 5,
                clear_depth: true,
                clear_stencil: false,
                depth_f32_bits: 1.0f32.to_bits(),
                stencil_u8: 0,
            },
            boundary(
                7,
                RenderOrderBoundary::PostProcessEnd,
                POST_PROCESS_RENDER_ROOT_RVA,
            ),
            boundary(
                8,
                RenderOrderBoundary::PresentBegin,
                PRESENT_RENDER_ROOT_RVA,
            ),
            boundary(9, RenderOrderBoundary::PresentEnd, PRESENT_RENDER_ROOT_RVA),
        ];
        let runtime = vec![RenderOutputRef::Particle {
            effect_instance_id: 7,
            bucket: 2,
            packet_ordinal: 0,
            draw_order: 0,
        }];
        let order = promote_unified_render_execution_order(
            &events,
            &runtime,
            &static_programs,
            &StaticOrderResolver,
        )
        .unwrap();
        assert_eq!(order.len(), 3);
        assert!(matches!(
            order[0],
            RenderExecutionRef::Static {
                operation_ordinal: 0
            }
        ));
        assert!(matches!(order[1], RenderExecutionRef::Runtime { .. }));
        assert!(matches!(
            order[2],
            RenderExecutionRef::Static {
                operation_ordinal: 1
            }
        ));
        assert!(promote_unified_render_execution_order(
            &events,
            &[],
            &static_programs,
            &StaticOrderResolver,
        )
        .is_err());
        assert!(promote_unified_render_execution_order(
            &events[..5],
            &runtime,
            &static_programs,
            &StaticOrderResolver,
        )
        .is_err());
    }

    #[test]
    fn render_order_observer_requires_complete_map_postprocess_present_frames() {
        let boundary = |sequence, kind, producer_rva| RawRenderOrderEvent::Boundary {
            sequence,
            kind,
            producer_rva,
        };
        let draw = RawRenderOrderEvent::FinalDraw {
            sequence: 2,
            call: FinalDrawCall::DrawIndexedInstanced,
            family: FinalDrawFamily::StaticOrPostProcess,
            producer_rva: INDEXED_INSTANCED_FINAL_DRAW_RVA,
            render_context_ptr: 1,
            shader_program_ptr: 2,
            arguments: [6, 4, 0, 0],
        };
        let events = vec![
            boundary(
                1,
                RenderOrderBoundary::MapSceneBegin,
                MAP_SCENE_RENDER_ROOT_RVA,
            ),
            draw,
            boundary(
                3,
                RenderOrderBoundary::MapSceneEnd,
                MAP_SCENE_RENDER_ROOT_RVA,
            ),
            boundary(
                4,
                RenderOrderBoundary::PostProcessBegin,
                POST_PROCESS_RENDER_ROOT_RVA,
            ),
            boundary(
                5,
                RenderOrderBoundary::PostProcessEnd,
                POST_PROCESS_RENDER_ROOT_RVA,
            ),
            boundary(
                6,
                RenderOrderBoundary::PresentBegin,
                PRESENT_RENDER_ROOT_RVA,
            ),
            boundary(7, RenderOrderBoundary::PresentEnd, PRESENT_RENDER_ROOT_RVA),
        ];
        validate_render_order_events(&events).unwrap();
        assert!(validate_render_order_events(&events[..6]).is_err());
        let mut gapped = events;
        if let RawRenderOrderEvent::Boundary { sequence, .. } = &mut gapped[3] {
            *sequence = 5;
        }
        assert!(validate_render_order_events(&gapped).is_err());
    }

    #[test]
    fn weather_selection_word_matches_frozen_integer_law() {
        assert_eq!(weather_selection_word([1, 0x5ea6_ba9e]), 0x1885_198a);
        assert_eq!(weather_selection_word([2, 0x5ea6_ba9e]), 0x0ff6_e14e);
    }

    #[test]
    fn typed_entity_direct_and_tiled_light_promotions_fail_closed() {
        let entity = promote_entity_submission(
            &RawEntitySubmission {
                submit_index: 3,
                pass_ordinal: 1,
                draw_order: 7,
                entity_instance_id: 9,
                entity_registration_token: Some("test_entity".into()),
                entity_object_ptr: 0x0800,
                entity_definition_ptr: 0x1000,
                renderer_ptr: 0x1800,
                render_context_ptr: 0x1900,
                mesh_draw_ptr: 0x2000,
                material_binding_ptr: 0x3000,
                entity_group_index: 0,
                mesh_draw_list_index: 0,
                entity_variant_u8: 0,
                mesh_draw_variant_u8: 0,
                pass_selector_i32: 0,
                bind_renderer_target_u8: 0,
                alternate_list_u8: 0,
                shader_program_ptr: 0x4000,
                additional_shader_stages_authenticated_null: true,
                stream_output_targets_authenticated_null: true,
                world_transform_4x4_f32_bits: [0; 16],
                state_flags: 2,
                skinning: RawSkinning::Palette3200(vec![4; 3_200]),
            },
            &EntityResolver,
        )
        .unwrap();
        assert!(matches!(entity.skinning, Skinning::Palette { .. }));
        assert_eq!(
            entity.content_identity.mesh.normalized_game_relative_path,
            "gfx/models/test.mesh"
        );

        let direct = promote_direct_visual(
            &RawDirectVisualObservation {
                sequence: 4,
                update_index: 2,
                producer_kind: DirectVisualKind::StrategicAirScene,
                producer_instance_id: 1,
                entity_instance_id: 9,
                effect_instance_id: None,
                operation: DirectVisualOperation::StateChange,
                entity_definition_ptr: 0x1000,
                semantic_state: "attack".into(),
                authored_state_index: 3,
                authored_event_index: 0,
                state_enter_sequence: 4,
                live_phase_f32_bits: 0,
                world_transform_4x4_f32_bits: [0; 16],
                node_or_locator_token: None,
                visibility_flags: 1,
                direct_object_list_ordinal: 0,
                direct_object_count: 1,
                rng_counter: 5,
                rng_seed: 6,
                effect_expiry_update: None,
            },
            &DirectResolver,
        )
        .unwrap();
        assert_eq!(direct.resolved_entity_source_sha256, "9f".repeat(32));

        let mut index_map = vec![255; 16_384];
        index_map[0] = 3;
        let raw_light = RawTiledLightObservation {
            update_index: 2,
            writer_index: 1,
            first_sequence: 4,
            last_sequence: 5,
            grid_start_inv_cell_size_float4_bits: [0; 4],
            light_index_map_64x64_bgra8: index_map,
            light_data_map_128x1_rgba32f: vec![0; 2_048],
            ordered_slot_joins: vec![RawLightSlotJoin {
                light_slot: 3,
                light_instance_id: 12,
                definition_ptr: 0x5000,
            }],
        };
        let light = promote_tiled_light_observation(&raw_light, &LightResolver).unwrap();
        assert_eq!(light.ordered_slot_semantic_joins[0].light_slot, 3);

        let mut invalid = raw_light;
        invalid.light_index_map_64x64_bgra8[1] = 4;
        assert!(promote_tiled_light_observation(&invalid, &LightResolver).is_err());
    }

    #[test]
    fn static_capture_authority_materializes_only_exact_receipts() {
        let target_receipt = RawRenderTargetRegistrationObservation {
            registration_sequence: 6,
            capture_sequence: 7,
            producer_rva: GENERIC_TEXTURE_RTV_PRODUCER_RVA,
            recreate_generation: 0,
            attachment_ordinal: 0,
            kind: RawRegisteredRenderTargetKind::OpaqueRenderTarget {
                source_flags_u32: 2,
            },
            view_ptr: 0x9000,
            resource_ptr: 0xa000,
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
        .materialize()
        .unwrap();
        let target = target_receipt.target.clone();
        let content = RawVfsContentIdentity {
            logical_path: "gfx/fx/modded.shader".into(),
            winning_layer_ordinal: 2,
            layer_kind: "mod".into(),
            layer_id: "workshop-test".into(),
            source_sha256: "a".repeat(64),
            payload_sha256: "b".repeat(64),
            payload_bytes: 16,
        };
        let receipt = ObservedStaticDrawAuthority {
            capture_sequence: 7,
            shader_program_ptr: 0x1000,
            program_identity: RawProgramIdentity {
                observation_sequence: 7,
                shader_program_ptr: 0x1000,
                source: content.clone(),
                effect_token: "engine-observed-effect".into(),
                vertex_entry_token: "VS".into(),
                pixel_entry_token: "PS".into(),
                ordered_defines: Vec::new(),
            },
            ordered_texture_semantics: vec![RawStaticShaderResourceSemantic {
                stage: RawShaderStage::Pixel,
                slot: 0,
                semantic_token: "reflected_texture".into(),
                source: RawStaticShaderResourceSource::UsedContent,
            }],
            ordered_sampler_semantics: vec![RawSamplerSemantic {
                stage: RawShaderStage::Pixel,
                slot: 0,
                semantic_token: "reflected_sampler".into(),
            }],
            ordered_constant_buffer_semantics: Vec::new(),
            ordered_color_targets: vec![target],
            depth_target: None,
            ordered_content: vec![RawStaticContentJoin {
                content_key: "opaque-mod-shader".into(),
                content,
            }],
        };
        let core = RawStaticD3dDrawState {
            capture_sequence: 7,
            shader_program_ptr: 0x1000,
            render_context_ptr: 0x2000,
            device_context_ptr: 0x3000,
            vertex_shader_ptr: 0x4000,
            pixel_shader_ptr: 0x5000,
            geometry_shader_ptr: None,
            hull_shader_ptr: None,
            domain_shader_ptr: None,
            compute_shader_ptr: None,
            geometry_entry_token: None,
            hull_entry_token: None,
            domain_entry_token: None,
            compute_entry_token: None,
            dynamic_shader_linkage_present: false,
            input_layout_ptr: None,
            blend_state_ptr: None,
            blend_factor_rgba_f32_bits: [0; 4],
            sample_mask_u32: u32::MAX,
            depth_stencil_state_ptr: None,
            stencil_reference_u32: 0,
            rasterizer_state_ptr: None,
            primitive_topology_u32: 4,
            ordered_texture_objects: vec![RawTextureObject {
                stage: RawShaderStage::Pixel,
                slot: 0,
                shader_resource_view_ptr: 0x6000,
                resource_ptr: 0x7000,
            }],
            ordered_sampler_objects: vec![RawStageSlotObject {
                stage: RawShaderStage::Pixel,
                slot: 0,
                object_ptr: 0x8000,
            }],
            ordered_constant_buffer_objects: Vec::new(),
            ordered_vertex_buffer_objects: Vec::new(),
            index_buffer: None,
            ordered_stream_output_buffers: Vec::new(),
            ordered_color_target_view_ptrs: vec![0x9000],
            depth_stencil_view_ptr: None,
            ordered_viewports_f32_bits: vec![[0; 6]],
            ordered_scissor_rects_i32: Vec::new(),
            draw_call: RawStaticDrawCall::Draw {
                vertex_count: 3,
                start_vertex: 0,
            },
            ordered_texture_semantics: Vec::new(),
            ordered_sampler_semantics: Vec::new(),
            ordered_constant_buffer_semantics: Vec::new(),
            ordered_color_targets: Vec::new(),
            depth_target: None,
            ordered_content: Vec::new(),
        };
        let process = RawProcessEpoch {
            process_id: 4,
            process_start_nonce: 5,
            executable_sha256: "c".repeat(64),
        };
        let launch_epoch = RawCaptureLaunchEpoch {
            process_id: 4,
            process_start_nonce: 5,
            observer_installed_qpc: 6,
            d3d_device_created_qpc: 7,
            first_creation_sequence: 1,
            hook_epoch_nonce: 8,
            hook_set_receipt_sha256: "d".repeat(64),
            capture_receipt_sha256: "e".repeat(64),
        };
        let mut catalog = RawRenderCatalogCapture {
            process,
            launch_epoch,
            submit_index: 9,
            frame_sequence: 10,
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
            render_target_view_snapshots: vec![RawRenderTargetViewSnapshot {
                snapshot_sequence: 7,
                view_ptr: 0x9000,
                resource_ptr: 0xa000,
                view_kind: RawOutputViewKind::Color,
                desc: RawResourceViewDesc {
                    dxgi_format_u32: 28,
                    view_dimension_u32: 4,
                    flags_u32: 0,
                    union_words_u32: [0; 4],
                },
                resource_dimension_u32: 3,
                resource_width: 64,
                resource_height: 32,
                resource_depth: 1,
                resource_mip_levels: 1,
                resource_array_size: 1,
                resource_sample_count: 1,
                resource_sample_quality: 0,
            }],
        };
        assert!(
            ProductionStaticCaptureAuthority::derive_opaque_draw_output_targets(&core, &catalog)
                .is_err()
        );
        let mut stale_core = core.clone();
        stale_core.capture_sequence = 8;
        assert!(
            ProductionStaticCaptureAuthority::derive_opaque_draw_output_targets(
                &stale_core,
                &catalog
            )
            .is_err()
        );
        let mut role_sink = ProductionStaticProgramCaptureSink::new(
            catalog.process.clone(),
            catalog.launch_epoch.clone(),
            catalog.submit_index,
            catalog.frame_sequence,
        )
        .unwrap();
        role_sink
            .authority_mut()
            .observe_draw(receipt.clone())
            .unwrap();
        role_sink.push_static_draw(core.clone()).unwrap();
        role_sink
            .push_order_operation(RawStaticOrderOperation::Draw {
                sequence: 1,
                capture_sequence: core.capture_sequence,
            })
            .unwrap();
        assert!(role_sink
            .finish(catalog.clone())
            .unwrap_err()
            .contains("requires an exact engine registration receipt"));
        let mut authority = ProductionStaticCaptureAuthority::default();
        authority.observe_draw(receipt).unwrap();
        let materialized = authority.materialize_draw(core).unwrap();
        assert_eq!(
            materialized.ordered_content[0].content_key,
            "opaque-mod-shader"
        );
        authority.validate_consumed().unwrap();
        authority.finalize_catalog(&mut catalog).unwrap();
        assert_eq!(catalog.program_identities.len(), 1);
    }
}
