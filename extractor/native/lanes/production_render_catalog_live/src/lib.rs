use portable_content_manifest::{
    normalize_game_relative_path, ContentLayerKind, ContentManifest, UsedContentKey,
};
use portable_render_stream::*;
use production_render_catalog_capture_contract::*;
use production_render_submit_live_lane::{
    AuthenticatedRenderCatalog, EntityDrawCatalogKey, ParticleDrawCatalogKey, PromotionError,
    RawEntitySubmission, RawParticlePacket, RawParticleSubmit, ResolvedEntityCatalogRow,
    ResolvedParticleCatalogRow,
};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

pub trait AdmittedVfsPayloads {
    fn payload(&self, content: &UsedContentKey) -> Option<&[u8]>;

    fn payload_range(
        &self,
        content: &UsedContentKey,
        byte_offset: u64,
        byte_count: u64,
        range_sha256: &str,
    ) -> Option<&[u8]> {
        let full = self.payload(content)?;
        if full.len() as u64 != content.payload_bytes || hex_sha256(full) != content.payload_sha256
        {
            return None;
        }
        let start = usize::try_from(byte_offset).ok()?;
        let count = usize::try_from(byte_count).ok()?;
        let end = start.checked_add(count)?;
        let range = full.get(start..end)?;
        (hex_sha256(range) == range_sha256).then_some(range)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogBuildError(pub String);

impl fmt::Display for CatalogBuildError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.write_str(&self.0)
    }
}

impl std::error::Error for CatalogBuildError {}

impl From<PromotionError> for CatalogBuildError {
    fn from(error: PromotionError) -> Self {
        Self(format!(
            "authenticated catalog rejected derived rows: {error:?}"
        ))
    }
}

fn invalid<T>(message: impl Into<String>) -> Result<T, CatalogBuildError> {
    Err(CatalogBuildError(message.into()))
}

pub fn build_authenticated_render_catalog<V: AdmittedVfsPayloads>(
    manifest: &ContentManifest,
    raw: &RawParticleSubmit,
    capture: &RawRenderCatalogCapture,
    admitted_vfs: &V,
) -> Result<AuthenticatedRenderCatalog, CatalogBuildError> {
    manifest
        .validate()
        .map_err(|error| CatalogBuildError(format!("content manifest: {error}")))?;
    capture
        .validate_launch_binding()
        .map_err(|error| CatalogBuildError(format!("capture launch receipt: {error}")))?;
    if manifest.manifest_digest_sha256.is_none() {
        return invalid("content manifest is not finalized");
    }
    if raw.submit_index == 0
        || capture.process.process_id == 0
        || capture.process.process_start_nonce == 0
        || capture.submit_index == 0
        || capture.frame_sequence == 0
    {
        return invalid("capture has an incomplete process/frame/submit epoch");
    }
    if capture.process.executable_sha256 != manifest.source_game.executable_sha256 {
        return invalid("capture executable disagrees with source-game manifest");
    }
    if capture.submit_index != raw.submit_index {
        return invalid("capture belongs to a different raw submit");
    }
    let expected_draws = raw
        .packets
        .len()
        .checked_add(raw.entity_submissions.len())
        .ok_or_else(|| CatalogBuildError("draw count overflow".into()))?;
    if capture.draw_states.len() != expected_draws || capture.draw_semantics.len() != expected_draws
    {
        return invalid("capture is missing draws/semantics or contains extras");
    }

    let content = ManifestContentIndex::new(manifest)?;
    let mut builder = Builder::new(capture, &content, admitted_vfs)?;
    let mut states = index_unique(&capture.draw_states, "draw state", |row| {
        row.identity.clone()
    })?;
    let mut semantics = index_unique(&capture.draw_semantics, "draw semantics", |row| {
        row.identity.clone()
    })?;

    let mut particle_rows = Vec::with_capacity(raw.packets.len());
    for packet in &raw.packets {
        let identity = particle_identity(packet);
        let state = states
            .remove(&identity)
            .ok_or_else(|| CatalogBuildError("particle has no exact final-draw state".into()))?;
        let semantic = semantics.remove(&identity).ok_or_else(|| {
            CatalogBuildError("particle has no exact semantic observation".into())
        })?;
        validate_state_identity(
            state,
            RawDrawFamily::Particle,
            raw.submit_index,
            packet.draw_order,
        )?;
        let row = builder.particle_row(packet, state, semantic)?;
        particle_rows.push((ParticleDrawCatalogKey::from(packet), row));
    }

    let mut entity_rows = Vec::with_capacity(raw.entity_submissions.len());
    for entity in &raw.entity_submissions {
        let identity = entity_identity(entity);
        let state = states
            .remove(&identity)
            .ok_or_else(|| CatalogBuildError("entity has no exact final-draw state".into()))?;
        let semantic = semantics
            .remove(&identity)
            .ok_or_else(|| CatalogBuildError("entity has no exact semantic observation".into()))?;
        validate_state_identity(
            state,
            RawDrawFamily::Entity,
            raw.submit_index,
            entity.draw_order,
        )?;
        validate_entity_raw_join(entity, state)?;
        let row = builder.entity_row(entity, state, semantic)?;
        entity_rows.push((EntityDrawCatalogKey::from(entity), row));
    }
    if !states.is_empty() || !semantics.is_empty() {
        return invalid("capture contains a stale or extra draw/semantic row");
    }
    builder.finish_exact_closure()?;
    AuthenticatedRenderCatalog::new(manifest, particle_rows, entity_rows).map_err(Into::into)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthenticatedStaticProgramSet {
    pub ordered_programs: Vec<StaticMapPassProgram>,
    pub ordered_execution: Vec<RenderExecutionRef>,
    pub authenticated_texture_snapshots: Vec<AuthenticatedTextureSnapshotPacket>,
}

impl AuthenticatedStaticProgramSet {
    pub fn validate_canonical_partition(&self) -> Result<(), CatalogBuildError> {
        if self.ordered_programs.len() != self.ordered_execution.len() {
            return invalid("canonical static program/execution cardinality differs");
        }
        for (index, (program, execution)) in self
            .ordered_programs
            .iter()
            .zip(&self.ordered_execution)
            .enumerate()
        {
            let ordinal = u32::try_from(index)
                .map_err(|_| CatalogBuildError("canonical static ordinal overflow".into()))?;
            if program.program_ordinal != ordinal
                || program.pass_token != canonical_static_operation_token(ordinal)
                || program.ordered_operations.len() != 1
                || program.ordered_operations[0].operation_ordinal != ordinal
                || execution
                    != &(RenderExecutionRef::Static {
                        operation_ordinal: ordinal,
                    })
            {
                return invalid(
                    "canonical static program was relabeled, merged, split, or reordered",
                );
            }
            if let StaticMapCommand::Draw { draw } = &program.ordered_operations[0].command {
                if draw.draw_token != canonical_static_draw_token(ordinal) {
                    return invalid("canonical static draw was relabeled");
                }
            }
        }
        Ok(())
    }
}

pub fn portable_render_authority(
    capture: &RawRenderCatalogCapture,
    effective_vfs_manifest_sha256: String,
    effect_catalog_sha256: String,
    entity_animation_catalog_sha256: String,
) -> Result<RenderAuthority, CatalogBuildError> {
    portable_render_authority_with_input(
        capture,
        effective_vfs_manifest_sha256,
        effect_catalog_sha256,
        entity_animation_catalog_sha256,
        None,
    )
}

pub const RECORDED_INPUT_VARIANT_PRODUCTION_ENABLED: bool = true;
pub const RECORDED_INPUT_VARIANT_AUTHORITY_MANIFEST_SHA256: Option<&str> =
    Some("39b71a6fb75e53e31136e1909452b85106baf83addeb78da92901113cd98f0b0");
pub const A729_EXECUTABLE_SHA256: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawRecordedPointerInputObservation {
    pub executable_sha256: String,
    pub process_id: u32,
    pub process_start_nonce: u64,
    pub hook_epoch_nonce: u64,
    pub observation_sequence: u64,
    pub prior_observation_qpc_ticks: u64,
    pub selected_present_qpc_ticks: u64,
    pub present_sample_started_qpc_ticks: u64,
    pub present_sample_completed_qpc_ticks: u64,
    pub selected_present_receipt_sha256: String,
    pub client_extent: [u32; 2],
    pub prior_client_position: [i32; 2],
    pub current_client_position: [i32; 2],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthenticatedRecordedInputVariant {
    pub executable_sha256: String,
    pub process_id: u32,
    pub process_start_nonce: u64,
    pub hook_epoch_nonce: u64,
    pub observation_sequence: u64,
    pub prior_observation_qpc_ticks: u64,
    pub selected_present_qpc_ticks: u64,
    pub present_sample_started_qpc_ticks: u64,
    pub present_sample_completed_qpc_ticks: u64,
    pub selected_present_receipt_sha256: String,
    pub client_extent: [u32; 2],
    pub prior_client_position: [i32; 2],
    pub current_client_position: [i32; 2],
    pub variant: RecordedInputVariant,
    pub authority_manifest_sha256: String,
    pub observation_receipt_sha256: String,
}

impl AuthenticatedRecordedInputVariant {
    pub fn computed_receipt_sha256(&self) -> Result<String, CatalogBuildError> {
        let variant = match self.variant {
            RecordedInputVariant::MouseKeyboard => 1u8,
            RecordedInputVariant::Keyboard => 2,
            RecordedInputVariant::Controller => 3,
        };
        let mut hasher = Sha256::new();
        hasher.update(b"hoi4.a729.recorded-input-variant-observation.v3\0");
        hasher.update(self.executable_sha256.as_bytes());
        hasher.update(self.process_id.to_le_bytes());
        hasher.update(self.process_start_nonce.to_le_bytes());
        hasher.update(self.hook_epoch_nonce.to_le_bytes());
        hasher.update(self.observation_sequence.to_le_bytes());
        hasher.update(self.prior_observation_qpc_ticks.to_le_bytes());
        hasher.update(self.selected_present_qpc_ticks.to_le_bytes());
        hasher.update(self.present_sample_started_qpc_ticks.to_le_bytes());
        hasher.update(self.present_sample_completed_qpc_ticks.to_le_bytes());
        hasher.update(self.selected_present_receipt_sha256.as_bytes());
        for value in self.client_extent {
            hasher.update(value.to_le_bytes());
        }
        for value in self
            .prior_client_position
            .into_iter()
            .chain(self.current_client_position)
        {
            hasher.update(value.to_le_bytes());
        }
        hasher.update([variant]);
        hasher.update(self.authority_manifest_sha256.as_bytes());
        Ok(format!("{:x}", hasher.finalize()))
    }

    pub fn validate(&self, capture: &RawRenderCatalogCapture) -> Result<(), CatalogBuildError> {
        if !RECORDED_INPUT_VARIANT_PRODUCTION_ENABLED
            || !RECORDED_INPUT_VARIANT_AUTHORITY_MANIFEST_SHA256
                .is_some_and(|expected| self.authority_manifest_sha256 == expected)
            || self.executable_sha256 != capture.process.executable_sha256
            || self.executable_sha256 != A729_EXECUTABLE_SHA256
            || self.process_id == 0
            || self.process_id != capture.process.process_id
            || self.process_start_nonce != capture.process.process_start_nonce
            || self.process_start_nonce != capture.launch_epoch.process_start_nonce
            || self.hook_epoch_nonce != capture.launch_epoch.hook_epoch_nonce
            || self.observation_sequence == 0
            || self.prior_observation_qpc_ticks == 0
            || self.selected_present_qpc_ticks <= self.prior_observation_qpc_ticks
            || self.present_sample_started_qpc_ticks < self.selected_present_qpc_ticks
            || self.present_sample_completed_qpc_ticks < self.present_sample_started_qpc_ticks
            || self.client_extent[0] == 0
            || self.client_extent[1] == 0
            || self.prior_client_position == self.current_client_position
            || self.prior_client_position[0] < 0
            || self.prior_client_position[1] < 0
            || self.current_client_position[0] < 0
            || self.current_client_position[1] < 0
            || self.prior_client_position[0] as u32 >= self.client_extent[0]
            || self.prior_client_position[1] as u32 >= self.client_extent[1]
            || self.current_client_position[0] as u32 >= self.client_extent[0]
            || self.current_client_position[1] as u32 >= self.client_extent[1]
        {
            return invalid(
                "recorded input variant lacks frozen same-launch observation authority",
            );
        }
        for (label, value) in [
            (
                "input-variant authority manifest",
                &self.authority_manifest_sha256,
            ),
            (
                "input-variant selected-Present receipt",
                &self.selected_present_receipt_sha256,
            ),
            (
                "input-variant observation receipt",
                &self.observation_receipt_sha256,
            ),
        ] {
            if value.len() != 64
                || !value
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            {
                return invalid(format!("{label} is not canonical lowercase SHA-256"));
            }
        }
        if self.computed_receipt_sha256()? != self.observation_receipt_sha256 {
            return invalid("recorded input variant observation receipt mismatch");
        }
        Ok(())
    }
}

pub fn authenticate_recorded_pointer_input(
    observation: RawRecordedPointerInputObservation,
) -> Result<AuthenticatedRecordedInputVariant, CatalogBuildError> {
    let inside = |position: [i32; 2], extent: [u32; 2]| {
        position[0] >= 0
            && position[1] >= 0
            && (position[0] as u32) < extent[0]
            && (position[1] as u32) < extent[1]
    };
    if observation.executable_sha256 != A729_EXECUTABLE_SHA256
        || observation.process_id == 0
        || observation.process_start_nonce == 0
        || observation.hook_epoch_nonce == 0
        || observation.observation_sequence == 0
        || observation.prior_observation_qpc_ticks == 0
        || observation.selected_present_qpc_ticks <= observation.prior_observation_qpc_ticks
        || observation.present_sample_started_qpc_ticks < observation.selected_present_qpc_ticks
        || observation.present_sample_completed_qpc_ticks
            < observation.present_sample_started_qpc_ticks
        || observation.client_extent[0] == 0
        || observation.client_extent[1] == 0
        || observation.prior_client_position == observation.current_client_position
        || !inside(observation.prior_client_position, observation.client_extent)
        || !inside(
            observation.current_client_position,
            observation.client_extent,
        )
        || observation.selected_present_receipt_sha256.len() != 64
        || !observation
            .selected_present_receipt_sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return invalid("raw recorded pointer observation is incomplete or unsupported");
    }
    let mut receipt = AuthenticatedRecordedInputVariant {
        executable_sha256: observation.executable_sha256,
        process_id: observation.process_id,
        process_start_nonce: observation.process_start_nonce,
        hook_epoch_nonce: observation.hook_epoch_nonce,
        observation_sequence: observation.observation_sequence,
        prior_observation_qpc_ticks: observation.prior_observation_qpc_ticks,
        selected_present_qpc_ticks: observation.selected_present_qpc_ticks,
        present_sample_started_qpc_ticks: observation.present_sample_started_qpc_ticks,
        present_sample_completed_qpc_ticks: observation.present_sample_completed_qpc_ticks,
        selected_present_receipt_sha256: observation.selected_present_receipt_sha256,
        client_extent: observation.client_extent,
        prior_client_position: observation.prior_client_position,
        current_client_position: observation.current_client_position,
        variant: RecordedInputVariant::MouseKeyboard,
        authority_manifest_sha256: RECORDED_INPUT_VARIANT_AUTHORITY_MANIFEST_SHA256
            .ok_or_else(|| CatalogBuildError("recorded input authority is disabled".into()))?
            .to_string(),
        observation_receipt_sha256: String::new(),
    };
    receipt.observation_receipt_sha256 = receipt.computed_receipt_sha256()?;
    Ok(receipt)
}

pub fn portable_render_authority_with_input(
    capture: &RawRenderCatalogCapture,
    effective_vfs_manifest_sha256: String,
    effect_catalog_sha256: String,
    entity_animation_catalog_sha256: String,
    recorded_input: Option<&AuthenticatedRecordedInputVariant>,
) -> Result<RenderAuthority, CatalogBuildError> {
    capture
        .validate_launch_binding()
        .map_err(|error| CatalogBuildError(format!("capture launch receipt: {error}")))?;
    for (label, value) in [
        ("effective VFS manifest", &effective_vfs_manifest_sha256),
        ("effect catalog", &effect_catalog_sha256),
        ("entity animation catalog", &entity_animation_catalog_sha256),
    ] {
        if value.len() != 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return invalid(format!("{label} SHA-256 is not canonical lowercase hex"));
        }
    }
    let launch = &capture.launch_epoch;
    if let Some(recorded_input) = recorded_input {
        recorded_input.validate(capture)?;
    }
    Ok(RenderAuthority {
        process_id: launch.process_id,
        process_start_nonce: launch.process_start_nonce,
        observer_installed_qpc: launch.observer_installed_qpc,
        d3d_device_created_qpc: launch.d3d_device_created_qpc,
        first_creation_sequence: launch.first_creation_sequence,
        hook_epoch_nonce: launch.hook_epoch_nonce,
        hook_set_receipt_sha256: launch.hook_set_receipt_sha256.clone(),
        capture_receipt_sha256: launch.capture_receipt_sha256.clone(),
        launch_epoch_sha256: compute_render_launch_epoch_sha256(
            launch.process_id,
            launch.process_start_nonce,
            launch.observer_installed_qpc,
            launch.d3d_device_created_qpc,
            launch.first_creation_sequence,
            launch.hook_epoch_nonce,
            &launch.hook_set_receipt_sha256,
            &launch.capture_receipt_sha256,
        ),
        effective_vfs_manifest_sha256,
        effect_catalog_sha256,
        entity_animation_catalog_sha256,
        recorded_input_variant: recorded_input.map(|receipt| receipt.variant),
        recorded_input_receipt_sha256: recorded_input
            .map(|receipt| receipt.observation_receipt_sha256.clone()),
        recorded_input_observation: recorded_input.map(|receipt| RecordedInputObservationReceipt {
            executable_sha256: receipt.executable_sha256.clone(),
            process_id: receipt.process_id,
            process_start_nonce: receipt.process_start_nonce,
            hook_epoch_nonce: receipt.hook_epoch_nonce,
            observation_sequence: receipt.observation_sequence,
            prior_observation_qpc_ticks: receipt.prior_observation_qpc_ticks,
            selected_present_qpc_ticks: receipt.selected_present_qpc_ticks,
            present_sample_started_qpc_ticks: receipt.present_sample_started_qpc_ticks,
            present_sample_completed_qpc_ticks: receipt.present_sample_completed_qpc_ticks,
            selected_present_receipt_sha256: receipt.selected_present_receipt_sha256.clone(),
            client_extent: receipt.client_extent,
            prior_client_position: receipt.prior_client_position,
            current_client_position: receipt.current_client_position,
            variant: receipt.variant,
            authority_manifest_sha256: receipt.authority_manifest_sha256.clone(),
            observation_receipt_sha256: receipt.observation_receipt_sha256.clone(),
        }),
        particle_runtime_mode: ParticleRuntimeMode::SubmittedPacketReplay,
    })
}

