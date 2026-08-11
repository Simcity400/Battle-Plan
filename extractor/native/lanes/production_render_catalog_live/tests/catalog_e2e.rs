use portable_content_manifest::*;
use portable_render_stream::{BufferSource, RenderExecutionRef};
use production_render_catalog_capture_contract::*;
use production_render_catalog_live_lane::*;
use production_render_submit_live_lane::{
    canonicalize_packet, fnv1a32, promote_entity_submission, promote_runtime_execution_order,
    promote_submit, DefinitionIdentity, RawEntitySubmission, RawParticlePacket, RawParticleSubmit,
    RawSkinning, GENERIC_TEXTURE_RTV_PRODUCER_RVA, GPU_INSTANCE_STRIDE, PACKET_BYTES,
};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn payloads() -> BTreeMap<String, Vec<u8>> {
    BTreeMap::from([
        (
            "gfx/particles/rain.asset".into(),
            b"modded-rain-definition".to_vec(),
        ),
        (
            "gfx/entities/unit.asset".into(),
            b"modded-unit-definition".to_vec(),
        ),
        (
            "gfx/fx/runtime.shader".into(),
            b"shader-source-from-base".to_vec(),
        ),
        ("gfx/textures/cloud.dds".into(), vec![41, 42, 43, 44]),
        ("gfx/models/unit.mesh".into(), (0u8..60).collect()),
    ])
}

fn layer(
    ordinal: u32,
    kind: ContentLayerKind,
    id: &str,
    storage: LayerStorageKind,
    replace_paths: &[&str],
) -> RecordedContentLayer {
    RecordedContentLayer {
        ordinal,
        layer_kind: kind,
        layer_id: id.into(),
        display_name: id.into(),
        descriptor_sha256: (kind != ContentLayerKind::Base).then(|| hash(id.as_bytes())),
        source_container_sha256: (storage == LayerStorageKind::ZipArchive)
            .then(|| hash(format!("archive:{id}").as_bytes())),
        storage_kind: storage,
        replace_paths: replace_paths.iter().map(|value| (*value).into()).collect(),
        required_for_playback: true,
    }
}

fn required(
    key: &str,
    path: &str,
    kind: RequiredContentKind,
    layer_ordinal: u32,
    layer_kind: ContentLayerKind,
    layer_id: &str,
    bytes: &[u8],
) -> RequiredContent {
    RequiredContent {
        content_key: key.into(),
        kind,
        content: UsedContentKey {
            effective_vfs_manifest_sha256: "f".repeat(64),
            normalized_game_relative_path: path.into(),
            winning_layer_ordinal: layer_ordinal,
            layer_kind,
            layer_id: layer_id.into(),
            source_sha256: hash(bytes),
            payload_sha256: hash(bytes),
            payload_bytes: bytes.len() as u64,
        },
        observation_count: 1,
        consumers: vec!["extractor.production_render_catalog".into()],
    }
}

fn manifest() -> ContentManifest {
    let bytes = payloads();
    let mut value = ContentManifest {
        schema: SCHEMA.into(),
        product_modset_sha256: "9".repeat(64),
        source_game: SourceGameIdentity {
            version: "1.19.2.0".into(),
            build: "a729".into(),
            executable_sha256: "a".repeat(64),
            build_profile: "a729-870ebf".into(),
        },
        source_profile: SourceProfileIdentity {
            profile_id: "profile".into(),
            playset_id: "modded-playset".into(),
            selection_sha256: "0".repeat(64),
        },
        effective_vfs_manifest_sha256: "f".repeat(64),
        ordered_layers: vec![
            layer(
                0,
                ContentLayerKind::Base,
                "base",
                LayerStorageKind::Directory,
                &[],
            ),
            layer(
                1,
                ContentLayerKind::Dlc,
                "dlc-used",
                LayerStorageKind::ZipArchive,
                &[],
            ),
            layer(
                2,
                ContentLayerKind::Mod,
                "mod-override",
                LayerStorageKind::Directory,
                &["gfx/entities", "gfx/particles"],
            ),
        ],
        dlc_selection: vec![
            DlcSelection {
                dlc_id: "dlc-used".into(),
                enabled: true,
                required_for_playback: true,
            },
            DlcSelection {
                dlc_id: "dlc-not-used".into(),
                enabled: false,
                required_for_playback: false,
            },
        ],
        localisation_language: "l_english".into(),
        required_settings: BTreeMap::from([("graphics.texture_quality".into(), "high".into())]),
        required_content: vec![
            required(
                "entity",
                "gfx/entities/unit.asset",
                RequiredContentKind::Definition,
                2,
                ContentLayerKind::Mod,
                "mod-override",
                &bytes["gfx/entities/unit.asset"],
            ),
            required(
                "mesh",
                "gfx/models/unit.mesh",
                RequiredContentKind::Mesh,
                1,
                ContentLayerKind::Dlc,
                "dlc-used",
                &bytes["gfx/models/unit.mesh"],
            ),
            required(
                "particle",
                "gfx/particles/rain.asset",
                RequiredContentKind::Definition,
                2,
                ContentLayerKind::Mod,
                "mod-override",
                &bytes["gfx/particles/rain.asset"],
            ),
            required(
                "texture",
                "gfx/textures/cloud.dds",
                RequiredContentKind::Texture,
                1,
                ContentLayerKind::Dlc,
                "dlc-used",
                &bytes["gfx/textures/cloud.dds"],
            ),
            required(
                "shader",
                "gfx/fx/runtime.shader",
                RequiredContentKind::ShaderSource,
                0,
                ContentLayerKind::Base,
                "base",
                &bytes["gfx/fx/runtime.shader"],
            ),
        ],
        manifest_digest_sha256: None,
    };
    // A row won by a ZipArchive layer carries the layer's archive digest as
    // its source digest - that is the only shape the recorder's admission
    // emits, and manifest validation now binds it (content_manifest R3-2).
    // Directory-layer wins keep source == payload, which required() already
    // produces.
    for row in value.required_content.iter_mut() {
        if row.content.winning_layer_ordinal == 1 {
            row.content.source_sha256 = hash("archive:dlc-used".as_bytes());
        }
    }
    value.source_profile.selection_sha256 = value.computed_selection_sha256().unwrap();
    value.with_computed_digest().unwrap()
}

