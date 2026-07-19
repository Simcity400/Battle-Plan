use super::*;
use crate::PlaybackEnvironment;
use replay_core::{
    BlendDesc, BlendFactor, BlendOperation, BufferSource, Comparison, CompiledShaderPayload,
    ConstantBufferBinding, ContentLayerKind, CullMode, DepthDesc, DlcSelection, DrawCommand,
    EntityContentIdentity, EntityNativeSelection, EntitySubmission, FillMode, GeometryBinding,
    IndexFormat, InputElementDesc, InterpreterIndexBufferDesc, InterpreterVertexBufferDesc,
    LayerStorageKind, M02CombatFrame, M02LawState, O04CityFrame, O04CitySemantic,
    O04CitylightState, ParticleRuntimeMode, PipelineDesc, PreparedRenderTarget, PrimitiveTopology,
    ProductionRenderFrame, RasterizerDesc, RecordedContentLayer, RenderAuthority, RenderCheckpoint,
    RenderExecutionRef, RenderOutputRef, RenderOutputState, RenderSubmit, RenderTargetDesc,
    RequiredContent, ShaderProgram, ShaderResourceViewDesc, ShaderStage, Skinning,
    SourceGameIdentity, SourceProfileIdentity, StaticGeometrySource, StaticMapCommand,
    StaticMapDraw, StaticMapOperation, StaticMapPassProgram, StaticShaderResourceBinding,
    StaticShaderResourceSource, StencilFaceDesc, StencilOperation, TextureResourceDesc,
    TextureResourceDimension, VertexBufferBinding, ViewportDesc, O04_A729_EXECUTABLE_SHA256,
    O04_CITY_SCHEMA, O04_EVIDENCE_MANIFEST_SHA256, O04_LIVE_CITY_SUBMIT_BINDING_SHA256,
    O04_LIVE_CITY_SUBMIT_VERIFICATION_SHA256,
};
use snapshot::sha256::Sha256;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_root(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "production_map_{label}_{}_{}",
        std::process::id(),
        stamp
    ))
}

#[test]
fn unsupported_static_producer_transitively_omits_dependent_pass_chain() {
    let source = UsedContentKey {
        effective_vfs_manifest_sha256: "2".repeat(64),
        normalized_game_relative_path: "gfx/fx/cascade.shader".into(),
        winning_layer_ordinal: 0,
        layer_kind: ContentLayerKind::Base,
        layer_id: "base".into(),
        source_sha256: "3".repeat(64),
        payload_sha256: "3".repeat(64),
        payload_bytes: 4,
    };
    let mut unsupported_draw = opaque_static_draw(source.clone());
    unsupported_draw.geometry = StaticGeometrySource::InterpreterOutput {
        interpreter_token: "unsupported.cascade.interpreter.v1".into(),
        ordered_input_content_keys: Vec::new(),
        ordered_vertex_buffers: Vec::new(),
        index_buffer: None,
        stream_output_targets: Vec::new(),
        draw: DrawCommand::Draw {
            vertex_count: 3,
            start_vertex: 0,
        },
    };

    let dependency_binding = |producer| StaticShaderResourceBinding {
        stage: ShaderStage::Pixel,
        slot: 0,
        semantic_token: "cascade_input".into(),
        source: StaticShaderResourceSource::PreparedTarget {
            target: PreparedRenderTarget::OpaqueRenderTarget {
                attachment_ordinal: 4,
            },
            resource: TextureResourceDesc {
                dimension: TextureResourceDimension::Texture2d,
                width: 4,
                height: 4,
                depth: 1,
                mip_levels: 1,
                array_size: 1,
                dxgi_format_u32: 87,
                sample_count: 1,
                sample_quality: 0,
                usage_u32: 0,
                bind_flags_u32: 0,
                cpu_access_flags_u32: 0,
                misc_flags_u32: 0,
                ordered_subresources: Vec::new(),
            },
            view: ShaderResourceViewDesc::Texture2d {
                dxgi_format_u32: 87,
                most_detailed_mip: 0,
                mip_levels: 1,
            },
            producer,
        },
    };
    let mut first_dependent = opaque_static_draw(source.clone());
    first_dependent.ordered_shader_resources =
        vec![dependency_binding(RenderExecutionRef::Static {
            operation_ordinal: 10,
        })];
    let mut second_dependent = opaque_static_draw(source);
    second_dependent.ordered_shader_resources =
        vec![dependency_binding(RenderExecutionRef::Static {
            operation_ordinal: 20,
        })];

    let program = |program_ordinal, pass: &str, operation_ordinal, draw| StaticMapPassProgram {
        program_ordinal,
        pass_token: pass.into(),
        ordered_content_keys: Vec::new(),
        ordered_operations: vec![StaticMapOperation {
            operation_ordinal,
            payload_sha256: format!("{operation_ordinal:064x}"),
            command: StaticMapCommand::Draw { draw },
        }],
        payload_sha256: format!("{:064x}", program_ordinal + 1),
    };
    let frame = PreparedProductionMapFrame {
        content_manifest_sha256: "1".repeat(64),
        effective_vfs_manifest_sha256: "2".repeat(64),
        selected_submit_index: Some(1),
        selected_submit_global_order: Some(1),
        ordered_scene_constant_buffers: Vec::new(),
        ordered_render_operations: [10, 20, 30]
            .into_iter()
            .map(|operation_ordinal| RenderExecutionRef::Static { operation_ordinal })
            .collect(),
        ordered_static_map_programs: vec![
            program(0, "cascade.unsupported", 10, unsupported_draw),
            program(1, "cascade.first", 20, first_dependent),
            program(2, "cascade.second", 30, second_dependent),
        ],
        political_textures: Vec::new(),
        tiled_lights: Vec::new(),
        ordered_runtime_draws: Vec::new(),
        m02_combat: None,
        authenticated_ui_pass: None,
        m03_arrows: None,
        o04_cities: None,
        omitted_data_needs: Vec::new(),
    };
    let prepared = crate::prepare_production_d3d_frame(&frame, &BTreeMap::new()).unwrap();
    assert!(prepared.ordered_static_programs.is_empty());
    assert!(prepared.ordered_render_operations.is_empty());
    assert_eq!(prepared.omissions.len(), 3);
    assert!(prepared.omissions[0].reason.contains("unsupported"));
    assert!(prepared.omissions[1]
        .reason
        .contains("omitted static operation 10"));
    assert!(prepared.omissions[2]
        .reason
        .contains("omitted static operation 20"));

    let mut trace = ReplayConsumerTrace::default();
    let receipt = crate::execute_production_d3d_frame(&prepared, &mut trace).unwrap();
    assert!(receipt.ordered_draws.is_empty());
    assert_eq!(trace.calls, vec!["begin:false", "finish"]);
}

