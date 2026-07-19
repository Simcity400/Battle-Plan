//! Neutral live joins between engine content opens, D3D creation, reflection,
//! and exact d77/d80 semantic authority.
//!
//! Nothing in this module assigns a vanilla role or path. The hook supplies
//! authenticated observations; these materializers only validate exact joins
//! and convert them into the existing strict raw contracts.

use super::*;
use portable_content_manifest::{normalize_game_relative_path, UsedContentKey};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawEffectiveVfsWinnerRow {
    pub content: RawVfsContentIdentity,
    /// Hash of an archive/container when the winning file came from one.
    /// Directory-backed winners retain `None`.
    pub source_container_sha256: Option<String>,
    pub source_byte_offset: u64,
    pub source_byte_count: u64,
    pub source_range_sha256: String,
}

impl RawEffectiveVfsWinnerRow {
    pub fn validate(&self) -> Result<(), String> {
        let source_shape_valid = match &self.source_container_sha256 {
            Some(hash) => {
                valid_lower_sha256(hash)
                    && *hash == self.content.source_sha256
                    && valid_lower_sha256(&self.source_range_sha256)
            }
            None => {
                self.source_byte_offset == 0
                    && self.source_byte_count == self.content.payload_bytes
                    && self.source_range_sha256 == self.content.payload_sha256
                    && self.content.source_sha256 == self.content.payload_sha256
            }
        };
        if !valid_raw_content(&self.content)
            || !source_shape_valid
            || !valid_lower_sha256(&self.source_range_sha256)
            || self
                .source_byte_offset
                .checked_add(self.source_byte_count)
                .is_none()
        {
            return Err("effective-VFS winner row is incomplete or inconsistent".into());
        }
        Ok(())
    }
}

/// Fully admitted, ephemeral winner index delivered by the authenticated H4CT
/// channel. This type is never serialized into the portable replay wholesale.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawEffectiveVfsWinnerIndexBinding {
    pub process_start_nonce: u64,
    pub authority_manifest_sha256: String,
    pub declared_row_count: u64,
    pub ordered_rows_sha256: String,
    pub ordered_rows: Vec<RawEffectiveVfsWinnerRow>,
}

impl RawEffectiveVfsWinnerIndexBinding {
    pub fn canonical_rows_bytes(rows: &[RawEffectiveVfsWinnerRow]) -> Result<Vec<u8>, String> {
        let mut out = b"hoi4-effective-vfs-winners-v1\0".to_vec();
        put_u64(
            &mut out,
            u64::try_from(rows.len()).map_err(|_| "winner row count overflow")?,
        );
        let mut prior_path: Option<&str> = None;
        for row in rows {
            row.validate()?;
            if prior_path.is_some_and(|prior| prior >= row.content.logical_path.as_str()) {
                return Err(
                    "effective-VFS winner rows are duplicated or not canonically ordered".into(),
                );
            }
            prior_path = Some(&row.content.logical_path);
            put_string(&mut out, &row.content.logical_path)?;
            put_u32(&mut out, row.content.winning_layer_ordinal);
            put_string(&mut out, &row.content.layer_kind)?;
            put_string(&mut out, &row.content.layer_id)?;
            put_string(&mut out, &row.content.source_sha256)?;
            put_string(&mut out, &row.content.payload_sha256)?;
            put_u64(&mut out, row.content.payload_bytes);
            match &row.source_container_sha256 {
                Some(hash) => {
                    out.push(1);
                    put_string(&mut out, hash)?;
                }
                None => out.push(0),
            }
            put_u64(&mut out, row.source_byte_offset);
            put_u64(&mut out, row.source_byte_count);
            put_string(&mut out, &row.source_range_sha256)?;
        }
        Ok(out)
    }

    pub fn canonical_rows_sha256(rows: &[RawEffectiveVfsWinnerRow]) -> Result<String, String> {
        Ok(format!(
            "{:x}",
            Sha256::digest(Self::canonical_rows_bytes(rows)?)
        ))
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.process_start_nonce == 0
            || !valid_lower_sha256(&self.authority_manifest_sha256)
            || self.declared_row_count != self.ordered_rows.len() as u64
            || !valid_lower_sha256(&self.ordered_rows_sha256)
            || Self::canonical_rows_sha256(&self.ordered_rows)? != self.ordered_rows_sha256
        {
            return Err(
                "effective-VFS winner index binding is incomplete or hash-mismatched".into(),
            );
        }
        Ok(())
    }

    pub fn authorize_content(&self, content: &RawVfsContentIdentity) -> Result<(), String> {
        self.validate()?;
        let path = content.logical_path.as_str();
        let index = self
            .ordered_rows
            .binary_search_by(|row| row.content.logical_path.as_str().cmp(path))
            .map_err(|_| "effective-VFS content is omitted from the authenticated winner index")?;
        if self.ordered_rows[index].content != *content {
            return Err("effective-VFS content disagrees with the authenticated winner".into());
        }
        Ok(())
    }

    pub fn authorize_open(&self, open: &RawEffectiveVfsOpenObservation) -> Result<(), String> {
        open.validate()?;
        if open.authority_manifest_sha256 != self.authority_manifest_sha256 {
            return Err("effective-VFS open uses another authority manifest".into());
        }
        self.authorize_content(&open.content)
    }
}

fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put_string(out: &mut Vec<u8>, value: &str) -> Result<(), String> {
    let bytes = value.as_bytes();
    put_u32(
        out,
        u32::try_from(bytes.len()).map_err(|_| "winner row string length overflow")?,
    );
    out.extend_from_slice(bytes);
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RawD3dCreatedObjectKind {
    Buffer,
    Texture1d,
    Texture2d,
    Texture3d,
    VertexShader,
    PixelShader,
    GeometryShader,
    HullShader,
    DomainShader,
    ComputeShader,
}

/// Exact engine resolver/open observation before a parser or resource builder
/// consumes the returned stream/data handle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawEffectiveVfsOpenObservation {
    pub observation_sequence: u64,
    pub authority_manifest_sha256: String,
    pub canonical_logical_path: String,
    pub content: RawVfsContentIdentity,
    pub stream_handle: usize,
    pub data_ptr: Option<usize>,
    pub opened_payload_sha256: String,
    pub opened_payload_bytes: u64,
}