fn content(manifest: &ContentManifest, path: &str) -> UsedContentKey {
    manifest
        .required_content
        .iter()
        .find(|value| value.content.normalized_game_relative_path == path)
        .unwrap()
        .content
        .clone()
}

fn observed(key: &UsedContentKey) -> RawVfsContentIdentity {
    RawVfsContentIdentity {
        logical_path: key.normalized_game_relative_path.clone(),
        winning_layer_ordinal: key.winning_layer_ordinal,
        layer_kind: match key.layer_kind {
            ContentLayerKind::Base => "base",
            ContentLayerKind::IntegratedDlc => "integrated_dlc",
            ContentLayerKind::Dlc => "dlc",
            ContentLayerKind::Mod => "mod",
        }
        .into(),
        layer_id: key.layer_id.clone(),
        source_sha256: key.source_sha256.clone(),
        payload_sha256: key.payload_sha256.clone(),
        payload_bytes: key.payload_bytes,
    }
}

#[derive(Clone)]
struct TestVfs(BTreeMap<String, Vec<u8>>);

impl AdmittedVfsPayloads for TestVfs {
    fn payload(&self, content: &UsedContentKey) -> Option<&[u8]> {
        self.0
            .get(&content.normalized_game_relative_path)
            .map(Vec::as_slice)
    }
}

fn runtime_payload(bytes: Vec<u8>, reason: &str) -> RawPayloadSnapshot {
    RawPayloadSnapshot {
        captured_sha256: hash(&bytes),
        captured_bytes: bytes,
        source: RawPayloadSource::RuntimeGenerated {
            reason: reason.into(),
        },
    }
}

fn vfs_payload(key: &UsedContentKey, bytes: Vec<u8>, offset: u64) -> RawPayloadSnapshot {
    RawPayloadSnapshot {
        captured_sha256: hash(&bytes),
        source: RawPayloadSource::VfsRange {
            content: observed(key),
            source_byte_offset: offset,
            byte_count: bytes.len() as u64,
            range_sha256: hash(&bytes),
        },
        captured_bytes: bytes,
    }
}

fn packet() -> RawParticlePacket {
    let token = "rain_effect";
    let mut canonical = [0u8; PACKET_BYTES];
    canonical[0x40..0x44].copy_from_slice(&2u32.to_le_bytes());
    canonical[0x50..0x54].copy_from_slice(&GPU_INSTANCE_STRIDE.to_le_bytes());
    canonical[0x54..0x58].copy_from_slice(&4u32.to_le_bytes());
    canonical[0x58..0x5c].copy_from_slice(&1f32.to_bits().to_le_bytes());
    RawParticlePacket {
        effect_instance_id: 70,
        definition: DefinitionIdentity {
            registration_token: token.into(),
            definition_source_token: "gfx/particles/rain.asset".into(),
            registration_hash_u32: fnv1a32(token.as_bytes()),
        },
        render_list_class: 2,
        render_subbucket: 3,
        builder_call_ordinal: 0,
        packet_ordinal: 0,
        particle_type_definition_ptr: 0x7000,
        subsystem_selector_u32: 0,
        shader_program_index_u32: 0,
        texture_runtime_index_u32: 0,
        engine_effect_token: "PdxMeshParticle".into(),
        atlas_half_pixel_width_f32_bits: 0.5f32.to_bits(),
        atlas_half_pixel_height_f32_bits: 0.5f32.to_bits(),
        atlas_columns_f32_bits: 1.0f32.to_bits(),
        atlas_rows_f32_bits: 1.0f32.to_bits(),
        draw_order: 0,
        shader_program_ptr: 100,
        additional_shader_stages_authenticated_null: true,
        stream_output_targets_authenticated_null: true,
        ribbon_tail_order: None,
        canonical_packet_112_bytes: canonicalize_packet(canonical).to_vec(),
        packet_matrix_4x4_f32_bits: [0; 16],
        live_count: 2,
        max_count: 4,
        ordered_gpu_instance_records: vec![7; 96],
        elapsed_f32_bits: 1f32.to_bits(),
        scale_f32_bits: 1f32.to_bits(),
        flags_u16: 0,
        parent_entity_instance_id: None,
    }
}