fn write(root: &Path, rel: &str, bytes: &[u8]) {
    let path = root.join(rel.replace('/', "\\"));
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

fn layer_kind(kind: &str) -> ContentLayerKind {
    match kind {
        "integrated_dlc" => ContentLayerKind::IntegratedDlc,
        "dlc" => ContentLayerKind::Dlc,
        "mod" => ContentLayerKind::Mod,
        _ => ContentLayerKind::Base,
    }
}

fn resolved_used_content(
    resolver: &GameFileResolver,
    logical_path: &str,
    effective_vfs_manifest_sha256: &str,
) -> UsedContentKey {
    let actual = resolver
        .resolve_required_with_provenance(logical_path, "test production content")
        .unwrap();
    UsedContentKey {
        effective_vfs_manifest_sha256: effective_vfs_manifest_sha256.into(),
        normalized_game_relative_path: actual.logical_path.to_ascii_lowercase(),
        winning_layer_ordinal: actual.source.ordinal as u32,
        layer_kind: layer_kind(&actual.source.kind),
        layer_id: actual.source.source_id,
        source_sha256: actual
            .source
            .archive_sha256
            .unwrap_or_else(|| actual.sha256.clone()),
        payload_sha256: actual.sha256,
        payload_bytes: actual.byte_len,
    }
}

fn admitted_renderer(
    game: &Path,
    mod_root: &Path,
    optional_dlc_enabled: bool,
) -> (ProductionMapRenderer, ContentManifest, GameFileResolver) {
    admitted_renderer_with_binding(
        game,
        mod_root,
        optional_dlc_enabled,
        "gfx/FX/map.shader",
        "terrain-main-shader",
        vec!["test.legacy_evidence.terrain".into()],
    )
}

fn admitted_renderer_with_binding(
    game: &Path,
    mod_root: &Path,
    optional_dlc_enabled: bool,
    logical_path: &str,
    content_key: &str,
    consumers: Vec<String>,
) -> (ProductionMapRenderer, ContentManifest, GameFileResolver) {
    admitted_renderer_with_bindings(
        game,
        mod_root,
        optional_dlc_enabled,
        vec![(
            logical_path,
            content_key,
            RequiredContentKind::ShaderSource,
            consumers,
        )],
    )
}

fn admitted_renderer_with_bindings(
    game: &Path,
    mod_root: &Path,
    optional_dlc_enabled: bool,
    bindings: Vec<(&str, &str, RequiredContentKind, Vec<String>)>,
) -> (ProductionMapRenderer, ContentManifest, GameFileResolver) {
    let resolver = GameFileResolver::with_mod_paths(game, vec![mod_root.to_path_buf()]).unwrap();
    let vfs = "ab".repeat(32);
    let required_content = bindings
        .into_iter()
        .map(
            |(logical_path, content_key, kind, consumers)| RequiredContent {
                content_key: content_key.into(),
                kind,
                content: resolved_used_content(&resolver, logical_path, &vfs),
                observation_count: 1,
                consumers,
            },
        )
        .collect::<Vec<_>>();
    let used_ordinals = required_content
        .iter()
        .map(|row| row.content.winning_layer_ordinal as usize)
        .collect::<BTreeSet<_>>();
    let ordered_layers = resolver
        .active_sources()
        .into_iter()
        .map(|source| RecordedContentLayer {
            ordinal: source.ordinal as u32,
            layer_kind: layer_kind(&source.kind),
            layer_id: source.source_id.clone(),
            display_name: source.source_id,
            descriptor_sha256: source.descriptor_sha256,
            source_container_sha256: source.archive_sha256.clone(),
            storage_kind: if source.archive_sha256.is_some() {
                LayerStorageKind::ZipArchive
            } else {
                LayerStorageKind::Directory
            },
            replace_paths: source.replace_path,
            required_for_playback: used_ordinals.contains(&source.ordinal),
        })
        .collect::<Vec<_>>();
    let source_game = SourceGameIdentity {
        version: "1.19.2.0".into(),
        build: "a729".into(),
        executable_sha256: "87".repeat(32),
        build_profile: "test".into(),
    };
    let mut manifest = ContentManifest {
        schema: replay_core::PORTABLE_CONTENT_MANIFEST_SCHEMA.into(),
        source_game: source_game.clone(),
        source_profile: SourceProfileIdentity {
            profile_id: "portable-test".into(),
            playset_id: "portable-test".into(),
            selection_sha256: "00".repeat(32),
        },
        effective_vfs_manifest_sha256: vfs,
        ordered_layers,
        dlc_selection: vec![DlcSelection {
            dlc_id: "dlc/optional_unused".into(),
            enabled: optional_dlc_enabled,
            required_for_playback: false,
        }],
        localisation_language: "l_english".into(),
        required_settings: BTreeMap::new(),
        required_content,
        manifest_digest_sha256: None,
    };
    manifest.source_profile.selection_sha256 = manifest.computed_selection_sha256().unwrap();
    manifest = manifest.with_computed_digest().unwrap();
    // Optional, unused DLC may differ at playback and never becomes a required
    // layer or fallback asset source.
    let inventory = resolver
        .playback_inventory(
            &manifest,
            PlaybackEnvironment {
                source_game,
                source_profile: manifest.source_profile.clone(),
                dlc_selection: vec![DlcSelection {
                    dlc_id: "dlc/optional_unused".into(),
                    enabled: !optional_dlc_enabled,
                    required_for_playback: false,
                }],
                localisation_language: "l_english".into(),
                settings: BTreeMap::new(),
            },
        )
        .unwrap();
    let admission = manifest.admit_playback(&inventory).unwrap();
    let admitted_resolver = resolver.with_content_admission(&admission).unwrap();
    let renderer = ProductionMapRenderer::new(&admitted_resolver, &admission, &manifest).unwrap();
    (renderer, manifest, admitted_resolver)
}

fn opaque_static_draw(source: UsedContentKey) -> StaticMapDraw {
    let vertex_shader = vec![1, 2, 3, 4];
    let pixel_shader = vec![5, 6, 7, 8];
    let mut vertices = Vec::new();
    for value in [-1.0f32, -1.0, -1.0, 3.0, 3.0, -1.0] {
        vertices.extend_from_slice(&value.to_le_bytes());
    }
    let stencil = StencilFaceDesc {
        fail_operation: StencilOperation::Keep,
        depth_fail_operation: StencilOperation::Keep,
        pass_operation: StencilOperation::Keep,
        comparison: Comparison::Always,
    };
    StaticMapDraw {
        draw_token: "test.mod_defined_opaque".into(),
        shader_program: ShaderProgram {
            source,
            effect_token: "test.opaque".into(),
            vertex_entry_token: "vs_main".into(),
            pixel_entry_token: "ps_main".into(),
            ordered_defines: Vec::new(),
            compiled_vertex_shader: CompiledShaderPayload {
                compiler_identity: "opaque_renderer_test".into(),
                target_profile: "vs_5_0".into(),
                compiler_flags_u64: 0,
                sha256: Sha256::hex(&vertex_shader),
                bytecode: vertex_shader,
            },
            compiled_pixel_shader: CompiledShaderPayload {
                compiler_identity: "opaque_renderer_test".into(),
                target_profile: "ps_5_0".into(),
                compiler_flags_u64: 0,
                sha256: Sha256::hex(&pixel_shader),
                bytecode: pixel_shader,
            },
            geometry_shader: None,
            hull_shader: None,
            domain_shader: None,
            compute_shader: None,
        },
        ordered_shader_resources: Vec::new(),
        pipeline: PipelineDesc {
            topology: PrimitiveTopology::TriangleList,
            blend: BlendDesc {
                alpha_to_coverage_enabled: false,
                independent_blend_enabled: false,
                enabled: false,
                source_color: BlendFactor::One,
                destination_color: BlendFactor::Zero,
                color_operation: BlendOperation::Add,
                source_alpha: BlendFactor::One,
                destination_alpha: BlendFactor::Zero,
                alpha_operation: BlendOperation::Add,
                write_mask_rgba_u8: 0xf,
                additional_targets: Vec::new(),
                blend_factor_rgba_f32_bits: [0; 4],
                sample_mask_u32: u32::MAX,
            },
            depth: DepthDesc {
                enabled: false,
                write_enabled: false,
                comparison: Comparison::Always,
                stencil_enabled: false,
                stencil_read_mask_u8: 0xff,
                stencil_write_mask_u8: 0xff,
                front_face: stencil.clone(),
                back_face: stencil,
                stencil_reference_u32: 0,
            },
            rasterizer: RasterizerDesc {
                fill: FillMode::Solid,
                cull: CullMode::None,
                front_counter_clockwise: false,
                depth_clip_enabled: true,
                scissor_enabled: false,
                depth_bias_i32: 0,
                depth_bias_clamp_f32_bits: 0,
                slope_scaled_depth_bias_f32_bits: 0,
                multisample_enabled: false,
                antialiased_line_enabled: false,
            },
            samplers: Vec::new(),
            input_layout: vec![InputElementDesc {
                semantic_token: "POSITION".into(),
                semantic_index: 0,
                dxgi_format_u32: 16,
                input_slot: 0,
                aligned_byte_offset: 0,
                per_instance: false,
                instance_step_rate: 0,
            }],
            index_format: IndexFormat::None,
            static_indices_u16: Vec::new(),
        },
        geometry: StaticGeometrySource::Exact {
            geometry: GeometryBinding {
                ordered_vertex_buffers: vec![VertexBufferBinding {
                    input_slot: 0,
                    stride_bytes: 8,
                    byte_offset: 0,
                    source: BufferSource::Inline {
                        sha256: Sha256::hex(&vertices),
                        bytes: vertices,
                    },
                }],
                index_buffer: None,
                stream_output_targets: Vec::new(),
                draw: DrawCommand::Draw {
                    vertex_count: 3,
                    start_vertex: 0,
                },
            },
        },
        ordered_constant_buffers: Vec::new(),
        output_state: RenderOutputState {
            ordered_color_targets: vec![RenderTargetDesc {
                prepared_target: PreparedRenderTarget::OpaqueRenderTarget {
                    attachment_ordinal: 4,
                },
                width: 4,
                height: 4,
                dxgi_format_u32: 87,
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
                width_f32_bits: 4f32.to_bits(),
                height_f32_bits: 4f32.to_bits(),
                min_depth_f32_bits: 0,
                max_depth_f32_bits: 1f32.to_bits(),
            },
            scissor: None,
        },
        payload_sha256: String::new(),
    }
}

fn frame_with_static_program(
    manifest: &ContentManifest,
    program: StaticMapPassProgram,
) -> ProductionRenderFrame {
    let raw_bytes = (0u8..32).collect::<Vec<_>>();
    let binding = ConstantBufferBinding {
        stage_mask_u8: 0x03,
        slot: 0,
        byte_offset: 16,
        byte_count: 16,
        sha256: Sha256::hex(&raw_bytes),
        raw_bytes,
        semantic_layout: "SceneCameraAndClock".into(),
    };
    let operation_ordinal = program.ordered_operations[0].operation_ordinal;
    ProductionRenderFrame {
        schema: replay_core::PRODUCTION_RENDER_SCHEMA.into(),
        authority: RenderAuthority {
            process_id: 7,
            process_start_nonce: 8,
            observer_installed_qpc: 9,
            d3d_device_created_qpc: 10,
            first_creation_sequence: 1,
            hook_epoch_nonce: 11,
            hook_set_receipt_sha256: "a".repeat(64),
            capture_receipt_sha256: "b".repeat(64),
            launch_epoch_sha256: replay_core::compute_render_launch_epoch_sha256(
                7,
                8,
                9,
                10,
                1,
                11,
                &"a".repeat(64),
                &"b".repeat(64),
            ),
            effective_vfs_manifest_sha256: manifest.effective_vfs_manifest_sha256.clone(),
            effect_catalog_sha256: "3".repeat(64),
            entity_animation_catalog_sha256: "4".repeat(64),
            recorded_input_variant: None,
            particle_runtime_mode: ParticleRuntimeMode::SubmittedPacketReplay,
        },
        update_index: 16_959_360,
        first_sequence: 10,
        last_sequence: 20,
        presentation_qpc_frequency_hz: 10_000_000,
        display_submit_index: Some("12".into()),
        selected_present: replay_core::SelectedPresentReceipt::sealed(
            1,
            "c".repeat(64),
            12,
            0,
            60_759_359,
            200,
            300,
        ),
        submits: vec![RenderSubmit {
            submit_index: 12,
            global_order: 2,
            presentation_qpc_ticks: 200,
            interpolation_phase_f32_bits: 0,
            ordered_scene_constant_buffers: vec![binding],
            ordered_static_map_programs: vec![program],
            effect_lifecycle_events: Vec::new(),
            particles: Vec::new(),
            entity_presentations: Vec::new(),
            particle_packet_set_sha256: "5".repeat(64),
            entities: Vec::new(),
            ordered_draws: Vec::new(),
            ordered_render_operations: vec![RenderExecutionRef::Static { operation_ordinal }],
            direct_visuals: Vec::new(),
            light_lifecycle_events: Vec::new(),
            tiled_lights: Vec::new(),
            weather_membership: Vec::new(),
            political_textures: Vec::new(),
        }],
        m02_combat: None,
        m03_arrows: None,
        o04_cities: None,
        authenticated_ui_pass: None,
        checkpoint: RenderCheckpoint {
            last_sequence: 20,
            last_update_index: 16_959_360,
            last_submit_index: Some("12".into()),
            next_stable_ids: BTreeMap::new(),
            active_effect_lifecycle: Vec::new(),
            active_entity_presentation: Vec::new(),
            active_direct_visual_producers: Vec::new(),
            active_light_lifecycle: Vec::new(),
            latest_tiled_light_payload: None,
            weather_state_and_ordered_membership: Vec::new(),
            active_effect_set_sha256: "6".repeat(64),
            ordered_particle_packets_sha256: "7".repeat(64),
            ordered_entity_submissions_sha256: "8".repeat(64),
            ordered_direct_visual_producers_sha256: "9".repeat(64),
            ordered_light_submissions_sha256: "a".repeat(64),
            weather_membership_sha256: "b".repeat(64),
        },
    }
}

fn o04_transform_bits() -> [u32; 16] {
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
        12.0f32.to_bits(),
        34.0f32.to_bits(),
        56.0f32.to_bits(),
        1.0f32.to_bits(),
    ]
}

fn add_o04_fixture_depth(output: &mut RenderOutputState) {
    output.depth_target = Some(RenderTargetDesc {
        prepared_target: PreparedRenderTarget::OpaqueRenderTarget {
            attachment_ordinal: 9,
        },
        width: 4,
        height: 4,
        dxgi_format_u32: 45,
        mip_level: 0,
        sample_count: 1,
        sample_quality: 0,
        persistent_content: None,
        exact_resource: None,
    });
}

fn fixture_entity_submission(
    output: &RenderOutputRef,
    entity_asset: UsedContentKey,
    mesh: UsedContentKey,
    shader: UsedContentKey,
) -> EntitySubmission {
    let RenderOutputRef::Entity {
        pass_ordinal,
        entity_instance_id,
        draw_order,
    } = output
    else {
        panic!("O04 fixture output must be an entity")
    };
    let native_selection = EntityNativeSelection {
        entity_registration_token: "city_fixture".into(),
        entity_group_index: 1,
        mesh_draw_list_index: 2,
        entity_variant_u8: 3,
        mesh_draw_variant_u8: 4,
        pass_selector_i32: -5,
        alternate_list_u8: 6,
        bind_renderer_target_u8: 7,
    };
    let native_selection_sha256 = native_selection.computed_sha256().unwrap();
    let state_flags = native_selection.packed_state_flags();
    let mut draw = opaque_static_draw(shader);
    add_o04_fixture_depth(&mut draw.output_state);
    let StaticGeometrySource::Exact { geometry } = draw.geometry else {
        unreachable!()
    };
    EntitySubmission {
        pass_ordinal: *pass_ordinal,
        draw_order: *draw_order,
        entity_instance_id: *entity_instance_id,
        resolved_entity_sha256: entity_asset.payload_sha256.clone(),
        content_identity: EntityContentIdentity {
            resolved_entity_asset: entity_asset,
            mesh,
            native_selection,
            native_selection_sha256,
        },
        shader_program: draw.shader_program,
        ordered_texture_bindings: Vec::new(),
        pipeline: draw.pipeline,
        geometry,
        ordered_constant_buffers: Vec::new(),
        output_state: draw.output_state,
        world_transform_4x4_f32_bits: o04_transform_bits(),
        state_flags,
        skinning: Skinning::NotSkinned,
    }
}

fn attach_o04_city(
    frame: &mut ProductionRenderFrame,
    entity_asset: UsedContentKey,
    mesh: UsedContentKey,
    shader: UsedContentKey,
) {
    let outputs = (0..3)
        .map(|pass_ordinal| RenderOutputRef::Entity {
            pass_ordinal: 20 + pass_ordinal,
            entity_instance_id: 81,
            draw_order: 100 + pass_ordinal,
        })
        .collect::<Vec<_>>();
    let submit = &mut frame.submits[0];
    submit.entities = outputs
        .iter()
        .map(|output| {
            fixture_entity_submission(output, entity_asset.clone(), mesh.clone(), shader.clone())
        })
        .collect();
    submit.ordered_draws = outputs.clone();
    submit.ordered_render_operations.extend(
        outputs
            .iter()
            .cloned()
            .map(|output| RenderExecutionRef::Runtime { output }),
    );
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
            entity_asset,
            mesh,
            world_transform_4x4_f32_bits: o04_transform_bits(),
            citylight_state: O04CitylightState::RegisteredCitylight,
            referenced_outputs: outputs.clone(),
        }],
        ordered_outputs: outputs,
    });
}