impl RawEffectiveVfsOpenObservation {
    pub fn from_used_content(
        observation_sequence: u64,
        content: &UsedContentKey,
        stream_handle: usize,
        data_ptr: Option<usize>,
    ) -> Result<Self, String> {
        let raw = RawVfsContentIdentity {
            logical_path: content.normalized_game_relative_path.clone(),
            winning_layer_ordinal: content.winning_layer_ordinal,
            layer_kind: match content.layer_kind {
                portable_content_manifest::ContentLayerKind::Base => "base",
                portable_content_manifest::ContentLayerKind::IntegratedDlc => "integrated_dlc",
                portable_content_manifest::ContentLayerKind::Dlc => "dlc",
                portable_content_manifest::ContentLayerKind::Mod => "mod",
            }
            .to_string(),
            layer_id: content.layer_id.clone(),
            source_sha256: content.source_sha256.clone(),
            payload_sha256: content.payload_sha256.clone(),
            payload_bytes: content.payload_bytes,
        };
        let observation = Self {
            observation_sequence,
            authority_manifest_sha256: content.effective_vfs_manifest_sha256.clone(),
            canonical_logical_path: content.normalized_game_relative_path.clone(),
            content: raw,
            stream_handle,
            data_ptr,
            opened_payload_sha256: content.payload_sha256.clone(),
            opened_payload_bytes: content.payload_bytes,
        };
        observation.validate()?;
        Ok(observation)
    }

    pub fn validate(&self) -> Result<(), String> {
        let canonical = normalize_game_relative_path(&self.canonical_logical_path)?;
        if self.observation_sequence == 0
            || self.stream_handle == 0
            || self.data_ptr == Some(0)
            || canonical != self.canonical_logical_path
            || self.content.logical_path != self.canonical_logical_path
            || !valid_lower_sha256(&self.authority_manifest_sha256)
            || !valid_raw_content(&self.content)
            || self.opened_payload_sha256 != self.content.payload_sha256
            || self.opened_payload_bytes != self.content.payload_bytes
        {
            return Err("effective-VFS open observation is incomplete or non-canonical".into());
        }
        Ok(())
    }
}

/// D3D creation event whose source range explicitly names the engine open
/// receipt. Pointer/hash coincidence alone is never accepted as correlation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawD3dContentCreationObservation {
    pub creation_sequence: u64,
    pub object_ptr: usize,
    pub object_kind: RawD3dCreatedObjectKind,
    pub originating_open_sequence: u64,
    pub source_stream_handle: usize,
    pub source_data_ptr: Option<usize>,
    pub source_byte_offset: u64,
    pub source_byte_count: u64,
    pub source_range_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawContentCreationReceipt {
    pub open_sequence: u64,
    pub creation_sequence: u64,
    pub object_ptr: usize,
    pub object_kind: RawD3dCreatedObjectKind,
    pub stream_handle: usize,
    pub data_ptr: Option<usize>,
    pub source_byte_offset: u64,
    pub source_byte_count: u64,
    pub source_range_sha256: String,
    pub authority_manifest_sha256: String,
    pub content: RawVfsContentIdentity,
}