fn entity() -> RawEntitySubmission {
    RawEntitySubmission {
        submit_index: 7,
        pass_ordinal: 1,
        draw_order: 1,
        entity_instance_id: 80,
        entity_registration_token: Some("unit".into()),
        entity_object_ptr: 201,
        entity_definition_ptr: 202,
        renderer_ptr: 203,
        render_context_ptr: 204,
        mesh_draw_ptr: 205,
        material_binding_ptr: 206,
        entity_group_index: 0,
        mesh_draw_list_index: 0,
        entity_variant_u8: 0,
        mesh_draw_variant_u8: 0,
        pass_selector_i32: 0,
        bind_renderer_target_u8: 0,
        alternate_list_u8: 0,
        shader_program_ptr: 100,
        additional_shader_stages_authenticated_null: true,
        stream_output_targets_authenticated_null: true,
        world_transform_4x4_f32_bits: [0; 16],
        state_flags: 0,
        skinning: RawSkinning::NotSkinned,
    }
}

fn raw_submit() -> RawParticleSubmit {
    RawParticleSubmit {
        submit_index: 7,
        camera_carrier_ordinal: 0,
        camera_carrier: None,
        packets: vec![packet()],
        entity_submissions: vec![entity()],
    }
}

fn particle_identity() -> RawDrawIdentity {
    let packet = packet();
    RawDrawIdentity::Particle {
        effect_instance_id: packet.effect_instance_id,
        registration_token: packet.definition.registration_token,
        definition_source_token: packet.definition.definition_source_token,
        builder_call_ordinal: packet.builder_call_ordinal,
        packet_ordinal: packet.packet_ordinal,
        draw_order: packet.draw_order,
        shader_program_ptr: packet.shader_program_ptr,
    }
}

fn entity_identity() -> RawDrawIdentity {
    let value = entity();
    RawDrawIdentity::Entity {
        submit_index: value.submit_index,
        pass_ordinal: value.pass_ordinal,
        entity_instance_id: value.entity_instance_id,
        draw_order: value.draw_order,
        entity_object_ptr: value.entity_object_ptr,
        entity_definition_ptr: value.entity_definition_ptr,
        renderer_ptr: value.renderer_ptr,
        render_context_ptr: value.render_context_ptr,
        mesh_draw_ptr: value.mesh_draw_ptr,
        material_binding_ptr: value.material_binding_ptr,
        shader_program_ptr: value.shader_program_ptr,
    }
}

fn draw_state(identity: RawDrawIdentity, family: RawDrawFamily, sequence: u64) -> RawD3dDrawState {
    let particle = family == RawDrawFamily::Particle;
    RawD3dDrawState {
        final_draw_sequence: sequence,
        identity,
        family,
        submit_index: 7,
        draw_order: if particle { 0 } else { 1 },
        render_context_ptr: if particle { 11 } else { 204 },
        device_context_ptr: 12,
        shader_program_ptr: 100,
        vertex_shader_ptr: 101,
        pixel_shader_ptr: 102,
        geometry_shader_ptr: None,
        hull_shader_ptr: None,
        domain_shader_ptr: None,
        compute_shader_ptr: None,
        dynamic_shader_linkage_present: false,
        input_layout_ptr: Some(if particle { 103 } else { 2030 }),
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
            shader_resource_view_ptr: 106,
            resource_ptr: 105,
        }],
        ordered_sampler_objects: vec![RawStageSlotObject {
            stage: RawShaderStage::Pixel,
            slot: 0,
            object_ptr: 104,
        }],
        ordered_constant_buffer_objects: vec![RawStageSlotObject {
            stage: RawShaderStage::Vertex,
            slot: 0,
            object_ptr: if particle { 109 } else { 209 },
        }],
        ordered_vertex_buffer_objects: vec![RawVertexBufferObject {
            input_slot: 0,
            buffer_ptr: if particle { 107 } else { 207 },
            stride_bytes: 48,
            byte_offset: 0,
        }],
        index_buffer: Some(RawIndexBufferObject {
            buffer_ptr: if particle { 108 } else { 208 },
            dxgi_format_u32: 57,
            byte_offset: 0,
        }),
        ordered_stream_output_buffer_ptrs: vec![],
        ordered_color_target_view_ptrs: vec![110],
        depth_stencil_view_ptr: None,
        ordered_viewports_f32_bits: vec![[
            0,
            0,
            32f32.to_bits(),
            32f32.to_bits(),
            0,
            1f32.to_bits(),
        ]],
        ordered_scissor_rects_i32: vec![],
        draw_call: if particle {
            RawDrawCall::DrawIndexedInstanced {
                index_count_per_instance: 6,
                instance_count: 2,
                start_index: 0,
                base_vertex: 0,
                start_instance: 0,
            }
        } else {
            RawDrawCall::DrawIndexed {
                index_count: 6,
                start_index: 0,
                base_vertex: 0,
            }
        },
    }
}

