//! Neutral d80 static/post-process capture rows.
//!
//! Pass/program identities are opaque SHA-256 values derived by the observer
//! from captured registration/state graphs. They carry no vanilla role name
//! and never select playback behavior.

use super::*;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawCaptureLaunchEpoch {
    pub process_id: u32,
    pub process_start_nonce: u64,
    pub observer_installed_qpc: u64,
    pub d3d_device_created_qpc: u64,
    pub first_creation_sequence: u64,
    pub hook_epoch_nonce: u64,
    pub hook_set_receipt_sha256: String,
    /// Final committed transcript receipt. Rows remain buffered and cannot be
    /// drained while this value is pending.
    pub capture_receipt_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawStaticContentJoin {
    /// Opaque portable manifest key selected by the recorder. It is never a
    /// filename, role, or catalog lookup hint.
    pub content_key: String,
    pub content: RawVfsContentIdentity,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RawStaticShaderResourceSource {
    UsedContent,
    /// Exact selected-Present SnowMudData upload joined by the process-local
    /// resource identity while capture is still in-process, after the exact
    /// draw-time resource snapshot matches H row-for-row. Only the pointer-free
    /// producer ordinal and payload digest survive promotion.
    SnowMudTexture {
        producer_ordinal: u64,
        payload_sha256: String,
    },
    PreparedTarget {
        target: RawOutputTargetSemantic,
        producer_capture_sequence: u64,
    },
    PoliticalTexture {
        /// Exact portable political upload family observed at the upload/bind
        /// join; no texture identity is inferred from a slot.
        texture_kind: String,
        target_binding_identity: String,
        upload_order: u32,
        payload_sha256: String,
    },
    TiledLightIndex {
        writer_index: u64,
        payload_sha256: String,
    },
    TiledLightData {
        writer_index: u64,
        payload_sha256: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawStaticShaderResourceSemantic {
    pub stage: RawShaderStage,
    pub slot: u8,
    pub semantic_token: String,
    pub source: RawStaticShaderResourceSource,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RawStaticDrawCall {
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
pub struct RawStaticD3dDrawState {
    pub capture_sequence: u64,
    pub shader_program_ptr: usize,
    pub render_context_ptr: usize,
    pub device_context_ptr: usize,
    pub vertex_shader_ptr: usize,
    pub pixel_shader_ptr: usize,
    pub geometry_shader_ptr: Option<usize>,
    pub hull_shader_ptr: Option<usize>,
    pub domain_shader_ptr: Option<usize>,
    pub compute_shader_ptr: Option<usize>,
    pub geometry_entry_token: Option<String>,
    pub hull_entry_token: Option<String>,
    pub domain_entry_token: Option<String>,
    pub compute_entry_token: Option<String>,
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
    pub ordered_stream_output_buffers: Vec<RawStaticStreamOutputBuffer>,
    pub ordered_color_target_view_ptrs: Vec<usize>,
    pub depth_stencil_view_ptr: Option<usize>,
    pub ordered_viewports_f32_bits: Vec<[u32; 6]>,
    pub ordered_scissor_rects_i32: Vec<[i32; 4]>,
    pub draw_call: RawStaticDrawCall,
    pub ordered_texture_semantics: Vec<RawStaticShaderResourceSemantic>,
    pub ordered_sampler_semantics: Vec<RawSamplerSemantic>,
    pub ordered_constant_buffer_semantics: Vec<RawConstantBufferSemantic>,
    pub ordered_color_targets: Vec<RawOutputTargetSemantic>,
    pub depth_target: Option<RawOutputTargetSemantic>,
    /// Exact effective-VFS winners observed while resolving this program.
    pub ordered_content: Vec<RawStaticContentJoin>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawStaticStreamOutputBuffer {
    pub slot: u32,
    pub buffer_ptr: usize,
    /// Exact D3D11 offset; `u32::MAX` preserves append semantics.
    pub byte_offset: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RawStaticOrderOperation {
    Draw {
        sequence: u64,
        capture_sequence: u64,
    },
    ClearColor {
        sequence: u64,
        capture_sequence: u64,
        target_object_ptr: usize,
        target_view_ptr: usize,
        target: RawOutputTargetSemantic,
        rgba_f32_bits: [u32; 4],
    },
    ClearDepthStencil {
        sequence: u64,
        capture_sequence: u64,
        target_object_ptr: usize,
        target_view_ptr: usize,
        target: RawOutputTargetSemantic,
        clear_depth: bool,
        clear_stencil: bool,
        depth_f32_bits: u32,
        stencil_u8: u8,
    },
}

impl RawStaticOrderOperation {
    pub fn sequence(&self) -> u64 {
        match self {
            Self::Draw { sequence, .. }
            | Self::ClearColor { sequence, .. }
            | Self::ClearDepthStencil { sequence, .. } => *sequence,
        }
    }
    pub fn capture_sequence(&self) -> u64 {
        match self {
            Self::Draw {
                capture_sequence, ..
            }
            | Self::ClearColor {
                capture_sequence, ..
            }
            | Self::ClearDepthStencil {
                capture_sequence, ..
            } => *capture_sequence,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawStaticProgramCapture {
    pub process: RawProcessEpoch,
    pub launch_epoch: RawCaptureLaunchEpoch,
    pub submit_index: u64,
    pub frame_sequence: u64,
    pub ordered_operations: Vec<RawStaticOrderOperation>,
    pub draws: Vec<RawStaticD3dDrawState>,
    /// Shared immutable creation/snapshot registry for every pointer above.
    pub catalog: RawRenderCatalogCapture,
}

fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

impl RawStaticProgramCapture {
    pub fn validate_complete(&self) -> Result<(), String> {
        self.catalog.validate_launch_binding()?;
        if self.process.process_id == 0
            || self.process.process_start_nonce == 0
            || self.submit_index == 0
            || self.frame_sequence == 0
        {
            return Err("static capture has an incomplete process/frame/submit epoch".into());
        }
        if self.launch_epoch.process_id != self.process.process_id
            || self.launch_epoch.process_start_nonce != self.process.process_start_nonce
            || self.launch_epoch.observer_installed_qpc == 0
            || self.launch_epoch.d3d_device_created_qpc == 0
            || self.launch_epoch.observer_installed_qpc > self.launch_epoch.d3d_device_created_qpc
            || self.launch_epoch.first_creation_sequence == 0
            || self.launch_epoch.hook_epoch_nonce == 0
            || !valid_hash(&self.launch_epoch.hook_set_receipt_sha256)
            || !valid_hash(&self.launch_epoch.capture_receipt_sha256)
        {
            return Err("static capture lacks proof that observation preceded D3D creation".into());
        }
        if self.catalog.process != self.process
            || self.catalog.launch_epoch != self.launch_epoch
            || self.catalog.submit_index != self.submit_index
            || self.catalog.frame_sequence != self.frame_sequence
        {
            return Err("static capture and immutable registry epochs differ".into());
        }
        if self.ordered_operations.is_empty() || self.draws.is_empty() {
            return Err("static capture contains no authenticated draw program".into());
        }
        let mut prior = None;
        let mut draw_refs = BTreeSet::new();
        for operation in &self.ordered_operations {
            let sequence = operation.sequence();
            if sequence == 0
                || operation.capture_sequence() == 0
                || prior.is_some_and(|p| sequence != p + 1)
            {
                return Err("static operation order is zero, gapped, or repeated".into());
            }
            prior = Some(sequence);
            match operation {
                RawStaticOrderOperation::Draw {
                    capture_sequence, ..
                } => {
                    if !draw_refs.insert(*capture_sequence) {
                        return Err("static draw is referenced more than once".into());
                    }
                }
                RawStaticOrderOperation::ClearColor {
                    capture_sequence,
                    target_object_ptr,
                    target_view_ptr,
                    target,
                    rgba_f32_bits,
                    ..
                } => {
                    if *target_object_ptr == 0
                        || *target_view_ptr == 0
                        || target.view_ptr != *target_view_ptr
                        || rgba_f32_bits
                            .iter()
                            .any(|v| !f32::from_bits(*v).is_finite())
                        || !self.catalog.render_target_view_snapshots.iter().any(|v| {
                            v.view_ptr == *target_view_ptr
                                && v.snapshot_sequence == *capture_sequence
                        })
                    {
                        return Err(
                            "static color clear is incomplete or lacks an exact output snapshot"
                                .into(),
                        );
                    }
                }
                RawStaticOrderOperation::ClearDepthStencil {
                    capture_sequence,
                    target_object_ptr,
                    target_view_ptr,
                    target,
                    clear_depth,
                    clear_stencil,
                    depth_f32_bits,
                    ..
                } => {
                    let depth = f32::from_bits(*depth_f32_bits);
                    if *target_object_ptr == 0
                        || *target_view_ptr == 0
                        || target.view_ptr != *target_view_ptr
                        || (!*clear_depth && !*clear_stencil)
                        || (*clear_depth && (!depth.is_finite() || !(0.0..=1.0).contains(&depth)))
                        || !self.catalog.render_target_view_snapshots.iter().any(|v| {
                            v.view_ptr == *target_view_ptr
                                && v.snapshot_sequence == *capture_sequence
                        })
                    {
                        return Err(
                            "static depth clear is incomplete or lacks an exact output snapshot"
                                .into(),
                        );
                    }
                }
            }
        }
        let mut draws = BTreeMap::new();
        for draw in &self.draws {
            if draw.capture_sequence == 0
                || draw.shader_program_ptr == 0
                || draw.render_context_ptr == 0
                || draw.device_context_ptr == 0
                || draw.vertex_shader_ptr == 0
                || draw.pixel_shader_ptr == 0
                || draw.dynamic_shader_linkage_present
                || draw.ordered_color_targets.is_empty()
                || draw.ordered_content.is_empty()
                || draws.insert(draw.capture_sequence, draw).is_some()
            {
                return Err("static draw identity/state is incomplete or duplicated".into());
            }
            for (pointer, token) in [
                (draw.geometry_shader_ptr, draw.geometry_entry_token.as_ref()),
                (draw.hull_shader_ptr, draw.hull_entry_token.as_ref()),
                (draw.domain_shader_ptr, draw.domain_entry_token.as_ref()),
                (draw.compute_shader_ptr, draw.compute_entry_token.as_ref()),
            ] {
                if pointer.is_some() != token.is_some()
                    || token.is_some_and(|value| value.trim().is_empty())
                {
                    return Err(
                        "static optional shader pointer/entry identity is incomplete".into(),
                    );
                }
            }
            for semantic in &draw.ordered_texture_semantics {
                match &semantic.source {
                    RawStaticShaderResourceSource::SnowMudTexture {
                        producer_ordinal,
                        payload_sha256,
                    } if *producer_ordinal == 0 || !valid_hash(payload_sha256) => {
                        return Err("static SnowMudTexture identity is incomplete".into())
                    }
                    RawStaticShaderResourceSource::PoliticalTexture {
                        texture_kind,
                        target_binding_identity,
                        payload_sha256,
                        ..
                    } if !matches!(
                        texture_kind.as_str(),
                        "primary"
                            | "secondary"
                            | "gradient_border_channel_1"
                            | "gradient_border_channel_2"
                            | "gradient_border_channel_3"
                    ) || target_binding_identity.trim().is_empty()
                        || !valid_hash(payload_sha256) =>
                    {
                        return Err("static political texture identity is incomplete".into())
                    }
                    RawStaticShaderResourceSource::PoliticalTexture { .. } => {}
                    _ => {}
                }
            }
            let mut content = BTreeSet::new();
            for join in &draw.ordered_content {
                let winner = &join.content;
                if join.content_key.trim().is_empty()
                    || winner.logical_path.trim().is_empty()
                    || !valid_hash(&winner.payload_sha256)
                    || !content.insert(join.content_key.clone())
                {
                    return Err("static draw VFS closure is invalid or duplicated".into());
                }
            }
            self.validate_creation_closure(draw)?;
        }
        if draw_refs != draws.keys().copied().collect() {
            return Err("static draw/order closure is incomplete or contains extras".into());
        }
        Ok(())
    }

    fn validate_creation_closure(&self, draw: &RawStaticD3dDrawState) -> Result<(), String> {
        for pointer in [
            Some(draw.vertex_shader_ptr),
            Some(draw.pixel_shader_ptr),
            draw.geometry_shader_ptr,
            draw.hull_shader_ptr,
            draw.domain_shader_ptr,
            draw.compute_shader_ptr,
        ]
        .into_iter()
        .flatten()
        {
            if !self.catalog.shader_creations.iter().any(|v| {
                v.shader_ptr == pointer
                    && v.creation_sequence >= self.launch_epoch.first_creation_sequence
                    && v.creation_sequence <= draw.capture_sequence
            }) {
                return Err(
                    "static draw references a shader without an immutable creation receipt".into(),
                );
            }
        }
        if !self.catalog.program_identities.iter().any(|v| {
            v.shader_program_ptr == draw.shader_program_ptr
                && v.observation_sequence == draw.capture_sequence
        }) {
            return Err(
                "static draw references a program without an exact authenticated VFS identity"
                    .into(),
            );
        }
        if draw.input_layout_ptr.is_some_and(|p| {
            !self.catalog.input_layout_creations.iter().any(|v| {
                v.input_layout_ptr == p
                    && v.creation_sequence >= self.launch_epoch.first_creation_sequence
                    && v.creation_sequence <= draw.capture_sequence
            })
        }) || draw.blend_state_ptr.is_some_and(|p| {
            !self.catalog.blend_state_creations.iter().any(|v| {
                v.blend_state_ptr == p
                    && v.creation_sequence >= self.launch_epoch.first_creation_sequence
                    && v.creation_sequence <= draw.capture_sequence
            })
        }) || draw.depth_stencil_state_ptr.is_some_and(|p| {
            !self.catalog.depth_stencil_state_creations.iter().any(|v| {
                v.depth_stencil_state_ptr == p
                    && v.creation_sequence >= self.launch_epoch.first_creation_sequence
                    && v.creation_sequence <= draw.capture_sequence
            })
        }) || draw.rasterizer_state_ptr.is_some_and(|p| {
            !self.catalog.rasterizer_state_creations.iter().any(|v| {
                v.rasterizer_state_ptr == p
                    && v.creation_sequence >= self.launch_epoch.first_creation_sequence
                    && v.creation_sequence <= draw.capture_sequence
            })
        }) {
            return Err(
                "static draw references pipeline state without an immutable creation receipt"
                    .into(),
            );
        }
        if draw.ordered_vertex_buffer_objects.iter().any(|v| {
            !self.catalog.buffer_snapshots.iter().any(|s| {
                s.buffer_ptr == v.buffer_ptr && s.snapshot_sequence == draw.capture_sequence
            })
        }) || draw.index_buffer.as_ref().is_some_and(|v| {
            !self.catalog.buffer_snapshots.iter().any(|s| {
                s.buffer_ptr == v.buffer_ptr && s.snapshot_sequence == draw.capture_sequence
            })
        }) || draw.ordered_constant_buffer_objects.iter().any(|v| {
            !self.catalog.buffer_snapshots.iter().any(|s| {
                s.buffer_ptr == v.object_ptr && s.snapshot_sequence == draw.capture_sequence
            })
        }) || draw.ordered_stream_output_buffers.iter().any(|v| {
            v.buffer_ptr == 0
                || !self.catalog.buffer_snapshots.iter().any(|s| {
                    s.buffer_ptr == v.buffer_ptr && s.snapshot_sequence == draw.capture_sequence
                })
        }) {
            return Err(
                "static draw references a buffer without an exact draw-time snapshot".into(),
            );
        }
        if draw.ordered_texture_objects.iter().any(|v| {
            !self.catalog.shader_resource_view_snapshots.iter().any(|s| {
                s.shader_resource_view_ptr == v.shader_resource_view_ptr
                    && s.resource_ptr == v.resource_ptr
                    && s.snapshot_sequence == draw.capture_sequence
            }) || !self.catalog.texture_resource_snapshots.iter().any(|s| {
                s.resource_ptr == v.resource_ptr && s.snapshot_sequence == draw.capture_sequence
            })
        }) {
            return Err(
                "static draw references an SRV/texture without an exact draw-time snapshot".into(),
            );
        }
        if draw.ordered_sampler_objects.iter().any(|v| {
            !self.catalog.sampler_state_creations.iter().any(|s| {
                s.sampler_state_ptr == v.object_ptr
                    && s.creation_sequence >= self.launch_epoch.first_creation_sequence
                    && s.creation_sequence <= draw.capture_sequence
            })
        }) {
            return Err(
                "static draw references a sampler without an immutable creation receipt".into(),
            );
        }
        if draw.ordered_color_target_view_ptrs.iter().any(|v| {
            !self
                .catalog
                .render_target_view_snapshots
                .iter()
                .any(|s| s.view_ptr == *v && s.snapshot_sequence == draw.capture_sequence)
        }) || draw.depth_stencil_view_ptr.is_some_and(|v| {
            !self
                .catalog
                .render_target_view_snapshots
                .iter()
                .any(|s| s.view_ptr == v && s.snapshot_sequence == draw.capture_sequence)
        }) {
            return Err(
                "static draw references an output view without an exact draw-time snapshot".into(),
            );
        }
        Ok(())
    }
}

/// Hook-facing sink. Implementations must queue the state atomically with the
/// matching raw order event; dropping either side invalidates the whole frame.
pub trait RawStaticProgramCaptureSink {
    fn push_static_draw(&mut self, state: RawStaticD3dDrawState) -> Result<(), String>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_registry(process: RawProcessEpoch) -> RawRenderCatalogCapture {
        RawRenderCatalogCapture {
            process,
            launch_epoch: RawCaptureLaunchEpoch {
                process_id: 7,
                process_start_nonce: 8,
                observer_installed_qpc: 9,
                d3d_device_created_qpc: 10,
                first_creation_sequence: 1,
                hook_epoch_nonce: 11,
                hook_set_receipt_sha256: "b".repeat(64),
                capture_receipt_sha256: "c".repeat(64),
            },
            submit_index: 1,
            frame_sequence: 1,
            draw_states: vec![],
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
        }
    }

    #[test]
    fn late_attach_and_empty_program_fail_closed() {
        let process = RawProcessEpoch {
            process_id: 7,
            process_start_nonce: 8,
            executable_sha256: "a".repeat(64),
        };
        let mut capture = RawStaticProgramCapture {
            process: process.clone(),
            launch_epoch: RawCaptureLaunchEpoch {
                process_id: 7,
                process_start_nonce: 8,
                observer_installed_qpc: 20,
                d3d_device_created_qpc: 10,
                first_creation_sequence: 1,
                hook_epoch_nonce: 11,
                hook_set_receipt_sha256: "b".repeat(64),
                capture_receipt_sha256: "c".repeat(64),
            },
            submit_index: 1,
            frame_sequence: 1,
            ordered_operations: vec![],
            draws: vec![],
            catalog: empty_registry(process),
        };
        assert!(capture
            .validate_complete()
            .unwrap_err()
            .contains("preceded D3D creation"));
        capture.launch_epoch.observer_installed_qpc = 9;
        assert!(capture
            .validate_complete()
            .unwrap_err()
            .contains("no authenticated draw"));
    }
}