pub fn correlate_content_creation(
    open: &RawEffectiveVfsOpenObservation,
    creation: &RawD3dContentCreationObservation,
) -> Result<RawContentCreationReceipt, String> {
    open.validate()?;
    let end = creation
        .source_byte_offset
        .checked_add(creation.source_byte_count)
        .ok_or("D3D content source range overflow")?;
    if creation.creation_sequence == 0
        || creation.object_ptr == 0
        || creation.originating_open_sequence != open.observation_sequence
        || creation.source_stream_handle != open.stream_handle
        || creation.source_data_ptr != open.data_ptr
        || creation.source_byte_count == 0
        || end > open.opened_payload_bytes
        || !valid_lower_sha256(&creation.source_range_sha256)
        || (creation.source_byte_offset == 0
            && creation.source_byte_count == open.opened_payload_bytes
            && creation.source_range_sha256 != open.opened_payload_sha256)
    {
        return Err("D3D creation does not exactly correlate to its content-open receipt".into());
    }
    Ok(RawContentCreationReceipt {
        open_sequence: open.observation_sequence,
        creation_sequence: creation.creation_sequence,
        object_ptr: creation.object_ptr,
        object_kind: creation.object_kind,
        stream_handle: open.stream_handle,
        data_ptr: open.data_ptr,
        source_byte_offset: creation.source_byte_offset,
        source_byte_count: creation.source_byte_count,
        source_range_sha256: creation.source_range_sha256.clone(),
        authority_manifest_sha256: open.authority_manifest_sha256.clone(),
        content: open.content.clone(),
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawShaderSignatureParameter {
    pub semantic_token: String,
    pub semantic_index: u32,
    pub register_index: u32,
    pub system_value_type_u32: u32,
    pub component_type_u32: u32,
    pub mask_u8: u8,
    pub read_write_mask_u8: u8,
    pub stream_index: u32,
    pub min_precision_u32: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum RawReflectedResourceKind {
    Texture,
    Sampler,
    ConstantBuffer,
    Other(u32),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawReflectedConstantBufferVariable {
    pub name: String,
    pub start_offset: u32,
    pub byte_count: u32,
    pub flags_u32: u32,
    pub type_token: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawShaderReflectedResource {
    pub semantic_token: String,
    pub kind: RawReflectedResourceKind,
    pub bind_point: u32,
    pub bind_count: u32,
    pub flags_u32: u32,
    pub return_type_u32: u32,
    pub dimension_u32: u32,
    pub sample_count: u32,
    pub constant_buffer_byte_count: Option<u32>,
    pub constant_buffer_layout_token: Option<String>,
    pub ordered_variables: Vec<RawReflectedConstantBufferVariable>,
}

/// Materialized output of D3DReflect (or an authenticated compiler observer)
/// for the exact bytecode passed to shader creation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawShaderBytecodeReflectionObservation {
    pub reflection_sequence: u64,
    pub creation_sequence: u64,
    pub shader_ptr: usize,
    pub shader_program_ptr: usize,
    pub stage: RawShaderStage,
    pub bytecode_sha256: String,
    pub bytecode_bytes: u64,
    pub entry_token: String,
    pub target_profile: String,
    pub creator_token: String,
    pub ordered_inputs: Vec<RawShaderSignatureParameter>,
    pub ordered_outputs: Vec<RawShaderSignatureParameter>,
    pub ordered_resources: Vec<RawShaderReflectedResource>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawReflectedStageSemantics {
    pub stage: RawShaderStage,
    pub entry_token: String,
    /// Complete ordered reflection output, including resource kinds not yet
    /// representable by the strict d77/d80 draw contracts.
    pub ordered_resources: Vec<RawShaderReflectedResource>,
    pub ordered_texture_semantics: Vec<RawTextureSemantic>,
    pub ordered_sampler_semantics: Vec<RawSamplerSemantic>,
    pub ordered_constant_buffer_semantics: Vec<RawConstantBufferSemantic>,
    pub ordered_inputs: Vec<RawShaderSignatureParameter>,
    pub ordered_outputs: Vec<RawShaderSignatureParameter>,
}

impl RawShaderBytecodeReflectionObservation {
    pub fn validate_against_creation(&self, creation: &RawShaderCreation) -> Result<(), String> {
        if self.reflection_sequence == 0
            || self.creation_sequence != creation.creation_sequence
            || self.shader_ptr != creation.shader_ptr
            || self.shader_program_ptr == 0
            || self.stage != creation.stage
            || self.bytecode_sha256 != creation.bytecode.captured_sha256
            || self.bytecode_bytes != creation.bytecode.captured_bytes.len() as u64
            || self.entry_token.trim().is_empty()
            || self.target_profile != creation.target_profile
            || self.creator_token.trim().is_empty()
            || !valid_lower_sha256(&self.bytecode_sha256)
        {
            return Err("shader reflection disagrees with exact creation bytecode".into());
        }
        validate_signatures(&self.ordered_inputs, "input")?;
        validate_signatures(&self.ordered_outputs, "output")?;
        validate_reflected_resources(&self.ordered_resources)?;
        Ok(())
    }

    pub fn materialize_stage_semantics(
        &self,
        creation: &RawShaderCreation,
    ) -> Result<RawReflectedStageSemantics, String> {
        self.validate_against_creation(creation)?;
        let mut textures = Vec::new();
        let mut samplers = Vec::new();
        let mut constant_buffers = Vec::new();
        for resource in &self.ordered_resources {
            if resource.bind_count != 1 {
                return Err(
                    "arrayed reflected resource is not representable by strict d77/d80 semantics"
                        .into(),
                );
            }
            let slot = u8::try_from(resource.bind_point)
                .map_err(|_| "reflected bind point exceeds portable slot range")?;
            match resource.kind {
                RawReflectedResourceKind::Texture => textures.push(RawTextureSemantic {
                    stage: self.stage,
                    slot,
                    semantic_token: resource.semantic_token.clone(),
                }),
                RawReflectedResourceKind::Sampler => samplers.push(RawSamplerSemantic {
                    stage: self.stage,
                    slot,
                    semantic_token: resource.semantic_token.clone(),
                }),
                RawReflectedResourceKind::ConstantBuffer => {
                    constant_buffers.push(RawConstantBufferSemantic {
                        stage_mask_u8: stage_bit(self.stage),
                        slot,
                        byte_offset: 0,
                        byte_count: resource
                            .constant_buffer_byte_count
                            .expect("validated constant buffer byte count"),
                        semantic_layout: resource
                            .constant_buffer_layout_token
                            .clone()
                            .expect("validated constant buffer layout token"),
                    })
                }
                RawReflectedResourceKind::Other(_) => {
                    return Err(
                        "reflected resource kind is not representable by strict d77/d80 semantics"
                            .into(),
                    )
                }
            }
        }
        Ok(RawReflectedStageSemantics {
            stage: self.stage,
            entry_token: self.entry_token.clone(),
            ordered_resources: self.ordered_resources.clone(),
            ordered_texture_semantics: textures,
            ordered_sampler_semantics: samplers,
            ordered_constant_buffer_semantics: constant_buffers,
            ordered_inputs: self.ordered_inputs.clone(),
            ordered_outputs: self.ordered_outputs.clone(),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawProgramRegistrationObservation {
    pub observation_sequence: u64,
    pub shader_program_ptr: usize,
    pub source_open_sequence: u64,
    pub effect_token: String,
    pub vertex_entry_token: String,
    pub pixel_entry_token: String,
    pub ordered_defines: Vec<RawShaderDefine>,
}

pub fn materialize_program_identity(
    registration: &RawProgramRegistrationObservation,
    source_open: &RawEffectiveVfsOpenObservation,
    vertex_reflection: &RawShaderBytecodeReflectionObservation,
    pixel_reflection: &RawShaderBytecodeReflectionObservation,
) -> Result<RawProgramIdentity, String> {
    source_open.validate()?;
    if registration.observation_sequence == 0
        || registration.shader_program_ptr == 0
        || registration.source_open_sequence != source_open.observation_sequence
        || registration.effect_token.trim().is_empty()
        || registration.vertex_entry_token.trim().is_empty()
        || registration.pixel_entry_token.trim().is_empty()
        || vertex_reflection.stage != RawShaderStage::Vertex
        || pixel_reflection.stage != RawShaderStage::Pixel
        || vertex_reflection.shader_program_ptr != registration.shader_program_ptr
        || pixel_reflection.shader_program_ptr != registration.shader_program_ptr
        || vertex_reflection.entry_token != registration.vertex_entry_token
        || pixel_reflection.entry_token != registration.pixel_entry_token
        || registration
            .ordered_defines
            .iter()
            .any(|define| define.name.trim().is_empty() || define.scope.trim().is_empty())
    {
        return Err(
            "program registration does not exactly join source and reflected stages".into(),
        );
    }
    Ok(RawProgramIdentity {
        observation_sequence: registration.observation_sequence,
        shader_program_ptr: registration.shader_program_ptr,
        source: source_open.content.clone(),
        effect_token: registration.effect_token.clone(),
        vertex_entry_token: registration.vertex_entry_token.clone(),
        pixel_entry_token: registration.pixel_entry_token.clone(),
        ordered_defines: registration.ordered_defines.clone(),
    })
}

pub const WINDOW_BACKBUFFER_RTV_PRODUCER_RVA: u64 = 0x20fddd0;
pub const GENERIC_TEXTURE_RTV_PRODUCER_RVA: u64 = 0x20fda10;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RawRegisteredRenderTargetKind {
    SwapchainBackBuffer { buffer_index: u32 },
    OpaqueRenderTarget { source_flags_u32: u32 },
}

/// Exact post-success target registration joined to one observed OM use.
/// Pointer fields are transient registry keys only. Stable tokens are derived
/// solely from the producer, generation/order, kind, and complete descriptors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawRenderTargetRegistrationObservation {
    pub registration_sequence: u64,
    pub capture_sequence: u64,
    pub producer_rva: u64,
    pub recreate_generation: u64,
    pub attachment_ordinal: u32,
    pub kind: RawRegisteredRenderTargetKind,
    pub view_ptr: usize,
    pub resource_ptr: usize,
    pub resource_desc: RawTexture2dDesc,
    pub rtv_desc: RawResourceViewDesc,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawRenderTargetRegistrationReceipt {
    pub registration_sequence: u64,
    pub capture_sequence: u64,
    pub producer_rva: u64,
    pub recreate_generation: u64,
    pub attachment_ordinal: u32,
    pub kind: RawRegisteredRenderTargetKind,
    pub view_ptr: usize,
    pub resource_ptr: usize,
    pub resource_desc: RawTexture2dDesc,
    pub rtv_desc: RawResourceViewDesc,
    pub resource_identity_sha256: String,
    pub view_identity_sha256: String,
    pub target: RawOutputTargetSemantic,
}

impl RawRenderTargetRegistrationObservation {
    pub fn materialize(&self) -> Result<RawRenderTargetRegistrationReceipt, String> {
        if self.registration_sequence == 0
            || self.capture_sequence == 0
            || self.registration_sequence > self.capture_sequence
            || self.producer_rva == 0
            || self.view_ptr == 0
            || self.resource_ptr == 0
            || self.resource_desc.width == 0
            || self.resource_desc.height == 0
            || self.resource_desc.mip_levels == 0
            || self.resource_desc.array_size == 0
            || self.resource_desc.sample_count == 0
            || self.rtv_desc.view_dimension_u32 == 0
            || self.rtv_desc.flags_u32 != 0
        {
            return Err("render-target registration observation is incomplete".into());
        }
        let prepared_target = match self.kind {
            RawRegisteredRenderTargetKind::SwapchainBackBuffer { buffer_index }
                if self.producer_rva == WINDOW_BACKBUFFER_RTV_PRODUCER_RVA && buffer_index == 0 =>
            {
                RawPreparedTarget::SwapchainBackBuffer
            }
            RawRegisteredRenderTargetKind::OpaqueRenderTarget { source_flags_u32 }
                if self.producer_rva == GENERIC_TEXTURE_RTV_PRODUCER_RVA
                    && source_flags_u32 & 2 != 0 =>
            {
                RawPreparedTarget::OpaqueRenderTarget {
                    attachment_ordinal: self.attachment_ordinal,
                }
            }
            _ => return Err("render-target producer/kind receipt is not proven".into()),
        };
        let (resource_token, producer_token) = self.canonical_tokens()?;
        Ok(RawRenderTargetRegistrationReceipt {
            registration_sequence: self.registration_sequence,
            capture_sequence: self.capture_sequence,
            producer_rva: self.producer_rva,
            recreate_generation: self.recreate_generation,
            attachment_ordinal: self.attachment_ordinal,
            kind: self.kind.clone(),
            view_ptr: self.view_ptr,
            resource_ptr: self.resource_ptr,
            resource_desc: self.resource_desc.clone(),
            rtv_desc: self.rtv_desc.clone(),
            resource_identity_sha256: resource_token,
            view_identity_sha256: producer_token,
            target: RawOutputTargetSemantic {
                view_ptr: self.view_ptr,
                prepared_target,
                resource_token: None,
                producer_pass_token: None,
                persistent_content: None,
            },
        })
    }

    fn canonical_tokens(&self) -> Result<(String, String), String> {
        let (kind_u8, buffer_index) = match self.kind {
            RawRegisteredRenderTargetKind::SwapchainBackBuffer { buffer_index } => {
                (0u8, buffer_index)
            }
            RawRegisteredRenderTargetKind::OpaqueRenderTarget { .. } => (1u8, u32::MAX),
        };
        let mut resource = b"hoi4.xn15l.rt-resource.v1\0".to_vec();
        put_u64(&mut resource, self.registration_sequence);
        put_u64(&mut resource, self.producer_rva);
        put_u64(&mut resource, self.recreate_generation);
        resource.push(kind_u8);
        put_u32(&mut resource, buffer_index);
        for value in [
            self.resource_desc.width,
            self.resource_desc.height,
            self.resource_desc.mip_levels,
            self.resource_desc.array_size,
            self.resource_desc.dxgi_format_u32,
            self.resource_desc.sample_count,
            self.resource_desc.sample_quality,
            self.resource_desc.usage_u32,
            self.resource_desc.bind_flags_u32,
            self.resource_desc.cpu_access_flags_u32,
            self.resource_desc.misc_flags_u32,
        ] {
            put_u32(&mut resource, value);
        }
        let resource_digest = Sha256::digest(&resource);
        let resource_token = format!("sha256:{resource_digest:x}");
        let mut view = b"hoi4.xn15l.rt-view.v1\0".to_vec();
        view.extend_from_slice(&resource_digest);
        put_u64(&mut view, self.registration_sequence);
        view.push(0);
        put_u32(&mut view, self.attachment_ordinal);
        put_u32(&mut view, self.rtv_desc.dxgi_format_u32);
        put_u32(&mut view, self.rtv_desc.view_dimension_u32);
        put_u32(&mut view, self.rtv_desc.flags_u32);
        for value in self.rtv_desc.union_words_u32 {
            put_u32(&mut view, value);
        }
        Ok((resource_token, format!("sha256:{:x}", Sha256::digest(view))))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawLiveDrawAuthorityObservation {
    pub observation_sequence: u64,
    pub identity: RawDrawIdentity,
    pub content_identity: RawDrawContentIdentity,
    pub program_registration: RawProgramRegistrationObservation,
    pub source_open: RawEffectiveVfsOpenObservation,
    pub vertex_creation: RawShaderCreation,
    pub pixel_creation: RawShaderCreation,
    pub vertex_reflection: RawShaderBytecodeReflectionObservation,
    pub pixel_reflection: RawShaderBytecodeReflectionObservation,
    pub ordered_color_targets: Vec<RawRenderTargetRegistrationReceipt>,
    pub depth_target: Option<RawRenderTargetRegistrationReceipt>,
}

pub fn materialize_d77_draw_authority(
    observed: &RawLiveDrawAuthorityObservation,
) -> Result<(RawProgramIdentity, RawDrawSemantics), String> {
    let program = materialize_program_identity(
        &observed.program_registration,
        &observed.source_open,
        &observed.vertex_reflection,
        &observed.pixel_reflection,
    )?;
    let vertex = observed
        .vertex_reflection
        .materialize_stage_semantics(&observed.vertex_creation)?;
    let pixel = observed
        .pixel_reflection
        .materialize_stage_semantics(&observed.pixel_creation)?;
    if observed.observation_sequence != observed.program_registration.observation_sequence
        || observed.ordered_color_targets.is_empty()
        || observed
            .ordered_color_targets
            .iter()
            .any(|receipt| receipt.capture_sequence != observed.observation_sequence)
        || observed
            .depth_target
            .as_ref()
            .is_some_and(|receipt| receipt.capture_sequence != observed.observation_sequence)
    {
        return Err("d77 draw authority observation is incomplete or stale".into());
    }
    let mut texture_semantics = vertex.ordered_texture_semantics;
    texture_semantics.extend(pixel.ordered_texture_semantics);
    let mut sampler_semantics = vertex.ordered_sampler_semantics;
    sampler_semantics.extend(pixel.ordered_sampler_semantics);
    let mut constant_buffer_semantics = vertex.ordered_constant_buffer_semantics;
    constant_buffer_semantics.extend(pixel.ordered_constant_buffer_semantics);
    Ok((
        program,
        RawDrawSemantics {
            observation_sequence: observed.observation_sequence,
            identity: observed.identity.clone(),
            content_identity: observed.content_identity.clone(),
            ordered_texture_semantics: texture_semantics,
            ordered_sampler_semantics: sampler_semantics,
            ordered_constant_buffer_semantics: constant_buffer_semantics,
            ordered_color_targets: observed
                .ordered_color_targets
                .iter()
                .map(|receipt| receipt.target.clone())
                .collect(),
            depth_target: observed
                .depth_target
                .as_ref()
                .map(|receipt| receipt.target.clone()),
        },
    ))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawStaticTextureSourceObservation {
    pub stage: RawShaderStage,
    pub slot: u8,
    pub source: RawStaticShaderResourceSource,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawStaticAuthorityObservation {
    pub capture_sequence: u64,
    pub shader_program_ptr: usize,
    pub program_registration: RawProgramRegistrationObservation,
    pub source_open: RawEffectiveVfsOpenObservation,
    pub vertex_creation: RawShaderCreation,
    pub pixel_creation: RawShaderCreation,
    pub vertex_reflection: RawShaderBytecodeReflectionObservation,
    pub pixel_reflection: RawShaderBytecodeReflectionObservation,
    pub ordered_texture_sources: Vec<RawStaticTextureSourceObservation>,
    pub ordered_color_targets: Vec<RawRenderTargetRegistrationReceipt>,
    pub depth_target: Option<RawRenderTargetRegistrationReceipt>,
    pub ordered_content: Vec<RawStaticContentJoin>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawStaticAuthorityMaterialization {
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

pub fn materialize_d80_static_authority(
    observed: &RawStaticAuthorityObservation,
) -> Result<RawStaticAuthorityMaterialization, String> {
    let program_identity = materialize_program_identity(
        &observed.program_registration,
        &observed.source_open,
        &observed.vertex_reflection,
        &observed.pixel_reflection,
    )?;
    let vertex = observed
        .vertex_reflection
        .materialize_stage_semantics(&observed.vertex_creation)?;
    let pixel = observed
        .pixel_reflection
        .materialize_stage_semantics(&observed.pixel_creation)?;
    let mut textures = vertex.ordered_texture_semantics;
    textures.extend(pixel.ordered_texture_semantics);
    if observed.capture_sequence == 0
        || observed.shader_program_ptr != program_identity.shader_program_ptr
        || observed.program_registration.observation_sequence != observed.capture_sequence
        || textures.len() != observed.ordered_texture_sources.len()
        || observed.ordered_content.is_empty()
        || observed.ordered_color_targets.is_empty()
    {
        return Err("d80 static authority observation is incomplete".into());
    }
    let ordered_texture_semantics = textures
        .into_iter()
        .zip(&observed.ordered_texture_sources)
        .map(|(semantic, source)| {
            if semantic.stage != source.stage || semantic.slot != source.slot {
                return Err("d80 reflected texture/source order disagrees".to_string());
            }
            Ok(RawStaticShaderResourceSemantic {
                stage: semantic.stage,
                slot: semantic.slot,
                semantic_token: semantic.semantic_token,
                source: source.source.clone(),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut samplers = vertex.ordered_sampler_semantics;
    samplers.extend(pixel.ordered_sampler_semantics);
    let mut constant_buffers = vertex.ordered_constant_buffer_semantics;
    constant_buffers.extend(pixel.ordered_constant_buffer_semantics);
    for receipt in observed
        .ordered_color_targets
        .iter()
        .chain(observed.depth_target.iter())
    {
        if receipt.capture_sequence != observed.capture_sequence {
            return Err("d80 output target receipt is stale".into());
        }
    }
    let mut content_keys = BTreeSet::new();
    if observed.ordered_content.iter().any(|join| {
        join.content_key.trim().is_empty()
            || !valid_raw_content(&join.content)
            || !content_keys.insert(join.content_key.clone())
    }) {
        return Err("d80 content joins are incomplete or repeated".into());
    }
    Ok(RawStaticAuthorityMaterialization {
        capture_sequence: observed.capture_sequence,
        shader_program_ptr: observed.shader_program_ptr,
        program_identity,
        ordered_texture_semantics,
        ordered_sampler_semantics: samplers,
        ordered_constant_buffer_semantics: constant_buffers,
        ordered_color_targets: observed
            .ordered_color_targets
            .iter()
            .map(|receipt| receipt.target.clone())
            .collect(),
        depth_target: observed
            .depth_target
            .as_ref()
            .map(|receipt| receipt.target.clone()),
        ordered_content: observed.ordered_content.clone(),
    })
}

fn validate_signatures(rows: &[RawShaderSignatureParameter], label: &str) -> Result<(), String> {
    let mut keys = BTreeSet::new();
    for row in rows {
        if row.semantic_token.trim().is_empty()
            || row.mask_u8 == 0
            || row.mask_u8 & !0x0f != 0
            || row.read_write_mask_u8 & !row.mask_u8 != 0
            || !keys.insert((
                row.semantic_token.to_ascii_lowercase(),
                row.semantic_index,
                row.register_index,
                row.stream_index,
            ))
        {
            return Err(format!(
                "reflected {label} signature is invalid or repeated"
            ));
        }
    }
    Ok(())
}

fn validate_reflected_resources(rows: &[RawShaderReflectedResource]) -> Result<(), String> {
    let mut keys = BTreeSet::new();
    for row in rows {
        if row.semantic_token.trim().is_empty() || row.bind_count == 0 {
            return Err("reflected shader resource is invalid".into());
        }
        let end = row
            .bind_point
            .checked_add(row.bind_count)
            .ok_or("reflected shader resource bind range overflows")?;
        for slot in row.bind_point..end {
            if !keys.insert((row.kind, slot)) {
                return Err("reflected shader resource bind ranges overlap".into());
            }
        }
        match row.kind {
            RawReflectedResourceKind::ConstantBuffer
                if row
                    .constant_buffer_byte_count
                    .is_none_or(|bytes| bytes == 0)
                    || row
                        .constant_buffer_layout_token
                        .as_deref()
                        .is_none_or(str::is_empty) =>
            {
                return Err("reflected constant buffer lacks exact layout authority".into())
            }
            RawReflectedResourceKind::ConstantBuffer => {
                let byte_count = row.constant_buffer_byte_count.expect("checked");
                let mut prior_end = 0u32;
                for variable in &row.ordered_variables {
                    let end = variable
                        .start_offset
                        .checked_add(variable.byte_count)
                        .ok_or("reflected constant-buffer variable overflows")?;
                    if variable.name.trim().is_empty()
                        || variable.type_token.trim().is_empty()
                        || variable.byte_count == 0
                        || variable.start_offset < prior_end
                        || end > byte_count
                    {
                        return Err("reflected constant-buffer layout is invalid".into());
                    }
                    prior_end = end;
                }
            }
            _ if row.constant_buffer_byte_count.is_some()
                || row.constant_buffer_layout_token.is_some()
                || !row.ordered_variables.is_empty() =>
            {
                return Err("non-cbuffer reflection row carries cbuffer layout".into())
            }
            _ => {}
        }
    }
    Ok(())
}

fn stage_bit(stage: RawShaderStage) -> u8 {
    match stage {
        RawShaderStage::Vertex => 1,
        RawShaderStage::Pixel => 2,
        RawShaderStage::Geometry => 4,
        RawShaderStage::Hull => 8,
        RawShaderStage::Domain => 16,
        RawShaderStage::Compute => 32,
    }
}

fn valid_raw_content(content: &RawVfsContentIdentity) -> bool {
    normalize_game_relative_path(&content.logical_path)
        .is_ok_and(|path| path == content.logical_path)
        && !content.layer_id.trim().is_empty()
        && matches!(
            content.layer_kind.as_str(),
            "base" | "integrated_dlc" | "dlc" | "mod"
        )
        && valid_lower_sha256(&content.source_sha256)
        && valid_lower_sha256(&content.payload_sha256)
}

fn valid_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;
    use portable_content_manifest::effective_vfs::{
        EffectiveVfs, EffectiveVfsLayerMount, EffectiveVfsResolvedFile,
    };
    use portable_content_manifest::{ContentLayerKind, LayerStorageKind};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "render-live-authority-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn write(root: &Path, rel: &str, bytes: &[u8]) {
        let path = root.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    fn effective_mod_winner() -> (PathBuf, EffectiveVfsResolvedFile) {
        let root = temp_root("vfs");
        let base = root.join("base");
        let modification = root.join("mod");
        write(&base, "gfx/FX/program.shader", b"base shader");
        write(&base, "interface/base_only.gui", b"base GUI");
        write(&modification, "gfx/FX/program.shader", b"mod shader");
        write(&modification, "interface/mod_only.gui", b"mod GUI");
        let vfs = EffectiveVfs::new(
            "a".repeat(64),
            [
                EffectiveVfsLayerMount {
                    ordinal: 0,
                    layer_kind: ContentLayerKind::Base,
                    layer_id: "base".into(),
                    storage_kind: LayerStorageKind::Directory,
                    local_source: base,
                    source_container_sha256: None,
                    replace_paths: Vec::new(),
                },
                EffectiveVfsLayerMount {
                    ordinal: 7,
                    layer_kind: ContentLayerKind::Mod,
                    layer_id: "mod/authority.mod".into(),
                    storage_kind: LayerStorageKind::Directory,
                    local_source: modification,
                    source_container_sha256: None,
                    replace_paths: vec!["interface".into()],
                },
            ],
        )
        .unwrap();
        assert!(vfs.resolve("interface/base_only.gui").unwrap().is_none());
        assert_eq!(
            vfs.resolve_required("interface/mod_only.gui", "mod GUI")
                .unwrap()
                .content
                .winning_layer_ordinal,
            7
        );
        let winner = vfs
            .resolve_required("gfx/FX/program.shader", "program")
            .unwrap();
        assert_eq!(winner.bytes, b"mod shader");
        assert_eq!(winner.content.layer_id, "mod/authority.mod");
        (root, winner)
    }

    fn creation(
        stage: RawShaderStage,
        sequence: u64,
        shader_ptr: usize,
        bytes: &[u8],
        sha256: &str,
    ) -> RawShaderCreation {
        RawShaderCreation {
            creation_sequence: sequence,
            shader_ptr,
            stage,
            compiler_identity: "observed-d3dcompiler".into(),
            target_profile: match stage {
                RawShaderStage::Vertex => "vs_5_0",
                RawShaderStage::Pixel => "ps_5_0",
                _ => "other_5_0",
            }
            .into(),
            compiler_flags_u64: 0,
            compiler_invocation: None,
            bytecode: RawPayloadSnapshot {
                captured_bytes: bytes.to_vec(),
                captured_sha256: sha256.into(),
                source: RawPayloadSource::RuntimeGenerated {
                    reason: "observed_compiler_output".into(),
                },
            },
        }
    }

    fn reflection(
        stage: RawShaderStage,
        sequence: u64,
        creation: &RawShaderCreation,
        program_ptr: usize,
        entry: &str,
        resources: Vec<RawShaderReflectedResource>,
    ) -> RawShaderBytecodeReflectionObservation {
        RawShaderBytecodeReflectionObservation {
            reflection_sequence: sequence,
            creation_sequence: creation.creation_sequence,
            shader_ptr: creation.shader_ptr,
            shader_program_ptr: program_ptr,
            stage,
            bytecode_sha256: creation.bytecode.captured_sha256.clone(),
            bytecode_bytes: creation.bytecode.captured_bytes.len() as u64,
            entry_token: entry.into(),
            target_profile: creation.target_profile.clone(),
            creator_token: "ID3D11Device::CreateShader+D3DReflect".into(),
            ordered_inputs: vec![RawShaderSignatureParameter {
                semantic_token: "POSITION".into(),
                semantic_index: 0,
                register_index: 0,
                system_value_type_u32: 0,
                component_type_u32: 3,
                mask_u8: 0x07,
                read_write_mask_u8: 0x07,
                stream_index: 0,
                min_precision_u32: 0,
            }],
            ordered_outputs: vec![RawShaderSignatureParameter {
                semantic_token: if stage == RawShaderStage::Vertex {
                    "SV_POSITION"
                } else {
                    "SV_TARGET"
                }
                .into(),
                semantic_index: 0,
                register_index: 0,
                system_value_type_u32: 1,
                component_type_u32: 3,
                mask_u8: 0x0f,
                read_write_mask_u8: 0x0f,
                stream_index: 0,
                min_precision_u32: 0,
            }],
            ordered_resources: resources,
        }
    }

    #[test]
    fn effective_vfs_open_correlates_to_exact_d3d_creation() {
        let (root, winner) = effective_mod_winner();
        let open = RawEffectiveVfsOpenObservation::from_used_content(
            3,
            &winner.content,
            0x100,
            Some(0x200),
        )
        .unwrap();
        let creation = RawD3dContentCreationObservation {
            creation_sequence: 4,
            object_ptr: 0x300,
            object_kind: RawD3dCreatedObjectKind::Texture2d,
            originating_open_sequence: 3,
            source_stream_handle: 0x100,
            source_data_ptr: Some(0x200),
            source_byte_offset: 0,
            source_byte_count: winner.content.payload_bytes,
            source_range_sha256: winner.content.payload_sha256.clone(),
        };
        let receipt = correlate_content_creation(&open, &creation).unwrap();
        assert_eq!(receipt.content.winning_layer_ordinal, 7);
        assert_eq!(receipt.content.logical_path, "gfx/fx/program.shader");
        let mut stale = creation;
        stale.originating_open_sequence = 2;
        assert!(correlate_content_creation(&open, &stale).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reflection_order_and_exact_entries_materialize_d77_and_d80() {
        let (root, winner) = effective_mod_winner();
        let source_open = RawEffectiveVfsOpenObservation::from_used_content(
            10,
            &winner.content,
            0x100,
            Some(0x200),
        )
        .unwrap();
        let vs = creation(RawShaderStage::Vertex, 11, 0x501, b"vs", &"b".repeat(64));
        let ps = creation(RawShaderStage::Pixel, 12, 0x502, b"ps", &"c".repeat(64));
        let vs_reflection = reflection(
            RawShaderStage::Vertex,
            13,
            &vs,
            0x900,
            "VertexMain",
            Vec::new(),
        );
        let ps_reflection = reflection(
            RawShaderStage::Pixel,
            14,
            &ps,
            0x900,
            "PixelMain",
            vec![
                RawShaderReflectedResource {
                    semantic_token: "DiffuseMap".into(),
                    kind: RawReflectedResourceKind::Texture,
                    bind_point: 0,
                    bind_count: 1,
                    flags_u32: 0,
                    return_type_u32: 5,
                    dimension_u32: 4,
                    sample_count: 0,
                    constant_buffer_byte_count: None,
                    constant_buffer_layout_token: None,
                    ordered_variables: Vec::new(),
                },
                RawShaderReflectedResource {
                    semantic_token: "DiffuseSampler".into(),
                    kind: RawReflectedResourceKind::Sampler,
                    bind_point: 0,
                    bind_count: 1,
                    flags_u32: 0,
                    return_type_u32: 0,
                    dimension_u32: 0,
                    sample_count: 0,
                    constant_buffer_byte_count: None,
                    constant_buffer_layout_token: None,
                    ordered_variables: Vec::new(),
                },
                RawShaderReflectedResource {
                    semantic_token: "MaterialBuffer".into(),
                    kind: RawReflectedResourceKind::ConstantBuffer,
                    bind_point: 1,
                    bind_count: 1,
                    flags_u32: 0,
                    return_type_u32: 0,
                    dimension_u32: 0,
                    sample_count: 0,
                    constant_buffer_byte_count: Some(16),
                    constant_buffer_layout_token: Some("float4:tint@0".into()),
                    ordered_variables: vec![RawReflectedConstantBufferVariable {
                        name: "Tint".into(),
                        start_offset: 0,
                        byte_count: 16,
                        flags_u32: 0,
                        type_token: "float4".into(),
                    }],
                },
            ],
        );
        let registration = RawProgramRegistrationObservation {
            observation_sequence: 20,
            shader_program_ptr: 0x900,
            source_open_sequence: 10,
            effect_token: "observed_effect".into(),
            vertex_entry_token: "VertexMain".into(),
            pixel_entry_token: "PixelMain".into(),
            ordered_defines: vec![RawShaderDefine {
                name: "MOD_VARIANT".into(),
                value: "1".into(),
                scope: "effect".into(),
            }],
        };
        let target = RawRenderTargetRegistrationObservation {
            registration_sequence: 19,
            capture_sequence: 20,
            producer_rva: GENERIC_TEXTURE_RTV_PRODUCER_RVA,
            recreate_generation: 0,
            attachment_ordinal: 0,
            kind: RawRegisteredRenderTargetKind::OpaqueRenderTarget {
                source_flags_u32: 2,
            },
            view_ptr: 0xa00,
            resource_ptr: 0xa10,
            resource_desc: RawTexture2dDesc {
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
            rtv_desc: RawResourceViewDesc {
                dxgi_format_u32: 28,
                view_dimension_u32: 4,
                flags_u32: 0,
                union_words_u32: [0; 4],
            },
        }
        .materialize()
        .unwrap();
        let identity = RawDrawIdentity::Particle {
            effect_instance_id: 1,
            registration_token: "observed_registration".into(),
            definition_source_token: "observed_definition".into(),
            builder_call_ordinal: 0,
            packet_ordinal: 0,
            draw_order: 0,
            shader_program_ptr: 0x900,
        };
        let content_identity = RawDrawContentIdentity::Particle {
            registration_token: "observed_registration".into(),
            definition_source_token: "observed_definition".into(),
            subsystem_selector_u32: 0,
            definition_content: source_open.content.clone(),
            texture_content: source_open.content.clone(),
            authored_shader_token: "observed_effect".into(),
            engine_effect_token: "observed_effect".into(),
            atlas_columns_u32: 1,
            atlas_rows_u32: 1,
            atlas_half_pixel_width_f32_bits: 0.5f32.to_bits(),
            atlas_half_pixel_height_f32_bits: 0.5f32.to_bits(),
        };
        let observed = RawLiveDrawAuthorityObservation {
            observation_sequence: 20,
            identity,
            content_identity,
            program_registration: registration.clone(),
            source_open: source_open.clone(),
            vertex_creation: vs.clone(),
            pixel_creation: ps.clone(),
            vertex_reflection: vs_reflection.clone(),
            pixel_reflection: ps_reflection.clone(),
            ordered_color_targets: vec![target.clone()],
            depth_target: None,
        };
        let (program, semantics) = materialize_d77_draw_authority(&observed).unwrap();
        assert_eq!(program.source.winning_layer_ordinal, 7);
        assert_eq!(
            semantics.ordered_texture_semantics[0].semantic_token,
            "DiffuseMap"
        );
        assert_eq!(
            semantics.ordered_constant_buffer_semantics[0].stage_mask_u8,
            2
        );
        assert_eq!(
            observed
                .pixel_reflection
                .materialize_stage_semantics(&observed.pixel_creation)
                .unwrap()
                .ordered_resources
                .len(),
            3
        );

        let static_observed = RawStaticAuthorityObservation {
            capture_sequence: 20,
            shader_program_ptr: 0x900,
            program_registration: registration,
            source_open: source_open.clone(),
            vertex_creation: vs,
            pixel_creation: ps,
            vertex_reflection: vs_reflection,
            pixel_reflection: ps_reflection,
            ordered_texture_sources: vec![RawStaticTextureSourceObservation {
                stage: RawShaderStage::Pixel,
                slot: 0,
                source: RawStaticShaderResourceSource::UsedContent,
            }],
            ordered_color_targets: vec![target],
            depth_target: None,
            ordered_content: vec![RawStaticContentJoin {
                content_key: "observed_program_source".into(),
                content: source_open.content,
            }],
        };
        let static_authority = materialize_d80_static_authority(&static_observed).unwrap();
        assert_eq!(static_authority.ordered_texture_semantics.len(), 1);
        assert_eq!(static_authority.ordered_sampler_semantics.len(), 1);
        assert_eq!(static_authority.ordered_constant_buffer_semantics.len(), 1);

        let ordered = BTreeSet::from([
            RawReflectedResourceKind::Other(2),
            RawReflectedResourceKind::ConstantBuffer,
            RawReflectedResourceKind::Sampler,
            RawReflectedResourceKind::Texture,
            RawReflectedResourceKind::Other(1),
        ])
        .into_iter()
        .collect::<Vec<_>>();
        assert_eq!(
            ordered,
            vec![
                RawReflectedResourceKind::Texture,
                RawReflectedResourceKind::Sampler,
                RawReflectedResourceKind::ConstantBuffer,
                RawReflectedResourceKind::Other(1),
                RawReflectedResourceKind::Other(2),
            ]
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn winner_index_binding_rejects_omission_order_and_hash_drift() {
        let (root, winner) = effective_mod_winner();
        let content = RawVfsContentIdentity {
            logical_path: winner.content.normalized_game_relative_path.clone(),
            winning_layer_ordinal: winner.content.winning_layer_ordinal,
            layer_kind: "mod".into(),
            layer_id: winner.content.layer_id.clone(),
            source_sha256: winner.content.source_sha256.clone(),
            payload_sha256: winner.content.payload_sha256.clone(),
            payload_bytes: winner.content.payload_bytes,
        };
        let row = RawEffectiveVfsWinnerRow {
            content: content.clone(),
            source_container_sha256: None,
            source_byte_offset: 0,
            source_byte_count: content.payload_bytes,
            source_range_sha256: content.payload_sha256.clone(),
        };
        let hash =
            RawEffectiveVfsWinnerIndexBinding::canonical_rows_sha256(&[row.clone()]).unwrap();
        let binding = RawEffectiveVfsWinnerIndexBinding {
            process_start_nonce: 99,
            authority_manifest_sha256: winner.content.effective_vfs_manifest_sha256.clone(),
            declared_row_count: 1,
            ordered_rows_sha256: hash,
            ordered_rows: vec![row.clone()],
        };
        binding.validate().unwrap();
        let open = RawEffectiveVfsOpenObservation::from_used_content(
            44,
            &winner.content,
            0x100,
            Some(0x200),
        )
        .unwrap();
        binding.authorize_open(&open).unwrap();

        let mut omitted = binding.clone();
        omitted.declared_row_count = 2;
        assert!(omitted.validate().is_err());
        let mut drifted = binding.clone();
        drifted.ordered_rows_sha256 = "f".repeat(64);
        assert!(drifted.validate().is_err());
        assert!(
            RawEffectiveVfsWinnerIndexBinding::canonical_rows_sha256(&[row.clone(), row,]).is_err()
        );
        fs::remove_dir_all(root).unwrap();
    }
}