fn semantics(
    identity: RawDrawIdentity,
    sequence: u64,
    content_identity: RawDrawContentIdentity,
) -> RawDrawSemantics {
    RawDrawSemantics {
        observation_sequence: sequence,
        identity,
        content_identity,
        ordered_texture_semantics: vec![RawTextureSemantic {
            stage: RawShaderStage::Pixel,
            slot: 0,
            semantic_token: "DiffuseMap".into(),
        }],
        ordered_sampler_semantics: vec![RawSamplerSemantic {
            stage: RawShaderStage::Pixel,
            slot: 0,
            semantic_token: "DiffuseSampler".into(),
        }],
        ordered_constant_buffer_semantics: vec![RawConstantBufferSemantic {
            stage_mask_u8: 1,
            slot: 0,
            byte_offset: 0,
            byte_count: 16,
            semantic_layout: "ProjectionMatrix".into(),
        }],
        ordered_color_targets: vec![
            RawRenderTargetRegistrationObservation {
                registration_sequence: sequence - 1,
                capture_sequence: sequence,
                producer_rva: GENERIC_TEXTURE_RTV_PRODUCER_RVA,
                recreate_generation: 0,
                attachment_ordinal: 0,
                kind: RawRegisteredRenderTargetKind::OpaqueRenderTarget {
                    source_flags_u32: 2,
                },
                view_ptr: 110,
                resource_ptr: 5000,
                resource_desc: RawTexture2dDesc {
                    width: 32,
                    height: 32,
                    mip_levels: 1,
                    array_size: 1,
                    dxgi_format_u32: 10,
                    sample_count: 1,
                    sample_quality: 0,
                    usage_u32: 0,
                    bind_flags_u32: 0x28,
                    cpu_access_flags_u32: 0,
                    misc_flags_u32: 0,
                },
                rtv_desc: RawResourceViewDesc {
                    dxgi_format_u32: 10,
                    view_dimension_u32: 4,
                    flags_u32: 0,
                    union_words_u32: [0; 4],
                },
            }
            .materialize()
            .unwrap()
            .target,
        ],
        depth_target: None,
    }
}

fn input_layout(pointer: usize, per_instance: bool) -> RawInputLayoutCreation {
    RawInputLayoutCreation {
        creation_sequence: 1,
        input_layout_ptr: pointer,
        ordered_elements: vec![RawInputElement {
            semantic_token: "POSITION".into(),
            semantic_index: 0,
            dxgi_format_u32: 6,
            input_slot: 0,
            aligned_byte_offset: 0,
            input_slot_class_u32: u32::from(per_instance),
            instance_step_rate: u32::from(per_instance),
        }],
    }
}