pub fn build_authenticated_static_programs<V: AdmittedVfsPayloads>(
    manifest: &ContentManifest,
    capture: &RawStaticProgramCapture,
    admitted_vfs: &V,
) -> Result<AuthenticatedStaticProgramSet, CatalogBuildError> {
    manifest
        .validate()
        .map_err(|error| CatalogBuildError(format!("content manifest: {error}")))?;
    if manifest.manifest_digest_sha256.is_none() {
        return invalid("content manifest is not finalized");
    }
    capture.validate_complete().map_err(CatalogBuildError)?;
    if capture.process.executable_sha256 != manifest.source_game.executable_sha256 {
        return invalid("static capture executable disagrees with source-game manifest");
    }
    let content = ManifestContentIndex::new(manifest)?;
    let mut builder = Builder::new(&capture.catalog, &content, admitted_vfs)?;
    let mut draws = index_unique(&capture.draws, "static draw", |row| row.capture_sequence)?;
    let producer_ordinals = capture
        .ordered_operations
        .iter()
        .enumerate()
        .map(|(ordinal, operation)| {
            Ok((
                operation.capture_sequence(),
                u32::try_from(ordinal)
                    .map_err(|_| CatalogBuildError("static operation ordinal overflow".into()))?,
            ))
        })
        .collect::<Result<BTreeMap<_, _>, CatalogBuildError>>()?;
    if producer_ordinals.len() != capture.ordered_operations.len() {
        return invalid("static capture repeats an operation capture sequence");
    }

    let mut programs: Vec<StaticMapPassProgram> = Vec::new();
    let mut ordered_execution = Vec::with_capacity(capture.ordered_operations.len());
    for (ordinal, operation) in capture.ordered_operations.iter().enumerate() {
        let operation_ordinal = u32::try_from(ordinal)
            .map_err(|_| CatalogBuildError("static operation ordinal overflow".into()))?;
        let (command, content_joins) = match operation {
            RawStaticOrderOperation::Draw {
                capture_sequence, ..
            } => {
                let draw = draws.remove(capture_sequence).ok_or_else(|| {
                    CatalogBuildError("static order references a missing draw".into())
                })?;
                let command = StaticMapCommand::Draw {
                    draw: builder.static_draw(draw, &producer_ordinals, operation_ordinal)?,
                };
                (command, draw.ordered_content.as_slice())
            }
            RawStaticOrderOperation::ClearColor {
                capture_sequence,
                target_view_ptr,
                target,
                rgba_f32_bits,
                ..
            } => (
                StaticMapCommand::ClearColor {
                    target: builder.static_output_target(
                        *target_view_ptr,
                        RawOutputViewKind::Color,
                        target,
                        *capture_sequence,
                        operation_ordinal,
                    )?,
                    rgba_f32_bits: *rgba_f32_bits,
                },
                &[][..],
            ),
            RawStaticOrderOperation::ClearDepthStencil {
                capture_sequence,
                target_view_ptr,
                target,
                clear_depth,
                clear_stencil,
                depth_f32_bits,
                stencil_u8,
                ..
            } => (
                StaticMapCommand::ClearDepthStencil {
                    target: builder.static_output_target(
                        *target_view_ptr,
                        RawOutputViewKind::DepthStencil,
                        target,
                        *capture_sequence,
                        operation_ordinal,
                    )?,
                    clear_depth: *clear_depth,
                    clear_stencil: *clear_stencil,
                    depth_f32_bits: *depth_f32_bits,
                    stencil_u8: *stencil_u8,
                },
                &[][..],
            ),
        };
        programs.push(StaticMapPassProgram {
            program_ordinal: operation_ordinal,
            pass_token: canonical_static_operation_token(operation_ordinal),
            ordered_content_keys: Vec::new(),
            ordered_operations: Vec::new(),
            payload_sha256: String::new(),
        });
        let program = programs.last_mut().expect("program was inserted");
        for join in content_joins {
            content.resolve_join(join)?;
            if !program.ordered_content_keys.contains(&join.content_key) {
                program.ordered_content_keys.push(join.content_key.clone());
            }
        }
        program.ordered_operations.push(StaticMapOperation {
            operation_ordinal,
            payload_sha256: String::new(),
            command,
        });
        ordered_execution.push(RenderExecutionRef::Static { operation_ordinal });
    }
    if !draws.is_empty() {
        return invalid("static capture contains draws absent from order");
    }
    for program in &mut programs {
        program
            .seal_payload_hashes()
            .map_err(|error| CatalogBuildError(format!("seal static program: {error}")))?;
    }
    let authenticated_texture_snapshots = builder
        .authenticated_texture_snapshots
        .values()
        .cloned()
        .collect();
    builder.finish_exact_closure()?;
    let result = AuthenticatedStaticProgramSet {
        ordered_programs: programs,
        ordered_execution,
        authenticated_texture_snapshots,
    };
    result.validate_canonical_partition()?;
    Ok(result)
}

fn particle_identity(packet: &RawParticlePacket) -> RawDrawIdentity {
    RawDrawIdentity::Particle {
        effect_instance_id: packet.effect_instance_id,
        registration_token: packet.definition.registration_token.clone(),
        definition_source_token: packet.definition.definition_source_token.clone(),
        builder_call_ordinal: packet.builder_call_ordinal,
        packet_ordinal: packet.packet_ordinal,
        draw_order: packet.draw_order,
        shader_program_ptr: packet.shader_program_ptr,
    }
}

fn entity_identity(entity: &RawEntitySubmission) -> RawDrawIdentity {
    RawDrawIdentity::Entity {
        submit_index: entity.submit_index,
        pass_ordinal: entity.pass_ordinal,
        entity_instance_id: entity.entity_instance_id,
        draw_order: entity.draw_order,
        entity_object_ptr: entity.entity_object_ptr,
        entity_definition_ptr: entity.entity_definition_ptr,
        renderer_ptr: entity.renderer_ptr,
        render_context_ptr: entity.render_context_ptr,
        mesh_draw_ptr: entity.mesh_draw_ptr,
        material_binding_ptr: entity.material_binding_ptr,
        shader_program_ptr: entity.shader_program_ptr,
    }
}

fn validate_state_identity(
    state: &RawD3dDrawState,
    family: RawDrawFamily,
    submit_index: u64,
    draw_order: u32,
) -> Result<(), CatalogBuildError> {
    if state.final_draw_sequence == 0
        || state.family != family
        || state.submit_index != submit_index
        || state.draw_order != draw_order
        || state.render_context_ptr == 0
        || state.device_context_ptr == 0
        || state.shader_program_ptr == 0
        || state.vertex_shader_ptr == 0
        || state.pixel_shader_ptr == 0
        || state.dynamic_shader_linkage_present
        || state.geometry_shader_ptr.is_some()
        || state.hull_shader_ptr.is_some()
        || state.domain_shader_ptr.is_some()
        || state.compute_shader_ptr.is_some()
        || !state.ordered_stream_output_buffer_ptrs.is_empty()
    {
        return invalid("final draw has incomplete or unsupported live state");
    }
    for pointer in [
        state.input_layout_ptr,
        state.blend_state_ptr,
        state.depth_stencil_state_ptr,
        state.rasterizer_state_ptr,
        state.index_buffer.as_ref().map(|value| value.buffer_ptr),
        state.depth_stencil_view_ptr,
    ] {
        if pointer == Some(0) {
            return invalid("final draw uses Some(0) instead of authenticated null");
        }
    }
    Ok(())
}

fn validate_entity_raw_join(
    entity: &RawEntitySubmission,
    state: &RawD3dDrawState,
) -> Result<(), CatalogBuildError> {
    if entity.submit_index == 0
        || entity.entity_instance_id == 0
        || entity.entity_object_ptr == 0
        || entity.entity_definition_ptr == 0
        || entity.renderer_ptr == 0
        || entity.render_context_ptr == 0
        || entity.mesh_draw_ptr == 0
        || entity.material_binding_ptr == 0
        || entity.shader_program_ptr == 0
        || entity.render_context_ptr != state.render_context_ptr
        || entity.shader_program_ptr != state.shader_program_ptr
        || !entity.additional_shader_stages_authenticated_null
        || !entity.stream_output_targets_authenticated_null
    {
        return invalid("entity raw join disagrees with final-draw state");
    }
    Ok(())
}

fn index_unique<'a, T, K: Ord>(
    rows: &'a [T],
    label: &str,
    key: impl Fn(&T) -> K,
) -> Result<BTreeMap<K, &'a T>, CatalogBuildError> {
    let mut out = BTreeMap::new();
    for row in rows {
        if out.insert(key(row), row).is_some() {
            return invalid(format!("capture repeats {label}"));
        }
    }
    Ok(out)
}