#[derive(Default)]
struct ReplayConsumerTrace {
    calls: Vec<String>,
}

impl crate::ProductionD3dExecutionBridge for ReplayConsumerTrace {
    fn begin_frame(&mut self, frame: &crate::PreparedProductionD3dFrame) -> Result<()> {
        self.calls.push(format!("begin:{}", frame.target.is_some()));
        Ok(())
    }

    fn execute_static_operation(
        &mut self,
        global_ordinal: u32,
        pass: &StaticMapPassProgram,
        _: &[ConstantBufferBinding],
        operation: &StaticMapOperation,
    ) -> Result<()> {
        let token = match &operation.command {
            StaticMapCommand::Draw { draw } => draw.draw_token.as_str(),
            StaticMapCommand::ClearColor { .. } => "clear_color",
            StaticMapCommand::ClearDepthStencil { .. } => "clear_depth_stencil",
        };
        self.calls
            .push(format!("{global_ordinal}:{}:{token}", pass.pass_token));
        Ok(())
    }

    fn execute_runtime_draw(
        &mut self,
        _: u32,
        _: &[ConstantBufferBinding],
        _: &PreparedRuntimeDraw,
    ) -> Result<()> {
        panic!("retained static-program proof unexpectedly executed a runtime draw")
    }

    fn execute_m03_draw(
        &mut self,
        _: u32,
        _: &[ConstantBufferBinding],
        _: &replay_core::M03ArrowDraw,
    ) -> Result<()> {
        panic!("retained static-program proof unexpectedly executed an M03 draw")
    }

    fn finish_frame(&mut self) -> Result<()> {
        self.calls.push("finish".into());
        Ok(())
    }
}

#[derive(Default)]
struct O04ConsumerTrace {
    calls: Vec<String>,
}

impl crate::ProductionD3dExecutionBridge for O04ConsumerTrace {
    fn begin_frame(&mut self, frame: &crate::PreparedProductionD3dFrame) -> Result<()> {
        self.calls.push(format!("begin:{}", frame.target.is_some()));
        Ok(())
    }

    fn execute_static_operation(
        &mut self,
        global_ordinal: u32,
        pass: &StaticMapPassProgram,
        _: &[ConstantBufferBinding],
        _: &StaticMapOperation,
    ) -> Result<()> {
        self.calls
            .push(format!("{global_ordinal}:static:{}", pass.pass_token));
        Ok(())
    }

    fn execute_runtime_draw(
        &mut self,
        global_ordinal: u32,
        _: &[ConstantBufferBinding],
        draw: &PreparedRuntimeDraw,
    ) -> Result<()> {
        let family = if matches!(draw, PreparedRuntimeDraw::O04City { .. }) {
            "o04"
        } else {
            "runtime"
        };
        self.calls.push(format!("{global_ordinal}:{family}"));
        Ok(())
    }

    fn execute_m03_draw(
        &mut self,
        _: u32,
        _: &[ConstantBufferBinding],
        _: &replay_core::M03ArrowDraw,
    ) -> Result<()> {
        panic!("O04 fixture unexpectedly executed an M03 draw")
    }

    fn finish_frame(&mut self) -> Result<()> {
        self.calls.push("finish".into());
        Ok(())
    }
}