fn capture(manifest: &ContentManifest) -> RawRenderCatalogCapture {
    let shader = observed(&content(manifest, "gfx/fx/runtime.shader"));
    let texture = content(manifest, "gfx/textures/cloud.dds");
    let mesh = content(manifest, "gfx/models/unit.mesh");
    let texture_bytes = payloads()["gfx/textures/cloud.dds"].clone();
    let mesh_bytes = payloads()["gfx/models/unit.mesh"].clone();
    let packet = packet();
    let mut captures = RawRenderCatalogCapture {
        process: RawProcessEpoch {
            process_id: 44,
            process_start_nonce: 55,
            executable_sha256: manifest.source_game.executable_sha256.clone(),
        },
        launch_epoch: RawCaptureLaunchEpoch {
            process_id: 44,
            process_start_nonce: 55,
            observer_installed_qpc: 1,
            d3d_device_created_qpc: 2,
            first_creation_sequence: 1,
            hook_epoch_nonce: 56,
            hook_set_receipt_sha256: "e".repeat(64),
            capture_receipt_sha256: "f".repeat(64),
        },
        submit_index: 7,
        frame_sequence: 9,
        draw_states: vec![
            draw_state(particle_identity(), RawDrawFamily::Particle, 10),
            draw_state(entity_identity(), RawDrawFamily::Entity, 20),
        ],
        draw_semantics: vec![
            semantics(
                particle_identity(),
                10,
                RawDrawContentIdentity::Particle {
                    registration_token: packet.definition.registration_token.clone(),
                    definition_source_token: packet.definition.definition_source_token.clone(),
                    subsystem_selector_u32: packet.subsystem_selector_u32,
                    definition_content: observed(&content(manifest, "gfx/particles/rain.asset")),
                    texture_content: observed(&texture),
                    authored_shader_token: "PdxMeshParticle".into(),
                    engine_effect_token: packet.engine_effect_token.clone(),
                    atlas_columns_u32: 1,
                    atlas_rows_u32: 1,
                    atlas_half_pixel_width_f32_bits: packet.atlas_half_pixel_width_f32_bits,
                    atlas_half_pixel_height_f32_bits: packet.atlas_half_pixel_height_f32_bits,
                },
            ),
            semantics(
                entity_identity(),
                20,
                RawDrawContentIdentity::Entity {
                    resolved_entity_asset: observed(&content(manifest, "gfx/entities/unit.asset")),
                    mesh: observed(&mesh),
                    entity_registration_token: "unit".into(),
                    entity_group_index: 0,
                    mesh_draw_list_index: 0,
                    entity_variant_u8: 0,
                    mesh_draw_variant_u8: 0,
                    pass_selector_i32: 0,
                    alternate_list_u8: 0,
                    bind_renderer_target_u8: 0,
                },
            ),
        ],
        program_identities: [10, 20]
            .into_iter()
            .map(|sequence| RawProgramIdentity {
                observation_sequence: sequence,
                shader_program_ptr: 100,
                registration_root_rva: 1,
                source: shader.clone(),
                effect_token: "PdxMeshParticle".into(),
                vertex_entry_token: "VertexMain".into(),
                pixel_entry_token: "PixelMain".into(),
                ordered_defines: vec![],
            })
            .collect(),
        shader_creations: vec![
            RawShaderCreation {
                creation_sequence: 1,
                shader_ptr: 101,
                stage: RawShaderStage::Vertex,
                compiler_identity: "fxc-10.1".into(),
                target_profile: "vs_5_0".into(),
                compiler_flags_u64: 0,
                compiler_invocation: None,
                bytecode: runtime_payload(vec![1; 16], "compiled_shader_bytecode"),
            },
            RawShaderCreation {
                creation_sequence: 1,
                shader_ptr: 102,
                stage: RawShaderStage::Pixel,
                compiler_identity: "fxc-10.1".into(),
                target_profile: "ps_5_0".into(),
                compiler_flags_u64: 0,
                compiler_invocation: None,
                bytecode: runtime_payload(vec![2; 16], "compiled_shader_bytecode"),
            },
        ],
        input_layout_creations: vec![input_layout(103, true), input_layout(2030, false)],
        blend_state_creations: vec![],
        depth_stencil_state_creations: vec![],
        rasterizer_state_creations: vec![],
        sampler_state_creations: vec![RawSamplerStateCreation {
            creation_sequence: 1,
            sampler_state_ptr: 104,
            filter_u32: 0x15,
            address_u_u32: 3,
            address_v_u32: 3,
            address_w_u32: 3,
            mip_lod_bias_f32_bits: 0,
            max_anisotropy: 1,
            comparison_u32: 1,
            border_color_rgba_f32_bits: [0; 4],
            min_lod_f32_bits: 0,
            max_lod_f32_bits: f32::INFINITY.to_bits(),
        }],
        buffer_snapshots: vec![],
        texture_resource_snapshots: vec![],
        shader_resource_view_snapshots: vec![],
        render_target_view_snapshots: vec![],
    };
    for sequence in [10, 20] {
        captures
            .texture_resource_snapshots
            .push(RawTextureResourceSnapshot {
                snapshot_sequence: sequence,
                resource_ptr: 105,
                dimension_u32: 4,
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
                content: Some(observed(&texture)),
                ordered_subresources: vec![RawTextureSubresourceSnapshot {
                    subresource_index: 0,
                    mip_level: 0,
                    array_slice: 0,
                    row_pitch: 4,
                    slice_pitch: 4,
                    payload: vfs_payload(&texture, texture_bytes.clone(), 0),
                }],
            });
        captures
            .shader_resource_view_snapshots
            .push(RawShaderResourceViewSnapshot {
                snapshot_sequence: sequence,
                shader_resource_view_ptr: 106,
                resource_ptr: 105,
                desc: RawResourceViewDesc {
                    dxgi_format_u32: 28,
                    view_dimension_u32: 4,
                    flags_u32: 0,
                    union_words_u32: [0, 1, 0, 0],
                },
            });
        captures
            .render_target_view_snapshots
            .push(RawRenderTargetViewSnapshot {
                snapshot_sequence: sequence,
                view_ptr: 110,
                resource_ptr: 5000,
                view_kind: RawOutputViewKind::Color,
                desc: RawResourceViewDesc {
                    dxgi_format_u32: 10,
                    view_dimension_u32: 4,
                    flags_u32: 0,
                    union_words_u32: [0; 4],
                },
                resource_dimension_u32: 4,
                resource_width: 32,
                resource_height: 32,
                resource_depth: 1,
                resource_mip_levels: 1,
                resource_array_size: 1,
                resource_sample_count: 1,
                resource_sample_quality: 0,
            });
    }
    captures.buffer_snapshots.extend([
        RawBufferSnapshot {
            snapshot_sequence: 10,
            buffer_ptr: 107,
            byte_width: 96,
            usage_u32: 2,
            bind_flags_u32: 1,
            cpu_access_flags_u32: 0x10000,
            misc_flags_u32: 0,
            structure_byte_stride: 48,
            payload: RawPayloadSnapshot {
                captured_bytes: packet.ordered_gpu_instance_records.clone(),
                captured_sha256: hash(&packet.ordered_gpu_instance_records),
                source: RawPayloadSource::ParticleInstanceRecords,
            },
        },
        RawBufferSnapshot {
            snapshot_sequence: 10,
            buffer_ptr: 108,
            byte_width: 12,
            usage_u32: 1,
            bind_flags_u32: 2,
            cpu_access_flags_u32: 0,
            misc_flags_u32: 0,
            structure_byte_stride: 0,
            payload: runtime_payload(
                vec![0, 0, 1, 0, 2, 0, 2, 0, 1, 0, 3, 0],
                "engine_static_particle_quad",
            ),
        },
        RawBufferSnapshot {
            snapshot_sequence: 10,
            buffer_ptr: 109,
            byte_width: 16,
            usage_u32: 2,
            bind_flags_u32: 4,
            cpu_access_flags_u32: 0x10000,
            misc_flags_u32: 0,
            structure_byte_stride: 0,
            payload: runtime_payload(vec![3; 16], "dynamic_constant_buffer"),
        },
        RawBufferSnapshot {
            snapshot_sequence: 20,
            buffer_ptr: 207,
            byte_width: 48,
            usage_u32: 0,
            bind_flags_u32: 1,
            cpu_access_flags_u32: 0,
            misc_flags_u32: 0,
            structure_byte_stride: 0,
            payload: vfs_payload(&mesh, mesh_bytes[0..48].to_vec(), 0),
        },
        RawBufferSnapshot {
            snapshot_sequence: 20,
            buffer_ptr: 208,
            byte_width: 12,
            usage_u32: 0,
            bind_flags_u32: 2,
            cpu_access_flags_u32: 0,
            misc_flags_u32: 0,
            structure_byte_stride: 0,
            payload: vfs_payload(&mesh, mesh_bytes[48..60].to_vec(), 48),
        },
        RawBufferSnapshot {
            snapshot_sequence: 20,
            buffer_ptr: 209,
            byte_width: 16,
            usage_u32: 2,
            bind_flags_u32: 4,
            cpu_access_flags_u32: 0x10000,
            misc_flags_u32: 0,
            structure_byte_stride: 0,
            payload: runtime_payload(vec![4; 16], "dynamic_constant_buffer"),
        },
    ]);
    captures
}