fn latest_creation<'a, T>(
    rows: &BTreeMap<(usize, u64), &'a T>,
    pointer: usize,
    draw_sequence: u64,
) -> Option<((usize, u64), &'a T)> {
    rows.range((pointer, 0)..=(pointer, draw_sequence))
        .next_back()
        .map(|(key, value)| (*key, *value))
}

struct ManifestContentIndex<'a> {
    by_path: BTreeMap<&'a str, &'a UsedContentKey>,
    by_key: BTreeMap<&'a str, &'a UsedContentKey>,
}

impl<'a> ManifestContentIndex<'a> {
    fn new(manifest: &'a ContentManifest) -> Result<Self, CatalogBuildError> {
        let mut by_path = BTreeMap::new();
        let mut by_key = BTreeMap::new();
        for required in &manifest.required_content {
            if by_path
                .insert(
                    required.content.normalized_game_relative_path.as_str(),
                    &required.content,
                )
                .is_some()
            {
                return invalid("manifest repeats an effective path");
            }
            if by_key
                .insert(required.content_key.as_str(), &required.content)
                .is_some()
            {
                return invalid("manifest repeats a content key");
            }
        }
        Ok(Self { by_path, by_key })
    }

    fn resolve(&self, raw: &RawVfsContentIdentity) -> Result<UsedContentKey, CatalogBuildError> {
        let path = normalize_game_relative_path(&raw.logical_path)
            .map_err(|error| CatalogBuildError(format!("observed VFS path: {error}")))?;
        let expected = self.by_path.get(path.as_str()).ok_or_else(|| {
            CatalogBuildError(format!("content is absent from finalized VFS: {path}"))
        })?;
        let layer_kind = parse_layer_kind(&raw.layer_kind)?;
        if expected.winning_layer_ordinal != raw.winning_layer_ordinal
            || expected.layer_kind != layer_kind
            || expected.layer_id != raw.layer_id
            || expected.source_sha256 != raw.source_sha256
            || expected.payload_sha256 != raw.payload_sha256
            || expected.payload_bytes != raw.payload_bytes
        {
            return invalid(format!(
                "observed VFS winner disagrees with manifest: {path}"
            ));
        }
        Ok((*expected).clone())
    }

    fn resolve_join(
        &self,
        raw: &RawStaticContentJoin,
    ) -> Result<UsedContentKey, CatalogBuildError> {
        let resolved = self.resolve(&raw.content)?;
        if self.by_key.get(raw.content_key.as_str()).copied() != Some(&resolved) {
            return invalid(format!(
                "static content key does not identify its observed VFS winner: {}",
                raw.content_key
            ));
        }
        Ok(resolved)
    }
}

fn parse_layer_kind(value: &str) -> Result<ContentLayerKind, CatalogBuildError> {
    match value {
        "base" => Ok(ContentLayerKind::Base),
        "integrated_dlc" => Ok(ContentLayerKind::IntegratedDlc),
        "dlc" => Ok(ContentLayerKind::Dlc),
        "mod" => Ok(ContentLayerKind::Mod),
        _ => invalid("raw VFS content has an unknown layer kind"),
    }
}

struct Builder<'a, V> {
    content: &'a ManifestContentIndex<'a>,
    admitted_vfs: &'a V,
    programs: BTreeMap<(usize, u64), &'a RawProgramIdentity>,
    shaders: BTreeMap<(usize, u64), &'a RawShaderCreation>,
    input_layouts: BTreeMap<(usize, u64), &'a RawInputLayoutCreation>,
    blends: BTreeMap<(usize, u64), &'a RawBlendStateCreation>,
    depths: BTreeMap<(usize, u64), &'a RawDepthStencilStateCreation>,
    rasters: BTreeMap<(usize, u64), &'a RawRasterizerStateCreation>,
    samplers: BTreeMap<(usize, u64), &'a RawSamplerStateCreation>,
    buffers: BTreeMap<(usize, u64), &'a RawBufferSnapshot>,
    textures: BTreeMap<(usize, u64), &'a RawTextureResourceSnapshot>,
    srvs: BTreeMap<(usize, u64), &'a RawShaderResourceViewSnapshot>,
    output_views: BTreeMap<(usize, u64), &'a RawRenderTargetViewSnapshot>,
    used_programs: BTreeSet<(usize, u64)>,
    used_shaders: BTreeSet<(usize, u64)>,
    used_input_layouts: BTreeSet<(usize, u64)>,
    used_blends: BTreeSet<(usize, u64)>,
    used_depths: BTreeSet<(usize, u64)>,
    used_rasters: BTreeSet<(usize, u64)>,
    used_samplers: BTreeSet<(usize, u64)>,
    used_buffers: BTreeSet<(usize, u64)>,
    used_textures: BTreeSet<(usize, u64)>,
    used_srvs: BTreeSet<(usize, u64)>,
    used_output_views: BTreeSet<(usize, u64)>,
    authenticated_texture_snapshots: BTreeMap<String, AuthenticatedTextureSnapshotPacket>,
}

impl<'a, V: AdmittedVfsPayloads> Builder<'a, V> {
    fn new(
        capture: &'a RawRenderCatalogCapture,
        content: &'a ManifestContentIndex<'a>,
        admitted_vfs: &'a V,
    ) -> Result<Self, CatalogBuildError> {
        Ok(Self {
            content,
            admitted_vfs,
            programs: index_unique(&capture.program_identities, "program identity", |v| {
                (v.shader_program_ptr, v.observation_sequence)
            })?,
            shaders: index_unique(&capture.shader_creations, "shader creation", |v| {
                (v.shader_ptr, v.creation_sequence)
            })?,
            input_layouts: index_unique(
                &capture.input_layout_creations,
                "input-layout creation",
                |v| (v.input_layout_ptr, v.creation_sequence),
            )?,
            blends: index_unique(
                &capture.blend_state_creations,
                "blend-state creation",
                |v| (v.blend_state_ptr, v.creation_sequence),
            )?,
            depths: index_unique(
                &capture.depth_stencil_state_creations,
                "depth-state creation",
                |v| (v.depth_stencil_state_ptr, v.creation_sequence),
            )?,
            rasters: index_unique(
                &capture.rasterizer_state_creations,
                "rasterizer creation",
                |v| (v.rasterizer_state_ptr, v.creation_sequence),
            )?,
            samplers: index_unique(&capture.sampler_state_creations, "sampler creation", |v| {
                (v.sampler_state_ptr, v.creation_sequence)
            })?,
            buffers: index_unique(&capture.buffer_snapshots, "buffer snapshot", |v| {
                (v.buffer_ptr, v.snapshot_sequence)
            })?,
            textures: index_unique(
                &capture.texture_resource_snapshots,
                "texture snapshot",
                |v| (v.resource_ptr, v.snapshot_sequence),
            )?,
            srvs: index_unique(
                &capture.shader_resource_view_snapshots,
                "SRV snapshot",
                |v| (v.shader_resource_view_ptr, v.snapshot_sequence),
            )?,
            output_views: index_unique(
                &capture.render_target_view_snapshots,
                "output-view snapshot",
                |v| (v.view_ptr, v.snapshot_sequence),
            )?,
            used_programs: BTreeSet::new(),
            used_shaders: BTreeSet::new(),
            used_input_layouts: BTreeSet::new(),
            used_blends: BTreeSet::new(),
            used_depths: BTreeSet::new(),
            used_rasters: BTreeSet::new(),
            used_samplers: BTreeSet::new(),
            used_buffers: BTreeSet::new(),
            used_textures: BTreeSet::new(),
            used_srvs: BTreeSet::new(),
            used_output_views: BTreeSet::new(),
            authenticated_texture_snapshots: BTreeMap::new(),
        })
    }

    fn particle_row(
        &mut self,
        packet: &RawParticlePacket,
        state: &RawD3dDrawState,
        semantic: &RawDrawSemantics,
    ) -> Result<ResolvedParticleCatalogRow, CatalogBuildError> {
        if semantic.observation_sequence != state.final_draw_sequence
            || packet.shader_program_ptr != state.shader_program_ptr
            || !packet.additional_shader_stages_authenticated_null
            || !packet.stream_output_targets_authenticated_null
        {
            return invalid("particle raw join disagrees with final draw");
        }
        let (
            definition,
            texture,
            authored_shader_token,
            engine_effect_token,
            subsystem_selector_u32,
            atlas_rows,
            atlas_columns,
            atlas_half_pixel_width_f32_bits,
            atlas_half_pixel_height_f32_bits,
        ) = match &semantic.content_identity {
            RawDrawContentIdentity::Particle {
                registration_token,
                definition_source_token,
                subsystem_selector_u32,
                definition_content,
                texture_content,
                authored_shader_token,
                engine_effect_token,
                atlas_columns_u32,
                atlas_rows_u32,
                atlas_half_pixel_width_f32_bits,
                atlas_half_pixel_height_f32_bits,
            } => {
                if registration_token != &packet.definition.registration_token
                    || definition_source_token != &packet.definition.definition_source_token
                    || *subsystem_selector_u32 != packet.subsystem_selector_u32
                    || packet.packet_ordinal != packet.subsystem_selector_u32
                    || packet.engine_effect_token != *engine_effect_token
                    || packet.atlas_columns_f32_bits != (*atlas_columns_u32 as f32).to_bits()
                    || packet.atlas_rows_f32_bits != (*atlas_rows_u32 as f32).to_bits()
                    || packet.atlas_half_pixel_width_f32_bits != *atlas_half_pixel_width_f32_bits
                    || packet.atlas_half_pixel_height_f32_bits != *atlas_half_pixel_height_f32_bits
                {
                    return invalid(
                        "particle selector/atlas authority disagrees with live receipt",
                    );
                }
                (
                    self.content.resolve(definition_content)?,
                    self.content.resolve(texture_content)?,
                    authored_shader_token.clone(),
                    engine_effect_token.clone(),
                    *subsystem_selector_u32,
                    *atlas_rows_u32,
                    *atlas_columns_u32,
                    *atlas_half_pixel_width_f32_bits,
                    *atlas_half_pixel_height_f32_bits,
                )
            }
            _ => return invalid("particle draw has entity content semantics"),
        };
        let shader_program = self.shader_program(state)?;
        let ordered_texture_bindings = self.textures(state, semantic)?;
        if shader_program.effect_token != engine_effect_token
            || !ordered_texture_bindings
                .iter()
                .any(|binding| binding.content == texture)
        {
            return invalid("particle live program/texture disagrees with subsystem authority");
        }
        let pipeline = self.pipeline(state, &semantic.ordered_sampler_semantics)?;
        let geometry = self.geometry(state, Some(packet))?;
        let ordered_constant_buffers =
            self.constant_buffers(state, &semantic.ordered_constant_buffer_semantics)?;
        let output_state = self.output_state(
            state,
            &semantic.ordered_color_targets,
            semantic.depth_target.as_ref(),
        )?;
        let content_identity = ParticleContentIdentity {
            definition_content: definition.clone(),
            subsystem_selector_u32,
            texture_content: texture,
            authored_shader_token,
            engine_effect_token,
            atlas_columns_u32: atlas_columns,
            atlas_rows_u32: atlas_rows,
            atlas_half_pixel_width_f32_bits,
            atlas_half_pixel_height_f32_bits,
        };
        let content_identity_sha256 = content_identity
            .computed_sha256()
            .map_err(CatalogBuildError)?;
        Ok(ResolvedParticleCatalogRow {
            registration_token: packet.definition.registration_token.clone(),
            definition_source_token: packet.definition.definition_source_token.clone(),
            resolved_definition_sha256: definition.payload_sha256.clone(),
            definition_content: definition,
            content_identity,
            content_identity_sha256,
            shader_program,
            ordered_texture_bindings,
            pipeline,
            geometry,
            ordered_constant_buffers,
            output_state,
            atlas_rows,
            atlas_columns,
        })
    }

    fn entity_row(
        &mut self,
        entity: &RawEntitySubmission,
        state: &RawD3dDrawState,
        semantic: &RawDrawSemantics,
    ) -> Result<ResolvedEntityCatalogRow, CatalogBuildError> {
        if semantic.observation_sequence != state.final_draw_sequence {
            return invalid("entity semantic observation is stale");
        }
        let content_identity = match &semantic.content_identity {
            RawDrawContentIdentity::Entity {
                resolved_entity_asset,
                mesh,
                entity_registration_token,
                entity_group_index,
                mesh_draw_list_index,
                entity_variant_u8,
                mesh_draw_variant_u8,
                pass_selector_i32,
                alternate_list_u8,
                bind_renderer_target_u8,
            } => {
                let native_selection = EntityNativeSelection {
                    entity_registration_token: entity_registration_token.clone(),
                    entity_group_index: *entity_group_index,
                    mesh_draw_list_index: *mesh_draw_list_index,
                    entity_variant_u8: *entity_variant_u8,
                    mesh_draw_variant_u8: *mesh_draw_variant_u8,
                    pass_selector_i32: *pass_selector_i32,
                    alternate_list_u8: *alternate_list_u8,
                    bind_renderer_target_u8: *bind_renderer_target_u8,
                };
                if entity.entity_registration_token.as_deref()
                    != Some(native_selection.entity_registration_token.as_str())
                    || entity.entity_group_index != native_selection.entity_group_index
                    || entity.mesh_draw_list_index != native_selection.mesh_draw_list_index
                    || entity.entity_variant_u8 != native_selection.entity_variant_u8
                    || entity.mesh_draw_variant_u8 != native_selection.mesh_draw_variant_u8
                    || entity.pass_selector_i32 != native_selection.pass_selector_i32
                    || entity.alternate_list_u8 != native_selection.alternate_list_u8
                    || entity.bind_renderer_target_u8 != native_selection.bind_renderer_target_u8
                    || entity.state_flags != native_selection.packed_state_flags()
                {
                    return invalid(
                        "entity selector-native authority disagrees with the live draw",
                    );
                }
                let native_selection_sha256 = native_selection
                    .computed_sha256()
                    .map_err(CatalogBuildError)?;
                EntityContentIdentity {
                    resolved_entity_asset: self.content.resolve(resolved_entity_asset)?,
                    mesh: self.content.resolve(mesh)?,
                    native_selection,
                    native_selection_sha256,
                }
            }
            _ => return invalid("entity draw has particle content semantics"),
        };
        Ok(ResolvedEntityCatalogRow {
            resolved_entity_sha256: content_identity
                .resolved_entity_asset
                .payload_sha256
                .clone(),
            content_identity,
            shader_program: self.shader_program(state)?,
            ordered_texture_bindings: self.textures(state, semantic)?,
            pipeline: self.pipeline(state, &semantic.ordered_sampler_semantics)?,
            geometry: self.geometry(state, None)?,
            ordered_constant_buffers: self
                .constant_buffers(state, &semantic.ordered_constant_buffer_semantics)?,
            output_state: self.output_state(
                state,
                &semantic.ordered_color_targets,
                semantic.depth_target.as_ref(),
            )?,
        })
    }

