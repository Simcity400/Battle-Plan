//! Exact production constructors for d77/d80 authority observations.
//!
//! The constructors consume only authenticated effective-VFS payloads and
//! opaque hook receipts. They never derive a semantic role from a path, slot,
//! object address, or captured vanilla catalog.

use crate::{
    observe_live_draw_authority, observe_static_authority, RawD3dDrawState, RawDrawContentIdentity,
    RawDrawIdentity, RawEffectiveVfsOpenObservation, RawEffectiveVfsWinnerIndexBinding,
    RawLiveDrawAuthorityObservation, RawProgramRegistrationObservation,
    RawRenderTargetRegistrationObservation, RawRenderTargetRegistrationReceipt,
    RawShaderBytecodeReflectionObservation, RawShaderCreation, RawShaderDefine,
    RawShaderResourceViewSnapshot, RawShaderStage, RawStaticAuthorityObservation,
    RawStaticContentJoin, RawStaticD3dDrawState, RawStaticShaderResourceSource,
    RawStaticTextureSourceObservation, RawTextureObject, RawTextureResourceSnapshot,
    RawVfsContentIdentity,
};
use portable_content_manifest::{normalize_game_relative_path, ContentLayerKind, UsedContentKey};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Cursor;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExactVfsOpenEvidence {
    pub observation_sequence: u64,
    pub ordered_logical_path_tokens: Vec<String>,
    pub stream_handle: usize,
    pub data_ptr: Option<usize>,
    pub opened_payload_sha256: String,
    pub opened_payload_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExactProgramAuthorityEvidence {
    pub path_join_sequence: u64,
    pub shader_program_ptr: usize,
    pub registration_root_rva: u32,
    pub effect_token: String,
    pub ordered_defines: Vec<RawShaderDefine>,
    pub source_open: ExactVfsOpenEvidence,
    pub vertex_creation: RawShaderCreation,
    pub pixel_creation: RawShaderCreation,
    pub vertex_reflection: RawShaderBytecodeReflectionObservation,
    pub pixel_reflection: RawShaderBytecodeReflectionObservation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProgramAuthorityDomain {
    MapOrOpaqueStatic,
    AuthenticatedGui,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OpaqueRenderTargetGraphEvidence {
    pub registrations: Vec<RawRenderTargetRegistrationObservation>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExactStaticTextureSourceEvidence {
    pub capture_sequence: u64,
    pub stage: RawShaderStage,
    pub slot: u8,
    pub shader_resource_view_ptr: usize,
    pub resource_ptr: usize,
    pub source: RawStaticShaderResourceSource,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExactStaticDrawEvidence {
    pub draw: RawStaticD3dDrawState,
    pub texture_resource_snapshots: Vec<RawTextureResourceSnapshot>,
    pub shader_resource_view_snapshots: Vec<RawShaderResourceViewSnapshot>,
    pub explicit_texture_sources: Vec<ExactStaticTextureSourceEvidence>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExactAuthorityPayloadRequest {
    pub content: RawVfsContentIdentity,
    pub byte_offset: u64,
    pub byte_count: u64,
    pub range_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExactLiveDrawContentEvidence {
    Particle {
        effect_instance_id: u64,
        registration_token: String,
        definition_source_token: String,
        subsystem_selector_u32: u32,
        definition_content: RawVfsContentIdentity,
        texture_content: RawVfsContentIdentity,
        authored_shader_token: String,
        engine_effect_token: String,
        explicit_engine_effect_token_crosscheck: Option<String>,
        atlas_columns_u32: u32,
        atlas_rows_u32: u32,
        atlas_half_pixel_width_f32_bits: u32,
        atlas_half_pixel_height_f32_bits: u32,
    },
    Entity {
        entity_instance_id: u64,
        entity_definition_ptr: usize,
        mesh_draw_ptr: usize,
        material_binding_ptr: usize,
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

/// In-process view of exact decompressed VFS ranges admitted over H4CT.
/// Implementations must be path-free and process-local; the winner index is
/// supplied separately so a payload cannot confer authority by itself.
pub trait AdmittedAuthorityPayloads {
    fn payload_range(
        &self,
        content: &RawVfsContentIdentity,
        byte_offset: u64,
        byte_count: u64,
        range_sha256: &str,
    ) -> Option<&[u8]>;

    fn payload(&self, content: &RawVfsContentIdentity) -> Option<&[u8]> {
        self.payload_range(content, 0, content.payload_bytes, &content.payload_sha256)
    }
}

/// Hydrates a raw archive-open receipt from the authenticated winner index.
/// The engine path token must name exactly one winner; payload digest/length
/// are copied from that winner and still require H4CT byte verification when
/// `materialize_exact_vfs_open` runs.
pub fn hydrate_exact_vfs_open_evidence(
    winner_index: &RawEffectiveVfsWinnerIndexBinding,
    observation_sequence: u64,
    ordered_logical_path_tokens: Vec<String>,
    stream_handle: usize,
    data_ptr: Option<usize>,
) -> Result<(ExactVfsOpenEvidence, ExactAuthorityPayloadRequest), String> {
    if observation_sequence == 0
        || stream_handle == 0
        || data_ptr == Some(0)
        || ordered_logical_path_tokens.is_empty()
    {
        return Err("raw effective-VFS open receipt is incomplete".into());
    }
    let content = unique_winner_for_path_tokens(winner_index, &ordered_logical_path_tokens)?;
    Ok((
        ExactVfsOpenEvidence {
            observation_sequence,
            ordered_logical_path_tokens,
            stream_handle,
            data_ptr,
            opened_payload_sha256: content.payload_sha256.clone(),
            opened_payload_bytes: content.payload_bytes,
        },
        ExactAuthorityPayloadRequest {
            content: content.clone(),
            byte_offset: 0,
            byte_count: content.payload_bytes,
            range_sha256: content.payload_sha256.clone(),
        },
    ))
}

pub struct ProductionAuthorityObservationProducer<'a, V: AdmittedAuthorityPayloads> {
    winner_index: &'a RawEffectiveVfsWinnerIndexBinding,
    admitted_payloads: &'a V,
}

impl<'a, V: AdmittedAuthorityPayloads> ProductionAuthorityObservationProducer<'a, V> {
    pub fn new(
        winner_index: &'a RawEffectiveVfsWinnerIndexBinding,
        admitted_payloads: &'a V,
    ) -> Result<Self, String> {
        winner_index.validate()?;
        Ok(Self {
            winner_index,
            admitted_payloads,
        })
    }

    /// Enumerates the exact full winner payloads that must be fetched over the
    /// still-authenticated H4CT session before the live constructor runs.
    pub fn required_live_authority_payloads(
        &self,
        draw: &RawD3dDrawState,
        content: &ExactLiveDrawContentEvidence,
        program: &ExactProgramAuthorityEvidence,
        targets: &OpaqueRenderTargetGraphEvidence,
    ) -> Result<Vec<ExactAuthorityPayloadRequest>, String> {
        let mut required = BTreeMap::new();
        add_full_payload(
            self.winner_index,
            &mut required,
            source_content_for_open(self.winner_index, &program.source_open)?,
        )?;
        match content {
            ExactLiveDrawContentEvidence::Particle {
                definition_content,
                texture_content,
                ..
            } => {
                add_full_payload(self.winner_index, &mut required, definition_content)?;
                add_full_payload(self.winner_index, &mut required, texture_content)?;
            }
            ExactLiveDrawContentEvidence::Entity {
                resolved_entity_asset,
                mesh,
                ..
            } => {
                add_full_payload(self.winner_index, &mut required, resolved_entity_asset)?;
                add_full_payload(self.winner_index, &mut required, mesh)?;
            }
        }
        let (colors, depth) = targets.materialize_for_draw(
            draw.final_draw_sequence,
            &draw.ordered_color_target_view_ptrs,
            draw.depth_stencil_view_ptr,
        )?;
        for receipt in colors.iter().chain(depth.iter()) {
            if let Some(content) = &receipt.target.persistent_content {
                add_full_payload(self.winner_index, &mut required, content)?;
            }
        }
        Ok(required.into_values().collect())
    }

    /// Enumerates the exact full winner payloads required by one d80 draw.
    pub fn required_static_authority_payloads(
        &self,
        evidence: &ExactStaticDrawEvidence,
        program: &ExactProgramAuthorityEvidence,
        targets: &OpaqueRenderTargetGraphEvidence,
    ) -> Result<Vec<ExactAuthorityPayloadRequest>, String> {
        let mut required = BTreeMap::new();
        add_full_payload(
            self.winner_index,
            &mut required,
            source_content_for_open(self.winner_index, &program.source_open)?,
        )?;
        for resource in &evidence.texture_resource_snapshots {
            if let Some(content) = &resource.content {
                add_full_payload(self.winner_index, &mut required, content)?;
            }
        }
        let draw = &evidence.draw;
        let (colors, depth) = targets.materialize_for_draw(
            draw.capture_sequence,
            &draw.ordered_color_target_view_ptrs,
            draw.depth_stencil_view_ptr,
        )?;
        for receipt in colors.iter().chain(depth.iter()) {
            if let Some(content) = &receipt.target.persistent_content {
                add_full_payload(self.winner_index, &mut required, content)?;
            }
        }
        for source in &evidence.explicit_texture_sources {
            if let RawStaticShaderResourceSource::PreparedTarget { target, .. } = &source.source {
                if let Some(content) = &target.persistent_content {
                    add_full_payload(self.winner_index, &mut required, content)?;
                }
            }
        }
        Ok(required.into_values().collect())
    }

    /// Enumerates the normal exact-pipeline payloads plus the selected GUI
    /// definition that grants the neutral engine-program lookup its GUI
    /// authority. The definition is not inserted into the D3D draw's resource
    /// list: it is an owner/registration join, while the reflected bindings
    /// remain the complete set of resources consumed by the draw.
    pub fn required_gui_static_authority_payloads(
        &self,
        evidence: &ExactStaticDrawEvidence,
        program: &ExactProgramAuthorityEvidence,
        targets: &OpaqueRenderTargetGraphEvidence,
        selected_gui_definition: &UsedContentKey,
    ) -> Result<Vec<ExactAuthorityPayloadRequest>, String> {
        let mut required = self
            .required_static_authority_payloads(evidence, program, targets)?
            .into_iter()
            .map(|request| {
                (
                    (
                        request.content.logical_path.clone(),
                        request.content.winning_layer_ordinal,
                        request.content.payload_sha256.clone(),
                    ),
                    request,
                )
            })
            .collect::<BTreeMap<_, _>>();
        let gui = self.authenticated_gui_definition_content(selected_gui_definition)?;
        add_full_payload(self.winner_index, &mut required, gui)?;
        Ok(required.into_values().collect())
    }

    pub fn build_live_draw_authority(
        &self,
        submit: &crate::RawParticleSubmit,
        draw: &RawD3dDrawState,
        content: &ExactLiveDrawContentEvidence,
        program: ExactProgramAuthorityEvidence,
        targets: &OpaqueRenderTargetGraphEvidence,
    ) -> Result<RawLiveDrawAuthorityObservation, String> {
        if draw.final_draw_sequence == 0
            || draw.submit_index != submit.submit_index
            || draw.shader_program_ptr != program.shader_program_ptr
            || draw.vertex_shader_ptr != program.vertex_creation.shader_ptr
            || draw.pixel_shader_ptr != program.pixel_creation.shader_ptr
        {
            return Err("live authority producer received a mismatched hook draw".into());
        }
        if let ExactLiveDrawContentEvidence::Particle {
            engine_effect_token,
            ..
        } = content
        {
            if engine_effect_token != &program.effect_token {
                return Err(
                    "particle authored shader token disagrees with the live program".into(),
                );
            }
        }
        let content_identity = self.resolve_live_content_identity(submit, draw, content)?;
        let (source_open, registration) = self.build_program(
            draw.final_draw_sequence,
            &program,
            ProgramAuthorityDomain::MapOrOpaqueStatic,
        )?;
        let (ordered_color_targets, depth_target) = targets.materialize_for_draw(
            draw.final_draw_sequence,
            &draw.ordered_color_target_view_ptrs,
            draw.depth_stencil_view_ptr,
        )?;
        self.authorize_target_content(&ordered_color_targets, depth_target.as_ref())?;
        let observed = RawLiveDrawAuthorityObservation {
            observation_sequence: draw.final_draw_sequence,
            identity: draw.identity.clone(),
            content_identity,
            program_registration: registration,
            source_open,
            vertex_creation: program.vertex_creation,
            pixel_creation: program.pixel_creation,
            vertex_reflection: program.vertex_reflection,
            pixel_reflection: program.pixel_reflection,
            ordered_color_targets,
            depth_target,
        };
        crate::materialize_d77_draw_authority(&observed)?;
        Ok(observed)
    }

    pub fn build_and_observe_live_draw_authority(
        &self,
        submit_index: u64,
        submit: &crate::RawParticleSubmit,
        draw: &RawD3dDrawState,
        content: &ExactLiveDrawContentEvidence,
        program: ExactProgramAuthorityEvidence,
        targets: &OpaqueRenderTargetGraphEvidence,
    ) -> Result<(), String> {
        let observed = self.build_live_draw_authority(submit, draw, content, program, targets)?;
        observe_live_draw_authority(submit_index, observed)
    }

    pub fn build_static_draw_authority(
        &self,
        evidence: &ExactStaticDrawEvidence,
        program: ExactProgramAuthorityEvidence,
        targets: &OpaqueRenderTargetGraphEvidence,
    ) -> Result<RawStaticAuthorityObservation, String> {
        self.build_static_draw_authority_for_domain(
            evidence,
            program,
            targets,
            ProgramAuthorityDomain::MapOrOpaqueStatic,
        )
    }

    /// Builds one generic-GUI exact-pipeline authority row. Success requires
    /// all ordinary D3D/reflection/VFS joins and an exact selected-owner
    /// `interface/*.gui` winner from the same authenticated VFS manifest.
    /// Unknown, non-interface, stale, or non-admitted GUI definitions fail
    /// closed; a map registration root is never relabelled as GUI authority.
    pub fn build_gui_static_draw_authority(
        &self,
        evidence: &ExactStaticDrawEvidence,
        program: ExactProgramAuthorityEvidence,
        targets: &OpaqueRenderTargetGraphEvidence,
        selected_gui_definition: &UsedContentKey,
    ) -> Result<RawStaticAuthorityObservation, String> {
        let gui = self.authenticated_gui_definition_content(selected_gui_definition)?;
        self.resolve_content(gui)?;
        self.build_static_draw_authority_for_domain(
            evidence,
            program,
            targets,
            ProgramAuthorityDomain::AuthenticatedGui,
        )
    }

    fn build_static_draw_authority_for_domain(
        &self,
        evidence: &ExactStaticDrawEvidence,
        program: ExactProgramAuthorityEvidence,
        targets: &OpaqueRenderTargetGraphEvidence,
        domain: ProgramAuthorityDomain,
    ) -> Result<RawStaticAuthorityObservation, String> {
        let draw = &evidence.draw;
        if draw.capture_sequence == 0
            || draw.shader_program_ptr != program.shader_program_ptr
            || draw.vertex_shader_ptr != program.vertex_creation.shader_ptr
            || draw.pixel_shader_ptr != program.pixel_creation.shader_ptr
        {
            return Err("static authority producer received a mismatched hook draw".into());
        }
        let (source_open, registration) =
            self.build_program(draw.capture_sequence, &program, domain)?;
        let (ordered_color_targets, depth_target) = targets.materialize_for_draw(
            draw.capture_sequence,
            &draw.ordered_color_target_view_ptrs,
            draw.depth_stencil_view_ptr,
        )?;
        self.authorize_target_content(&ordered_color_targets, depth_target.as_ref())?;
        let vertex = program
            .vertex_reflection
            .materialize_stage_semantics(&program.vertex_creation)?;
        let pixel = program
            .pixel_reflection
            .materialize_stage_semantics(&program.pixel_creation)?;
        let reflected_textures = vertex
            .ordered_texture_semantics
            .iter()
            .chain(&pixel.ordered_texture_semantics)
            .cloned()
            .collect::<Vec<_>>();
        let mut ordered_texture_sources = Vec::with_capacity(reflected_textures.len());
        let mut content = vec![source_open.content.clone()];
        let mut used_views = BTreeSet::new();
        let mut used_resources = BTreeSet::new();
        let mut used_explicit = 0usize;
        for semantic in reflected_textures {
            let object = unique_texture_object(
                &draw.ordered_texture_objects,
                semantic.stage,
                semantic.slot,
            )?;
            let view = unique_by(
                &evidence.shader_resource_view_snapshots,
                |row| row.shader_resource_view_ptr == object.shader_resource_view_ptr,
                "shader-resource view",
            )?;
            if view.resource_ptr != object.resource_ptr {
                return Err("static texture object and view name different resources".into());
            }
            used_views.insert(view.shader_resource_view_ptr);
            used_resources.insert(object.resource_ptr);
            let resource = unique_by(
                &evidence.texture_resource_snapshots,
                |row| row.resource_ptr == object.resource_ptr,
                "texture resource",
            )?;
            let source = if let Some(file_content) = &resource.content {
                let bytes = self.resolve_content(file_content)?;
                validate_file_texture(resource, &file_content.logical_path, bytes)?;
                content.push(file_content.clone());
                RawStaticShaderResourceSource::UsedContent
            } else {
                let exact = unique_by(
                    &evidence.explicit_texture_sources,
                    |row| {
                        row.capture_sequence == draw.capture_sequence
                            && row.stage == semantic.stage
                            && row.slot == semantic.slot
                            && row.shader_resource_view_ptr == object.shader_resource_view_ptr
                            && row.resource_ptr == object.resource_ptr
                    },
                    "explicit static texture source",
                )?;
                validate_explicit_source(exact, targets, &mut content)?;
                used_explicit += 1;
                exact.source.clone()
            };
            ordered_texture_sources.push(RawStaticTextureSourceObservation {
                stage: semantic.stage,
                slot: semantic.slot,
                source,
            });
        }
        if used_views.len() != evidence.shader_resource_view_snapshots.len()
            || used_resources.len() != evidence.texture_resource_snapshots.len()
            || used_explicit != evidence.explicit_texture_sources.len()
        {
            return Err("static texture evidence contains unmatched or repeated rows".into());
        }
        for row in &content {
            self.authorize_content(row)?;
        }
        let ordered_content = opaque_content_joins(content)?;
        if !draw.ordered_content.is_empty() && draw.ordered_content != ordered_content {
            return Err("hook static content rows disagree with exact producer joins".into());
        }
        let observed = RawStaticAuthorityObservation {
            capture_sequence: draw.capture_sequence,
            shader_program_ptr: draw.shader_program_ptr,
            program_registration: registration,
            source_open,
            vertex_creation: program.vertex_creation,
            pixel_creation: program.pixel_creation,
            vertex_reflection: program.vertex_reflection,
            pixel_reflection: program.pixel_reflection,
            ordered_texture_sources,
            ordered_color_targets,
            depth_target,
            ordered_content,
        };
        let materialized = crate::materialize_d80_static_authority(&observed)?;
        if !draw.ordered_texture_semantics.is_empty()
            && draw.ordered_texture_semantics != materialized.ordered_texture_semantics
            || !draw.ordered_sampler_semantics.is_empty()
                && draw.ordered_sampler_semantics != materialized.ordered_sampler_semantics
            || !draw.ordered_constant_buffer_semantics.is_empty()
                && draw.ordered_constant_buffer_semantics
                    != materialized.ordered_constant_buffer_semantics
            || !draw.ordered_color_targets.is_empty()
                && draw.ordered_color_targets != materialized.ordered_color_targets
            || draw.depth_target.is_some() && draw.depth_target != materialized.depth_target
        {
            return Err(
                "hook static semantics disagree with exact producer materialization".into(),
            );
        }
        Ok(observed)
    }

    pub fn build_and_observe_static_draw_authority(
        &self,
        frame_sequence: u64,
        evidence: &ExactStaticDrawEvidence,
        program: ExactProgramAuthorityEvidence,
        targets: &OpaqueRenderTargetGraphEvidence,
    ) -> Result<(), String> {
        let observed = self.build_static_draw_authority(evidence, program, targets)?;
        observe_static_authority(frame_sequence, observed)
    }

    pub fn build_and_observe_gui_static_draw_authority(
        &self,
        frame_sequence: u64,
        evidence: &ExactStaticDrawEvidence,
        program: ExactProgramAuthorityEvidence,
        targets: &OpaqueRenderTargetGraphEvidence,
        selected_gui_definition: &UsedContentKey,
    ) -> Result<(), String> {
        let observed = self.build_gui_static_draw_authority(
            evidence,
            program,
            targets,
            selected_gui_definition,
        )?;
        observe_static_authority(frame_sequence, observed)
    }

    fn build_program(
        &self,
        capture_sequence: u64,
        evidence: &ExactProgramAuthorityEvidence,
        domain: ProgramAuthorityDomain,
    ) -> Result<
        (
            RawEffectiveVfsOpenObservation,
            RawProgramRegistrationObservation,
        ),
        String,
    > {
        if capture_sequence == 0
            || evidence.path_join_sequence == 0
            || evidence.path_join_sequence > capture_sequence
            || evidence.shader_program_ptr == 0
            || evidence.effect_token.trim().is_empty()
            || evidence.vertex_creation.stage != RawShaderStage::Vertex
            || evidence.pixel_creation.stage != RawShaderStage::Pixel
            || evidence.vertex_reflection.shader_program_ptr != evidence.shader_program_ptr
            || evidence.pixel_reflection.shader_program_ptr != evidence.shader_program_ptr
        {
            return Err("program path/reflection join is incomplete or stale".into());
        }
        let mut open_evidence = evidence.source_open.clone();
        for creation in [&evidence.vertex_creation, &evidence.pixel_creation] {
            if let Some(invocation) = &creation.compiler_invocation {
                if invocation.entry_token.trim().is_empty()
                    || invocation.ordered_defines != evidence.ordered_defines
                {
                    return Err("compiler invocation disagrees with the exact program join".into());
                }
                open_evidence
                    .ordered_logical_path_tokens
                    .push(invocation.source_name.clone());
            }
        }
        let source_open =
            materialize_exact_vfs_open(self.winner_index, self.admitted_payloads, &open_evidence)?;
        for creation in [&evidence.vertex_creation, &evidence.pixel_creation] {
            if let Some(invocation) = &creation.compiler_invocation {
                if invocation.source_payload_sha256 != source_open.opened_payload_sha256
                    || invocation.source_payload_bytes != source_open.opened_payload_bytes
                {
                    return Err(
                        "compiler source payload disagrees with the admitted VFS open".into(),
                    );
                }
            }
        }
        evidence
            .vertex_reflection
            .validate_against_creation(&evidence.vertex_creation)?;
        evidence
            .pixel_reflection
            .validate_against_creation(&evidence.pixel_creation)?;
        let authenticated_root = match domain {
            ProgramAuthorityDomain::MapOrOpaqueStatic => {
                crate::is_authenticated_map_registration_root(evidence.registration_root_rva)
            }
            ProgramAuthorityDomain::AuthenticatedGui => {
                crate::is_authenticated_gui_program_registration_root(
                    evidence.registration_root_rva,
                )
            }
        };
        if !authenticated_root {
            return Err(match domain {
                ProgramAuthorityDomain::MapOrOpaqueStatic => {
                    "map/static program authority names an unauthenticated registration root"
                }
                ProgramAuthorityDomain::AuthenticatedGui => {
                    "GUI program authority is not a neutral exact engine lookup"
                }
            }
            .into());
        }
        let registration = RawProgramRegistrationObservation {
            observation_sequence: capture_sequence,
            shader_program_ptr: evidence.shader_program_ptr,
            registration_root_rva: evidence.registration_root_rva,
            source_open_sequence: source_open.observation_sequence,
            effect_token: evidence.effect_token.clone(),
            vertex_entry_token: evidence.vertex_reflection.entry_token.clone(),
            pixel_entry_token: evidence.pixel_reflection.entry_token.clone(),
            ordered_defines: evidence.ordered_defines.clone(),
        };
        crate::materialize_program_identity(
            &registration,
            &source_open,
            &evidence.vertex_reflection,
            &evidence.pixel_reflection,
        )?;
        Ok((source_open, registration))
    }

    fn resolve_live_content_identity(
        &self,
        submit: &crate::RawParticleSubmit,
        draw: &RawD3dDrawState,
        evidence: &ExactLiveDrawContentEvidence,
    ) -> Result<RawDrawContentIdentity, String> {
        match (&draw.identity, evidence) {
            (
                RawDrawIdentity::Particle { .. },
                ExactLiveDrawContentEvidence::Particle {
                    effect_instance_id,
                    registration_token,
                    definition_source_token,
                    subsystem_selector_u32,
                    definition_content,
                    texture_content,
                    authored_shader_token,
                    engine_effect_token,
                    explicit_engine_effect_token_crosscheck,
                    atlas_columns_u32,
                    atlas_rows_u32,
                    atlas_half_pixel_width_f32_bits,
                    atlas_half_pixel_height_f32_bits,
                },
            ) => {
                let packet = unique_by(
                    &submit.packets,
                    |packet| particle_identity_matches(packet, &draw.identity),
                    "particle draw identity",
                )?;
                if packet.effect_instance_id != *effect_instance_id
                    || packet.definition.registration_token != *registration_token
                    || packet.definition.definition_source_token != *definition_source_token
                    || packet.packet_ordinal != *subsystem_selector_u32
                    || packet.subsystem_selector_u32 != *subsystem_selector_u32
                    || packet.engine_effect_token != *engine_effect_token
                    || explicit_engine_effect_token_crosscheck
                        .as_ref()
                        .is_some_and(|expected| expected != engine_effect_token)
                    || packet.shader_program_ptr != draw.shader_program_ptr
                    || packet.atlas_columns_f32_bits != (*atlas_columns_u32 as f32).to_bits()
                    || packet.atlas_rows_f32_bits != (*atlas_rows_u32 as f32).to_bits()
                    || packet.atlas_half_pixel_width_f32_bits != *atlas_half_pixel_width_f32_bits
                    || packet.atlas_half_pixel_height_f32_bits != *atlas_half_pixel_height_f32_bits
                    || *atlas_rows_u32 == 0
                    || *atlas_columns_u32 == 0
                {
                    return Err(
                        "particle runtime registration identity disagrees with the draw".into(),
                    );
                }
                if registration_token.trim().is_empty()
                    || definition_source_token.trim().is_empty()
                    || authored_shader_token.trim().is_empty()
                    || engine_effect_token.trim().is_empty()
                {
                    return Err("particle runtime registration identity is empty".into());
                }
                if *atlas_rows_u32 == 0 || *atlas_columns_u32 == 0 {
                    return Err("particle atlas dimensions are zero".into());
                }
                self.authorize_content(definition_content)?;
                self.authorize_content(texture_content)?;
                Ok(RawDrawContentIdentity::Particle {
                    registration_token: registration_token.clone(),
                    definition_source_token: definition_source_token.clone(),
                    subsystem_selector_u32: *subsystem_selector_u32,
                    definition_content: definition_content.clone(),
                    texture_content: texture_content.clone(),
                    authored_shader_token: authored_shader_token.clone(),
                    engine_effect_token: engine_effect_token.clone(),
                    atlas_columns_u32: *atlas_columns_u32,
                    atlas_rows_u32: *atlas_rows_u32,
                    atlas_half_pixel_width_f32_bits: *atlas_half_pixel_width_f32_bits,
                    atlas_half_pixel_height_f32_bits: *atlas_half_pixel_height_f32_bits,
                })
            }
            (
                RawDrawIdentity::Entity { .. },
                ExactLiveDrawContentEvidence::Entity {
                    entity_instance_id,
                    entity_definition_ptr,
                    mesh_draw_ptr,
                    material_binding_ptr,
                    resolved_entity_asset,
                    mesh,
                    entity_registration_token,
                    pass_selector_i32,
                    alternate_list_u8,
                    entity_group_index,
                    mesh_draw_list_index,
                    entity_variant_u8,
                    mesh_draw_variant_u8,
                    bind_renderer_target_u8,
                },
            ) => {
                let entity = unique_by(
                    &submit.entity_submissions,
                    |entity| entity_identity_matches(entity, &draw.identity),
                    "entity draw identity",
                )?;
                if entity.entity_instance_id != *entity_instance_id
                    || entity.entity_definition_ptr != *entity_definition_ptr
                    || entity.mesh_draw_ptr != *mesh_draw_ptr
                    || entity.material_binding_ptr != *material_binding_ptr
                    || entity.entity_registration_token.as_deref()
                        != Some(entity_registration_token.as_str())
                    || entity.pass_selector_i32 != *pass_selector_i32
                    || entity.alternate_list_u8 != *alternate_list_u8
                    || entity.entity_group_index != *entity_group_index
                    || entity.mesh_draw_list_index != *mesh_draw_list_index
                    || entity.entity_variant_u8 != *entity_variant_u8
                    || entity.mesh_draw_variant_u8 != *mesh_draw_variant_u8
                    || entity.bind_renderer_target_u8 != *bind_renderer_target_u8
                    || entity_registration_token.trim().is_empty()
                    || entity.state_flags
                        != (u32::from(*entity_variant_u8)
                            | (u32::from(*mesh_draw_variant_u8) << 8)
                            | (u32::from(*bind_renderer_target_u8) << 16)
                            | (u32::from(*alternate_list_u8) << 24))
                {
                    return Err(
                        "entity runtime registration identity disagrees with the draw".into(),
                    );
                }
                self.authorize_content(resolved_entity_asset)?;
                self.authorize_content(mesh)?;
                Ok(RawDrawContentIdentity::Entity {
                    resolved_entity_asset: resolved_entity_asset.clone(),
                    mesh: mesh.clone(),
                    entity_registration_token: entity_registration_token.clone(),
                    pass_selector_i32: *pass_selector_i32,
                    alternate_list_u8: *alternate_list_u8,
                    entity_group_index: *entity_group_index,
                    mesh_draw_list_index: *mesh_draw_list_index,
                    entity_variant_u8: *entity_variant_u8,
                    mesh_draw_variant_u8: *mesh_draw_variant_u8,
                    bind_renderer_target_u8: *bind_renderer_target_u8,
                })
            }
            _ => Err("live content evidence names the wrong draw family".into()),
        }
    }

    fn resolve_content(&self, content: &RawVfsContentIdentity) -> Result<&[u8], String> {
        self.winner_index.authorize_content(content)?;
        let payload = self
            .admitted_payloads
            .payload(content)
            .ok_or("exact decompressed winner payload was not admitted over H4CT")?;
        if payload.len() as u64 != content.payload_bytes
            || hex_sha(payload) != content.payload_sha256
        {
            return Err("admitted H4CT payload differs from the authenticated winner".into());
        }
        Ok(payload)
    }

    fn authorize_content(&self, content: &RawVfsContentIdentity) -> Result<(), String> {
        self.resolve_content(content).map(|_| ())
    }

    fn authenticated_gui_definition_content(
        &self,
        selected: &UsedContentKey,
    ) -> Result<&RawVfsContentIdentity, String> {
        selected.validate()?;
        if selected.effective_vfs_manifest_sha256 != self.winner_index.authority_manifest_sha256 {
            return Err("selected GUI definition uses another effective-VFS manifest".into());
        }
        let normalized = normalize_game_relative_path(&selected.normalized_game_relative_path)?;
        if normalized != selected.normalized_game_relative_path
            || !normalized.starts_with("interface/")
            || !normalized.ends_with(".gui")
        {
            return Err(
                "selected GUI definition is not a canonical interface/*.gui winner".into(),
            );
        }
        let index = self
            .winner_index
            .ordered_rows
            .binary_search_by(|row| row.content.logical_path.as_str().cmp(normalized.as_str()))
            .map_err(|_| "selected GUI definition is absent from the effective-VFS winners")?;
        let content = &self.winner_index.ordered_rows[index].content;
        if raw_to_used_content(self.winner_index, content)? != *selected {
            return Err("selected GUI definition disagrees with its authenticated winner".into());
        }
        Ok(content)
    }

    fn authorize_target_content(
        &self,
        colors: &[RawRenderTargetRegistrationReceipt],
        depth: Option<&RawRenderTargetRegistrationReceipt>,
    ) -> Result<(), String> {
        for receipt in colors.iter().chain(depth) {
            if let Some(content) = &receipt.target.persistent_content {
                self.authorize_content(content)?;
            }
        }
        Ok(())
    }
}

pub fn materialize_exact_vfs_open(
    winner_index: &RawEffectiveVfsWinnerIndexBinding,
    admitted_payloads: &impl AdmittedAuthorityPayloads,
    evidence: &ExactVfsOpenEvidence,
) -> Result<RawEffectiveVfsOpenObservation, String> {
    if evidence.observation_sequence == 0
        || evidence.stream_handle == 0
        || evidence.data_ptr == Some(0)
        || evidence.ordered_logical_path_tokens.is_empty()
        || evidence.opened_payload_bytes == 0
        || !valid_sha(&evidence.opened_payload_sha256)
    {
        return Err("effective-VFS open evidence is incomplete".into());
    }
    let content = source_content_for_open(winner_index, evidence)?;
    let payload = admitted_payloads
        .payload(content)
        .ok_or("effective-VFS open full payload was not admitted over H4CT")?;
    if payload.len() as u64 != evidence.opened_payload_bytes
        || hex_sha(payload) != evidence.opened_payload_sha256
    {
        return Err("effective-VFS open payload differs from admitted H4CT bytes".into());
    }
    RawEffectiveVfsOpenObservation::from_used_content(
        evidence.observation_sequence,
        &raw_to_used_content(winner_index, content)?,
        evidence.stream_handle,
        evidence.data_ptr,
    )
}

fn source_content_for_open<'a>(
    winner_index: &'a RawEffectiveVfsWinnerIndexBinding,
    evidence: &ExactVfsOpenEvidence,
) -> Result<&'a RawVfsContentIdentity, String> {
    winner_index.validate()?;
    if evidence.ordered_logical_path_tokens.is_empty()
        || evidence.opened_payload_bytes == 0
        || !valid_sha(&evidence.opened_payload_sha256)
    {
        return Err("effective-VFS open content receipt is incomplete".into());
    }
    let mut candidates = BTreeMap::new();
    for token in &evidence.ordered_logical_path_tokens {
        let normalized = normalize_game_relative_path(token)?;
        candidates.entry(normalized.clone()).or_insert(normalized);
    }
    let mut exact = Vec::new();
    for path in candidates.into_values() {
        let Ok(index) = winner_index
            .ordered_rows
            .binary_search_by(|row| row.content.logical_path.as_str().cmp(path.as_str()))
        else {
            continue;
        };
        let content = &winner_index.ordered_rows[index].content;
        if content.payload_bytes == evidence.opened_payload_bytes
            && content.payload_sha256 == evidence.opened_payload_sha256
        {
            exact.push(content);
        }
    }
    if exact.len() != 1 {
        return Err("effective-VFS open payload has no unique exact path-token match".into());
    }
    Ok(exact.pop().expect("exact cardinality checked"))
}

fn unique_winner_for_path_tokens<'a>(
    winner_index: &'a RawEffectiveVfsWinnerIndexBinding,
    ordered_logical_path_tokens: &[String],
) -> Result<&'a RawVfsContentIdentity, String> {
    winner_index.validate()?;
    let mut paths = BTreeSet::new();
    for token in ordered_logical_path_tokens {
        paths.insert(normalize_game_relative_path(token)?);
    }
    let matching = paths
        .into_iter()
        .filter_map(|path| {
            winner_index
                .ordered_rows
                .binary_search_by(|row| row.content.logical_path.as_str().cmp(path.as_str()))
                .ok()
                .map(|index| &winner_index.ordered_rows[index].content)
        })
        .collect::<Vec<_>>();
    if matching.len() != 1 {
        return Err("effective-VFS open path tokens have no unique authenticated winner".into());
    }
    Ok(matching[0])
}

fn add_full_payload(
    winner_index: &RawEffectiveVfsWinnerIndexBinding,
    required: &mut BTreeMap<(String, u32, String), ExactAuthorityPayloadRequest>,
    content: &RawVfsContentIdentity,
) -> Result<(), String> {
    winner_index.authorize_content(content)?;
    required
        .entry((
            content.logical_path.clone(),
            content.winning_layer_ordinal,
            content.payload_sha256.clone(),
        ))
        .or_insert_with(|| ExactAuthorityPayloadRequest {
            content: content.clone(),
            byte_offset: 0,
            byte_count: content.payload_bytes,
            range_sha256: content.payload_sha256.clone(),
        });
    Ok(())
}

impl OpaqueRenderTargetGraphEvidence {
    fn materialize_for_draw(
        &self,
        capture_sequence: u64,
        ordered_color_views: &[usize],
        depth_view: Option<usize>,
    ) -> Result<
        (
            Vec<RawRenderTargetRegistrationReceipt>,
            Option<RawRenderTargetRegistrationReceipt>,
        ),
        String,
    > {
        if capture_sequence == 0 || ordered_color_views.is_empty() {
            return Err("draw has no exact output-target sequence".into());
        }
        let colors = ordered_color_views
            .iter()
            .map(|view| self.unique_receipt(capture_sequence, *view))
            .collect::<Result<Vec<_>, _>>()?;
        let depth = depth_view
            .map(|view| self.unique_receipt(capture_sequence, view))
            .transpose()?;
        Ok((colors, depth))
    }

    fn unique_receipt(
        &self,
        capture_sequence: u64,
        view_ptr: usize,
    ) -> Result<RawRenderTargetRegistrationReceipt, String> {
        unique_by(
            &self.registrations,
            |row| row.capture_sequence == capture_sequence && row.view_ptr == view_ptr,
            "opaque render-target registration",
        )?
        .materialize()
    }
}

fn validate_explicit_source(
    evidence: &ExactStaticTextureSourceEvidence,
    targets: &OpaqueRenderTargetGraphEvidence,
    content: &mut Vec<RawVfsContentIdentity>,
) -> Result<(), String> {
    match &evidence.source {
        RawStaticShaderResourceSource::UsedContent => {
            return Err(
                "used-content texture source omitted a file-backed resource snapshot".into(),
            )
        }
        RawStaticShaderResourceSource::AuthenticatedSnapshot => {}
        RawStaticShaderResourceSource::SnowMudTexture {
            producer_ordinal,
            payload_sha256,
        } => {
            if *producer_ordinal == 0 || !valid_sha(payload_sha256) {
                return Err("SnowMudTexture source receipt is incomplete".into());
            }
        }
        RawStaticShaderResourceSource::PreparedTarget {
            target,
            producer_capture_sequence,
        } => {
            let receipt = targets.unique_receipt(*producer_capture_sequence, target.view_ptr)?;
            if receipt.resource_ptr != evidence.resource_ptr || receipt.target != *target {
                return Err(
                    "prepared-target texture edge disagrees with the opaque target graph".into(),
                );
            }
            if let Some(persistent) = &target.persistent_content {
                content.push(persistent.clone());
            }
        }
        RawStaticShaderResourceSource::PoliticalTexture {
            texture_kind,
            target_binding_identity,
            upload_order,
            payload_sha256,
        } => {
            if texture_kind.trim().is_empty()
                || target_binding_identity.trim().is_empty()
                || *upload_order == 0
                || !valid_sha(payload_sha256)
            {
                return Err("political texture receipt is incomplete".into());
            }
        }
        RawStaticShaderResourceSource::TiledLightIndex {
            writer_index,
            payload_sha256,
        }
        | RawStaticShaderResourceSource::TiledLightData {
            writer_index,
            payload_sha256,
        } => {
            if *writer_index == 0 || !valid_sha(payload_sha256) {
                return Err("tiled-light texture receipt is incomplete".into());
            }
        }
    }
    Ok(())
}

fn opaque_content_joins(
    content: Vec<RawVfsContentIdentity>,
) -> Result<Vec<RawStaticContentJoin>, String> {
    let mut unique = BTreeSet::new();
    let mut rows = Vec::new();
    for content in content {
        let identity = (
            content.logical_path.clone(),
            content.winning_layer_ordinal,
            content.payload_sha256.clone(),
        );
        if unique.insert(identity) {
            let mut preimage = Vec::new();
            preimage.extend_from_slice(content.layer_kind.as_bytes());
            preimage.push(0);
            preimage.extend_from_slice(content.layer_id.as_bytes());
            preimage.push(0);
            preimage.extend_from_slice(content.logical_path.as_bytes());
            preimage.push(0);
            preimage.extend_from_slice(content.payload_sha256.as_bytes());
            rows.push(RawStaticContentJoin {
                content_key: format!("{:x}", Sha256::digest(preimage)),
                content,
            });
        }
    }
    if rows.is_empty() {
        return Err("static authority has no exact content joins".into());
    }
    Ok(rows)
}

fn raw_to_used_content(
    winner_index: &RawEffectiveVfsWinnerIndexBinding,
    content: &RawVfsContentIdentity,
) -> Result<UsedContentKey, String> {
    winner_index.authorize_content(content)?;
    let layer_kind = match content.layer_kind.as_str() {
        "base" => ContentLayerKind::Base,
        "integrated_dlc" => ContentLayerKind::IntegratedDlc,
        "dlc" => ContentLayerKind::Dlc,
        "mod" => ContentLayerKind::Mod,
        _ => return Err("effective-VFS winner has an unsupported layer kind".into()),
    };
    let used = UsedContentKey {
        effective_vfs_manifest_sha256: winner_index.authority_manifest_sha256.clone(),
        normalized_game_relative_path: content.logical_path.clone(),
        winning_layer_ordinal: content.winning_layer_ordinal,
        layer_kind,
        layer_id: content.layer_id.clone(),
        source_sha256: content.source_sha256.clone(),
        payload_sha256: content.payload_sha256.clone(),
        payload_bytes: content.payload_bytes,
    };
    used.validate()?;
    Ok(used)
}

fn unique_texture_object(
    rows: &[RawTextureObject],
    stage: RawShaderStage,
    slot: u8,
) -> Result<&RawTextureObject, String> {
    unique_by(
        rows,
        |row| row.stage == stage && row.slot == slot,
        "reflected texture object",
    )
}

fn unique_by<'a, T>(
    rows: &'a [T],
    predicate: impl Fn(&T) -> bool,
    label: &str,
) -> Result<&'a T, String> {
    let mut matching = rows.iter().filter(|row| predicate(row));
    let row = matching
        .next()
        .ok_or_else(|| format!("{label} is missing"))?;
    if matching.next().is_some() {
        return Err(format!("{label} is ambiguous"));
    }
    Ok(row)
}

fn particle_identity_matches(
    packet: &crate::RawParticlePacket,
    identity: &RawDrawIdentity,
) -> bool {
    matches!(
        identity,
        RawDrawIdentity::Particle {
            effect_instance_id,
            registration_token,
            definition_source_token,
            builder_call_ordinal,
            packet_ordinal,
            draw_order,
            shader_program_ptr,
        } if *effect_instance_id == packet.effect_instance_id
            && registration_token == &packet.definition.registration_token
            && definition_source_token == &packet.definition.definition_source_token
            && *builder_call_ordinal == packet.builder_call_ordinal
            && *packet_ordinal == packet.packet_ordinal
            && *draw_order == packet.draw_order
            && *shader_program_ptr == packet.shader_program_ptr
    )
}

fn entity_identity_matches(
    entity: &crate::RawEntitySubmission,
    identity: &RawDrawIdentity,
) -> bool {
    matches!(
        identity,
        RawDrawIdentity::Entity {
            submit_index,
            pass_ordinal,
            entity_instance_id,
            draw_order,
            entity_object_ptr,
            entity_definition_ptr,
            renderer_ptr,
            render_context_ptr,
            mesh_draw_ptr,
            material_binding_ptr,
            shader_program_ptr,
        } if *submit_index == entity.submit_index
            && *pass_ordinal == entity.pass_ordinal
            && *entity_instance_id == entity.entity_instance_id
            && *draw_order == entity.draw_order
            && *entity_object_ptr == entity.entity_object_ptr
            && *entity_definition_ptr == entity.entity_definition_ptr
            && *renderer_ptr == entity.renderer_ptr
            && *render_context_ptr == entity.render_context_ptr
            && *mesh_draw_ptr == entity.mesh_draw_ptr
            && *material_binding_ptr == entity.material_binding_ptr
            && *shader_program_ptr == entity.shader_program_ptr
    )
}

#[derive(Clone, Debug)]
struct DecodedTexture {
    width: u32,
    height: u32,
    mip_levels: u32,
    array_size: u32,
    dxgi_format_u32: u32,
    subresources: Vec<DecodedSubresource>,
}

#[derive(Clone, Debug)]
struct DecodedSubresource {
    index: u32,
    mip: u32,
    array_slice: u32,
    row_pitch: u32,
    slice_pitch: u32,
    bytes: Vec<u8>,
    source_byte_offset: Option<u64>,
}

fn validate_file_texture(
    snapshot: &RawTextureResourceSnapshot,
    logical_path: &str,
    bytes: &[u8],
) -> Result<(), String> {
    let extension = logical_path
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .ok_or("file-backed texture path has no extension")?;
    let decoded = match extension.as_str() {
        "dds" => decode_dds(bytes)?,
        "png" => decode_png(bytes, snapshot.dxgi_format_u32)?,
        _ => return Err("file-backed texture format is not an exact supported decoder".into()),
    };
    if snapshot.dimension_u32 != 3
        || snapshot.width != decoded.width
        || snapshot.height != decoded.height
        || snapshot.depth != 1
        || snapshot.mip_levels != decoded.mip_levels
        || snapshot.array_size != decoded.array_size
        || snapshot.dxgi_format_u32 != decoded.dxgi_format_u32
        || snapshot.sample_count != 1
        || snapshot.sample_quality != 0
        || snapshot.ordered_subresources.len() != decoded.subresources.len()
    {
        return Err("decoded texture descriptor differs from the D3D snapshot".into());
    }
    for (captured, expected) in snapshot
        .ordered_subresources
        .iter()
        .zip(&decoded.subresources)
    {
        if captured.subresource_index != expected.index
            || captured.mip_level != expected.mip
            || captured.array_slice != expected.array_slice
            || captured.row_pitch != expected.row_pitch
            || captured.slice_pitch != expected.slice_pitch
            || captured.payload.captured_bytes != expected.bytes
            || captured.payload.captured_sha256 != hex_sha(&expected.bytes)
        {
            return Err("decoded texture subresource differs from the exact D3D upload".into());
        }
        match (&captured.payload.source, expected.source_byte_offset) {
            (
                crate::RawPayloadSource::VfsRange {
                    content,
                    source_byte_offset,
                    byte_count,
                    range_sha256,
                },
                Some(expected_offset),
            ) if Some(content) == snapshot.content.as_ref()
                && *source_byte_offset == expected_offset
                && *byte_count == expected.bytes.len() as u64
                && *range_sha256 == hex_sha(&expected.bytes) => {}
            (crate::RawPayloadSource::RuntimeGenerated { reason }, None)
                if !reason.trim().is_empty() => {}
            _ => {
                return Err(
                    "texture upload source is not the exact VFS range or decoded payload".into(),
                )
            }
        }
    }
    Ok(())
}

fn decode_png(bytes: &[u8], dxgi_format_u32: u32) -> Result<DecodedTexture, String> {
    if !matches!(dxgi_format_u32, 28 | 87) {
        return Err("PNG exact decoder supports only RGBA8/BGRA8 D3D resources".into());
    }
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder
        .read_info()
        .map_err(|error| format!("decode PNG header: {error}"))?;
    let mut buffer = vec![0; reader.output_buffer_size()];
    let info = reader
        .next_frame(&mut buffer)
        .map_err(|error| format!("decode PNG payload: {error}"))?;
    let input = &buffer[..info.buffer_size()];
    let mut rgba = Vec::with_capacity(info.width as usize * info.height as usize * 4);
    match info.color_type {
        png::ColorType::Rgba => rgba.extend_from_slice(input),
        png::ColorType::Rgb => input.chunks_exact(3).for_each(|pixel| {
            rgba.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
        }),
        png::ColorType::Grayscale => input.iter().for_each(|value| {
            rgba.extend_from_slice(&[*value, *value, *value, 255]);
        }),
        png::ColorType::GrayscaleAlpha => input.chunks_exact(2).for_each(|pixel| {
            rgba.extend_from_slice(&[pixel[0], pixel[0], pixel[0], pixel[1]]);
        }),
        png::ColorType::Indexed => {
            return Err("PNG palette was not expanded by the exact decoder".into())
        }
    }
    if dxgi_format_u32 == 87 {
        for pixel in rgba.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
    }
    let row_pitch = info.width.checked_mul(4).ok_or("PNG row pitch overflow")?;
    let slice_pitch = row_pitch
        .checked_mul(info.height)
        .ok_or("PNG slice pitch overflow")?;
    if rgba.len() != slice_pitch as usize {
        return Err("PNG decoder produced a non-exact pixel count".into());
    }
    Ok(DecodedTexture {
        width: info.width,
        height: info.height,
        mip_levels: 1,
        array_size: 1,
        dxgi_format_u32,
        subresources: vec![DecodedSubresource {
            index: 0,
            mip: 0,
            array_slice: 0,
            row_pitch,
            slice_pitch,
            bytes: rgba,
            source_byte_offset: None,
        }],
    })
}

fn decode_dds(bytes: &[u8]) -> Result<DecodedTexture, String> {
    if bytes.len() < 128 || &bytes[..4] != b"DDS " || le_u32(bytes, 4)? != 124 {
        return Err("DDS header is missing or malformed".into());
    }
    let height = le_u32(bytes, 12)?;
    let width = le_u32(bytes, 16)?;
    let mip_levels = le_u32(bytes, 28)?.max(1);
    let pixel_flags = le_u32(bytes, 80)?;
    let fourcc = &bytes[84..88];
    let mut array_size = 1u32;
    let (format, data_offset) = if fourcc == b"DX10" {
        if bytes.len() < 148 || le_u32(bytes, 132)? != 3 {
            return Err("DDS DX10 resource is not an exact Texture2D".into());
        }
        array_size = le_u32(bytes, 140)?;
        (le_u32(bytes, 128)?, 148usize)
    } else if pixel_flags & 0x4 != 0 {
        let format = match fourcc {
            b"DXT1" => 71,
            b"DXT3" => 74,
            b"DXT5" => 77,
            b"ATI1" | b"BC4U" => 80,
            b"ATI2" | b"BC5U" => 83,
            _ => return Err("DDS FourCC is not an exact supported format".into()),
        };
        (format, 128usize)
    } else if le_u32(bytes, 88)? == 32 {
        let masks = (
            le_u32(bytes, 92)?,
            le_u32(bytes, 96)?,
            le_u32(bytes, 100)?,
            le_u32(bytes, 104)?,
        );
        let format = match masks {
            (0x000000ff, 0x0000ff00, 0x00ff0000, 0xff000000) => 28,
            (0x00ff0000, 0x0000ff00, 0x000000ff, 0xff000000) => 87,
            _ => return Err("DDS 32-bit channel masks are unsupported".into()),
        };
        (format, 128usize)
    } else {
        return Err("DDS pixel format is unsupported".into());
    };
    if width == 0 || height == 0 || array_size == 0 || mip_levels == 0 {
        return Err("DDS dimensions are zero".into());
    }
    let mut cursor = data_offset;
    let mut subresources = Vec::new();
    for array_slice in 0..array_size {
        for mip in 0..mip_levels {
            let mip_width = (width >> mip).max(1);
            let mip_height = (height >> mip).max(1);
            let (row_pitch, row_count) = match format {
                71 | 80 => (
                    ((mip_width + 3) / 4).max(1) * 8,
                    ((mip_height + 3) / 4).max(1),
                ),
                74 | 77 | 83 => (
                    ((mip_width + 3) / 4).max(1) * 16,
                    ((mip_height + 3) / 4).max(1),
                ),
                28 | 87 => (
                    mip_width.checked_mul(4).ok_or("DDS row pitch overflow")?,
                    mip_height,
                ),
                _ => return Err("DDS DX10 format is not an exact supported layout".into()),
            };
            let slice_pitch = row_pitch
                .checked_mul(row_count)
                .ok_or("DDS slice pitch overflow")?;
            let end = cursor
                .checked_add(slice_pitch as usize)
                .ok_or("DDS subresource range overflow")?;
            let payload = bytes
                .get(cursor..end)
                .ok_or("DDS subresource payload is truncated")?
                .to_vec();
            subresources.push(DecodedSubresource {
                index: mip + array_slice * mip_levels,
                mip,
                array_slice,
                row_pitch,
                slice_pitch,
                bytes: payload,
                source_byte_offset: Some(cursor as u64),
            });
            cursor = end;
        }
    }
    if cursor != bytes.len() {
        return Err("DDS payload has trailing or unaccounted bytes".into());
    }
    Ok(DecodedTexture {
        width,
        height,
        mip_levels,
        array_size,
        dxgi_format_u32: format,
        subresources,
    })
}

fn le_u32(bytes: &[u8], offset: usize) -> Result<u32, String> {
    bytes
        .get(offset..offset + 4)
        .and_then(|slice| slice.try_into().ok())
        .map(u32::from_le_bytes)
        .ok_or_else(|| "binary texture header is truncated".into())
}

fn hex_sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn valid_sha(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RawPayloadSnapshot, RawPayloadSource, RawTextureSubresourceSnapshot};
    #[derive(Default)]
    struct TestPayloads(Vec<(RawVfsContentIdentity, Vec<u8>)>);

    impl AdmittedAuthorityPayloads for TestPayloads {
        fn payload_range(
            &self,
            content: &RawVfsContentIdentity,
            byte_offset: u64,
            byte_count: u64,
            range_sha256: &str,
        ) -> Option<&[u8]> {
            let bytes = &self.0.iter().find(|(key, _)| key == content)?.1;
            let start = usize::try_from(byte_offset).ok()?;
            let end = start.checked_add(usize::try_from(byte_count).ok()?)?;
            let range = bytes.get(start..end)?;
            (hex_sha(range) == range_sha256).then_some(range)
        }
    }

    fn content(
        path: &str,
        bytes: &[u8],
        ordinal: u32,
        layer_kind: &str,
        layer_id: &str,
        source_sha256: String,
    ) -> RawVfsContentIdentity {
        RawVfsContentIdentity {
            logical_path: path.into(),
            winning_layer_ordinal: ordinal,
            layer_kind: layer_kind.into(),
            layer_id: layer_id.into(),
            source_sha256,
            payload_sha256: hex_sha(bytes),
            payload_bytes: bytes.len() as u64,
        }
    }

    fn binding(rows: Vec<crate::RawEffectiveVfsWinnerRow>) -> RawEffectiveVfsWinnerIndexBinding {
        RawEffectiveVfsWinnerIndexBinding {
            process_start_nonce: 7,
            authority_manifest_sha256: "a".repeat(64),
            declared_row_count: rows.len() as u64,
            ordered_rows_sha256: RawEffectiveVfsWinnerIndexBinding::canonical_rows_sha256(&rows)
                .unwrap(),
            ordered_rows: rows,
        }
    }

    fn loose_row(path: &str, bytes: &[u8]) -> crate::RawEffectiveVfsWinnerRow {
        let content = content(path, bytes, 0, "base", "base", hex_sha(bytes));
        crate::RawEffectiveVfsWinnerRow {
            source_container_sha256: None,
            source_byte_offset: 0,
            source_byte_count: bytes.len() as u64,
            source_range_sha256: hex_sha(bytes),
            content,
        }
    }

    fn dummy_creation(stage: RawShaderStage, pointer: usize) -> RawShaderCreation {
        RawShaderCreation {
            creation_sequence: 1,
            shader_ptr: pointer,
            stage,
            compiler_identity: "captured-compiler".into(),
            target_profile: if stage == RawShaderStage::Vertex {
                "vs_5_0"
            } else {
                "ps_5_0"
            }
            .into(),
            compiler_flags_u64: 0,
            compiler_invocation: None,
            bytecode: RawPayloadSnapshot {
                captured_bytes: vec![1],
                captured_sha256: hex_sha(&[1]),
                source: RawPayloadSource::RuntimeGenerated {
                    reason: "test".into(),
                },
            },
        }
    }

    fn dummy_reflection(
        stage: RawShaderStage,
        pointer: usize,
    ) -> RawShaderBytecodeReflectionObservation {
        RawShaderBytecodeReflectionObservation {
            reflection_sequence: 1,
            creation_sequence: 1,
            shader_ptr: pointer,
            shader_program_ptr: 0x100,
            stage,
            bytecode_sha256: hex_sha(&[1]),
            bytecode_bytes: 1,
            entry_token: if stage == RawShaderStage::Vertex {
                "VS"
            } else {
                "PS"
            }
            .into(),
            target_profile: if stage == RawShaderStage::Vertex {
                "vs_5_0"
            } else {
                "ps_5_0"
            }
            .into(),
            creator_token: "captured-reflection".into(),
            ordered_inputs: Vec::new(),
            ordered_outputs: Vec::new(),
            ordered_resources: Vec::new(),
        }
    }

    fn program(source: &RawVfsContentIdentity) -> ExactProgramAuthorityEvidence {
        ExactProgramAuthorityEvidence {
            path_join_sequence: 1,
            shader_program_ptr: 0x100,
            registration_root_rva: crate::MAP_EFFECT_REGISTRATION_ROOT_RVA,
            effect_token: "exact-effect".into(),
            ordered_defines: Vec::new(),
            source_open: ExactVfsOpenEvidence {
                observation_sequence: 1,
                ordered_logical_path_tokens: vec![source.logical_path.clone()],
                stream_handle: 1,
                data_ptr: None,
                opened_payload_sha256: source.payload_sha256.clone(),
                opened_payload_bytes: source.payload_bytes,
            },
            vertex_creation: dummy_creation(RawShaderStage::Vertex, 0x101),
            pixel_creation: dummy_creation(RawShaderStage::Pixel, 0x102),
            vertex_reflection: dummy_reflection(RawShaderStage::Vertex, 0x101),
            pixel_reflection: dummy_reflection(RawShaderStage::Pixel, 0x102),
        }
    }

    fn live_draw() -> RawD3dDrawState {
        RawD3dDrawState {
            final_draw_sequence: 10,
            identity: RawDrawIdentity::Particle {
                effect_instance_id: 1,
                registration_token: "particle".into(),
                definition_source_token: "particle_file".into(),
                builder_call_ordinal: 0,
                packet_ordinal: 0,
                draw_order: 0,
                shader_program_ptr: 0x100,
            },
            family: crate::RawDrawFamily::Particle,
            submit_index: 1,
            draw_order: 0,
            render_context_ptr: 1,
            device_context_ptr: 2,
            shader_program_ptr: 0x100,
            vertex_shader_ptr: 0x101,
            pixel_shader_ptr: 0x102,
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
            ordered_texture_objects: Vec::new(),
            ordered_sampler_objects: Vec::new(),
            ordered_constant_buffer_objects: Vec::new(),
            ordered_vertex_buffer_objects: Vec::new(),
            index_buffer: None,
            ordered_stream_output_buffer_ptrs: Vec::new(),
            ordered_color_target_view_ptrs: vec![0x500],
            depth_stencil_view_ptr: None,
            ordered_viewports_f32_bits: Vec::new(),
            ordered_scissor_rects_i32: Vec::new(),
            draw_call: crate::RawDrawCall::Draw {
                vertex_count: 1,
                start_vertex: 0,
            },
        }
    }

    fn target_graph(capture_sequence: u64) -> OpaqueRenderTargetGraphEvidence {
        OpaqueRenderTargetGraphEvidence {
            registrations: vec![RawRenderTargetRegistrationObservation {
                registration_sequence: capture_sequence - 1,
                capture_sequence,
                producer_rva: crate::GENERIC_TEXTURE_RTV_PRODUCER_RVA,
                recreate_generation: 0,
                attachment_ordinal: 0,
                kind: crate::RawRegisteredRenderTargetKind::OpaqueRenderTarget {
                    source_flags_u32: 2,
                },
                view_ptr: 0x500,
                resource_ptr: 0x501,
                resource_desc: crate::RawTexture2dDesc {
                    width: 4,
                    height: 4,
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
                rtv_desc: crate::RawResourceViewDesc {
                    dxgi_format_u32: 28,
                    view_dimension_u32: 4,
                    flags_u32: 0,
                    union_words_u32: [0; 4],
                },
            }],
        }
    }

    fn static_draw() -> RawStaticD3dDrawState {
        RawStaticD3dDrawState {
            capture_sequence: 20,
            shader_program_ptr: 0x100,
            render_context_ptr: 1,
            device_context_ptr: 2,
            vertex_shader_ptr: 0x101,
            pixel_shader_ptr: 0x102,
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
            ordered_texture_objects: Vec::new(),
            ordered_sampler_objects: Vec::new(),
            ordered_constant_buffer_objects: Vec::new(),
            ordered_vertex_buffer_objects: Vec::new(),
            index_buffer: None,
            ordered_stream_output_buffers: Vec::new(),
            ordered_color_target_view_ptrs: vec![0x500],
            depth_stencil_view_ptr: None,
            ordered_viewports_f32_bits: Vec::new(),
            ordered_scissor_rects_i32: Vec::new(),
            draw_call: crate::RawStaticDrawCall::Draw {
                vertex_count: 1,
                start_vertex: 0,
            },
            ordered_texture_semantics: Vec::new(),
            ordered_sampler_semantics: Vec::new(),
            ordered_constant_buffer_semantics: Vec::new(),
            ordered_color_targets: Vec::new(),
            depth_target: None,
            ordered_content: Vec::new(),
        }
    }

    #[test]
    fn exact_open_uses_admitted_archive_winner_and_rejects_ambiguity() {
        let archive_sha = "b".repeat(64);
        let program = content(
            "gfx/fx/program.shader",
            b"modded",
            1,
            "mod",
            "mod/override.mod",
            archive_sha.clone(),
        );
        let twin = content(
            "gfx/fx/twin.shader",
            b"modded",
            1,
            "mod",
            "mod/override.mod",
            archive_sha.clone(),
        );
        let index = binding(vec![
            crate::RawEffectiveVfsWinnerRow {
                content: program.clone(),
                source_container_sha256: Some(archive_sha.clone()),
                source_byte_offset: 93,
                source_byte_count: 8,
                source_range_sha256: "c".repeat(64),
            },
            crate::RawEffectiveVfsWinnerRow {
                content: twin.clone(),
                source_container_sha256: Some(archive_sha),
                source_byte_offset: 187,
                source_byte_count: 8,
                source_range_sha256: "d".repeat(64),
            },
        ]);
        let payloads = TestPayloads(vec![
            (program.clone(), b"modded".to_vec()),
            (twin, b"modded".to_vec()),
        ]);
        let producer = ProductionAuthorityObservationProducer::new(&index, &payloads).unwrap();
        assert_eq!(producer.winner_index.process_start_nonce, 7);
        let open = materialize_exact_vfs_open(
            &index,
            &payloads,
            &ExactVfsOpenEvidence {
                observation_sequence: 1,
                ordered_logical_path_tokens: vec!["gfx/fx/program.shader".into()],
                stream_handle: 2,
                data_ptr: Some(3),
                opened_payload_sha256: hex_sha(b"modded"),
                opened_payload_bytes: 6,
            },
        )
        .unwrap();
        assert_eq!(open.content.layer_kind, "mod");
        assert_eq!(open.content.source_sha256, "b".repeat(64));
        assert!(materialize_exact_vfs_open(
            &index,
            &payloads,
            &ExactVfsOpenEvidence {
                observation_sequence: 2,
                ordered_logical_path_tokens: vec![
                    "gfx/fx/program.shader".into(),
                    "gfx/fx/twin.shader".into(),
                ],
                stream_handle: 4,
                data_ptr: None,
                opened_payload_sha256: hex_sha(b"modded"),
                opened_payload_bytes: 6,
            },
        )
        .unwrap_err()
        .contains("unique"));
        assert!(materialize_exact_vfs_open(
            &index,
            &payloads,
            &ExactVfsOpenEvidence {
                observation_sequence: 0,
                ordered_logical_path_tokens: vec!["gfx/fx/program.shader".into()],
                stream_handle: 4,
                data_ptr: None,
                opened_payload_sha256: hex_sha(b"modded"),
                opened_payload_bytes: 6,
            },
        )
        .is_err());
    }

    #[test]
    fn authority_payload_rosters_cover_all_live_categories_and_reject_unbound_content() {
        let mut rows = [
            ("gfx/entities/entity.asset", b"entity".as_slice()),
            ("gfx/fx/program.shader", b"program".as_slice()),
            ("gfx/models/entity.mesh", b"mesh".as_slice()),
            ("gfx/particles/definition.asset", b"particle".as_slice()),
            ("gfx/particles/texture.dds", b"texture".as_slice()),
        ]
        .into_iter()
        .map(|(path, bytes)| loose_row(path, bytes))
        .collect::<Vec<_>>();
        rows.sort_by(|left, right| left.content.logical_path.cmp(&right.content.logical_path));
        let index = binding(rows);
        let payloads = TestPayloads::default();
        let producer = ProductionAuthorityObservationProducer::new(&index, &payloads).unwrap();
        let source = index.ordered_rows[1].content.clone();
        let definition = index.ordered_rows[3].content.clone();
        let texture = index.ordered_rows[4].content.clone();
        let particle = ExactLiveDrawContentEvidence::Particle {
            effect_instance_id: 1,
            registration_token: "particle".into(),
            definition_source_token: "particle_file".into(),
            subsystem_selector_u32: 0,
            definition_content: definition.clone(),
            texture_content: texture.clone(),
            authored_shader_token: "effect".into(),
            engine_effect_token: "effect".into(),
            explicit_engine_effect_token_crosscheck: Some("effect".into()),
            atlas_columns_u32: 1,
            atlas_rows_u32: 1,
            atlas_half_pixel_width_f32_bits: (0.5f32 / 4.0).to_bits(),
            atlas_half_pixel_height_f32_bits: (0.5f32 / 4.0).to_bits(),
        };
        let particle_requests = producer
            .required_live_authority_payloads(
                &live_draw(),
                &particle,
                &program(&source),
                &target_graph(10),
            )
            .unwrap();
        assert_eq!(particle_requests.len(), 3);
        assert!(particle_requests.iter().all(|request| {
            request.byte_offset == 0
                && request.byte_count == request.content.payload_bytes
                && request.range_sha256 == request.content.payload_sha256
        }));

        let entity = ExactLiveDrawContentEvidence::Entity {
            entity_instance_id: 1,
            entity_definition_ptr: 2,
            mesh_draw_ptr: 3,
            material_binding_ptr: 4,
            resolved_entity_asset: index.ordered_rows[0].content.clone(),
            mesh: index.ordered_rows[2].content.clone(),
            entity_registration_token: "test_entity".into(),
            pass_selector_i32: 0,
            alternate_list_u8: 0,
            entity_group_index: 0,
            mesh_draw_list_index: 0,
            entity_variant_u8: 0,
            mesh_draw_variant_u8: 0,
            bind_renderer_target_u8: 0,
        };
        assert_eq!(
            producer
                .required_live_authority_payloads(
                    &live_draw(),
                    &entity,
                    &program(&source),
                    &target_graph(10),
                )
                .unwrap()
                .len(),
            3
        );
        let mut unbound = definition;
        unbound.logical_path = "gfx/particles/unbound.asset".into();
        let unbound = ExactLiveDrawContentEvidence::Particle {
            effect_instance_id: 1,
            registration_token: "particle".into(),
            definition_source_token: "particle_file".into(),
            subsystem_selector_u32: 0,
            definition_content: unbound,
            texture_content: texture,
            authored_shader_token: "effect".into(),
            engine_effect_token: "effect".into(),
            explicit_engine_effect_token_crosscheck: Some("effect".into()),
            atlas_columns_u32: 1,
            atlas_rows_u32: 1,
            atlas_half_pixel_width_f32_bits: (0.5f32 / 4.0).to_bits(),
            atlas_half_pixel_height_f32_bits: (0.5f32 / 4.0).to_bits(),
        };
        assert!(producer
            .required_live_authority_payloads(
                &live_draw(),
                &unbound,
                &program(&source),
                &target_graph(10),
            )
            .is_err());
    }

    #[test]
    fn static_authority_payload_roster_dedupes_file_content_and_omits_transient_targets() {
        let mut rows = [
            ("gfx/fx/static.shader", b"program".as_slice()),
            ("gfx/textures/file.dds", b"file".as_slice()),
        ]
        .into_iter()
        .map(|(path, bytes)| loose_row(path, bytes))
        .collect::<Vec<_>>();
        rows.sort_by(|left, right| left.content.logical_path.cmp(&right.content.logical_path));
        let index = binding(rows);
        let payloads = TestPayloads::default();
        let producer = ProductionAuthorityObservationProducer::new(&index, &payloads).unwrap();
        let file = index.ordered_rows[1].content.clone();
        let evidence = ExactStaticDrawEvidence {
            draw: static_draw(),
            texture_resource_snapshots: vec![
                RawTextureResourceSnapshot {
                    snapshot_sequence: 20,
                    resource_ptr: 0x700,
                    dimension_u32: 3,
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
                    content: Some(file.clone()),
                    ordered_subresources: Vec::new(),
                },
                RawTextureResourceSnapshot {
                    snapshot_sequence: 20,
                    resource_ptr: 0x701,
                    dimension_u32: 3,
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
                    content: Some(file.clone()),
                    ordered_subresources: Vec::new(),
                },
            ],
            shader_resource_view_snapshots: Vec::new(),
            explicit_texture_sources: Vec::new(),
        };
        let requests = producer
            .required_static_authority_payloads(
                &evidence,
                &program(&index.ordered_rows[0].content),
                &target_graph(20),
            )
            .unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.content == file)
                .count(),
            1
        );
    }

    fn texture_snapshot(
        content: &RawVfsContentIdentity,
        decoded: &DecodedTexture,
    ) -> RawTextureResourceSnapshot {
        RawTextureResourceSnapshot {
            snapshot_sequence: 1,
            resource_ptr: 2,
            dimension_u32: 3,
            width: decoded.width,
            height: decoded.height,
            depth: 1,
            mip_levels: decoded.mip_levels,
            array_size: decoded.array_size,
            dxgi_format_u32: decoded.dxgi_format_u32,
            sample_count: 1,
            sample_quality: 0,
            usage_u32: 0,
            bind_flags_u32: 8,
            cpu_access_flags_u32: 0,
            misc_flags_u32: 0,
            content: Some(content.clone()),
            ordered_subresources: decoded
                .subresources
                .iter()
                .map(|row| RawTextureSubresourceSnapshot {
                    subresource_index: row.index,
                    mip_level: row.mip,
                    array_slice: row.array_slice,
                    row_pitch: row.row_pitch,
                    slice_pitch: row.slice_pitch,
                    payload: RawPayloadSnapshot {
                        captured_bytes: row.bytes.clone(),
                        captured_sha256: hex_sha(&row.bytes),
                        source: match row.source_byte_offset {
                            Some(source_byte_offset) => RawPayloadSource::VfsRange {
                                content: content.clone(),
                                source_byte_offset,
                                byte_count: row.bytes.len() as u64,
                                range_sha256: hex_sha(&row.bytes),
                            },
                            None => RawPayloadSource::RuntimeGenerated {
                                reason: "exact_texture_decode".into(),
                            },
                        },
                    },
                })
                .collect(),
        }
    }

    #[test]
    fn png_and_dds_layouts_match_exact_subresources_and_reject_drift() {
        let mut png_bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png_bytes, 2, 1);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&[1, 2, 3, 4, 5, 6, 7, 8]).unwrap();
        }
        let png_decoded = decode_png(&png_bytes, 28).unwrap();
        assert_eq!(png_decoded.subresources[0].bytes, [1, 2, 3, 4, 5, 6, 7, 8]);

        let mut dds = vec![0u8; 128];
        dds[..4].copy_from_slice(b"DDS ");
        dds[4..8].copy_from_slice(&124u32.to_le_bytes());
        dds[12..16].copy_from_slice(&1u32.to_le_bytes());
        dds[16..20].copy_from_slice(&2u32.to_le_bytes());
        dds[28..32].copy_from_slice(&1u32.to_le_bytes());
        dds[76..80].copy_from_slice(&32u32.to_le_bytes());
        dds[80..84].copy_from_slice(&0x41u32.to_le_bytes());
        dds[88..92].copy_from_slice(&32u32.to_le_bytes());
        dds[92..96].copy_from_slice(&0x000000ffu32.to_le_bytes());
        dds[96..100].copy_from_slice(&0x0000ff00u32.to_le_bytes());
        dds[100..104].copy_from_slice(&0x00ff0000u32.to_le_bytes());
        dds[104..108].copy_from_slice(&0xff000000u32.to_le_bytes());
        dds.extend_from_slice(&[9, 10, 11, 12, 13, 14, 15, 16]);
        let dds_decoded = decode_dds(&dds).unwrap();
        assert_eq!(
            dds_decoded.subresources[0].bytes,
            [9, 10, 11, 12, 13, 14, 15, 16]
        );

        let png_content = content(
            "gfx/test.png",
            &png_bytes,
            0,
            "base",
            "base",
            hex_sha(&png_bytes),
        );
        let dds_content = content("gfx/test.dds", &dds, 0, "base", "base", hex_sha(&dds));
        validate_file_texture(
            &texture_snapshot(&png_content, &png_decoded),
            &png_content.logical_path,
            &png_bytes,
        )
        .unwrap();
        let mut dds_snapshot = texture_snapshot(&dds_content, &dds_decoded);
        validate_file_texture(&dds_snapshot, &dds_content.logical_path, &dds).unwrap();
        dds_snapshot.ordered_subresources[0].payload.captured_bytes[0] ^= 1;
        assert!(validate_file_texture(&dds_snapshot, &dds_content.logical_path, &dds).is_err());
        assert!(decode_png(&png_bytes, 71).is_err());
    }
}