fn static_content_join(
    manifest: &ContentManifest,
    raw: &RawVfsContentIdentity,
) -> RawStaticContentJoin {
    let row = manifest
        .required_content
        .iter()
        .find(|row| row.content.normalized_game_relative_path == raw.logical_path)
        .unwrap();
    RawStaticContentJoin {
        content_key: row.content_key.clone(),
        content: raw.clone(),
    }
}

fn static_capture(manifest: &ContentManifest) -> RawStaticProgramCapture {
    let mut catalog = capture(manifest);
    for snapshot in &mut catalog.buffer_snapshots {
        if matches!(
            snapshot.payload.source,
            RawPayloadSource::ParticleInstanceRecords
        ) {
            snapshot.payload.source = RawPayloadSource::RuntimeGenerated {
                reason: "static-e2e-buffer".into(),
            };
        }
    }
    let mut draws = Vec::new();
    for state in &catalog.draw_states {
        let semantic = catalog
            .draw_semantics
            .iter()
            .find(|row| row.observation_sequence == state.final_draw_sequence)
            .unwrap();
        let program = catalog
            .program_identities
            .iter()
            .find(|row| {
                row.shader_program_ptr == state.shader_program_ptr
                    && row.observation_sequence == state.final_draw_sequence
            })
            .unwrap();
        let mut ordered_content = vec![static_content_join(manifest, &program.source)];
        for object in &state.ordered_texture_objects {
            let texture = catalog
                .texture_resource_snapshots
                .iter()
                .find(|row| {
                    row.resource_ptr == object.resource_ptr
                        && row.snapshot_sequence == state.final_draw_sequence
                })
                .unwrap();
            if let Some(content) = &texture.content {
                ordered_content.push(static_content_join(manifest, content));
            }
        }
        for pointer in state
            .ordered_vertex_buffer_objects
            .iter()
            .map(|row| row.buffer_ptr)
            .chain(state.index_buffer.iter().map(|row| row.buffer_ptr))
            .chain(
                state
                    .ordered_constant_buffer_objects
                    .iter()
                    .map(|row| row.object_ptr),
            )
        {
            let snapshot = catalog
                .buffer_snapshots
                .iter()
                .find(|row| {
                    row.buffer_ptr == pointer && row.snapshot_sequence == state.final_draw_sequence
                })
                .unwrap();
            if let RawPayloadSource::VfsRange { content, .. } = &snapshot.payload.source {
                ordered_content.push(static_content_join(manifest, content));
            }
        }
        for target in semantic
            .ordered_color_targets
            .iter()
            .chain(semantic.depth_target.iter())
        {
            if let Some(content) = &target.persistent_content {
                ordered_content.push(static_content_join(manifest, content));
            }
        }
        let mut seen = std::collections::BTreeSet::new();
        ordered_content.retain(|join| seen.insert(join.content_key.clone()));
        draws.push(RawStaticD3dDrawState {
            capture_sequence: state.final_draw_sequence,
            shader_program_ptr: state.shader_program_ptr,
            render_context_ptr: state.render_context_ptr,
            device_context_ptr: state.device_context_ptr,
            vertex_shader_ptr: state.vertex_shader_ptr,
            pixel_shader_ptr: state.pixel_shader_ptr,
            geometry_shader_ptr: state.geometry_shader_ptr,
            hull_shader_ptr: state.hull_shader_ptr,
            domain_shader_ptr: state.domain_shader_ptr,
            compute_shader_ptr: state.compute_shader_ptr,
            geometry_entry_token: None,
            hull_entry_token: None,
            domain_entry_token: None,
            compute_entry_token: None,
            dynamic_shader_linkage_present: state.dynamic_shader_linkage_present,
            input_layout_ptr: state.input_layout_ptr,
            blend_state_ptr: state.blend_state_ptr,
            blend_factor_rgba_f32_bits: state.blend_factor_rgba_f32_bits,
            sample_mask_u32: state.sample_mask_u32,
            depth_stencil_state_ptr: state.depth_stencil_state_ptr,
            stencil_reference_u32: state.stencil_reference_u32,
            rasterizer_state_ptr: state.rasterizer_state_ptr,
            primitive_topology_u32: state.primitive_topology_u32,
            ordered_texture_objects: state.ordered_texture_objects.clone(),
            ordered_sampler_objects: state.ordered_sampler_objects.clone(),
            ordered_constant_buffer_objects: state.ordered_constant_buffer_objects.clone(),
            ordered_vertex_buffer_objects: state.ordered_vertex_buffer_objects.clone(),
            index_buffer: state.index_buffer.clone(),
            ordered_stream_output_buffers: Vec::new(),
            ordered_color_target_view_ptrs: state.ordered_color_target_view_ptrs.clone(),
            depth_stencil_view_ptr: state.depth_stencil_view_ptr,
            ordered_viewports_f32_bits: state.ordered_viewports_f32_bits.clone(),
            ordered_scissor_rects_i32: state.ordered_scissor_rects_i32.clone(),
            draw_call: match state.draw_call {
                RawDrawCall::Draw {
                    vertex_count,
                    start_vertex,
                } => RawStaticDrawCall::Draw {
                    vertex_count,
                    start_vertex,
                },
                RawDrawCall::DrawIndexed {
                    index_count,
                    start_index,
                    base_vertex,
                } => RawStaticDrawCall::DrawIndexed {
                    index_count,
                    start_index,
                    base_vertex,
                },
                RawDrawCall::DrawInstanced {
                    vertex_count_per_instance,
                    instance_count,
                    start_vertex,
                    start_instance,
                } => RawStaticDrawCall::DrawInstanced {
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
                } => RawStaticDrawCall::DrawIndexedInstanced {
                    index_count_per_instance,
                    instance_count,
                    start_index,
                    base_vertex,
                    start_instance,
                },
            },
            ordered_texture_semantics: semantic
                .ordered_texture_semantics
                .iter()
                .map(|row| RawStaticShaderResourceSemantic {
                    stage: row.stage,
                    slot: row.slot,
                    semantic_token: row.semantic_token.clone(),
                    source: RawStaticShaderResourceSource::UsedContent,
                })
                .collect(),
            ordered_sampler_semantics: semantic.ordered_sampler_semantics.clone(),
            ordered_constant_buffer_semantics: semantic.ordered_constant_buffer_semantics.clone(),
            ordered_color_targets: semantic.ordered_color_targets.clone(),
            depth_target: semantic.depth_target.clone(),
            ordered_content,
        });
    }
    let ordered_operations = draws
        .iter()
        .enumerate()
        .map(|(index, draw)| RawStaticOrderOperation::Draw {
            sequence: index as u64 + 1,
            capture_sequence: draw.capture_sequence,
        })
        .collect();
    let capture = RawStaticProgramCapture {
        process: catalog.process.clone(),
        launch_epoch: catalog.launch_epoch.clone(),
        submit_index: catalog.submit_index,
        frame_sequence: catalog.frame_sequence,
        ordered_operations,
        draws,
        catalog,
    };
    capture.validate_complete().unwrap();
    capture
}