    fn shader_program(
        &mut self,
        state: &RawD3dDrawState,
    ) -> Result<ShaderProgram, CatalogBuildError> {
        let program_key = (state.shader_program_ptr, state.final_draw_sequence);
        let program = self
            .programs
            .get(&program_key)
            .copied()
            .ok_or_else(|| CatalogBuildError("draw has no program identity snapshot".into()))?;
        self.used_programs.insert(program_key);
        if state.shader_program_ptr == 0
            || program.observation_sequence != state.final_draw_sequence
        {
            return invalid("program identity is stale or null");
        }
        let vertex = self.compiled_shader(
            state.vertex_shader_ptr,
            RawShaderStage::Vertex,
            state.final_draw_sequence,
        )?;
        let pixel = self.compiled_shader(
            state.pixel_shader_ptr,
            RawShaderStage::Pixel,
            state.final_draw_sequence,
        )?;
        let ordered_defines = program
            .ordered_defines
            .iter()
            .map(|define| {
                Ok(ShaderDefine {
                    name: define.name.clone(),
                    value: define.value.clone(),
                    scope: match define.scope.as_str() {
                        "engine_global" => ShaderDefineScope::EngineGlobal,
                        "effect" => ShaderDefineScope::Effect,
                        "entry_point" => ShaderDefineScope::EntryPoint,
                        _ => return invalid("shader define has unknown scope"),
                    },
                })
            })
            .collect::<Result<Vec<_>, CatalogBuildError>>()?;
        Ok(ShaderProgram {
            source: self.content.resolve(&program.source)?,
            producer_registration_root_rva: Some(program.registration_root_rva),
            effect_token: program.effect_token.clone(),
            vertex_entry_token: program.vertex_entry_token.clone(),
            pixel_entry_token: program.pixel_entry_token.clone(),
            ordered_defines,
            compiled_vertex_shader: vertex,
            compiled_pixel_shader: pixel,
            geometry_shader: None,
            hull_shader: None,
            domain_shader: None,
            compute_shader: None,
        })
    }

    fn compiled_shader(
        &mut self,
        pointer: usize,
        stage: RawShaderStage,
        draw_sequence: u64,
    ) -> Result<CompiledShaderPayload, CatalogBuildError> {
        let (creation_key, shader) = latest_creation(&self.shaders, pointer, draw_sequence)
            .ok_or_else(|| {
                CatalogBuildError("bound shader lacks an immutable creation record".into())
            })?;
        self.used_shaders.insert(creation_key);
        if pointer == 0
            || shader.stage != stage
            || shader.creation_sequence == 0
            || shader.creation_sequence > draw_sequence
        {
            return invalid("shader creation record is stale, future, null, or wrong-stage");
        }
        validate_payload_hash(&shader.bytecode)?;
        match &shader.bytecode.source {
            RawPayloadSource::RuntimeGenerated { reason } if !reason.trim().is_empty() => {}
            _ => return invalid("compiled shader bytecode lacks runtime-generated provenance"),
        }
        Ok(CompiledShaderPayload {
            compiler_identity: shader.compiler_identity.clone(),
            target_profile: shader.target_profile.clone(),
            compiler_flags_u64: shader.compiler_flags_u64,
            bytecode: shader.bytecode.captured_bytes.clone(),
            sha256: shader.bytecode.captured_sha256.clone(),
        })
    }

    fn textures(
        &mut self,
        state: &RawD3dDrawState,
        semantic: &RawDrawSemantics,
    ) -> Result<Vec<TextureBinding>, CatalogBuildError> {
        if state.ordered_texture_objects.len() != semantic.ordered_texture_semantics.len() {
            return invalid("texture object/semantic cardinality mismatch");
        }
        state
            .ordered_texture_objects
            .iter()
            .zip(&semantic.ordered_texture_semantics)
            .map(|(object, meaning)| {
                if object.stage != meaning.stage
                    || object.slot != meaning.slot
                    || object.shader_resource_view_ptr == 0
                    || object.resource_ptr == 0
                {
                    return invalid("texture object order disagrees with semantic bindings");
                }
                let srv_key = (object.shader_resource_view_ptr, state.final_draw_sequence);
                let srv =
                    self.srvs.get(&srv_key).copied().ok_or_else(|| {
                        CatalogBuildError("bound SRV lacks an exact snapshot".into())
                    })?;
                self.used_srvs.insert(srv_key);
                let texture_key = (object.resource_ptr, state.final_draw_sequence);
                let resource = self.textures.get(&texture_key).copied().ok_or_else(|| {
                    CatalogBuildError("bound texture lacks an exact snapshot".into())
                })?;
                self.used_textures.insert(texture_key);
                if srv.snapshot_sequence != state.final_draw_sequence
                    || resource.snapshot_sequence != state.final_draw_sequence
                    || srv.resource_ptr != object.resource_ptr
                    || srv.desc.flags_u32 != 0
                {
                    return invalid("texture/SRV snapshot is stale or pointer-mismatched");
                }
                let content_key = self.content.resolve(resource.content.as_ref().ok_or_else(|| {
                    CatalogBuildError("VFS-backed texture has no content identity".into())
                })?)?;
                let ordered_subresources = resource
                    .ordered_subresources
                    .iter()
                    .enumerate()
                    .map(|(ordinal, sub)| {
                        if sub.subresource_index as usize != ordinal {
                            return invalid("texture subresources are not contiguous");
                        }
                        match &sub.payload.source {
                            RawPayloadSource::VfsRange {
                                content: range_content,
                                ..
                            } if self.content.resolve(range_content)? == content_key => {}
                            _ => {
                                return invalid(
                                    "VFS-backed texture subresource is not an exact range of that admitted asset",
                                )
                            }
                        }
                        Ok(TextureSubresourcePayload {
                            subresource_index: sub.subresource_index,
                            mip_level: sub.mip_level,
                            array_slice: sub.array_slice,
                            row_pitch: sub.row_pitch,
                            slice_pitch: sub.slice_pitch,
                            source: self.buffer_source(&sub.payload, None)?,
                        })
                    })
                    .collect::<Result<Vec<_>, CatalogBuildError>>()?;
                Ok(TextureBinding {
                    stage: shader_stage(object.stage),
                    slot: object.slot,
                    semantic_token: meaning.semantic_token.clone(),
                    content: content_key,
                    resource: TextureResourceDesc {
                        dimension: texture_dimension(
                            resource.dimension_u32,
                            resource.misc_flags_u32,
                        )?,
                        width: resource.width,
                        height: resource.height,
                        depth: resource.depth,
                        mip_levels: resource.mip_levels,
                        array_size: resource.array_size,
                        dxgi_format_u32: resource.dxgi_format_u32,
                        sample_count: resource.sample_count,
                        sample_quality: resource.sample_quality,
                        usage_u32: resource.usage_u32,
                        bind_flags_u32: resource.bind_flags_u32,
                        cpu_access_flags_u32: resource.cpu_access_flags_u32,
                        misc_flags_u32: resource.misc_flags_u32,
                        ordered_subresources,
                    },
                    view: shader_resource_view(&srv.desc)?,
                })
            })
            .collect()
    }

    fn pipeline(
        &mut self,
        state: &RawD3dDrawState,
        sampler_semantics: &[RawSamplerSemantic],
    ) -> Result<PipelineDesc, CatalogBuildError> {
        let input_layout = match state.input_layout_ptr {
            Some(pointer) => {
                let (creation_key, row) =
                    latest_creation(&self.input_layouts, pointer, state.final_draw_sequence)
                        .ok_or_else(|| {
                            CatalogBuildError("input layout lacks creation record".into())
                        })?;
                self.used_input_layouts.insert(creation_key);
                if pointer == 0
                    || row.creation_sequence == 0
                    || row.creation_sequence > state.final_draw_sequence
                {
                    return invalid("input-layout creation is invalid or future");
                }
                row.ordered_elements
                    .iter()
                    .map(|element| {
                        if element.input_slot_class_u32 > 1 {
                            return invalid("input element has unknown slot class");
                        }
                        Ok(InputElementDesc {
                            semantic_token: element.semantic_token.clone(),
                            semantic_index: element.semantic_index,
                            dxgi_format_u32: element.dxgi_format_u32,
                            input_slot: element.input_slot,
                            aligned_byte_offset: element.aligned_byte_offset,
                            per_instance: element.input_slot_class_u32 == 1,
                            instance_step_rate: element.instance_step_rate,
                        })
                    })
                    .collect::<Result<Vec<_>, CatalogBuildError>>()?
            }
            None => Vec::new(),
        };
        let blend = self.blend(state)?;
        let depth = self.depth(state)?;
        let rasterizer = self.rasterizer(state)?;
        let samplers = self.sampler_descs(state, sampler_semantics)?;
        let index_format = match &state.index_buffer {
            Some(index) => index_format(index.dxgi_format_u32)?,
            None => IndexFormat::None,
        };
        Ok(PipelineDesc {
            topology: topology(state.primitive_topology_u32)?,
            blend,
            depth,
            rasterizer,
            samplers,
            input_layout,
            index_format,
            static_indices_u16: Vec::new(),
        })
    }

    fn blend(&mut self, state: &RawD3dDrawState) -> Result<BlendDesc, CatalogBuildError> {
        let owned_default;
        let raw = match state.blend_state_ptr {
            Some(pointer) => {
                let (creation_key, row) =
                    latest_creation(&self.blends, pointer, state.final_draw_sequence).ok_or_else(
                        || CatalogBuildError("blend state lacks creation record".into()),
                    )?;
                self.used_blends.insert(creation_key);
                if row.creation_sequence == 0 || row.creation_sequence > state.final_draw_sequence {
                    return invalid("blend creation is invalid or future");
                }
                row
            }
            None => {
                owned_default = default_blend();
                &owned_default
            }
        };
        if raw.ordered_targets.len() != 8 {
            return invalid("blend state does not contain eight D3D11 targets");
        }
        let independent = raw_bool(raw.independent_blend_enable_u32, "independent blend")?;
        let target0 = render_target_blend(&raw.ordered_targets[0])?;
        let additional_targets = if independent {
            raw.ordered_targets[1..]
                .iter()
                .map(render_target_blend)
                .collect::<Result<Vec<_>, _>>()?
        } else {
            Vec::new()
        };
        Ok(BlendDesc {
            alpha_to_coverage_enabled: raw_bool(
                raw.alpha_to_coverage_enable_u32,
                "alpha-to-coverage",
            )?,
            independent_blend_enabled: independent,
            enabled: target0.enabled,
            source_color: target0.source_color,
            destination_color: target0.destination_color,
            color_operation: target0.color_operation,
            source_alpha: target0.source_alpha,
            destination_alpha: target0.destination_alpha,
            alpha_operation: target0.alpha_operation,
            write_mask_rgba_u8: target0.write_mask_rgba_u8,
            additional_targets,
            blend_factor_rgba_f32_bits: state.blend_factor_rgba_f32_bits,
            sample_mask_u32: state.sample_mask_u32,
        })
    }

    fn depth(&mut self, state: &RawD3dDrawState) -> Result<DepthDesc, CatalogBuildError> {
        let owned_default;
        let raw = match state.depth_stencil_state_ptr {
            Some(pointer) => {
                let (creation_key, row) =
                    latest_creation(&self.depths, pointer, state.final_draw_sequence).ok_or_else(
                        || CatalogBuildError("depth state lacks creation record".into()),
                    )?;
                self.used_depths.insert(creation_key);
                if row.creation_sequence == 0 || row.creation_sequence > state.final_draw_sequence {
                    return invalid("depth creation is invalid or future");
                }
                row
            }
            None => {
                owned_default = default_depth();
                &owned_default
            }
        };
        Ok(DepthDesc {
            enabled: raw_bool(raw.depth_enable_u32, "depth enable")?,
            write_enabled: match raw.depth_write_mask_u32 {
                0 => false,
                1 => true,
                _ => return invalid("unknown depth write mask"),
            },
            comparison: comparison(raw.depth_comparison_u32)?,
            stencil_enabled: raw_bool(raw.stencil_enable_u32, "stencil enable")?,
            stencil_read_mask_u8: raw.stencil_read_mask_u8,
            stencil_write_mask_u8: raw.stencil_write_mask_u8,
            front_face: stencil_face(&raw.front_face)?,
            back_face: stencil_face(&raw.back_face)?,
            stencil_reference_u32: state.stencil_reference_u32,
        })
    }