#[test]
fn mod_override_replace_path_and_optional_dlc_are_vfs_driven() {
    let root = temp_root("override");
    let game = root.join("game");
    let mod_root = root.join("mod-content");
    write(&game, "gfx/FX/map.shader", b"base-shader");
    write(&game, "gfx/FX/removed.shader", b"must-not-fallback");
    write(
        &mod_root,
        "descriptor.mod",
        b"name=\"Override\"\ndescriptor=\"mod/override.mod\"\nreplace_path=\"gfx/FX\"\n",
    );
    write(&mod_root, "gfx/FX/map.shader", b"changed-mod-shader");

    let (mut renderer, manifest, admitted) = admitted_renderer(&game, &mod_root, true);
    let recorded = &manifest.required_content[0].content;
    assert_eq!(
        admitted.read_admitted_used_content(recorded).unwrap(),
        b"changed-mod-shader"
    );
    assert_eq!(recorded.layer_kind, ContentLayerKind::Mod);
    assert!(admitted
        .resolve("gfx/FX/removed.shader")
        .unwrap_err()
        .to_string()
        .contains("rejects unrecorded/default lookup"));
    let plan = renderer.prepare_frame(None).unwrap();
    assert!(plan
        .omitted_data_needs
        .iter()
        .any(|need| need.packet == "M02"));
    assert!(plan
        .omitted_data_needs
        .iter()
        .any(|need| need.packet == "M03"));
    assert!(plan
        .omitted_data_needs
        .iter()
        .all(|need| !need.feature.contains("static pass")));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn changed_or_missing_admitted_asset_fails_instead_of_substituting() {
    let root = temp_root("changed");
    let game = root.join("game");
    let mod_root = root.join("mod-content");
    write(&game, "gfx/FX/map.shader", b"base-shader");
    write(
        &mod_root,
        "descriptor.mod",
        b"name=\"Changed\"\ndescriptor=\"mod/changed.mod\"\n",
    );
    write(&mod_root, "gfx/FX/map.shader", b"recorded-shader");
    let (_, manifest, admitted) = admitted_renderer(&game, &mod_root, false);
    write(&mod_root, "gfx/FX/map.shader", b"mutated-after-admission");

    let error = admitted
        .playback_inventory(
            &manifest,
            PlaybackEnvironment {
                source_game: manifest.source_game.clone(),
                source_profile: manifest.source_profile.clone(),
                dlc_selection: Vec::new(),
                localisation_language: "l_english".into(),
                settings: BTreeMap::new(),
            },
        )
        .unwrap_err();
    assert!(error.to_string().contains("mismatch"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn relocated_replay_and_install_produce_identical_plan() {
    let root_a = temp_root("relocate_a");
    let root_b = temp_root("relocate_b");
    for root in [&root_a, &root_b] {
        write(&root.join("game"), "gfx/FX/map.shader", b"base-shader");
        write(
            &root.join("mod-content"),
            "descriptor.mod",
            b"name=\"Relocatable\"\ndescriptor=\"mod/relocatable.mod\"\n",
        );
        write(
            &root.join("mod-content"),
            "gfx/FX/map.shader",
            b"portable-shader",
        );
    }
    let (mut left, left_manifest, _) =
        admitted_renderer(&root_a.join("game"), &root_a.join("mod-content"), true);
    let (mut right, right_manifest, _) =
        admitted_renderer(&root_b.join("game"), &root_b.join("mod-content"), true);
    assert_eq!(
        left.prepare_frame(None).unwrap(),
        right.prepare_frame(None).unwrap()
    );
    assert!(left.resources().is_empty());
    assert!(right.resources().is_empty());
    assert_eq!(
        left_manifest.required_content[0].content,
        right_manifest.required_content[0].content
    );
    let _ = fs::remove_dir_all(root_a);
    let _ = fs::remove_dir_all(root_b);
}

#[test]
fn no_capture_entrypoint_is_reachable_from_production_renderer() {
    let source = include_str!("../production_map_renderer.rs");
    let forbidden = [
        "from_terrain_".to_owned() + "capture",
        "map_frame_".to_owned() + "bundle",
    ];
    for token in forbidden {
        assert!(
            !source.contains(&token),
            "capture dependency leaked: {token}"
        );
    }
}

#[test]
fn normal_renderer_has_no_fixed_role_classifier() {
    let source = include_str!("../production_map_renderer.rs");
    assert!(!source.contains("ProductionMapStaticPassKind"));
    assert!(!source.contains("PRODUCTION_MAP_CONSUMER_PREFIX"));
    assert!(!source.contains("ordered_static_passes"));
}

#[test]
fn prepared_plan_is_deterministic_json() {
    let root = temp_root("deterministic");
    let game = root.join("game");
    let mod_root = root.join("mod-content");
    write(&game, "gfx/FX/map.shader", b"base-shader");
    write(
        &mod_root,
        "descriptor.mod",
        b"name=\"D\"\ndescriptor=\"mod/d.mod\"\n",
    );
    write(&mod_root, "gfx/FX/map.shader", b"deterministic-shader");
    let (mut renderer, _, _) = admitted_renderer(&game, &mod_root, false);
    let first = serde_json::to_vec(&renderer.prepare_frame(None).unwrap()).unwrap();
    let second = serde_json::to_vec(&renderer.prepare_frame(None).unwrap()).unwrap();
    assert_eq!(first, second);
    assert_eq!(Sha256::hex(&first), Sha256::hex(&second));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn selected_submit_preserves_exact_scene_constant_buffers() {
    let root = temp_root("scene_cb");
    let game = root.join("game");
    let mod_root = root.join("mod-content");
    write(&game, "gfx/FX/map.shader", b"base-shader");
    write(
        &mod_root,
        "descriptor.mod",
        b"name=\"CB\"\ndescriptor=\"mod/cb.mod\"\n",
    );
    write(&mod_root, "gfx/FX/map.shader", b"scene-cb-shader");
    let (mut renderer, manifest, _) = admitted_renderer(&game, &mod_root, false);
    let raw_bytes = (0u8..32).collect::<Vec<_>>();
    let binding = ConstantBufferBinding {
        stage_mask_u8: 0x03,
        slot: 0,
        byte_offset: 16,
        byte_count: 16,
        sha256: Sha256::hex(&raw_bytes),
        raw_bytes,
        semantic_layout: "SceneCameraAndClock".into(),
    };
    let frame = ProductionRenderFrame {
        schema: replay_core::PRODUCTION_RENDER_SCHEMA.into(),
        authority: RenderAuthority {
            process_id: 7,
            process_start_nonce: 8,
            observer_installed_qpc: 9,
            d3d_device_created_qpc: 10,
            first_creation_sequence: 1,
            hook_epoch_nonce: 11,
            hook_set_receipt_sha256: "a".repeat(64),
            capture_receipt_sha256: "b".repeat(64),
            launch_epoch_sha256: replay_core::compute_render_launch_epoch_sha256(
                7,
                8,
                9,
                10,
                1,
                11,
                &"a".repeat(64),
                &"b".repeat(64),
            ),
            effective_vfs_manifest_sha256: manifest.effective_vfs_manifest_sha256.clone(),
            effect_catalog_sha256: "3".repeat(64),
            entity_animation_catalog_sha256: "4".repeat(64),
            recorded_input_variant: None,
            particle_runtime_mode: ParticleRuntimeMode::SubmittedPacketReplay,
        },
        update_index: 16_959_360,
        first_sequence: 10,
        last_sequence: 20,
        presentation_qpc_frequency_hz: 10_000_000,
        display_submit_index: Some("12".into()),
        selected_present: replay_core::SelectedPresentReceipt::sealed(
            1,
            "c".repeat(64),
            12,
            0,
            60_759_359,
            200,
            300,
        ),
        submits: vec![RenderSubmit {
            submit_index: 12,
            global_order: 2,
            presentation_qpc_ticks: 200,
            interpolation_phase_f32_bits: 0,
            ordered_scene_constant_buffers: vec![binding.clone()],
            ordered_static_map_programs: Vec::new(),
            effect_lifecycle_events: Vec::new(),
            particles: Vec::new(),
            entity_presentations: Vec::new(),
            particle_packet_set_sha256: "5".repeat(64),
            entities: Vec::new(),
            ordered_draws: Vec::new(),
            ordered_render_operations: Vec::new(),
            direct_visuals: Vec::new(),
            light_lifecycle_events: Vec::new(),
            tiled_lights: Vec::new(),
            weather_membership: Vec::new(),
            political_textures: Vec::new(),
        }],
        m02_combat: Some(M02CombatFrame {
            schema: "hoi4.production-m02-combat-indicators.v1".into(),
            update_index: 16_959_360,
            first_sequence: 10,
            last_sequence: 20,
            ui_pass_draw_count: 1167,
            validated_laws: vec![
                M02LawState::AActiveBaseline,
                M02LawState::BZoomBoundary,
                M02LawState::CResultLifecycle,
                M02LawState::DPortStrikeDiscriminator,
                M02LawState::EVisibilityControl,
            ],
            indicators: Vec::new(),
            ordered_draws: Vec::new(),
        }),
        m03_arrows: None,
        o04_cities: None,
        authenticated_ui_pass: None,
        checkpoint: RenderCheckpoint {
            last_sequence: 20,
            last_update_index: 16_959_360,
            last_submit_index: Some("12".into()),
            next_stable_ids: BTreeMap::new(),
            active_effect_lifecycle: Vec::new(),
            active_entity_presentation: Vec::new(),
            active_direct_visual_producers: Vec::new(),
            active_light_lifecycle: Vec::new(),
            latest_tiled_light_payload: None,
            weather_state_and_ordered_membership: Vec::new(),
            active_effect_set_sha256: "6".repeat(64),
            ordered_particle_packets_sha256: "7".repeat(64),
            ordered_entity_submissions_sha256: "8".repeat(64),
            ordered_direct_visual_producers_sha256: "9".repeat(64),
            ordered_light_submissions_sha256: "a".repeat(64),
            weather_membership_sha256: "b".repeat(64),
        },
    };
    let prepared = renderer.prepare_frame(Some(&frame)).unwrap();
    assert_eq!(prepared.selected_submit_index, Some(12));
    assert_eq!(prepared.ordered_scene_constant_buffers, vec![binding]);
    assert!(prepared.ordered_static_map_programs.is_empty());
    assert!(prepared.ordered_render_operations.is_empty());
    assert!(prepared.political_textures.is_empty());
    assert!(prepared.tiled_lights.is_empty());
    assert!(prepared.m02_combat.is_some());
    assert!(!prepared
        .omitted_data_needs
        .iter()
        .any(|need| need.packet == "M02"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn mod_defined_opaque_program_key_is_admitted_without_semantic_role_omission() {
    let root = temp_root("opaque_program_key");
    struct RemoveOnDrop(PathBuf);
    impl Drop for RemoveOnDrop {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = RemoveOnDrop(root.clone());
    let game = root.join("game");
    let mod_root = root.join("mod-content");
    write(
        &game,
        "gfx/FX/mod_defined_opaque.shader",
        b"base-opaque-shader",
    );
    write(
        &mod_root,
        "descriptor.mod",
        b"name=\"Opaque\"\ndescriptor=\"mod/opaque.mod\"\n",
    );
    write(
        &mod_root,
        "gfx/FX/mod_defined_opaque.shader",
        b"mod-defined-opaque-shader",
    );
    write(
        &mod_root,
        "gfx/FX/installed_but_unused.shader",
        b"must-not-enter-recording-or-renderer-cache",
    );

    let pre_resolver = GameFileResolver::with_mod_paths(&game, vec![mod_root.clone()]).unwrap();
    let expected_used = resolved_used_content(
        &pre_resolver,
        "gfx/FX/mod_defined_opaque.shader",
        &"ab".repeat(32),
    );

    let mut program = StaticMapPassProgram {
        program_ordinal: 0,
        pass_token: "diagnostic.mod_defined_opaque".into(),
        ordered_content_keys: vec!["mod-defined-opaque-key".into()],
        ordered_operations: vec![StaticMapOperation {
            operation_ordinal: 7,
            payload_sha256: String::new(),
            command: StaticMapCommand::Draw {
                draw: opaque_static_draw(expected_used),
            },
        }],
        payload_sha256: String::new(),
    };
    program.seal_payload_hashes().unwrap();
    let opaque_consumer = format!("production_map.static_program/{}", program.payload_sha256);
    let (mut renderer, manifest, admitted) = admitted_renderer_with_binding(
        &game,
        &mod_root,
        false,
        "gfx/FX/mod_defined_opaque.shader",
        "mod-defined-opaque-key",
        vec![opaque_consumer],
    );
    assert!(renderer.resources().is_empty());
    assert!(admitted
        .resolve("gfx/FX/installed_but_unused.shader")
        .unwrap_err()
        .to_string()
        .contains("rejects unrecorded/default lookup"));

    let raw_bytes = (0u8..32).collect::<Vec<_>>();
    let binding = ConstantBufferBinding {
        stage_mask_u8: 0x03,
        slot: 0,
        byte_offset: 16,
        byte_count: 16,
        sha256: Sha256::hex(&raw_bytes),
        raw_bytes,
        semantic_layout: "SceneCameraAndClock".into(),
    };
    let frame = ProductionRenderFrame {
        schema: replay_core::PRODUCTION_RENDER_SCHEMA.into(),
        authority: RenderAuthority {
            process_id: 7,
            process_start_nonce: 8,
            observer_installed_qpc: 9,
            d3d_device_created_qpc: 10,
            first_creation_sequence: 1,
            hook_epoch_nonce: 11,
            hook_set_receipt_sha256: "a".repeat(64),
            capture_receipt_sha256: "b".repeat(64),
            launch_epoch_sha256: replay_core::compute_render_launch_epoch_sha256(
                7,
                8,
                9,
                10,
                1,
                11,
                &"a".repeat(64),
                &"b".repeat(64),
            ),
            effective_vfs_manifest_sha256: manifest.effective_vfs_manifest_sha256.clone(),
            effect_catalog_sha256: "3".repeat(64),
            entity_animation_catalog_sha256: "4".repeat(64),
            recorded_input_variant: None,
            particle_runtime_mode: ParticleRuntimeMode::SubmittedPacketReplay,
        },
        update_index: 16_959_360,
        first_sequence: 10,
        last_sequence: 20,
        presentation_qpc_frequency_hz: 10_000_000,
        display_submit_index: Some("12".into()),
        selected_present: replay_core::SelectedPresentReceipt::sealed(
            1,
            "c".repeat(64),
            12,
            0,
            60_759_359,
            200,
            300,
        ),
        submits: vec![RenderSubmit {
            submit_index: 12,
            global_order: 2,
            presentation_qpc_ticks: 200,
            interpolation_phase_f32_bits: 0,
            ordered_scene_constant_buffers: vec![binding],
            ordered_static_map_programs: vec![program.clone()],
            effect_lifecycle_events: Vec::new(),
            particles: Vec::new(),
            entity_presentations: Vec::new(),
            particle_packet_set_sha256: "5".repeat(64),
            entities: Vec::new(),
            ordered_draws: Vec::new(),
            ordered_render_operations: vec![RenderExecutionRef::Static {
                operation_ordinal: 7,
            }],
            direct_visuals: Vec::new(),
            light_lifecycle_events: Vec::new(),
            tiled_lights: Vec::new(),
            weather_membership: Vec::new(),
            political_textures: Vec::new(),
        }],
        m02_combat: None,
        m03_arrows: None,
        o04_cities: None,
        authenticated_ui_pass: None,
        checkpoint: RenderCheckpoint {
            last_sequence: 20,
            last_update_index: 16_959_360,
            last_submit_index: Some("12".into()),
            next_stable_ids: BTreeMap::new(),
            active_effect_lifecycle: Vec::new(),
            active_entity_presentation: Vec::new(),
            active_direct_visual_producers: Vec::new(),
            active_light_lifecycle: Vec::new(),
            latest_tiled_light_payload: None,
            weather_state_and_ordered_membership: Vec::new(),
            active_effect_set_sha256: "6".repeat(64),
            ordered_particle_packets_sha256: "7".repeat(64),
            ordered_entity_submissions_sha256: "8".repeat(64),
            ordered_direct_visual_producers_sha256: "9".repeat(64),
            ordered_light_submissions_sha256: "a".repeat(64),
            weather_membership_sha256: "b".repeat(64),
        },
    };

    let prepared = renderer.prepare_frame(Some(&frame)).unwrap();
    assert_eq!(prepared.ordered_static_map_programs, vec![program]);
    assert_eq!(renderer.resources().len(), 1);
    let resource = renderer.resources().get("mod-defined-opaque-key").unwrap();
    assert_eq!(resource.bytes, b"mod-defined-opaque-shader");
    assert_eq!(resource.content.layer_kind, ContentLayerKind::Mod);
    assert!(prepared.omitted_data_needs.iter().all(|need| {
        !need.reason.contains("semantic") && !need.feature.contains("static pass")
    }));
}

#[test]
fn exact_target_initialization_is_sealed_validated_receipted_and_tamper_closed() {
    const SHADER_KEY: &str = "exact-target.shader";
    const INITIAL_KEY: &str = "exact-target.initial";
    const EXTRA_KEY: &str = "exact-target.extra";
    let root = temp_root("exact_target_binding");
    struct RemoveOnDrop(PathBuf);
    impl Drop for RemoveOnDrop {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = RemoveOnDrop(root.clone());
    let game = root.join("game");
    let mod_root = root.join("mod-content");
    let initial_bytes = (0..256).map(|value| value as u8).collect::<Vec<_>>();
    write(&game, "gfx/FX/exact_target.shader", b"exact-target-shader");
    write(&game, "gfx/exact_target_initial.bin", &initial_bytes);
    write(&game, "gfx/exact_target_extra.bin", b"unused-extra");
    write(
        &mod_root,
        "descriptor.mod",
        b"name=\"Exact target\"\ndescriptor=\"mod/exact-target.mod\"\n",
    );

    let pre_resolver = GameFileResolver::with_mod_paths(&game, vec![mod_root.clone()]).unwrap();
    let vfs = "ab".repeat(32);
    let shader = resolved_used_content(&pre_resolver, "gfx/FX/exact_target.shader", &vfs);
    let initial = resolved_used_content(&pre_resolver, "gfx/exact_target_initial.bin", &vfs);

    let make_program = || {
        let mut draw = opaque_static_draw(shader.clone());
        let inline_mip0 = vec![0x11; 16 * 8 * 4];
        let admitted_mip1 = initial_bytes[64..192].to_vec();
        draw.output_state.ordered_color_targets[0] = RenderTargetDesc {
            prepared_target: PreparedRenderTarget::OpaqueRenderTarget {
                attachment_ordinal: 4,
            },
            width: 8,
            height: 4,
            dxgi_format_u32: 28,
            mip_level: 1,
            sample_count: 1,
            sample_quality: 0,
            persistent_content: Some(initial.clone()),
            exact_resource: Some(replay_core::RenderTargetExactResource {
                resource: replay_core::RenderTargetResourceDesc {
                    dimension: TextureResourceDimension::Texture2d,
                    width: 16,
                    height: 8,
                    depth: 1,
                    mip_levels: 2,
                    array_size: 1,
                    dxgi_format_u32: 28,
                    sample_count: 1,
                    sample_quality: 0,
                    usage_u32: 0,
                    bind_flags_u32: 0x20,
                    cpu_access_flags_u32: 0,
                    misc_flags_u32: 0,
                },
                ordered_views: vec![replay_core::RenderTargetViewDesc::RenderTarget {
                    dxgi_format_u32: 28,
                    view: replay_core::RenderTargetAttachmentView::Texture2d { mip_slice: 1 },
                }],
                ordered_initial_subresources: vec![
                    replay_core::RenderTargetInitialSubresource {
                        subresource_ordinal: 0,
                        row_pitch: 64,
                        slice_pitch: 512,
                        source: replay_core::RenderTargetInitialSource::Inline {
                            sha256: Sha256::hex(&inline_mip0),
                            bytes: inline_mip0,
                        },
                    },
                    replay_core::RenderTargetInitialSubresource {
                        subresource_ordinal: 1,
                        row_pitch: 32,
                        slice_pitch: 128,
                        source: replay_core::RenderTargetInitialSource::UsedContentRange {
                            content: initial.clone(),
                            source_byte_offset: 64,
                            byte_count: 128,
                            range_sha256: Sha256::hex(&admitted_mip1),
                        },
                    },
                ],
            }),
        };
        let mrt_inline = vec![0x22; 8 * 4 * 4];
        draw.output_state
            .ordered_color_targets
            .push(RenderTargetDesc {
                prepared_target: PreparedRenderTarget::OpaqueRenderTarget {
                    attachment_ordinal: 5,
                },
                width: 8,
                height: 4,
                dxgi_format_u32: 28,
                mip_level: 0,
                sample_count: 1,
                sample_quality: 0,
                persistent_content: None,
                exact_resource: Some(replay_core::RenderTargetExactResource {
                    resource: replay_core::RenderTargetResourceDesc {
                        dimension: TextureResourceDimension::Texture2d,
                        width: 8,
                        height: 4,
                        depth: 1,
                        mip_levels: 1,
                        array_size: 1,
                        dxgi_format_u32: 28,
                        sample_count: 1,
                        sample_quality: 0,
                        usage_u32: 0,
                        bind_flags_u32: 0x20,
                        cpu_access_flags_u32: 0,
                        misc_flags_u32: 0,
                    },
                    ordered_views: vec![replay_core::RenderTargetViewDesc::RenderTarget {
                        dxgi_format_u32: 28,
                        view: replay_core::RenderTargetAttachmentView::Texture2d { mip_slice: 0 },
                    }],
                    ordered_initial_subresources: vec![
                        replay_core::RenderTargetInitialSubresource {
                            subresource_ordinal: 0,
                            row_pitch: 32,
                            slice_pitch: 128,
                            source: replay_core::RenderTargetInitialSource::Inline {
                                sha256: Sha256::hex(&mrt_inline),
                                bytes: mrt_inline,
                            },
                        },
                    ],
                }),
            });
        draw.pipeline.blend.independent_blend_enabled = true;
        draw.pipeline.blend.additional_targets = vec![replay_core::RenderTargetBlendDesc {
            enabled: false,
            source_color: BlendFactor::One,
            destination_color: BlendFactor::Zero,
            color_operation: BlendOperation::Add,
            source_alpha: BlendFactor::One,
            destination_alpha: BlendFactor::Zero,
            alpha_operation: BlendOperation::Add,
            write_mask_rgba_u8: 0xf,
        }];
        StaticMapPassProgram {
            program_ordinal: 0,
            pass_token: "exact.target.binding".into(),
            ordered_content_keys: vec![SHADER_KEY.into(), INITIAL_KEY.into()],
            ordered_operations: vec![StaticMapOperation {
                operation_ordinal: 7,
                payload_sha256: String::new(),
                command: StaticMapCommand::Draw { draw },
            }],
            payload_sha256: String::new(),
        }
    };
    let admit = |program: &StaticMapPassProgram| {
        let consumer = format!("production_map.static_program/{}", program.payload_sha256);
        admitted_renderer_with_bindings(
            &game,
            &mod_root,
            false,
            vec![
                (
                    "gfx/FX/exact_target.shader",
                    SHADER_KEY,
                    RequiredContentKind::ShaderSource,
                    vec![consumer.clone()],
                ),
                (
                    "gfx/exact_target_initial.bin",
                    INITIAL_KEY,
                    RequiredContentKind::Texture,
                    vec![consumer.clone()],
                ),
                (
                    "gfx/exact_target_extra.bin",
                    EXTRA_KEY,
                    RequiredContentKind::Texture,
                    vec![consumer],
                ),
            ],
        )
    };

    let mut program = make_program();
    program.seal_payload_hashes().unwrap();
    let (mut renderer, manifest, _) = admit(&program);
    let frame = frame_with_static_program(&manifest, program.clone());
    let plan = renderer.prepare_frame(Some(&frame)).unwrap();
    assert_eq!(renderer.resources().len(), 2);
    let prepared = crate::prepare_production_d3d_frame(&plan, renderer.resources()).unwrap();
    assert_eq!(prepared.ordered_static_used_content.len(), 2);
    assert_eq!(
        prepared.ordered_static_used_content[1],
        manifest.required_content[1].content
    );
    let mut trace = ReplayConsumerTrace::default();
    let receipt = crate::execute_production_d3d_frame(&prepared, &mut trace).unwrap();
    assert_eq!(receipt.ordered_used_content.len(), 2);
    assert!(receipt
        .ordered_used_content
        .iter()
        .any(|content| content == &manifest.required_content[1].content));
    let mut wrong_consumer_resources = renderer.resources().clone();
    wrong_consumer_resources
        .get_mut(INITIAL_KEY)
        .unwrap()
        .consumers = vec!["production_map.static_program/foreign".into()];
    let error = crate::prepare_production_d3d_frame(&plan, &wrong_consumer_resources)
        .unwrap_err()
        .to_string();
    assert!(error.contains("not bound to sealed consumer"), "{error}");

    let mut clear_program = make_program();
    let StaticMapCommand::Draw { draw } = &clear_program.ordered_operations[0].command else {
        unreachable!()
    };
    let clear_target = draw.output_state.ordered_color_targets[0].clone();
    clear_program.ordered_content_keys = vec![INITIAL_KEY.into()];
    clear_program.ordered_operations[0].command = StaticMapCommand::ClearColor {
        target: clear_target,
        rgba_f32_bits: [0; 4],
    };
    clear_program.seal_payload_hashes().unwrap();
    let (mut renderer, manifest, _) = admit(&clear_program);
    let clear_frame = frame_with_static_program(&manifest, clear_program);
    let clear_plan = renderer.prepare_frame(Some(&clear_frame)).unwrap();
    let clear_d3d = crate::prepare_production_d3d_frame(&clear_plan, renderer.resources()).unwrap();
    let mut clear_trace = ReplayConsumerTrace::default();
    let clear_receipt = crate::execute_production_d3d_frame(&clear_d3d, &mut clear_trace).unwrap();
    assert_eq!(clear_receipt.ordered_used_content.len(), 1);
    assert_eq!(
        clear_receipt.ordered_used_content[0],
        manifest.required_content[1].content
    );

    let mut omitted = make_program();
    omitted
        .ordered_content_keys
        .retain(|key| key != INITIAL_KEY);
    omitted.seal_payload_hashes().unwrap();
    let (mut renderer, manifest, _) = admit(&omitted);
    let error = renderer
        .prepare_frame(Some(&frame_with_static_program(&manifest, omitted)))
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("outside its ordered manifest keys"),
        "{error}"
    );

    let mut extra = make_program();
    extra.ordered_content_keys.push(EXTRA_KEY.into());
    extra.seal_payload_hashes().unwrap();
    let (mut renderer, manifest, _) = admit(&extra);
    let error = renderer
        .prepare_frame(Some(&frame_with_static_program(&manifest, extra)))
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("unused by its authenticated operations"),
        "{error}"
    );

    let mutate_initial =
        |program: &mut StaticMapPassProgram,
         mutate: &dyn Fn(&mut Vec<replay_core::RenderTargetInitialSubresource>)| {
            let StaticMapCommand::Draw { draw } = &mut program.ordered_operations[0].command else {
                unreachable!()
            };
            let exact = draw.output_state.ordered_color_targets[0]
                .exact_resource
                .as_mut()
                .unwrap();
            mutate(&mut exact.ordered_initial_subresources);
            program.seal_payload_hashes().unwrap();
        };

    let mut hash_tamper = make_program();
    mutate_initial(&mut hash_tamper, &|initials| {
        let replay_core::RenderTargetInitialSource::UsedContentRange { range_sha256, .. } =
            &mut initials[1].source
        else {
            unreachable!()
        };
        *range_sha256 = "00".repeat(32);
    });
    let (mut renderer, manifest, _) = admit(&hash_tamper);
    let plan = renderer
        .prepare_frame(Some(&frame_with_static_program(&manifest, hash_tamper)))
        .unwrap();
    let error = crate::prepare_production_d3d_frame(&plan, renderer.resources())
        .unwrap_err()
        .to_string();
    assert!(error.contains("content-range SHA-256 mismatch"), "{error}");

    let mut range_tamper = make_program();
    mutate_initial(&mut range_tamper, &|initials| {
        let replay_core::RenderTargetInitialSource::UsedContentRange {
            source_byte_offset, ..
        } = &mut initials[1].source
        else {
            unreachable!()
        };
        *source_byte_offset = 65;
    });
    let (mut renderer, manifest, _) = admit(&range_tamper);
    let plan = renderer
        .prepare_frame(Some(&frame_with_static_program(&manifest, range_tamper)))
        .unwrap();
    let error = crate::prepare_production_d3d_frame(&plan, renderer.resources())
        .unwrap_err()
        .to_string();
    assert!(error.contains("content-range SHA-256 mismatch"), "{error}");

    let mut order_tamper = make_program();
    mutate_initial(&mut order_tamper, &|initials| initials.swap(0, 1));
    let (mut renderer, manifest, _) = admit(&order_tamper);
    let error = renderer
        .prepare_frame(Some(&frame_with_static_program(&manifest, order_tamper)))
        .unwrap_err()
        .to_string();
    assert!(error.contains("out of range/order"), "{error}");
}

#[test]
fn o04_city_outputs_are_vfs_admitted_relocated_and_executed_exactly_once() {
    let root_a = temp_root("o04_relocate_a");
    let root_b = temp_root("o04_relocate_b");
    struct RemoveRoots(Vec<PathBuf>);
    impl Drop for RemoveRoots {
        fn drop(&mut self) {
            for root in &self.0 {
                let _ = fs::remove_dir_all(root);
            }
        }
    }
    let _cleanup = RemoveRoots(vec![root_a.clone(), root_b.clone()]);
    for root in [&root_a, &root_b] {
        write(
            &root.join("game"),
            "gfx/FX/o04.shader",
            b"stale-base-shader",
        );
        write(
            &root.join("game"),
            "gfx/entities/city.asset",
            b"stale-base-entity",
        );
        write(
            &root.join("game"),
            "gfx/models/city.mesh",
            b"stale-base-mesh",
        );
        write(
            &root.join("mod-content"),
            "descriptor.mod",
            b"name=\"O04 override\"\ndescriptor=\"mod/o04.mod\"\nreplace_path=\"gfx/FX\"\nreplace_path=\"gfx/entities\"\nreplace_path=\"gfx/models\"\n",
        );
        write(
            &root.join("mod-content"),
            "gfx/FX/o04.shader",
            b"portable-o04-shader",
        );
        write(
            &root.join("mod-content"),
            "gfx/entities/city.asset",
            b"portable-o04-entity",
        );
        write(
            &root.join("mod-content"),
            "gfx/models/city.mesh",
            b"portable-o04-mesh",
        );
    }

    let pre_resolver =
        GameFileResolver::with_mod_paths(&root_a.join("game"), vec![root_a.join("mod-content")])
            .unwrap();
    let shader = resolved_used_content(&pre_resolver, "gfx/FX/o04.shader", &"ab".repeat(32));
    let mut static_draw = opaque_static_draw(shader);
    add_o04_fixture_depth(&mut static_draw.output_state);
    let mut program = StaticMapPassProgram {
        program_ordinal: 0,
        pass_token: "map_viewer.production_map.o04_fixture".into(),
        ordered_content_keys: vec!["o04.shader".into()],
        ordered_operations: vec![StaticMapOperation {
            operation_ordinal: 7,
            payload_sha256: String::new(),
            command: StaticMapCommand::Draw { draw: static_draw },
        }],
        payload_sha256: String::new(),
    };
    program.seal_payload_hashes().unwrap();
    let static_consumer = format!("production_map.static_program/{}", program.payload_sha256);
    let bindings = || {
        vec![
            (
                "gfx/FX/o04.shader",
                "o04.shader",
                RequiredContentKind::ShaderSource,
                vec![static_consumer.clone(), "production_render.o04_city".into()],
            ),
            (
                "gfx/entities/city.asset",
                "o04.entity",
                RequiredContentKind::Definition,
                vec!["production_render.o04_city".into()],
            ),
            (
                "gfx/models/city.mesh",
                "o04.mesh",
                RequiredContentKind::Mesh,
                vec!["production_render.o04_city".into()],
            ),
        ]
    };
    let (mut left, left_manifest, _) = admitted_renderer_with_bindings(
        &root_a.join("game"),
        &root_a.join("mod-content"),
        false,
        bindings(),
    );
    let (mut right, right_manifest, _) = admitted_renderer_with_bindings(
        &root_b.join("game"),
        &root_b.join("mod-content"),
        false,
        bindings(),
    );
    assert_eq!(
        left_manifest.required_content,
        right_manifest.required_content
    );
    assert!(left_manifest
        .required_content
        .iter()
        .all(|row| row.content.layer_kind == ContentLayerKind::Mod));

    let content = |key: &str| {
        left_manifest
            .required_content
            .iter()
            .find(|row| row.content_key == key)
            .unwrap()
            .content
            .clone()
    };
    let mut frame = frame_with_static_program(&left_manifest, program);
    attach_o04_city(
        &mut frame,
        content("o04.entity"),
        content("o04.mesh"),
        content("o04.shader"),
    );
    frame.validate().unwrap();
    let encoded = serde_json::to_vec(&frame).unwrap();
    assert!(!std::str::from_utf8(&encoded).unwrap().contains("C:\\"));
    let relocated_path = root_b.join("shared").join("o04-frame.json");
    fs::create_dir_all(relocated_path.parent().unwrap()).unwrap();
    fs::write(&relocated_path, encoded).unwrap();
    let relocated: ProductionRenderFrame =
        serde_json::from_slice(&fs::read(&relocated_path).unwrap()).unwrap();

    let left_plan = left.prepare_frame(Some(&frame)).unwrap();
    let right_plan = right.prepare_frame(Some(&relocated)).unwrap();
    assert_eq!(left_plan, right_plan);
    assert!(left_plan.o04_cities.is_some());
    assert_eq!(
        left_plan
            .ordered_runtime_draws
            .iter()
            .filter(|draw| matches!(draw, PreparedRuntimeDraw::O04City { .. }))
            .count(),
        3
    );
    assert!(left_plan
        .omitted_data_needs
        .iter()
        .all(|need| need.packet != "O04"));

    let prepared = crate::prepare_production_d3d_frame(&left_plan, left.resources()).unwrap();
    let mut trace = O04ConsumerTrace::default();
    let receipt = crate::execute_production_d3d_frame(&prepared, &mut trace).unwrap();
    assert_eq!(
        receipt
            .ordered_draws
            .iter()
            .filter(|draw| draw.pass == "o04_city")
            .count(),
        3
    );
    assert_eq!(
        trace
            .calls
            .iter()
            .filter(|call| call.ends_with(":o04"))
            .count(),
        3
    );

    let mut without_carrier = frame.clone();
    without_carrier.o04_cities = None;
    let omitted = left.prepare_frame(Some(&without_carrier)).unwrap();
    assert!(omitted
        .ordered_runtime_draws
        .iter()
        .all(|draw| !matches!(draw, PreparedRuntimeDraw::O04City { .. })));
    assert!(omitted
        .omitted_data_needs
        .iter()
        .any(|need| need.packet == "O04"));
}

#[test]
fn o04_duplicate_dangling_and_unadmitted_references_fail_closed() {
    let root = temp_root("o04_negative");
    struct RemoveRoot(PathBuf);
    impl Drop for RemoveRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = RemoveRoot(root.clone());
    let game = root.join("game");
    let mod_root = root.join("mod-content");
    write(&game, "gfx/FX/o04.shader", b"base");
    write(&game, "gfx/entities/city.asset", b"base");
    write(&game, "gfx/models/city.mesh", b"base");
    write(
        &mod_root,
        "descriptor.mod",
        b"name=\"O04\"\ndescriptor=\"mod/o04.mod\"\n",
    );
    write(&mod_root, "gfx/FX/o04.shader", b"shader");
    write(&mod_root, "gfx/entities/city.asset", b"entity");
    write(&mod_root, "gfx/models/city.mesh", b"mesh");
    let (mut renderer, manifest, _) = admitted_renderer_with_bindings(
        &game,
        &mod_root,
        false,
        vec![
            (
                "gfx/FX/o04.shader",
                "o04.shader",
                RequiredContentKind::ShaderSource,
                vec!["production_render.o04_city".into()],
            ),
            (
                "gfx/entities/city.asset",
                "o04.entity",
                RequiredContentKind::Definition,
                vec!["production_render.o04_city".into()],
            ),
            (
                "gfx/models/city.mesh",
                "o04.mesh",
                RequiredContentKind::Mesh,
                vec!["production_render.o04_city".into()],
            ),
        ],
    );
    let content = |key: &str| {
        manifest
            .required_content
            .iter()
            .find(|row| row.content_key == key)
            .unwrap()
            .content
            .clone()
    };
    let mut seed_program = StaticMapPassProgram {
        program_ordinal: 0,
        pass_token: "o04.negative.seed".into(),
        ordered_content_keys: Vec::new(),
        ordered_operations: vec![StaticMapOperation {
            operation_ordinal: 7,
            payload_sha256: String::new(),
            command: StaticMapCommand::Draw {
                draw: opaque_static_draw(content("o04.shader")),
            },
        }],
        payload_sha256: String::new(),
    };
    seed_program.seal_payload_hashes().unwrap();
    let mut frame = frame_with_static_program(&manifest, seed_program);
    frame.submits[0].ordered_static_map_programs.clear();
    frame.submits[0].ordered_render_operations.clear();
    attach_o04_city(
        &mut frame,
        content("o04.entity"),
        content("o04.mesh"),
        content("o04.shader"),
    );
    frame.validate().unwrap();

    let mut duplicate = frame.clone();
    let first = duplicate.o04_cities.as_ref().unwrap().ordered_outputs[0].clone();
    let o04 = duplicate.o04_cities.as_mut().unwrap();
    o04.cities[0].referenced_outputs.push(first.clone());
    o04.ordered_outputs.push(first);
    assert!(renderer
        .prepare_frame(Some(&duplicate))
        .unwrap_err()
        .to_string()
        .contains("duplicated"));

    let mut dangling = frame.clone();
    let missing = RenderOutputRef::Entity {
        pass_ordinal: 99,
        entity_instance_id: 81,
        draw_order: 199,
    };
    let o04 = dangling.o04_cities.as_mut().unwrap();
    o04.cities[0].referenced_outputs[2] = missing.clone();
    o04.ordered_outputs[2] = missing;
    let error = renderer
        .prepare_frame(Some(&dangling))
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("display submit") || error.contains("entity submission"),
        "{error}"
    );

    let mut unadmitted = frame.clone();
    let mut foreign = content("o04.entity");
    foreign.normalized_game_relative_path = "gfx/entities/unadmitted.asset".into();
    for entity in &mut unadmitted.submits[0].entities {
        entity.content_identity.resolved_entity_asset = foreign.clone();
        entity.resolved_entity_sha256 = foreign.payload_sha256.clone();
    }
    unadmitted.o04_cities.as_mut().unwrap().cities[0].entity_asset = foreign;
    let error = renderer
        .prepare_frame(Some(&unadmitted))
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("unrecorded")
            || error.contains("not admitted")
            || error.contains("unlisted path")
            || error.contains("not bound to production_render.o04_city"),
        "{error}"
    );
}

#[test]
fn retained_capture_interpreter_relocates_through_vfs_and_executes_in_d77_order() {
    let capture_root = std::env::var_os("BATTLE_PLAN_RETAINED_PRODUCTION_COMPLETE_FRAME").or_else(|| std::env::var_os("HOI4_RETAINED_PRODUCTION_COMPLETE_FRAME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(
                r"C:\tmp\hoi4_rendering_law_capture\20260717T000008Z-production-complete-frame",
            )
        });
    let capture_gpu = capture_root.join("gpu");
    assert!(
        capture_gpu.is_dir(),
        "retained production-complete-frame evidence is absent: {}",
        capture_gpu.display()
    );
    let vertices =
        fs::read(capture_gpu.join("trees_frame_draw000_ia_vertex_buffer_slot0.bin")).unwrap();
    let indices = fs::read(capture_gpu.join("trees_frame_draw000_ia_index_buffer.bin")).unwrap();
    let vertex_shader =
        fs::read(capture_gpu.join("map_vertex_shader_02bd512fa7acd53f.bin")).unwrap();
    let pixel_shader =
        fs::read(capture_gpu.join("tree_pixel_shader_277b7b5e8964214f.bin")).unwrap();
    assert_eq!(vertices.len(), 168);
    assert_eq!(indices.len(), 6);
    assert_eq!(
        Sha256::hex(&vertices),
        "9471cd041ab3ab4f051a0089349078308f041a84adb72d742fa01b6c8537fd3d"
    );
    assert_eq!(
        Sha256::hex(&indices),
        "6594e6422836c2ffcad0f91ea8ac41f780e817dfcc956134a721e311b0408b32"
    );

    let root_a = temp_root("retained_static_a");
    let root_b = temp_root("retained_static_b");
    struct RemoveRoots(Vec<PathBuf>);
    impl Drop for RemoveRoots {
        fn drop(&mut self) {
            for root in &self.0 {
                let _ = fs::remove_dir_all(root);
            }
        }
    }
    let _cleanup = RemoveRoots(vec![root_a.clone(), root_b.clone()]);
    for root in [&root_a, &root_b] {
        let game = root.join("game");
        let mod_root = root.join("mod-content");
        write(&game, "gfx/FX/retained_tree_vs.bin", b"stale-base-shader");
        write(
            &game,
            "map/generated/retained_tree_vb.bin",
            b"stale-base-vertices",
        );
        write(
            &game,
            "map/generated/retained_tree_ib.bin",
            b"stale-base-indices",
        );
        write(
            &mod_root,
            "descriptor.mod",
            b"name=\"Retained portable tree\"\ndescriptor=\"mod/retained.mod\"\nreplace_path=\"gfx/FX\"\nreplace_path=\"map/generated\"\n",
        );
        write(&mod_root, "gfx/FX/retained_tree_vs.bin", &vertex_shader);
        write(&mod_root, "map/generated/retained_tree_vb.bin", &vertices);
        write(&mod_root, "map/generated/retained_tree_ib.bin", &indices);
    }

    const PASS: &str = "map_viewer.production_map.retained_tree";
    const SHADER_KEY: &str = "retained.tree.shader";
    const VERTEX_KEY: &str = "retained.tree.vertices";
    const INDEX_KEY: &str = "retained.tree.indices";
    let pre_resolver =
        GameFileResolver::with_mod_paths(&root_a.join("game"), vec![root_a.join("mod-content")])
            .unwrap();
    let vfs = "ab".repeat(32);
    let shader_content = resolved_used_content(&pre_resolver, "gfx/FX/retained_tree_vs.bin", &vfs);
    let vertex_content =
        resolved_used_content(&pre_resolver, "map/generated/retained_tree_vb.bin", &vfs);
    let index_content =
        resolved_used_content(&pre_resolver, "map/generated/retained_tree_ib.bin", &vfs);
    assert_eq!(shader_content.layer_kind, ContentLayerKind::Mod);
    assert_eq!(vertex_content.layer_kind, ContentLayerKind::Mod);
    assert_eq!(index_content.layer_kind, ContentLayerKind::Mod);

    let mut draw = opaque_static_draw(shader_content);
    draw.draw_token = "retained.tree.draw000".into();
    draw.shader_program.compiled_vertex_shader.bytecode = vertex_shader.clone();
    draw.shader_program.compiled_vertex_shader.sha256 = Sha256::hex(&vertex_shader);
    draw.shader_program.compiled_pixel_shader.bytecode = pixel_shader.clone();
    draw.shader_program.compiled_pixel_shader.sha256 = Sha256::hex(&pixel_shader);
    draw.pipeline.index_format = IndexFormat::R16Uint;
    draw.pipeline.static_indices_u16 = vec![0, 1, 2];
    draw.geometry = StaticGeometrySource::InterpreterOutput {
        interpreter_token: format!("{PASS}.{}", crate::RAW_VERTEX_INDEX_BUFFERS_V1),
        ordered_input_content_keys: vec![VERTEX_KEY.into(), INDEX_KEY.into()],
        ordered_vertex_buffers: vec![InterpreterVertexBufferDesc {
            input_slot: 0,
            stride_bytes: 56,
            byte_offset: 0,
            vertex_count: 3,
            payload_sha256: Sha256::hex(&vertices),
        }],
        index_buffer: Some(InterpreterIndexBufferDesc {
            format: IndexFormat::R16Uint,
            byte_offset: 0,
            index_count: 3,
            payload_sha256: Sha256::hex(&indices),
        }),
        stream_output_targets: Vec::new(),
        draw: DrawCommand::DrawIndexed {
            index_count: 3,
            start_index: 0,
            base_vertex: 0,
        },
    };
    let mut program = StaticMapPassProgram {
        program_ordinal: 0,
        pass_token: PASS.into(),
        ordered_content_keys: vec![SHADER_KEY.into(), VERTEX_KEY.into(), INDEX_KEY.into()],
        ordered_operations: vec![StaticMapOperation {
            operation_ordinal: 7,
            payload_sha256: String::new(),
            command: StaticMapCommand::Draw { draw },
        }],
        payload_sha256: String::new(),
    };
    program.seal_payload_hashes().unwrap();
    let sealed_consumer = format!("production_map.static_program/{}", program.payload_sha256);
    let bindings = || {
        vec![
            (
                "gfx/FX/retained_tree_vs.bin",
                SHADER_KEY,
                RequiredContentKind::CompiledShader,
                vec![sealed_consumer.clone()],
            ),
            (
                "map/generated/retained_tree_vb.bin",
                VERTEX_KEY,
                RequiredContentKind::Geometry,
                vec![PASS.into(), sealed_consumer.clone()],
            ),
            (
                "map/generated/retained_tree_ib.bin",
                INDEX_KEY,
                RequiredContentKind::Geometry,
                vec![PASS.into(), sealed_consumer.clone()],
            ),
        ]
    };
    let (mut left, left_manifest, _) = admitted_renderer_with_bindings(
        &root_a.join("game"),
        &root_a.join("mod-content"),
        true,
        bindings(),
    );
    let (mut right, right_manifest, _) = admitted_renderer_with_bindings(
        &root_b.join("game"),
        &root_b.join("mod-content"),
        true,
        bindings(),
    );
    assert_eq!(
        left_manifest.required_content,
        right_manifest.required_content
    );

    let frame = frame_with_static_program(&left_manifest, program);
    let encoded = serde_json::to_vec(&frame).unwrap();
    let portable_text = std::str::from_utf8(&encoded).unwrap();
    let capture_root_text = capture_root.to_string_lossy().into_owned();
    for forbidden in ["C:\\\\", "OneDrive", capture_root_text.as_str()] {
        assert!(
            !portable_text.contains(forbidden),
            "portable frame leaked {forbidden}"
        );
    }
    let source_file = root_a.join("recording").join("portable-static-frame.json");
    write(
        source_file.parent().unwrap(),
        source_file.file_name().unwrap().to_str().unwrap(),
        &encoded,
    );
    let relocated = root_b.join("shared").join("relocated-static-frame.json");
    fs::create_dir_all(relocated.parent().unwrap()).unwrap();
    fs::copy(&source_file, &relocated).unwrap();
    let relocated_frame: ProductionRenderFrame =
        serde_json::from_slice(&fs::read(&relocated).unwrap()).unwrap();

    let left_plan = left.prepare_frame(Some(&frame)).unwrap();
    let right_plan = right.prepare_frame(Some(&relocated_frame)).unwrap();
    assert_eq!(left_plan, right_plan);
    assert_eq!(left.resources(), right.resources());
    assert_eq!(right.resources()[VERTEX_KEY].bytes, vertices);
    assert_eq!(right.resources()[INDEX_KEY].bytes, indices);

    let left_d3d = crate::prepare_production_d3d_frame(&left_plan, left.resources()).unwrap();
    let right_d3d = crate::prepare_production_d3d_frame(&right_plan, right.resources()).unwrap();
    assert_eq!(left_d3d, right_d3d);
    let StaticMapCommand::Draw { draw } =
        &right_d3d.ordered_static_programs[0].ordered_operations[0].command
    else {
        unreachable!()
    };
    let StaticGeometrySource::Exact { geometry } = &draw.geometry else {
        panic!("supported retained interpreter was not materialized")
    };
    let BufferSource::Inline {
        bytes: materialized_vertices,
        ..
    } = &geometry.ordered_vertex_buffers[0].source
    else {
        panic!("materialized retained vertices are not exact inline bytes")
    };
    let BufferSource::Inline {
        bytes: materialized_indices,
        ..
    } = &geometry.index_buffer.as_ref().unwrap().source
    else {
        panic!("materialized retained indices are not exact inline bytes")
    };
    assert_eq!(materialized_vertices, &vertices);
    assert_eq!(materialized_indices, &indices);

    let mut left_trace = ReplayConsumerTrace::default();
    let left_receipt = crate::execute_production_d3d_frame(&left_d3d, &mut left_trace).unwrap();
    let mut right_trace = ReplayConsumerTrace::default();
    let right_receipt = crate::execute_production_d3d_frame(&right_d3d, &mut right_trace).unwrap();
    assert_eq!(left_receipt, right_receipt);
    assert_eq!(left_trace.calls, right_trace.calls);
    assert_eq!(
        right_trace.calls,
        vec![
            "begin:false",
            "0:map_viewer.production_map.retained_tree:retained.tree.draw000",
            "finish",
        ]
    );
    assert_eq!(right_receipt.ordered_draws.len(), 1);
    assert_eq!(right_receipt.ordered_draws[0].global_ordinal, 0);
    assert_eq!(right_receipt.ordered_used_content.len(), 3);

    let mut tampered_frame = relocated_frame.clone();
    tampered_frame.submits[0].ordered_static_map_programs[0]
        .pass_token
        .push_str(".tampered");
    let error = right
        .prepare_frame(Some(&tampered_frame))
        .unwrap_err()
        .to_string();
    assert!(error.contains("payload SHA-256 mismatch"), "{error}");

    let mut tampered_resources = left.resources().clone();
    tampered_resources.get_mut(VERTEX_KEY).unwrap().bytes[0] ^= 0xff;
    let error = crate::prepare_production_d3d_frame(&left_plan, &tampered_resources)
        .unwrap_err()
        .to_string();
    assert!(error.contains("payload SHA-256 mismatch"), "{error}");
}