#[test]
fn static_raw_builder_portable_relocation_with_mod_override_and_replace_path() {
    let manifest = manifest();
    let capture = static_capture(&manifest);
    let promoted =
        build_authenticated_static_programs(&manifest, &capture, &TestVfs(payloads())).unwrap();
    promoted.validate_canonical_partition().unwrap();
    assert_eq!(promoted.ordered_programs.len(), 2);
    let exact_draws = promoted
        .ordered_programs
        .iter()
        .flat_map(|program| &program.ordered_operations)
        .filter_map(|operation| match &operation.command {
            portable_render_stream::StaticMapCommand::Draw { draw } => Some(draw),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(exact_draws.len(), 2);
    assert!(exact_draws.iter().all(|draw| matches!(
        draw.geometry,
        portable_render_stream::StaticGeometrySource::Exact { .. }
    )));
    assert!(exact_draws.iter().any(|draw| {
        let portable_render_stream::StaticGeometrySource::Exact { geometry } = &draw.geometry
        else {
            return false;
        };
        geometry
            .ordered_vertex_buffers
            .iter()
            .any(|binding| matches!(binding.source, BufferSource::Inline { .. }))
    }));
    assert!(exact_draws.iter().any(|draw| {
        let portable_render_stream::StaticGeometrySource::Exact { geometry } = &draw.geometry
        else {
            return false;
        };
        geometry
            .ordered_vertex_buffers
            .iter()
            .any(|binding| matches!(binding.source, BufferSource::UsedContentRange { .. }))
            || geometry.index_buffer.as_ref().is_some_and(|binding| {
                matches!(binding.source, BufferSource::UsedContentRange { .. })
            })
    }));
    assert!(promoted
        .ordered_programs
        .iter()
        .flat_map(|program| &program.ordered_content_keys)
        .any(|key| key == "texture"));
    let portable = serde_json::to_vec(&promoted.ordered_programs).unwrap();
    let text = std::str::from_utf8(&portable).unwrap();
    for forbidden in ["_ptr", "C:\\\\", "OneDrive", "modded-rain-definition"] {
        assert!(!text.contains(forbidden), "portable d80 leaked {forbidden}");
    }
    let relocated: Vec<portable_render_stream::StaticMapPassProgram> =
        serde_json::from_slice(&portable).unwrap();
    assert_eq!(relocated, promoted.ordered_programs);

    let mut relabeled = promoted.clone();
    relabeled.ordered_programs[0].pass_token = "caller-relabel".into();
    relabeled.ordered_programs[0].seal_payload_hashes().unwrap();
    assert!(relabeled.validate_canonical_partition().is_err());
}

#[test]
fn capture_promotes_to_pointer_free_relocatable_portable_payload() {
    let manifest = manifest();
    let raw = raw_submit();
    let capture = capture(&manifest);
    let vfs = TestVfs(payloads());
    let catalog = build_authenticated_render_catalog(&manifest, &raw, &capture, &vfs).unwrap();
    let particles = promote_submit(&raw, &catalog).unwrap();
    let entities = raw
        .entity_submissions
        .iter()
        .map(|value| promote_entity_submission(value, &catalog).unwrap())
        .collect::<Vec<_>>();
    let order = promote_runtime_execution_order(&raw).unwrap();
    assert_eq!(order.len(), 2);
    assert!(matches!(order[0], RenderExecutionRef::Runtime { .. }));
    assert!(matches!(order[1], RenderExecutionRef::Runtime { .. }));

    assert!(matches!(
        particles[0].ordered_texture_bindings[0]
            .resource
            .ordered_subresources[0]
            .source,
        BufferSource::UsedContentRange { .. }
    ));
    assert!(matches!(
        entities[0].geometry.ordered_vertex_buffers[0].source,
        BufferSource::UsedContentRange { .. }
    ));
    assert!(matches!(
        entities[0].geometry.index_buffer.as_ref().unwrap().source,
        BufferSource::UsedContentRange { .. }
    ));

    let portable = serde_json::json!({
        "content_manifest": manifest,
        "particles": particles,
        "entities": entities,
        "runtime_order": order,
    });
    let bytes = serde_json::to_vec(&portable).unwrap();
    let text = std::str::from_utf8(&bytes).unwrap();
    for forbidden in [
        "shader_program_ptr",
        "render_context_ptr",
        "resource_ptr",
        "object_ptr",
        "C:\\\\",
        "OneDrive",
    ] {
        assert!(
            !text.contains(forbidden),
            "portable payload leaked {forbidden}"
        );
    }
    let relocated_bytes = bytes.clone();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&relocated_bytes).unwrap(),
        portable
    );
}