    fn rasterizer(&mut self, state: &RawD3dDrawState) -> Result<RasterizerDesc, CatalogBuildError> {
        let owned_default;
        let raw = match state.rasterizer_state_ptr {
            Some(pointer) => {
                let (creation_key, row) =
                    latest_creation(&self.rasters, pointer, state.final_draw_sequence).ok_or_else(
                        || CatalogBuildError("rasterizer lacks creation record".into()),
                    )?;
                self.used_rasters.insert(creation_key);
                if row.creation_sequence == 0 || row.creation_sequence > state.final_draw_sequence {
                    return invalid("rasterizer creation is invalid or future");
                }
                row
            }
            None => {
                owned_default = default_rasterizer();
                &owned_default
            }
        };
        Ok(RasterizerDesc {
            fill: match raw.fill_mode_u32 {
                2 => FillMode::Wireframe,
                3 => FillMode::Solid,
                _ => return invalid("unknown fill mode"),
            },
            cull: match raw.cull_mode_u32 {
                1 => CullMode::None,
                2 => CullMode::Front,
                3 => CullMode::Back,
                _ => return invalid("unknown cull mode"),
            },
            front_counter_clockwise: raw_bool(raw.front_counter_clockwise_u32, "front winding")?,
            depth_clip_enabled: raw_bool(raw.depth_clip_enable_u32, "depth clip")?,
            scissor_enabled: raw_bool(raw.scissor_enable_u32, "scissor")?,
            depth_bias_i32: raw.depth_bias_i32,
            depth_bias_clamp_f32_bits: raw.depth_bias_clamp_f32_bits,
            slope_scaled_depth_bias_f32_bits: raw.slope_scaled_depth_bias_f32_bits,
            multisample_enabled: raw_bool(raw.multisample_enable_u32, "multisample")?,
            antialiased_line_enabled: raw_bool(
                raw.antialiased_line_enable_u32,
                "antialiased line",
            )?,
        })
    }

    fn sampler_descs(
        &mut self,
        state: &RawD3dDrawState,
        sampler_semantics: &[RawSamplerSemantic],
    ) -> Result<Vec<SamplerDesc>, CatalogBuildError> {
        if state.ordered_sampler_objects.len() != sampler_semantics.len() {
            return invalid("sampler object/semantic cardinality mismatch");
        }
        state
            .ordered_sampler_objects
            .iter()
            .zip(sampler_semantics)
            .map(|(object, meaning)| {
                if object.stage != meaning.stage
                    || object.slot != meaning.slot
                    || object.object_ptr == 0
                {
                    return invalid("sampler object order disagrees with semantics");
                }
                let (creation_key, raw) =
                    latest_creation(&self.samplers, object.object_ptr, state.final_draw_sequence)
                        .ok_or_else(|| CatalogBuildError("sampler lacks creation record".into()))?;
                self.used_samplers.insert(creation_key);
                if raw.creation_sequence == 0 || raw.creation_sequence > state.final_draw_sequence {
                    return invalid("sampler creation is invalid or future");
                }
                let (min_filter, mag_filter, mip_filter, compared) = decode_filter(raw.filter_u32)?;
                Ok(SamplerDesc {
                    stage: shader_stage(object.stage),
                    slot: object.slot,
                    semantic_token: meaning.semantic_token.clone(),
                    min_filter,
                    mag_filter,
                    mip_filter,
                    address_u: address_mode(raw.address_u_u32)?,
                    address_v: address_mode(raw.address_v_u32)?,
                    address_w: address_mode(raw.address_w_u32)?,
                    comparison: if compared {
                        Some(comparison(raw.comparison_u32)?)
                    } else {
                        None
                    },
                    mip_lod_bias_f32_bits: raw.mip_lod_bias_f32_bits,
                    max_anisotropy: raw.max_anisotropy,
                    border_color_rgba_f32_bits: raw.border_color_rgba_f32_bits,
                    min_lod_f32_bits: raw.min_lod_f32_bits,
                    max_lod_f32_bits: raw.max_lod_f32_bits,
                })
            })
            .collect()
    }

    fn geometry(
        &mut self,
        state: &RawD3dDrawState,
        packet: Option<&RawParticlePacket>,
    ) -> Result<GeometryBinding, CatalogBuildError> {
        let ordered_vertex_buffers = state
            .ordered_vertex_buffer_objects
            .iter()
            .map(|binding| {
                if binding.buffer_ptr == 0 || binding.stride_bytes == 0 {
                    return invalid("vertex buffer join is null or zero-stride");
                }
                let snapshot_key = (binding.buffer_ptr, state.final_draw_sequence);
                let snapshot = self.buffers.get(&snapshot_key).copied().ok_or_else(|| {
                    CatalogBuildError("vertex buffer lacks draw-time snapshot".into())
                })?;
                self.used_buffers.insert(snapshot_key);
                self.validate_buffer_snapshot(snapshot, state.final_draw_sequence)?;
                Ok(VertexBufferBinding {
                    input_slot: binding.input_slot,
                    stride_bytes: binding.stride_bytes,
                    byte_offset: binding.byte_offset,
                    source: self.buffer_source(
                        &snapshot.payload,
                        packet.map(|p| p.ordered_gpu_instance_records.as_slice()),
                    )?,
                })
            })
            .collect::<Result<Vec<_>, CatalogBuildError>>()?;
        let index_buffer = state
            .index_buffer
            .as_ref()
            .map(|binding| {
                let snapshot_key = (binding.buffer_ptr, state.final_draw_sequence);
                let snapshot = self.buffers.get(&snapshot_key).copied().ok_or_else(|| {
                    CatalogBuildError("index buffer lacks draw-time snapshot".into())
                })?;
                self.used_buffers.insert(snapshot_key);
                self.validate_buffer_snapshot(snapshot, state.final_draw_sequence)?;
                Ok::<IndexBufferBinding, CatalogBuildError>(IndexBufferBinding {
                    format: index_format(binding.dxgi_format_u32)?,
                    byte_offset: binding.byte_offset,
                    source: self.buffer_source(&snapshot.payload, None)?,
                })
            })
            .transpose()?;
        Ok(GeometryBinding {
            ordered_vertex_buffers,
            index_buffer,
            stream_output_targets: Vec::new(),
            draw: draw_command(state.draw_call),
        })
    }

    fn constant_buffers(
        &mut self,
        state: &RawD3dDrawState,
        constant_buffer_semantics: &[RawConstantBufferSemantic],
    ) -> Result<Vec<ConstantBufferBinding>, CatalogBuildError> {
        if state.ordered_constant_buffer_objects.len() != constant_buffer_semantics.len() {
            return invalid("constant-buffer object/semantic cardinality mismatch");
        }
        state
            .ordered_constant_buffer_objects
            .iter()
            .zip(constant_buffer_semantics)
            .map(|(object, meaning)| {
                if object.object_ptr == 0
                    || object.slot != meaning.slot
                    || stage_bit(object.stage) != meaning.stage_mask_u8
                {
                    return invalid("constant-buffer object order disagrees with semantics");
                }
                let snapshot_key = (object.object_ptr, state.final_draw_sequence);
                let snapshot = self.buffers.get(&snapshot_key).copied().ok_or_else(|| {
                    CatalogBuildError("constant buffer lacks draw-time snapshot".into())
                })?;
                self.used_buffers.insert(snapshot_key);
                self.validate_buffer_snapshot(snapshot, state.final_draw_sequence)?;
                validate_payload_for_auth(
                    &snapshot.payload,
                    self.content,
                    self.admitted_vfs,
                    None,
                )?;
                if matches!(
                    snapshot.payload.source,
                    RawPayloadSource::ParticleInstanceRecords
                ) {
                    return invalid("constant buffer aliases particle instance records");
                }
                let start = meaning.byte_offset as usize;
                let end = start
                    .checked_add(meaning.byte_count as usize)
                    .ok_or_else(|| CatalogBuildError("constant-buffer range overflow".into()))?;
                let bytes = snapshot
                    .payload
                    .captured_bytes
                    .get(start..end)
                    .ok_or_else(|| {
                        CatalogBuildError("constant-buffer range is out of bounds".into())
                    })?
                    .to_vec();
                Ok(ConstantBufferBinding {
                    stage_mask_u8: meaning.stage_mask_u8,
                    slot: meaning.slot,
                    byte_offset: meaning.byte_offset,
                    byte_count: meaning.byte_count,
                    sha256: hex_sha256(&bytes),
                    raw_bytes: bytes,
                    semantic_layout: meaning.semantic_layout.clone(),
                })
            })
            .collect()
    }

    fn validate_buffer_snapshot(
        &self,
        snapshot: &RawBufferSnapshot,
        draw_sequence: u64,
    ) -> Result<(), CatalogBuildError> {
        if snapshot.buffer_ptr == 0
            || snapshot.snapshot_sequence != draw_sequence
            || snapshot.byte_width as usize != snapshot.payload.captured_bytes.len()
        {
            return invalid("buffer snapshot is stale, null, or byte-width mismatched");
        }
        Ok(())
    }

    fn buffer_source(
        &self,
        payload: &RawPayloadSnapshot,
        particle_records: Option<&[u8]>,
    ) -> Result<BufferSource, CatalogBuildError> {
        validate_payload_for_auth(payload, self.content, self.admitted_vfs, particle_records)?;
        match &payload.source {
            RawPayloadSource::VfsRange {
                content,
                source_byte_offset,
                byte_count,
                range_sha256,
            } => Ok(BufferSource::UsedContentRange {
                content: self.content.resolve(content)?,
                source_byte_offset: *source_byte_offset,
                byte_count: *byte_count,
                range_sha256: range_sha256.clone(),
            }),
            RawPayloadSource::RuntimeGenerated { reason } => {
                if reason.trim().is_empty() {
                    return invalid("runtime-generated bytes have no reason");
                }
                Ok(BufferSource::Inline {
                    bytes: payload.captured_bytes.clone(),
                    sha256: payload.captured_sha256.clone(),
                })
            }
            RawPayloadSource::ParticleInstanceRecords => Ok(BufferSource::ParticleInstanceRecords),
        }
    }