#[test]
fn builder_rejects_fabrication_tamper_stale_missing_extra_order_and_vfs_drift() {
    let manifest = manifest();
    let raw = raw_submit();
    let baseline = capture(&manifest);
    let vfs = TestVfs(payloads());
    assert!(build_authenticated_render_catalog(&manifest, &raw, &baseline, &vfs).is_ok());

    let mut tampered_shader = baseline.clone();
    tampered_shader.shader_creations[0].bytecode.captured_bytes[0] ^= 1;
    assert!(build_authenticated_render_catalog(&manifest, &raw, &tampered_shader, &vfs).is_err());

    let mut stale_program = baseline.clone();
    stale_program.program_identities[0].observation_sequence = 9;
    assert!(build_authenticated_render_catalog(&manifest, &raw, &stale_program, &vfs).is_err());

    let mut missing_draw = baseline.clone();
    missing_draw.draw_states.pop();
    assert!(build_authenticated_render_catalog(&manifest, &raw, &missing_draw, &vfs).is_err());

    let mut extra_snapshot = baseline.clone();
    let mut extra = extra_snapshot.buffer_snapshots[0].clone();
    extra.buffer_ptr = 9999;
    extra_snapshot.buffer_snapshots.push(extra);
    assert!(build_authenticated_render_catalog(&manifest, &raw, &extra_snapshot, &vfs).is_err());

    let mut wrong_order = baseline.clone();
    wrong_order.draw_semantics[0].ordered_texture_semantics[0].slot = 1;
    assert!(build_authenticated_render_catalog(&manifest, &raw, &wrong_order, &vfs).is_err());

    let mut wrong_pointer = baseline.clone();
    wrong_pointer.draw_states[0].ordered_texture_objects[0].shader_resource_view_ptr = 999;
    assert!(build_authenticated_render_catalog(&manifest, &raw, &wrong_pointer, &vfs).is_err());

    let mut fabricated_descriptor = baseline.clone();
    fabricated_descriptor.sampler_state_creations[0].filter_u32 = 0xffff_ffff;
    assert!(
        build_authenticated_render_catalog(&manifest, &raw, &fabricated_descriptor, &vfs).is_err()
    );

    let mut embedded_asset_copy = baseline.clone();
    embedded_asset_copy.texture_resource_snapshots[0].ordered_subresources[0]
        .payload
        .source = RawPayloadSource::RuntimeGenerated {
        reason: "decoded_authored_texture".into(),
    };
    assert!(
        build_authenticated_render_catalog(&manifest, &raw, &embedded_asset_copy, &vfs).is_err()
    );

    let mut changed_mod = baseline.clone();
    match &mut changed_mod.draw_semantics[0].content_identity {
        RawDrawContentIdentity::Particle {
            definition_content, ..
        } => {
            definition_content.layer_id = "different-mod".into();
        }
        _ => unreachable!(),
    }
    assert!(build_authenticated_render_catalog(&manifest, &raw, &changed_mod, &vfs).is_err());

    let mut changed_vfs_bytes = payloads();
    changed_vfs_bytes.insert("gfx/models/unit.mesh".into(), vec![99; 60]);
    assert!(build_authenticated_render_catalog(
        &manifest,
        &raw,
        &baseline,
        &TestVfs(changed_vfs_bytes)
    )
    .is_err());

    let mut missing_dlc_content = manifest.clone();
    missing_dlc_content
        .required_content
        .retain(|row| row.content.normalized_game_relative_path != "gfx/textures/cloud.dds");
    missing_dlc_content.manifest_digest_sha256 = None;
    missing_dlc_content.source_profile.selection_sha256 =
        missing_dlc_content.computed_selection_sha256().unwrap();
    missing_dlc_content = missing_dlc_content.with_computed_digest().unwrap();
    assert!(
        build_authenticated_render_catalog(&missing_dlc_content, &raw, &baseline, &vfs).is_err()
    );
}