    fn output_state(
        &mut self,
        state: &RawD3dDrawState,
        ordered_color_target_semantics: &[RawOutputTargetSemantic],
        depth_target_semantic: Option<&RawOutputTargetSemantic>,
    ) -> Result<RenderOutputState, CatalogBuildError> {
        if state.ordered_color_target_view_ptrs.len() != ordered_color_target_semantics.len()
            || state.depth_stencil_view_ptr.is_some() != depth_target_semantic.is_some()
            || state.ordered_viewports_f32_bits.len() != 1
            || state.ordered_scissor_rects_i32.len() > 1
        {
            return invalid("output target/viewport/scissor cardinality is not portable-exact");
        }
        let ordered_color_targets = state
            .ordered_color_target_view_ptrs
            .iter()
            .zip(ordered_color_target_semantics)
            .map(|(pointer, meaning)| {
                self.output_target(
                    *pointer,
                    RawOutputViewKind::Color,
                    meaning,
                    state.final_draw_sequence,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let depth_target = match (state.depth_stencil_view_ptr, depth_target_semantic) {
            (Some(pointer), Some(meaning)) => Some(self.output_target(
                pointer,
                RawOutputViewKind::DepthStencil,
                meaning,
                state.final_draw_sequence,
            )?),
            (None, None) => None,
            _ => return invalid("depth target semantic mismatch"),
        };
        let viewport = state.ordered_viewports_f32_bits[0];
        let scissor = state
            .ordered_scissor_rects_i32
            .first()
            .map(|value| ScissorDesc {
                left_i32: value[0],
                top_i32: value[1],
                right_i32: value[2],
                bottom_i32: value[3],
            });
        Ok(RenderOutputState {
            ordered_color_targets,
            depth_target,
            viewport: ViewportDesc {
                top_left_x_f32_bits: viewport[0],
                top_left_y_f32_bits: viewport[1],
                width_f32_bits: viewport[2],
                height_f32_bits: viewport[3],
                min_depth_f32_bits: viewport[4],
                max_depth_f32_bits: viewport[5],
            },
            scissor,
        })
    }

    fn static_output_state(
        &mut self,
        state: &RawD3dDrawState,
        ordered_color_targets: &[RawOutputTargetSemantic],
        depth_target: Option<&RawOutputTargetSemantic>,
        operation_ordinal: u32,
    ) -> Result<RenderOutputState, CatalogBuildError> {
        let canonical_color_targets = ordered_color_targets
            .iter()
            .map(|target| canonical_raw_output_semantic(target, operation_ordinal))
            .collect::<Vec<_>>();
        let canonical_depth_target =
            depth_target.map(|target| canonical_raw_output_semantic(target, operation_ordinal));
        let mut output = self.output_state(
            state,
            &canonical_color_targets,
            canonical_depth_target.as_ref(),
        )?;
        for target in &mut output.ordered_color_targets {
            target.prepared_target =
                canonicalize_prepared_target(&target.prepared_target, operation_ordinal);
        }
        if let Some(target) = &mut output.depth_target {
            target.prepared_target =
                canonicalize_prepared_target(&target.prepared_target, operation_ordinal);
        }
        Ok(output)
    }

    fn static_output_target(
        &mut self,
        pointer: usize,
        kind: RawOutputViewKind,
        meaning: &RawOutputTargetSemantic,
        draw_sequence: u64,
        operation_ordinal: u32,
    ) -> Result<RenderTargetDesc, CatalogBuildError> {
        let canonical_meaning = canonical_raw_output_semantic(meaning, operation_ordinal);
        let mut target = self.output_target(pointer, kind, &canonical_meaning, draw_sequence)?;
        target.prepared_target =
            canonicalize_prepared_target(&target.prepared_target, operation_ordinal);
        Ok(target)
    }

    fn output_target(
        &mut self,
        pointer: usize,
        kind: RawOutputViewKind,
        meaning: &RawOutputTargetSemantic,
        draw_sequence: u64,
    ) -> Result<RenderTargetDesc, CatalogBuildError> {
        if pointer == 0 || meaning.view_ptr != pointer {
            return invalid("output view pointer disagrees with semantic identity");
        }
        let snapshot_key = (pointer, draw_sequence);
        let raw = self
            .output_views
            .get(&snapshot_key)
            .copied()
            .ok_or_else(|| {
                CatalogBuildError("output view lacks exact descriptor/resource snapshot".into())
            })?;
        self.used_output_views.insert(snapshot_key);
        if raw.snapshot_sequence != draw_sequence
            || raw.view_kind != kind
            || raw.resource_ptr == 0
            || raw.desc.flags_u32 != 0
        {
            return invalid(
                "output-view snapshot is stale, wrong-kind, null, or unsupported flags",
            );
        }
        let mip_level = output_mip_level(&raw.desc, kind)?;
        let prepared_target = prepared_target(
            &meaning.prepared_target,
            meaning.resource_token.as_deref(),
            meaning.producer_pass_token.as_deref(),
        )?;
        Ok(RenderTargetDesc {
            prepared_target,
            width: mip_extent(raw.resource_width, mip_level),
            height: mip_extent(raw.resource_height, mip_level),
            dxgi_format_u32: raw.desc.dxgi_format_u32,
            mip_level,
            sample_count: raw.resource_sample_count,
            sample_quality: raw.resource_sample_quality,
            persistent_content: meaning
                .persistent_content
                .as_ref()
                .map(|value| self.content.resolve(value))
                .transpose()?,
            exact_resource: None,
        })
    }

    fn static_draw(
        &mut self,
        raw: &RawStaticD3dDrawState,
        producer_ordinals: &BTreeMap<u64, u32>,
        operation_ordinal: u32,
    ) -> Result<StaticMapDraw, CatalogBuildError> {
        let state = static_state_adapter(raw)?;
        let mut shader_program = self.shader_program(&state)?;
        shader_program.geometry_shader = self.additional_shader_stage(
            raw.geometry_shader_ptr,
            raw.geometry_entry_token.as_deref(),
            RawShaderStage::Geometry,
            raw.capture_sequence,
        )?;
        shader_program.hull_shader = self.additional_shader_stage(
            raw.hull_shader_ptr,
            raw.hull_entry_token.as_deref(),
            RawShaderStage::Hull,
            raw.capture_sequence,
        )?;
        shader_program.domain_shader = self.additional_shader_stage(
            raw.domain_shader_ptr,
            raw.domain_entry_token.as_deref(),
            RawShaderStage::Domain,
            raw.capture_sequence,
        )?;
        shader_program.compute_shader = self.additional_shader_stage(
            raw.compute_shader_ptr,
            raw.compute_entry_token.as_deref(),
            RawShaderStage::Compute,
            raw.capture_sequence,
        )?;
        let mut geometry = self.geometry(&state, None)?;
        geometry.stream_output_targets = raw
            .ordered_stream_output_buffers
            .iter()
            .map(|binding| {
                let key = (binding.buffer_ptr, raw.capture_sequence);
                let snapshot = self.buffers.get(&key).copied().ok_or_else(|| {
                    CatalogBuildError("stream-output buffer lacks draw-time snapshot".into())
                })?;
                self.used_buffers.insert(key);
                self.validate_buffer_snapshot(snapshot, raw.capture_sequence)?;
                Ok(StreamOutputTargetBinding {
                    slot: binding.slot,
                    buffer_identity: snapshot.payload.captured_sha256.clone(),
                    buffer_byte_width: snapshot.byte_width,
                    byte_offset: binding.byte_offset,
                })
            })
            .collect::<Result<Vec<_>, CatalogBuildError>>()?;
        Ok(StaticMapDraw {
            draw_token: canonical_static_draw_token(operation_ordinal),
            shader_program,
            ordered_shader_resources: self.static_resources(raw, producer_ordinals)?,
            pipeline: self.pipeline(&state, &raw.ordered_sampler_semantics)?,
            geometry: StaticGeometrySource::Exact { geometry },
            ordered_constant_buffers: self
                .constant_buffers(&state, &raw.ordered_constant_buffer_semantics)?,
            output_state: self.static_output_state(
                &state,
                &raw.ordered_color_targets,
                raw.depth_target.as_ref(),
                operation_ordinal,
            )?,
            payload_sha256: String::new(),
        })
    }

    fn additional_shader_stage(
        &mut self,
        pointer: Option<usize>,
        entry_token: Option<&str>,
        stage: RawShaderStage,
        draw_sequence: u64,
    ) -> Result<Option<AdditionalShaderStage>, CatalogBuildError> {
        match (pointer, entry_token) {
            (None, None) => Ok(None),
            (Some(pointer), Some(entry_token)) if !entry_token.trim().is_empty() => {
                Ok(Some(AdditionalShaderStage {
                    entry_token: entry_token.to_string(),
                    compiled: self.compiled_shader(pointer, stage, draw_sequence)?,
                }))
            }
            _ => invalid("optional shader pointer/entry token closure is incomplete"),
        }
    }

    fn static_resources(
        &mut self,
        raw: &RawStaticD3dDrawState,
        producer_ordinals: &BTreeMap<u64, u32>,
    ) -> Result<Vec<StaticShaderResourceBinding>, CatalogBuildError> {
        if raw.ordered_texture_objects.len() != raw.ordered_texture_semantics.len() {
            return invalid("static texture object/semantic cardinality mismatch");
        }
        raw.ordered_texture_objects
            .iter()
            .zip(&raw.ordered_texture_semantics)
            .map(|(object, semantic)| {
                if object.stage != semantic.stage
                    || object.slot != semantic.slot
                    || object.shader_resource_view_ptr == 0
                    || object.resource_ptr == 0
                    || semantic.semantic_token.trim().is_empty()
                {
                    return invalid("static texture object order disagrees with semantics");
                }
                let srv_key = (object.shader_resource_view_ptr, raw.capture_sequence);
                let srv = self.srvs.get(&srv_key).copied().ok_or_else(|| {
                    CatalogBuildError("static SRV lacks an exact snapshot".into())
                })?;
                self.used_srvs.insert(srv_key);
                if srv.resource_ptr != object.resource_ptr || srv.desc.flags_u32 != 0 {
                    return invalid("static SRV snapshot is pointer-mismatched or unsupported");
                }
                let texture_key = (object.resource_ptr, raw.capture_sequence);
                let texture = self.textures.get(&texture_key).copied().ok_or_else(|| {
                    CatalogBuildError("static texture lacks an exact snapshot".into())
                })?;
                self.used_textures.insert(texture_key);
                let expected_content = match &semantic.source {
                    RawStaticShaderResourceSource::UsedContent => {
                        Some(texture.content.as_ref().ok_or_else(|| {
                            CatalogBuildError("used-content texture has no VFS identity".into())
                        })?)
                    }
                    _ if texture.content.is_some() => {
                        return invalid("runtime static texture unexpectedly carries VFS identity")
                    }
                    _ => None,
                };
                let mut resource = self.static_texture_resource(texture, expected_content)?;
                let view = shader_resource_view(&srv.desc)?;
                let source = match &semantic.source {
                    RawStaticShaderResourceSource::UsedContent => {
                        let content = self.content.resolve(expected_content.expect("checked"))?;
                        if !raw.ordered_content.iter().any(|join| {
                            self.content
                                .resolve_join(join)
                                .is_ok_and(|joined| joined == content)
                        }) {
                            return invalid(
                                "used-content static texture is absent from ordered content joins",
                            );
                        }
                        StaticShaderResourceSource::UsedContent {
                            content,
                            resource,
                            view,
                        }
                    }
                    RawStaticShaderResourceSource::AuthenticatedSnapshot => {
                        if resource.ordered_subresources.is_empty() {
                            return invalid(
                                "authenticated draw-time texture snapshot has no subresources",
                            );
                        }
                        let packet = AuthenticatedTextureSnapshotPacket::sealed(resource)
                            .map_err(CatalogBuildError)?;
                        let snapshot_sha256 = packet.snapshot_sha256.clone();
                        if let Some(existing) =
                            self.authenticated_texture_snapshots.get(&snapshot_sha256)
                        {
                            if existing != &packet {
                                return invalid("authenticated texture snapshot digest collision");
                            }
                        } else {
                            self.authenticated_texture_snapshots
                                .insert(snapshot_sha256.clone(), packet);
                        }
                        StaticShaderResourceSource::AuthenticatedSnapshot {
                            snapshot_sha256,
                            view,
                        }
                    }
                    RawStaticShaderResourceSource::PreparedTarget {
                        target,
                        producer_capture_sequence,
                    } => {
                        let operation_ordinal = producer_ordinals
                            .get(producer_capture_sequence)
                            .copied()
                            .ok_or_else(|| {
                                CatalogBuildError(
                                    "prepared target producer is absent from static order".into(),
                                )
                            })?;
                        StaticShaderResourceSource::PreparedTarget {
                            target: canonical_prepared_target(
                                &target.prepared_target,
                                operation_ordinal,
                            ),
                            resource,
                            view,
                            producer: RenderExecutionRef::Static { operation_ordinal },
                        }
                    }
                    RawStaticShaderResourceSource::SnowMudTexture {
                        producer_ordinal,
                        payload_sha256,
                    } => {
                        resource.ordered_subresources.clear();
                        StaticShaderResourceSource::SnowMudTexture {
                            producer_ordinal: *producer_ordinal,
                            payload_sha256: payload_sha256.clone(),
                            resource,
                            view,
                        }
                    }
                    RawStaticShaderResourceSource::PoliticalTexture {
                        texture_kind,
                        target_binding_identity,
                        upload_order,
                        payload_sha256,
                    } => {
                        let texture_kind = match texture_kind.as_str() {
                            "primary" => PoliticalTextureKind::Primary,
                            "secondary" => PoliticalTextureKind::Secondary,
                            "gradient_border_channel_1" => {
                                PoliticalTextureKind::GradientBorderChannel1
                            }
                            "gradient_border_channel_2" => {
                                PoliticalTextureKind::GradientBorderChannel2
                            }
                            "gradient_border_channel_3" => {
                                PoliticalTextureKind::GradientBorderChannel3
                            }
                            _ => return invalid("static political texture kind is unknown"),
                        };
                        StaticShaderResourceSource::PoliticalTexture {
                            texture_kind,
                            target_binding_identity: target_binding_identity.clone(),
                            upload_order: *upload_order,
                            payload_sha256: payload_sha256.clone(),
                            resource,
                            view,
                        }
                    }
                    // The tiled-light textures are forbidden product authority (the engine
                    // writer anchors them to the live camera), so their captured
                    // subresource bytes and digests are stripped here exactly like the
                    // SnowMudTexture arm; only descriptor identity and writer
                    // vocabulary reach the wire.
                    RawStaticShaderResourceSource::TiledLightIndex { writer_index, .. } => {
                        let mut descriptor = resource;
                        descriptor.ordered_subresources.clear();
                        StaticShaderResourceSource::TiledLightIndex {
                            writer_index: *writer_index,
                            resource: descriptor,
                            view,
                        }
                    }
                    RawStaticShaderResourceSource::TiledLightData { writer_index, .. } => {
                        let mut descriptor = resource;
                        descriptor.ordered_subresources.clear();
                        StaticShaderResourceSource::TiledLightData {
                            writer_index: *writer_index,
                            resource: descriptor,
                            view,
                        }
                    }
                };
                Ok(StaticShaderResourceBinding {
                    stage: shader_stage(object.stage),
                    slot: object.slot,
                    semantic_token: semantic.semantic_token.clone(),
                    source,
                })
            })
            .collect()
    }

    fn static_texture_resource(
        &self,
        raw: &RawTextureResourceSnapshot,
        expected_content: Option<&RawVfsContentIdentity>,
    ) -> Result<TextureResourceDesc, CatalogBuildError> {
        let content = expected_content
            .map(|value| self.content.resolve(value))
            .transpose()?;
        let ordered_subresources = raw
            .ordered_subresources
            .iter()
            .enumerate()
            .map(|(ordinal, sub)| {
                if sub.subresource_index as usize != ordinal {
                    return invalid("static texture subresources are not contiguous");
                }
                if let Some(content) = &content {
                    match &sub.payload.source {
                        RawPayloadSource::VfsRange { content: range, .. }
                            if self.content.resolve(range)? == *content => {}
                        _ => {
                            return invalid(
                                "static VFS texture payload is not an exact admitted range",
                            )
                        }
                    }
                }
                Ok(TextureSubresourcePayload {
                    subresource_index: sub.subresource_index,
                    mip_level: sub.mip_level,
                    array_slice: sub.array_slice,
                    row_pitch: sub.row_pitch,
                    slice_pitch: sub.slice_pitch,
                    source: self.buffer_source(&sub.payload, None)?,
                })
            })
            .collect::<Result<Vec<_>, CatalogBuildError>>()?;
        Ok(TextureResourceDesc {
            dimension: texture_dimension(raw.dimension_u32, raw.misc_flags_u32)?,
            width: raw.width,
            height: raw.height,
            depth: raw.depth,
            mip_levels: raw.mip_levels,
            array_size: raw.array_size,
            dxgi_format_u32: raw.dxgi_format_u32,
            sample_count: raw.sample_count,
            sample_quality: raw.sample_quality,
            usage_u32: raw.usage_u32,
            bind_flags_u32: raw.bind_flags_u32,
            cpu_access_flags_u32: raw.cpu_access_flags_u32,
            misc_flags_u32: raw.misc_flags_u32,
            ordered_subresources,
        })
    }

    fn finish_exact_closure(self) -> Result<(), CatalogBuildError> {
        let checks = [
            (
                self.used_programs.len(),
                self.programs.len(),
                "program identities",
            ),
            (
                self.used_shaders.len(),
                self.shaders.len(),
                "shader creations",
            ),
            (
                self.used_input_layouts.len(),
                self.input_layouts.len(),
                "input-layout creations",
            ),
            (self.used_blends.len(), self.blends.len(), "blend creations"),
            (self.used_depths.len(), self.depths.len(), "depth creations"),
            (
                self.used_rasters.len(),
                self.rasters.len(),
                "rasterizer creations",
            ),
            (
                self.used_samplers.len(),
                self.samplers.len(),
                "sampler creations",
            ),
            (
                self.used_buffers.len(),
                self.buffers.len(),
                "buffer snapshots",
            ),
            (
                self.used_textures.len(),
                self.textures.len(),
                "texture snapshots",
            ),
            (self.used_srvs.len(), self.srvs.len(), "SRV snapshots"),
            (
                self.used_output_views.len(),
                self.output_views.len(),
                "output-view snapshots",
            ),
        ];
        for (used, total, label) in checks {
            if used != total {
                return invalid(format!("capture contains unused/stale {label}"));
            }
        }
        Ok(())
    }
}

fn static_state_adapter(raw: &RawStaticD3dDrawState) -> Result<RawD3dDrawState, CatalogBuildError> {
    let draw_order = u32::try_from(raw.capture_sequence)
        .map_err(|_| CatalogBuildError("static draw sequence exceeds portable u32 order".into()))?;
    Ok(RawD3dDrawState {
        final_draw_sequence: raw.capture_sequence,
        identity: RawDrawIdentity::Particle {
            effect_instance_id: raw.capture_sequence,
            registration_token: "canonical-static-program".into(),
            definition_source_token: "canonical-static-operation".into(),
            builder_call_ordinal: 0,
            packet_ordinal: 0,
            draw_order,
            shader_program_ptr: raw.shader_program_ptr,
        },
        family: RawDrawFamily::Particle,
        submit_index: 1,
        draw_order,
        render_context_ptr: raw.render_context_ptr,
        device_context_ptr: raw.device_context_ptr,
        shader_program_ptr: raw.shader_program_ptr,
        vertex_shader_ptr: raw.vertex_shader_ptr,
        pixel_shader_ptr: raw.pixel_shader_ptr,
        geometry_shader_ptr: raw.geometry_shader_ptr,
        hull_shader_ptr: raw.hull_shader_ptr,
        domain_shader_ptr: raw.domain_shader_ptr,
        compute_shader_ptr: raw.compute_shader_ptr,
        dynamic_shader_linkage_present: raw.dynamic_shader_linkage_present,
        input_layout_ptr: raw.input_layout_ptr,
        blend_state_ptr: raw.blend_state_ptr,
        blend_factor_rgba_f32_bits: raw.blend_factor_rgba_f32_bits,
        sample_mask_u32: raw.sample_mask_u32,
        depth_stencil_state_ptr: raw.depth_stencil_state_ptr,
        stencil_reference_u32: raw.stencil_reference_u32,
        rasterizer_state_ptr: raw.rasterizer_state_ptr,
        primitive_topology_u32: raw.primitive_topology_u32,
        ordered_texture_objects: raw.ordered_texture_objects.clone(),
        ordered_sampler_objects: raw.ordered_sampler_objects.clone(),
        ordered_constant_buffer_objects: raw.ordered_constant_buffer_objects.clone(),
        ordered_vertex_buffer_objects: raw.ordered_vertex_buffer_objects.clone(),
        index_buffer: raw.index_buffer.clone(),
        ordered_stream_output_buffer_ptrs: Vec::new(),
        ordered_color_target_view_ptrs: raw.ordered_color_target_view_ptrs.clone(),
        depth_stencil_view_ptr: raw.depth_stencil_view_ptr,
        ordered_viewports_f32_bits: raw.ordered_viewports_f32_bits.clone(),
        ordered_scissor_rects_i32: raw.ordered_scissor_rects_i32.clone(),
        draw_call: match raw.draw_call {
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
        },
    })
}

fn validate_payload_hash(payload: &RawPayloadSnapshot) -> Result<(), CatalogBuildError> {
    if payload.captured_bytes.is_empty()
        || hex_sha256(&payload.captured_bytes) != payload.captured_sha256
    {
        return invalid("raw payload is empty or hash-invalid");
    }
    Ok(())
}

fn validate_payload_for_auth<V: AdmittedVfsPayloads>(
    payload: &RawPayloadSnapshot,
    content: &ManifestContentIndex<'_>,
    admitted_vfs: &V,
    particle_records: Option<&[u8]>,
) -> Result<(), CatalogBuildError> {
    validate_payload_hash(payload)?;
    match &payload.source {
        RawPayloadSource::VfsRange {
            content: observed,
            source_byte_offset,
            byte_count,
            range_sha256,
        } => {
            let key = content.resolve(observed)?;
            if *byte_count == 0
                || *byte_count as usize != payload.captured_bytes.len()
                || range_sha256 != &payload.captured_sha256
            {
                return invalid("VFS range length/hash disagrees with captured bytes");
            }
            let admitted = admitted_vfs
                .payload_range(&key, *source_byte_offset, *byte_count, range_sha256)
                .ok_or_else(|| {
                    CatalogBuildError(
                        "admitted VFS range bytes are unavailable for exact authentication".into(),
                    )
                })?;
            if admitted.len() != payload.captured_bytes.len()
                || hex_sha256(admitted) != *range_sha256
                || admitted != payload.captured_bytes
            {
                return invalid("captured bytes do not equal admitted VFS range");
            }
        }
        RawPayloadSource::RuntimeGenerated { reason } => {
            if reason.trim().is_empty() {
                return invalid("runtime-generated payload has a blank reason");
            }
        }
        RawPayloadSource::ParticleInstanceRecords => {
            if particle_records != Some(payload.captured_bytes.as_slice()) {
                return invalid("particle-instance alias disagrees with packet records");
            }
        }
    }
    Ok(())
}

fn hex_sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn shader_stage(value: RawShaderStage) -> ShaderStage {
    match value {
        RawShaderStage::Vertex => ShaderStage::Vertex,
        RawShaderStage::Pixel => ShaderStage::Pixel,
        RawShaderStage::Geometry => ShaderStage::Geometry,
        RawShaderStage::Hull => ShaderStage::Hull,
        RawShaderStage::Domain => ShaderStage::Domain,
        RawShaderStage::Compute => ShaderStage::Compute,
    }
}
fn stage_bit(value: RawShaderStage) -> u8 {
    match value {
        RawShaderStage::Vertex => 1,
        RawShaderStage::Pixel => 2,
        RawShaderStage::Geometry => 4,
        RawShaderStage::Hull => 8,
        RawShaderStage::Domain => 16,
        RawShaderStage::Compute => 32,
    }
}
fn raw_bool(value: u32, label: &str) -> Result<bool, CatalogBuildError> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => invalid(format!("{label} is not a canonical D3D BOOL")),
    }
}

fn topology(value: u32) -> Result<PrimitiveTopology, CatalogBuildError> {
    match value {
        1 => Ok(PrimitiveTopology::PointList),
        2 => Ok(PrimitiveTopology::LineList),
        3 => Ok(PrimitiveTopology::LineStrip),
        4 => Ok(PrimitiveTopology::TriangleList),
        5 => Ok(PrimitiveTopology::TriangleStrip),
        _ => invalid("unsupported primitive topology"),
    }
}
fn index_format(value: u32) -> Result<IndexFormat, CatalogBuildError> {
    match value {
        57 => Ok(IndexFormat::R16Uint),
        42 => Ok(IndexFormat::R32Uint),
        _ => invalid("unsupported index format"),
    }
}
fn blend_factor(value: u32) -> Result<BlendFactor, CatalogBuildError> {
    match value {
        1 => Ok(BlendFactor::Zero),
        2 => Ok(BlendFactor::One),
        3 => Ok(BlendFactor::SourceColor),
        4 => Ok(BlendFactor::InverseSourceColor),
        5 => Ok(BlendFactor::SourceAlpha),
        6 => Ok(BlendFactor::InverseSourceAlpha),
        7 => Ok(BlendFactor::DestinationAlpha),
        8 => Ok(BlendFactor::InverseDestinationAlpha),
        9 => Ok(BlendFactor::DestinationColor),
        10 => Ok(BlendFactor::InverseDestinationColor),
        11 => Ok(BlendFactor::SourceAlphaSaturate),
        14 => Ok(BlendFactor::BlendFactor),
        15 => Ok(BlendFactor::InverseBlendFactor),
        16 => Ok(BlendFactor::Source1Color),
        17 => Ok(BlendFactor::InverseSource1Color),
        18 => Ok(BlendFactor::Source1Alpha),
        19 => Ok(BlendFactor::InverseSource1Alpha),
        _ => invalid("unsupported blend factor"),
    }
}
fn blend_operation(value: u32) -> Result<BlendOperation, CatalogBuildError> {
    match value {
        1 => Ok(BlendOperation::Add),
        2 => Ok(BlendOperation::Subtract),
        3 => Ok(BlendOperation::ReverseSubtract),
        4 => Ok(BlendOperation::Minimum),
        5 => Ok(BlendOperation::Maximum),
        _ => invalid("unsupported blend operation"),
    }
}
fn comparison(value: u32) -> Result<Comparison, CatalogBuildError> {
    match value {
        1 => Ok(Comparison::Never),
        2 => Ok(Comparison::Less),
        3 => Ok(Comparison::Equal),
        4 => Ok(Comparison::LessEqual),
        5 => Ok(Comparison::Greater),
        6 => Ok(Comparison::NotEqual),
        7 => Ok(Comparison::GreaterEqual),
        8 => Ok(Comparison::Always),
        _ => invalid("unsupported comparison"),
    }
}
fn stencil_operation(value: u32) -> Result<StencilOperation, CatalogBuildError> {
    match value {
        1 => Ok(StencilOperation::Keep),
        2 => Ok(StencilOperation::Zero),
        3 => Ok(StencilOperation::Replace),
        4 => Ok(StencilOperation::IncrementSaturate),
        5 => Ok(StencilOperation::DecrementSaturate),
        6 => Ok(StencilOperation::Invert),
        7 => Ok(StencilOperation::Increment),
        8 => Ok(StencilOperation::Decrement),
        _ => invalid("unsupported stencil operation"),
    }
}

fn render_target_blend(
    raw: &RawRenderTargetBlendDesc,
) -> Result<RenderTargetBlendDesc, CatalogBuildError> {
    Ok(RenderTargetBlendDesc {
        enabled: raw_bool(raw.blend_enable_u32, "blend enable")?,
        source_color: blend_factor(raw.source_color_u32)?,
        destination_color: blend_factor(raw.destination_color_u32)?,
        color_operation: blend_operation(raw.color_operation_u32)?,
        source_alpha: blend_factor(raw.source_alpha_u32)?,
        destination_alpha: blend_factor(raw.destination_alpha_u32)?,
        alpha_operation: blend_operation(raw.alpha_operation_u32)?,
        write_mask_rgba_u8: raw.write_mask_rgba_u8,
    })
}
fn stencil_face(raw: &RawStencilFaceDesc) -> Result<StencilFaceDesc, CatalogBuildError> {
    Ok(StencilFaceDesc {
        fail_operation: stencil_operation(raw.fail_operation_u32)?,
        depth_fail_operation: stencil_operation(raw.depth_fail_operation_u32)?,
        pass_operation: stencil_operation(raw.pass_operation_u32)?,
        comparison: comparison(raw.comparison_u32)?,
    })
}

fn default_blend() -> RawBlendStateCreation {
    RawBlendStateCreation {
        creation_sequence: 1,
        blend_state_ptr: 0,
        alpha_to_coverage_enable_u32: 0,
        independent_blend_enable_u32: 0,
        ordered_targets: (0..8)
            .map(|_| RawRenderTargetBlendDesc {
                blend_enable_u32: 0,
                source_color_u32: 2,
                destination_color_u32: 1,
                color_operation_u32: 1,
                source_alpha_u32: 2,
                destination_alpha_u32: 1,
                alpha_operation_u32: 1,
                write_mask_rgba_u8: 15,
            })
            .collect(),
    }
}
fn default_depth() -> RawDepthStencilStateCreation {
    let face = RawStencilFaceDesc {
        fail_operation_u32: 1,
        depth_fail_operation_u32: 1,
        pass_operation_u32: 1,
        comparison_u32: 8,
    };
    RawDepthStencilStateCreation {
        creation_sequence: 1,
        depth_stencil_state_ptr: 0,
        depth_enable_u32: 1,
        depth_write_mask_u32: 1,
        depth_comparison_u32: 2,
        stencil_enable_u32: 0,
        stencil_read_mask_u8: 255,
        stencil_write_mask_u8: 255,
        front_face: face.clone(),
        back_face: face,
    }
}
fn default_rasterizer() -> RawRasterizerStateCreation {
    RawRasterizerStateCreation {
        creation_sequence: 1,
        rasterizer_state_ptr: 0,
        fill_mode_u32: 3,
        cull_mode_u32: 3,
        front_counter_clockwise_u32: 0,
        depth_bias_i32: 0,
        depth_bias_clamp_f32_bits: 0,
        slope_scaled_depth_bias_f32_bits: 0,
        depth_clip_enable_u32: 1,
        scissor_enable_u32: 0,
        multisample_enable_u32: 0,
        antialiased_line_enable_u32: 0,
    }
}

fn texture_dimension(value: u32, misc: u32) -> Result<TextureResourceDimension, CatalogBuildError> {
    match value {
        3 => Ok(TextureResourceDimension::Texture1d),
        4 if misc & 4 != 0 => Ok(TextureResourceDimension::TextureCube),
        4 => Ok(TextureResourceDimension::Texture2d),
        5 => Ok(TextureResourceDimension::Texture3d),
        _ => invalid("unsupported texture resource dimension"),
    }
}

fn shader_resource_view(
    raw: &RawResourceViewDesc,
) -> Result<ShaderResourceViewDesc, CatalogBuildError> {
    let f = raw.union_words_u32;
    if raw.flags_u32 != 0 {
        return invalid("SRV has nonzero DSV flags field");
    }
    match raw.view_dimension_u32 {
        1 => Ok(ShaderResourceViewDesc::Buffer {
            dxgi_format_u32: raw.dxgi_format_u32,
            first_element: f[0],
            element_count: f[1],
            flags_u32: 0,
        }),
        2 => Ok(ShaderResourceViewDesc::Texture1d {
            dxgi_format_u32: raw.dxgi_format_u32,
            most_detailed_mip: f[0],
            mip_levels: f[1],
        }),
        3 => Ok(ShaderResourceViewDesc::Texture1dArray {
            dxgi_format_u32: raw.dxgi_format_u32,
            most_detailed_mip: f[0],
            mip_levels: f[1],
            first_array_slice: f[2],
            array_size: f[3],
        }),
        4 => Ok(ShaderResourceViewDesc::Texture2d {
            dxgi_format_u32: raw.dxgi_format_u32,
            most_detailed_mip: f[0],
            mip_levels: f[1],
        }),
        5 => Ok(ShaderResourceViewDesc::Texture2dArray {
            dxgi_format_u32: raw.dxgi_format_u32,
            most_detailed_mip: f[0],
            mip_levels: f[1],
            first_array_slice: f[2],
            array_size: f[3],
        }),
        6 => Ok(ShaderResourceViewDesc::Texture2dMultisampled {
            dxgi_format_u32: raw.dxgi_format_u32,
        }),
        7 => Ok(ShaderResourceViewDesc::Texture2dMultisampledArray {
            dxgi_format_u32: raw.dxgi_format_u32,
            first_array_slice: f[0],
            array_size: f[1],
        }),
        8 => Ok(ShaderResourceViewDesc::Texture3d {
            dxgi_format_u32: raw.dxgi_format_u32,
            most_detailed_mip: f[0],
            mip_levels: f[1],
        }),
        9 => Ok(ShaderResourceViewDesc::TextureCube {
            dxgi_format_u32: raw.dxgi_format_u32,
            most_detailed_mip: f[0],
            mip_levels: f[1],
        }),
        10 => Ok(ShaderResourceViewDesc::TextureCubeArray {
            dxgi_format_u32: raw.dxgi_format_u32,
            most_detailed_mip: f[0],
            mip_levels: f[1],
            first_face: f[2],
            cube_count: f[3],
        }),
        11 => Ok(ShaderResourceViewDesc::Buffer {
            dxgi_format_u32: raw.dxgi_format_u32,
            first_element: f[0],
            element_count: f[1],
            flags_u32: f[2],
        }),
        _ => invalid("unsupported SRV dimension"),
    }
}

fn address_mode(value: u32) -> Result<AddressMode, CatalogBuildError> {
    match value {
        1 => Ok(AddressMode::Wrap),
        2 => Ok(AddressMode::Mirror),
        3 => Ok(AddressMode::Clamp),
        4 => Ok(AddressMode::Border),
        _ => invalid("unsupported sampler address mode"),
    }
}
fn decode_filter(value: u32) -> Result<(Filter, Filter, Filter, bool), CatalogBuildError> {
    let compared = value & 0x80 != 0;
    let core = value & !0x80;
    if core == 0x55 {
        return Ok((
            Filter::Anisotropic,
            Filter::Anisotropic,
            Filter::Anisotropic,
            compared,
        ));
    }
    if core & !0x15 != 0 {
        return invalid("unsupported sampler filter encoding");
    }
    Ok((
        if core & 0x10 != 0 {
            Filter::Linear
        } else {
            Filter::Point
        },
        if core & 0x4 != 0 {
            Filter::Linear
        } else {
            Filter::Point
        },
        if core & 0x1 != 0 {
            Filter::Linear
        } else {
            Filter::Point
        },
        compared,
    ))
}
fn draw_command(raw: RawDrawCall) -> DrawCommand {
    match raw {
        RawDrawCall::Draw {
            vertex_count,
            start_vertex,
        } => DrawCommand::Draw {
            vertex_count,
            start_vertex,
        },
        RawDrawCall::DrawIndexed {
            index_count,
            start_index,
            base_vertex,
        } => DrawCommand::DrawIndexed {
            index_count,
            start_index,
            base_vertex,
        },
        RawDrawCall::DrawInstanced {
            vertex_count_per_instance,
            instance_count,
            start_vertex,
            start_instance,
        } => DrawCommand::DrawInstanced {
            vertex_count_per_instance,
            instance_count,
            start_vertex,
            start_instance,
        },
        RawDrawCall::DrawIndexedInstanced {
            index_count_per_instance,
            instance_count,
            start_index,
            base_vertex,
            start_instance,
        } => DrawCommand::DrawIndexedInstanced {
            index_count_per_instance,
            instance_count,
            start_index,
            base_vertex,
            start_instance,
        },
    }
}

fn prepared_target(
    raw: &RawPreparedTarget,
    resource: Option<&str>,
    producer: Option<&str>,
) -> Result<PreparedRenderTarget, CatalogBuildError> {
    if resource.is_some() || producer.is_some() {
        return invalid("opaque target carries a forbidden semantic token");
    }
    match raw {
        RawPreparedTarget::SwapchainBackBuffer => Ok(PreparedRenderTarget::SwapchainBackBuffer),
        RawPreparedTarget::OpaqueRenderTarget { attachment_ordinal } => {
            Ok(PreparedRenderTarget::OpaqueRenderTarget {
                attachment_ordinal: *attachment_ordinal,
            })
        }
    }
}

fn canonical_static_operation_token(operation_ordinal: u32) -> String {
    format!("opaque-static-operation-v1:{operation_ordinal}")
}

fn canonical_static_draw_token(operation_ordinal: u32) -> String {
    format!("opaque-static-draw-v1:{operation_ordinal}")
}

fn canonical_prepared_target(
    raw: &RawPreparedTarget,
    _producer_operation_ordinal: u32,
) -> PreparedRenderTarget {
    match raw {
        RawPreparedTarget::SwapchainBackBuffer => PreparedRenderTarget::SwapchainBackBuffer,
        RawPreparedTarget::OpaqueRenderTarget { attachment_ordinal } => {
            PreparedRenderTarget::OpaqueRenderTarget {
                attachment_ordinal: *attachment_ordinal,
            }
        }
    }
}

fn canonicalize_prepared_target(
    target: &PreparedRenderTarget,
    producer_operation_ordinal: u32,
) -> PreparedRenderTarget {
    let raw = match target {
        PreparedRenderTarget::SwapchainBackBuffer => RawPreparedTarget::SwapchainBackBuffer,
        PreparedRenderTarget::OpaqueRenderTarget { attachment_ordinal } => {
            RawPreparedTarget::OpaqueRenderTarget {
                attachment_ordinal: *attachment_ordinal,
            }
        }
    };
    canonical_prepared_target(&raw, producer_operation_ordinal)
}

fn canonical_raw_output_semantic(
    target: &RawOutputTargetSemantic,
    producer_operation_ordinal: u32,
) -> RawOutputTargetSemantic {
    let mut canonical = target.clone();
    let _ = producer_operation_ordinal;
    match target.prepared_target {
        RawPreparedTarget::SwapchainBackBuffer | RawPreparedTarget::OpaqueRenderTarget { .. } => {
            canonical.resource_token = None;
            canonical.producer_pass_token = None;
        }
    }
    canonical
}

fn output_mip_level(
    raw: &RawResourceViewDesc,
    kind: RawOutputViewKind,
) -> Result<u32, CatalogBuildError> {
    let f = raw.union_words_u32;
    match kind {
        RawOutputViewKind::Color => match raw.view_dimension_u32 {
            2 | 4 => Ok(f[0]),
            3 | 5 if f[1] == 0 && f[2] == 1 => Ok(f[0]),
            6 => Ok(0),
            7 if f[0] == 0 && f[1] == 1 => Ok(0),
            8 if f[1] == 0 && f[2] == 1 => Ok(f[0]),
            _ => invalid("output RTV view cannot be represented exactly"),
        },
        RawOutputViewKind::DepthStencil => match raw.view_dimension_u32 {
            1 | 3 => Ok(f[0]),
            2 | 4 if f[1] == 0 && f[2] == 1 => Ok(f[0]),
            5 => Ok(0),
            6 if f[0] == 0 && f[1] == 1 => Ok(0),
            _ => invalid("output DSV view cannot be represented exactly"),
        },
    }
}
fn mip_extent(value: u32, mip: u32) -> u32 {
    value.checked_shr(mip).unwrap_or(0).max(1)
}

#[cfg(test)]
mod static_identity_tests {
    use super::*;

    fn pointer_observation() -> RawRecordedPointerInputObservation {
        RawRecordedPointerInputObservation {
            executable_sha256: A729_EXECUTABLE_SHA256.into(),
            process_id: 41,
            process_start_nonce: 42,
            hook_epoch_nonce: 43,
            observation_sequence: 44,
            prior_observation_qpc_ticks: 100,
            selected_present_qpc_ticks: 200,
            present_sample_started_qpc_ticks: 210,
            present_sample_completed_qpc_ticks: 220,
            selected_present_receipt_sha256: "a".repeat(64),
            client_extent: [1920, 1080],
            prior_client_position: [10, 20],
            current_client_position: [11, 20],
        }
    }

    #[test]
    fn recorded_pointer_materializer_is_manifest_pinned_and_deterministic() {
        let receipt = authenticate_recorded_pointer_input(pointer_observation()).unwrap();
        assert_eq!(receipt.variant, RecordedInputVariant::MouseKeyboard);
        assert_eq!(
            receipt.authority_manifest_sha256,
            RECORDED_INPUT_VARIANT_AUTHORITY_MANIFEST_SHA256.unwrap()
        );
        assert_eq!(
            receipt.observation_receipt_sha256,
            receipt.computed_receipt_sha256().unwrap()
        );
    }

    #[test]
    fn stationary_outside_or_reordered_pointer_observations_reject() {
        for mutation in 0..4 {
            let mut observation = pointer_observation();
            match mutation {
                0 => observation.current_client_position = observation.prior_client_position,
                1 => observation.current_client_position = [1920, 20],
                2 => observation.selected_present_qpc_ticks = 100,
                _ => observation.present_sample_started_qpc_ticks = 199,
            }
            assert!(authenticate_recorded_pointer_input(observation).is_err());
        }
    }

    #[test]
    fn canonical_operation_domains_prevent_relabel_merge_and_split() {
        let first_pass = canonical_static_operation_token(3);
        let second_pass = canonical_static_operation_token(4);
        let first_draw = canonical_static_draw_token(3);
        assert_ne!(first_pass, second_pass, "adjacent operations cannot merge");
        assert_ne!(first_pass, first_draw, "pass/draw domains cannot collide");
        assert_eq!(first_pass, canonical_static_operation_token(3));

        let opaque = canonical_prepared_target(
            &RawPreparedTarget::OpaqueRenderTarget {
                attachment_ordinal: 2,
            },
            3,
        );
        let PreparedRenderTarget::OpaqueRenderTarget { attachment_ordinal } = opaque else {
            panic!("wrong canonical target kind")
        };
        assert_eq!(attachment_ordinal, 2);
    }

    fn clear_program(ordinal: u32) -> StaticMapPassProgram {
        let mut program = StaticMapPassProgram {
            program_ordinal: ordinal,
            pass_token: canonical_static_operation_token(ordinal),
            ordered_content_keys: Vec::new(),
            ordered_operations: vec![StaticMapOperation {
                operation_ordinal: ordinal,
                payload_sha256: String::new(),
                command: StaticMapCommand::ClearColor {
                    target: RenderTargetDesc {
                        prepared_target: PreparedRenderTarget::OpaqueRenderTarget {
                            attachment_ordinal: 0,
                        },
                        width: 4,
                        height: 4,
                        dxgi_format_u32: 28,
                        mip_level: 0,
                        sample_count: 1,
                        sample_quality: 0,
                        persistent_content: None,
                        exact_resource: None,
                    },
                    rgba_f32_bits: [0; 4],
                },
            }],
            payload_sha256: String::new(),
        };
        program.seal_payload_hashes().unwrap();
        program
    }

    #[test]
    fn canonical_partition_rejects_relabel_merge_and_split_even_when_resealed() {
        let base = AuthenticatedStaticProgramSet {
            ordered_programs: vec![clear_program(0), clear_program(1)],
            ordered_execution: vec![
                RenderExecutionRef::Static {
                    operation_ordinal: 0,
                },
                RenderExecutionRef::Static {
                    operation_ordinal: 1,
                },
            ],
            authenticated_texture_snapshots: Vec::new(),
        };
        base.validate_canonical_partition().unwrap();

        let mut relabeled = base.clone();
        relabeled.ordered_programs[0].pass_token = "caller-label".into();
        relabeled.ordered_programs[0].seal_payload_hashes().unwrap();
        assert!(relabeled.validate_canonical_partition().is_err());

        let mut merged = base.clone();
        let second = merged
            .ordered_programs
            .remove(1)
            .ordered_operations
            .remove(0);
        merged.ordered_programs[0].ordered_operations.push(second);
        merged.ordered_programs[0].seal_payload_hashes().unwrap();
        merged.ordered_execution.pop();
        assert!(merged.validate_canonical_partition().is_err());

        let mut split = base.clone();
        split.ordered_programs.insert(1, clear_program(1));
        assert!(split.validate_canonical_partition().is_err());
    }
}
