use super::*;

const FIXTURE_COLOR_ATTACHMENT_ORDINAL: u32 = 4;
const FIXTURE_DEPTH_ATTACHMENT_ORDINAL: u32 = 9;

#[derive(Default)]
struct TraceBridge {
    calls: Vec<String>,
}

impl ProductionD3dExecutionBridge for TraceBridge {
    fn begin_frame(&mut self, frame: &PreparedProductionD3dFrame) -> Result<()> {
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
        self.calls.push(format!(
            "{global_ordinal}:{}:{}",
            pass.pass_token,
            static_operation_token(operation)
        ));
        Ok(())
    }

    fn execute_runtime_draw(
        &mut self,
        global_ordinal: u32,
        _: &[ConstantBufferBinding],
        _: &PreparedRuntimeDraw,
    ) -> Result<()> {
        self.calls.push(format!("{global_ordinal}:runtime"));
        Ok(())
    }

    fn execute_m03_draw(
        &mut self,
        global_ordinal: u32,
        _: &[ConstantBufferBinding],
        _: &replay_core::M03ArrowDraw,
    ) -> Result<()> {
        self.calls.push(format!("{global_ordinal}:m03"));
        Ok(())
    }

    fn finish_frame(&mut self) -> Result<()> {
        self.calls.push("finish".into());
        Ok(())
    }
}

fn empty_frame() -> PreparedProductionMapFrame {
    PreparedProductionMapFrame {
        ordered_static_map_programs: Vec::new(),
        rail_ribbon: None,
        snow_mud_texture: None,
        tree_runtime: None,
        s1_shadow: None,
        political_textures: Vec::new(),
        tiled_lights: Vec::new(),
        content_manifest_sha256: "11".repeat(32),
        product_modset_sha256: "99".repeat(32),
        effective_vfs_manifest_sha256: "22".repeat(32),
        selected_submit_index: None,
        selected_submit_global_order: None,
        c03_camera_replay: None,
        ordered_scene_constant_buffers: Vec::new(),
        ordered_render_operations: Vec::new(),
        ordered_runtime_draws: Vec::new(),
        m02_combat: None,
        authenticated_ui_pass: None,
        m03_arrows: None,
        rail_supply_arrows: None,
        o04_cities: None,
        o05_buildings: None,
        omitted_data_needs: Vec::new(),
    }
}

fn static_clear(operation_ordinal: u32) -> StaticMapOperation {
    StaticMapOperation {
        operation_ordinal,
        payload_sha256: format!("{:064x}", operation_ordinal),
        command: StaticMapCommand::ClearColor {
            target: replay_core::RenderTargetDesc {
                prepared_target: replay_core::PreparedRenderTarget::OpaqueRenderTarget {
                    attachment_ordinal: FIXTURE_COLOR_ATTACHMENT_ORDINAL,
                },
                width: 8,
                height: 4,
                dxgi_format_u32: 28,
                mip_level: 0,
                sample_count: 1,
                sample_quality: 0,
                persistent_content: None,
                exact_resource: None,
            },
            rgba_f32_bits: [0, 0, 0, 0],
        },
    }
}

fn prepared_static_frame(operations: Vec<StaticMapOperation>) -> PreparedProductionD3dFrame {
    let ordered_render_operations = operations
        .iter()
        .map(|operation| RenderExecutionRef::Static {
            operation_ordinal: operation.operation_ordinal,
        })
        .collect();
    PreparedProductionD3dFrame {
        content_manifest_sha256: "11".repeat(32),
        product_modset_sha256: "99".repeat(32),
        effective_vfs_manifest_sha256: "22".repeat(32),
        c03_camera_replay: None,
        target: None,
        ordered_scene_constant_buffers: Vec::new(),
        ordered_static_programs: vec![StaticMapPassProgram {
            program_ordinal: 0,
            pass_token: "map_viewer.production_map.terrain".into(),
            ordered_content_keys: Vec::new(),
            ordered_operations: operations,
            payload_sha256: "33".repeat(32),
        }],
        rail_ribbon: None,
        mapname_d80_state: replay_core::MapnameD80State::Missing,
        tree_runtime: None,
        tree_materialized_execution_sha256: None,
        s1_shadow: None,
        ordered_static_used_content: Vec::new(),
        ordered_render_operations,
        snow_mud_texture: None,
        ordered_runtime_draws: Vec::new(),
        political_textures: Vec::new(),
        tiled_lights: Vec::new(),
        m02_combat: None,
        authenticated_ui_pass: None,
        m03_arrows: None,
        rail_supply_arrows: None,
        omissions: Vec::new(),
    }
}

#[test]
fn headless_execution_receipt_is_byte_deterministic() {
    let prepared = prepare_production_d3d_frame(&empty_frame(), &BTreeMap::new()).unwrap();
    let mut first_bridge = TraceBridge::default();
    let first = execute_production_d3d_frame(&prepared, &mut first_bridge).unwrap();
    let mut second_bridge = TraceBridge::default();
    let second = execute_production_d3d_frame(&prepared, &mut second_bridge).unwrap();
    assert_eq!(first, second);
    assert_eq!(first_bridge.calls, vec!["begin:false", "finish"]);
    assert_eq!(first_bridge.calls, second_bridge.calls);
    assert_eq!(first.deterministic_sha256.len(), 64);
    assert_eq!(first.mapname_d80_state, replay_core::MapnameD80State::Missing);
}

fn c03_binding() -> C03CameraReplayBinding {
    C03CameraReplayBinding {
        schema: crate::C03_CAMERA_REPLAY_BINDING_SCHEMA.into(),
        submit_index: 7,
        submit_global_order: 3,
        carrier_payload_sha256: "ab".repeat(32),
        event_batch_count: 2,
        raw_input_event_count: 4,
        map_pick_count: 1,
        current_eye_f32_bits: [1, 2, 3],
        target_eye_f32_bits: [4, 5, 6],
        target_look_at_f32_bits: [7, 8, 9],
        mode: crate::C03CameraReplayMode::RecordedEnvelopeStateOnly,
        raw_event_reapplication_count: 0,
        envelope_state_selection_count: 1,
    }
}

#[test]
fn v7_receipt_layout_seals_product_modset_and_concurrent_authority_fields_in_order() {
    let receipt = ProductionD3dExecutionReceipt {
        schema: PRODUCTION_D3D_EXECUTION_RECEIPT_SCHEMA.into(),
        content_manifest_sha256: "11".repeat(32),
        product_modset_sha256: "99".repeat(32),
        effective_vfs_manifest_sha256: "22".repeat(32),
        c03_camera_replay: Some(c03_binding()),
        tree_runtime_payload_sha256: Some("33".repeat(32)),
        tree_materialized_execution_sha256: Some("34".repeat(32)),
        s1_shadow_payload_sha256: Some("44".repeat(32)),
        s1_materialized_execution_sha256: Some("55".repeat(32)),
        mapname_d80_state: replay_core::MapnameD80State::Authenticated,
        target: None,
        ordered_draws: Vec::new(),
        ordered_used_content: Vec::new(),
        omissions: Vec::new(),
        deterministic_sha256: String::new(),
    };
    let json = serde_json::to_string(&receipt).unwrap();
    let ordered_fields = [
        "schema",
        "content_manifest_sha256",
        "product_modset_sha256",
        "effective_vfs_manifest_sha256",
        "c03_camera_replay",
        "tree_runtime_payload_sha256",
        "tree_materialized_execution_sha256",
        "s1_shadow_payload_sha256",
        "s1_materialized_execution_sha256",
        "mapname_d80_state",
        "target",
        "ordered_draws",
        "ordered_used_content",
        "omissions",
        "deterministic_sha256",
    ];
    let positions = ordered_fields.map(|field| {
        json.find(&format!("\"{field}\":"))
            .expect("v6 receipt field")
    });
    assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));

    let sealed = snapshot::sha256::Sha256::hex(&serde_json::to_vec(&receipt).unwrap());
    let mut missing_materialized_tree = receipt.clone();
    missing_materialized_tree.tree_materialized_execution_sha256 = None;
    let missing_tree_seal =
        snapshot::sha256::Sha256::hex(&serde_json::to_vec(&missing_materialized_tree).unwrap());
    assert_ne!(sealed, missing_tree_seal);

    let mut missing_materialized_s1 = receipt;
    missing_materialized_s1.s1_materialized_execution_sha256 = None;
    let missing_seal =
        snapshot::sha256::Sha256::hex(&serde_json::to_vec(&missing_materialized_s1).unwrap());
    assert_ne!(sealed, missing_seal);
}

#[test]
fn d3d_preparation_refuses_missing_product_modset_provenance() {
    let mut frame = empty_frame();
    frame.product_modset_sha256.clear();
    let error = prepare_production_d3d_frame(&frame, &BTreeMap::new()).unwrap_err();
    assert!(error.to_string().contains("product modset digest"), "{error}");
}

#[test]
fn c07_materialized_seal_without_a_selected_frame_carrier_fails_before_execution() {
    let mut prepared = prepared_static_frame(Vec::new());
    prepared.tree_materialized_execution_sha256 = Some("33".repeat(32));
    let mut bridge = TraceBridge::default();
    assert!(execute_production_d3d_frame(&prepared, &mut bridge)
        .unwrap_err()
        .to_string()
        .contains("carrier/execution seal nullability mismatch"));
    assert!(bridge.calls.is_empty());
}

#[test]
fn c03_binding_reaches_execution_receipt_without_reapplying_raw_events() {
    let mut frame = empty_frame();
    frame.selected_submit_index = Some(7);
    frame.selected_submit_global_order = Some(3);
    frame.c03_camera_replay = Some(c03_binding());
    let prepared = prepare_production_d3d_frame(&frame, &BTreeMap::new()).unwrap();
    let mut bridge = TraceBridge::default();
    let receipt = execute_production_d3d_frame(&prepared, &mut bridge).unwrap();
    let binding = receipt.c03_camera_replay.as_ref().unwrap();
    assert_eq!(binding, &c03_binding());
    assert_eq!(binding.raw_event_reapplication_count, 0);
    assert_eq!(binding.envelope_state_selection_count, 1);
    assert_eq!(receipt.schema, PRODUCTION_D3D_EXECUTION_RECEIPT_SCHEMA);

    let mut unbound = prepared;
    unbound.c03_camera_replay = None;
    let unbound_receipt =
        execute_production_d3d_frame(&unbound, &mut TraceBridge::default()).unwrap();
    assert_ne!(receipt.deterministic_sha256, unbound_receipt.deterministic_sha256);
}

#[test]
fn c03_binding_fails_closed_on_double_apply_or_selected_submit_mismatch() {
    let mut double_apply = empty_frame();
    double_apply.selected_submit_index = Some(7);
    double_apply.selected_submit_global_order = Some(3);
    let mut binding = c03_binding();
    binding.raw_event_reapplication_count = 1;
    double_apply.c03_camera_replay = Some(binding);
    assert!(prepare_production_d3d_frame(&double_apply, &BTreeMap::new())
        .unwrap_err()
        .to_string()
        .contains("identity/policy mismatch"));

    let mut wrong_submit = empty_frame();
    wrong_submit.selected_submit_index = Some(8);
    wrong_submit.selected_submit_global_order = Some(3);
    wrong_submit.c03_camera_replay = Some(c03_binding());
    assert!(prepare_production_d3d_frame(&wrong_submit, &BTreeMap::new())
        .unwrap_err()
        .to_string()
        .contains("different selected submit"));
}

#[test]
fn absent_portable_static_programs_produce_no_static_pixels() {
    let prepared = prepare_production_d3d_frame(&empty_frame(), &BTreeMap::new()).unwrap();
    assert!(prepared.ordered_static_programs.is_empty());
    assert!(prepared.ordered_static_used_content.is_empty());
}

#[test]
fn legacy_mapname_program_is_omitted_and_receipted_without_reconstruction() {
    let mut frame = empty_frame();
    let mut draw = fixture_static_draw();
    draw.shader_program.effect_token = "mapname".into();
    frame.ordered_static_map_programs = vec![StaticMapPassProgram {
        program_ordinal: 0,
        pass_token: "legacy-mapname".into(),
        ordered_content_keys: Vec::new(),
        ordered_operations: vec![StaticMapOperation {
            operation_ordinal: 7,
            payload_sha256: "55".repeat(32),
            command: StaticMapCommand::Draw { draw },
        }],
        payload_sha256: "66".repeat(32),
    }];
    frame.ordered_render_operations = vec![RenderExecutionRef::Static {
        operation_ordinal: 7,
    }];

    let prepared = prepare_production_d3d_frame(&frame, &BTreeMap::new()).unwrap();
    assert_eq!(
        prepared.mapname_d80_state,
        replay_core::MapnameD80State::LegacyUnstamped
    );
    assert!(prepared.ordered_static_programs.is_empty());
    assert!(prepared.ordered_render_operations.is_empty());
    assert!(prepared.omissions.iter().any(|omission| omission
        .reason
        .starts_with("B10 legacy mapname program omitted:")));

    let receipt =
        execute_production_d3d_frame(&prepared, &mut TraceBridge::default()).unwrap();
    assert_eq!(receipt.schema, PRODUCTION_D3D_EXECUTION_RECEIPT_SCHEMA);
    assert_eq!(
        receipt.mapname_d80_state,
        replay_core::MapnameD80State::LegacyUnstamped
    );
}

#[test]
fn authenticated_mapname_executes_but_a_legacy_claim_cannot_bypass_the_guard() {
    let mut raw = Vec::with_capacity(32);
    for bits in [
        0.5f32.to_bits(),
        (-5632.0f32).to_bits(),
        0,
        0,
        0,
        0,
        0,
        0,
    ] {
        raw.extend_from_slice(&bits.to_le_bytes());
    }
    let mut draw = fixture_static_draw();
    draw.shader_program.effect_token = "mapname".into();
    draw.ordered_constant_buffers = vec![ConstantBufferBinding {
        stage_mask_u8: 0b11,
        slot: 1,
        byte_offset: 0,
        byte_count: 32,
        sha256: snapshot::sha256::Sha256::hex(&raw),
        raw_bytes: raw,
        semantic_layout: replay_core::MAPNAME_TRANSP_OFFSET_SEMANTIC.into(),
    }];
    let operation = StaticMapOperation {
        operation_ordinal: 7,
        payload_sha256: "55".repeat(32),
        command: StaticMapCommand::Draw { draw },
    };
    let mut prepared = prepared_static_frame(vec![operation]);
    prepared.mapname_d80_state = replay_core::MapnameD80State::Authenticated;
    let receipt =
        execute_production_d3d_frame(&prepared, &mut TraceBridge::default()).unwrap();
    assert_eq!(
        receipt.mapname_d80_state,
        replay_core::MapnameD80State::Authenticated
    );

    prepared.mapname_d80_state = replay_core::MapnameD80State::LegacyUnstamped;
    let mut bridge = TraceBridge::default();
    assert!(execute_production_d3d_frame(&prepared, &mut bridge)
        .unwrap_err()
        .to_string()
        .contains("state/omission disagrees"));
    assert!(bridge.calls.is_empty());
}

#[test]
fn m02_carrier_without_authenticated_ui_pass_omits_entire_pixel_route() {
    let mut frame = empty_frame();
    frame.m02_combat = Some(replay_core::M02CombatFrame {
        schema: "hoi4.production-m02-combat-indicators.v1".into(),
        update_index: 7,
        first_sequence: 10,
        last_sequence: 20,
        ui_pass_draw_count: 1167,
        validated_laws: vec![
            replay_core::M02LawState::AActiveBaseline,
            replay_core::M02LawState::BZoomBoundary,
            replay_core::M02LawState::CResultLifecycle,
            replay_core::M02LawState::DPortStrikeDiscriminator,
            replay_core::M02LawState::EVisibilityControl,
        ],
        indicators: Vec::new(),
        ordered_draws: Vec::new(),
    });

    let prepared = prepare_production_d3d_frame(&frame, &BTreeMap::new()).unwrap();
    assert!(prepared.m02_combat.is_some());
    assert!(prepared.ordered_render_operations.is_empty());
    assert!(prepared.omissions.iter().any(|omission| {
        omission.pass == "m02_combat_indicators"
            && omission
                .reason
                .contains("authenticated selected-Present UI-pass operation stream is absent")
            && omission
                .reason
                .contains("entire M02 pixel route is omitted")
    }));
}

#[test]
fn unified_order_is_the_only_static_execution_sequence() {
    let frame = prepared_static_frame(vec![static_clear(5), static_clear(9)]);
    let mut bridge = TraceBridge::default();
    execute_production_d3d_frame(&frame, &mut bridge).unwrap();
    assert_eq!(
        bridge.calls,
        vec![
            "begin:false",
            "0:map_viewer.production_map.terrain:clear_color",
            "1:map_viewer.production_map.terrain:clear_color",
            "finish",
        ]
    );
}

#[test]
fn unified_order_rejects_duplicate_unknown_and_omitted_static_operations() {
    let base = prepared_static_frame(vec![static_clear(5), static_clear(9)]);
    for (order, expected) in [
        (
            vec![
                RenderExecutionRef::Static {
                    operation_ordinal: 5,
                },
                RenderExecutionRef::Static {
                    operation_ordinal: 5,
                },
            ],
            "repeats a static operation",
        ),
        (
            vec![
                RenderExecutionRef::Static {
                    operation_ordinal: 5,
                },
                RenderExecutionRef::Static {
                    operation_ordinal: 77,
                },
            ],
            "absent static operation",
        ),
        (
            vec![RenderExecutionRef::Static {
                operation_ordinal: 5,
            }],
            "does not cover every static/runtime operation",
        ),
    ] {
        let mut frame = base.clone();
        frame.ordered_render_operations = order;
        let error = execute_production_d3d_frame(&frame, &mut TraceBridge::default())
            .unwrap_err()
            .to_string();
        assert!(error.contains(expected), "{error}");
    }

    let mut no_order = base;
    no_order.ordered_render_operations.clear();
    assert!(
        execute_production_d3d_frame(&no_order, &mut TraceBridge::default())
            .unwrap_err()
            .to_string()
            .contains("no authoritative cross-family execution order")
    );
}

/// Structurally complete draw packet for boundary fixtures that must never be
/// executed; the typed refusal under test fires before any field is read.
fn fixture_static_draw() -> replay_core::StaticMapDraw {
    use replay_core::{
        BlendDesc, BlendFactor, BlendOperation, BufferSource, Comparison, CompiledShaderPayload,
        ContentLayerKind, CullMode, DepthDesc, DrawCommand, FillMode, GeometryBinding, IndexFormat,
        PipelineDesc, PrimitiveTopology, RasterizerDesc, RenderOutputState, ShaderProgram,
        StaticGeometrySource, StaticMapDraw, StencilFaceDesc, StencilOperation,
        VertexBufferBinding, ViewportDesc,
    };

    let bytecode = vec![0u8; 4];
    let compiled = |profile: &str| CompiledShaderPayload {
        compiler_identity: "d3dcompiler_test_fixture".into(),
        target_profile: profile.into(),
        compiler_flags_u64: 0,
        sha256: snapshot::sha256::Sha256::hex(&bytecode),
        bytecode: bytecode.clone(),
    };
    let stencil = StencilFaceDesc {
        fail_operation: StencilOperation::Keep,
        depth_fail_operation: StencilOperation::Keep,
        pass_operation: StencilOperation::Keep,
        comparison: Comparison::Always,
    };
    let vertices = vec![0u8; 24];
    StaticMapDraw {
        draw_token: "test.fixture.never_executed".into(),
        shader_program: ShaderProgram {
            source: replay_core::UsedContentKey {
                effective_vfs_manifest_sha256: "22".repeat(32),
                normalized_game_relative_path: "gfx/fx/fixture.hlsl".into(),
                winning_layer_ordinal: 0,
                layer_kind: ContentLayerKind::Base,
                layer_id: "base".into(),
                source_sha256: "77".repeat(32),
                payload_sha256: "77".repeat(32),
                payload_bytes: 1,
            },
            producer_registration_root_rva: None,
            effect_token: "test.fixture".into(),
            vertex_entry_token: "vs_main".into(),
            pixel_entry_token: "ps_main".into(),
            ordered_defines: Vec::new(),
            compiled_vertex_shader: compiled("vs_5_0"),
            compiled_pixel_shader: compiled("ps_5_0"),
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
            input_layout: Vec::new(),
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
                        sha256: snapshot::sha256::Sha256::hex(&vertices),
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
            ordered_color_targets: vec![replay_core::RenderTargetDesc {
                prepared_target: replay_core::PreparedRenderTarget::OpaqueRenderTarget {
                    attachment_ordinal: FIXTURE_COLOR_ATTACHMENT_ORDINAL,
                },
                width: 8,
                height: 4,
                dxgi_format_u32: 28,
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
                width_f32_bits: 8f32.to_bits(),
                height_f32_bits: 4f32.to_bits(),
                min_depth_f32_bits: 0,
                max_depth_f32_bits: 1f32.to_bits(),
            },
            scissor: None,
        },
        payload_sha256: "44".repeat(32),
    }
}

#[test]
fn m03_carrier_without_cross_family_order_is_a_typed_refusal() {
    use replay_core::{M03ArrowDraw, M03ArrowDrawMember, M03ArrowDrawRef, M03StableKey};

    // A populated M03 carrier with no authoritative cross-family execution
    // order must be refused, never silently executed with the M03 draws
    // absent from bridge calls and the receipt.
    let mut frame = prepared_static_frame(Vec::new());
    frame.m03_arrows = Some(replay_core::M03ArrowFrame {
        schema: "hoi4.production-m03-order-movement-arrows.v1".into(),
        executable_sha256: "88".repeat(32),
        static_semantic_proof_sha256: "99".repeat(32),
        update_index: 3,
        first_sequence: 10,
        last_sequence: 20,
        used_content: Vec::new(),
        rows: Vec::new(),
        draws: vec![M03ArrowDraw {
            engine_draw_ordinal: 0,
            draw: fixture_static_draw(),
            members: vec![M03ArrowDrawMember {
                stable_key: M03StableKey::Movement {
                    owner_country_id: 1,
                    division_id: 2,
                    movement_generation: 3,
                },
                member_ordinal: 0,
                first_vertex: 0,
                vertex_count: 3,
            }],
        }],
        ordered_draws: vec![M03ArrowDrawRef {
            engine_draw_ordinal: 0,
        }],
    });
    frame.ordered_static_programs.clear();
    frame.ordered_render_operations.clear();

    let mut bridge = TraceBridge::default();
    let error = execute_production_d3d_frame(&frame, &mut bridge)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("M03 arrow draws have no authoritative cross-family execution order"),
        "{error}"
    );
    // The refusal is total: nothing was drawn behind the missing order.
    assert_eq!(bridge.calls, vec!["begin:false"]);
}

#[test]
fn target_validation_rejects_format_order_and_sample_drift() {
    let base = ProductionMainSceneTargetDesc {
        color_attachment_ordinal: FIXTURE_COLOR_ATTACHMENT_ORDINAL,
        color_width: 64,
        color_height: 32,
        color_format: 10,
        color_samples: 4,
        color_quality: 0,
        depth_attachment_ordinal: FIXTURE_DEPTH_ATTACHMENT_ORDINAL,
        depth_width: 64,
        depth_height: 32,
        depth_format: 45,
        depth_samples: 4,
        depth_quality: 0,
    };
    base.validate().unwrap();
    let mut bad = base.clone();
    bad.depth_samples = 1;
    assert!(bad
        .validate()
        .unwrap_err()
        .to_string()
        .contains("multisample"));
    bad = base;
    bad.depth_width = 63;
    assert!(bad
        .validate()
        .unwrap_err()
        .to_string()
        .contains("dimensions"));
}

#[test]
fn production_module_has_no_capture_or_vanilla_asset_route() {
    let source = include_str!("../production_d3d.rs");
    for forbidden in [
        "--bundle",
        "--capture",
        "map_frame_bundle",
        "vanilla_",
        "Program Files",
    ] {
        assert!(
            !source.contains(forbidden),
            "forbidden production dependency: {forbidden}"
        );
    }
}

#[cfg(windows)]
#[test]
fn offscreen_d3d11_clear_readback_is_deterministic() {
    let desc = ProductionMainSceneTargetDesc {
        color_attachment_ordinal: FIXTURE_COLOR_ATTACHMENT_ORDINAL,
        color_width: 8,
        color_height: 4,
        color_format: 28,
        color_samples: 1,
        color_quality: 0,
        depth_attachment_ordinal: FIXTURE_DEPTH_ATTACHMENT_ORDINAL,
        depth_width: 8,
        depth_height: 4,
        depth_format: 45,
        depth_samples: 1,
        depth_quality: 0,
    };
    let mut backend = NativeProductionD3dBackend::create_offscreen(desc)
        .expect("Windows production D3D proof requires a working D3D11 device");
    backend.clear([0.25, 0.5, 0.75, 1.0], 1.0, 0).unwrap();
    let first = backend.readback().unwrap();
    let second = backend.readback().unwrap();
    assert_eq!(first, second);
    assert_eq!(first.bytes.len(), 8 * 4 * 4);
}

#[cfg(windows)]
#[test]
fn admitted_portable_exact_draw_changes_every_main_scene_pixel() {
    use replay_core::{
        BlendDesc, BlendFactor, BlendOperation, BufferSource, Comparison, CompiledShaderPayload,
        CullMode, DepthDesc, DrawCommand, FillMode, GeometryBinding, IndexFormat, InputElementDesc,
        PipelineDesc, PrimitiveTopology, RasterizerDesc, RenderOutputState, ShaderProgram,
        StaticGeometrySource, StaticMapDraw, StencilFaceDesc, StencilOperation,
        VertexBufferBinding, ViewportDesc,
    };
    use std::fs;

    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("map_viewer_native_draw_{nonce}"));
    struct RemoveOnDrop(std::path::PathBuf);
    impl Drop for RemoveOnDrop {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = RemoveOnDrop(root.clone());
    let shader_path = root.join("gfx").join("fx").join("portable_test.hlsl");
    fs::create_dir_all(shader_path.parent().unwrap()).unwrap();
    let hlsl = r#"
struct VSIn { float2 position : POSITION; };
float4 vs_main(VSIn input) : SV_Position { return float4(input.position, 0.0, 1.0); }
float4 ps_main() : SV_Target { return float4(1.0, 0.0, 0.0, 1.0); }
"#;
    fs::write(&shader_path, hlsl).unwrap();
    let vfs_hash = "22".repeat(32);
    let (resolver, source) = crate::GameFileResolver::vanilla(&root)
        .unwrap()
        .bind_exact_test_content("gfx/fx/portable_test.hlsl", &vfs_hash)
        .unwrap();

    let compile = |entry: &str, target: &str| -> Vec<u8> {
        unsafe {
            let blob = crate::d3d11_device::native_d3d11_compile_shader(hlsl, entry, target)
                .expect("production draw proof shader must compile");
            std::slice::from_raw_parts(
                (*blob.as_raw()).GetBufferPointer() as *const u8,
                (*blob.as_raw()).GetBufferSize(),
            )
            .to_vec()
        }
    };
    let vs = compile("vs_main", "vs_5_0");
    let ps = compile("ps_main", "ps_5_0");
    let desc = ProductionMainSceneTargetDesc {
        color_attachment_ordinal: FIXTURE_COLOR_ATTACHMENT_ORDINAL,
        color_width: 8,
        color_height: 4,
        color_format: 28,
        color_samples: 1,
        color_quality: 0,
        depth_attachment_ordinal: FIXTURE_DEPTH_ATTACHMENT_ORDINAL,
        depth_width: 8,
        depth_height: 4,
        depth_format: 45,
        depth_samples: 1,
        depth_quality: 0,
    };
    let mut backend = NativeProductionD3dBackend::create_offscreen(desc.clone())
        .expect("Windows production draw proof requires a working D3D11 device");
    backend.clear([0.0, 0.0, 0.0, 1.0], 1.0, 0).unwrap();

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
    let draw = StaticMapDraw {
        draw_token: "test.portable.solid_red".into(),
        shader_program: ShaderProgram {
            source,
            producer_registration_root_rva: None,
            effect_token: "test.solid_red".into(),
            vertex_entry_token: "vs_main".into(),
            pixel_entry_token: "ps_main".into(),
            ordered_defines: Vec::new(),
            compiled_vertex_shader: CompiledShaderPayload {
                compiler_identity: "d3dcompiler_test_fixture".into(),
                target_profile: "vs_5_0".into(),
                compiler_flags_u64: 0,
                sha256: snapshot::sha256::Sha256::hex(&vs),
                bytecode: vs,
            },
            compiled_pixel_shader: CompiledShaderPayload {
                compiler_identity: "d3dcompiler_test_fixture".into(),
                target_profile: "ps_5_0".into(),
                compiler_flags_u64: 0,
                sha256: snapshot::sha256::Sha256::hex(&ps),
                bytecode: ps,
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
                        sha256: snapshot::sha256::Sha256::hex(&vertices),
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
            ordered_color_targets: vec![replay_core::RenderTargetDesc {
                prepared_target: replay_core::PreparedRenderTarget::OpaqueRenderTarget {
                    attachment_ordinal: FIXTURE_COLOR_ATTACHMENT_ORDINAL,
                },
                width: 8,
                height: 4,
                dxgi_format_u32: 28,
                mip_level: 0,
                sample_count: 1,
                sample_quality: 0,
                persistent_content: None,
                exact_resource: None,
            }],
            depth_target: Some(replay_core::RenderTargetDesc {
                prepared_target: replay_core::PreparedRenderTarget::OpaqueRenderTarget {
                    attachment_ordinal: FIXTURE_DEPTH_ATTACHMENT_ORDINAL,
                },
                width: 8,
                height: 4,
                dxgi_format_u32: 45,
                mip_level: 0,
                sample_count: 1,
                sample_quality: 0,
                persistent_content: None,
                exact_resource: None,
            }),
            viewport: ViewportDesc {
                top_left_x_f32_bits: 0,
                top_left_y_f32_bits: 0,
                width_f32_bits: 8f32.to_bits(),
                height_f32_bits: 4f32.to_bits(),
                min_depth_f32_bits: 0,
                max_depth_f32_bits: 1f32.to_bits(),
            },
            scissor: None,
        },
        payload_sha256: "44".repeat(32),
    };
    let frame = PreparedProductionD3dFrame {
        content_manifest_sha256: "11".repeat(32),
        product_modset_sha256: "99".repeat(32),
        effective_vfs_manifest_sha256: vfs_hash,
        c03_camera_replay: None,
        target: Some(desc),
        ordered_scene_constant_buffers: Vec::new(),
        ordered_static_programs: vec![StaticMapPassProgram {
            program_ordinal: 0,
            pass_token: "map_viewer.production_map.terrain".into(),
            ordered_content_keys: vec!["test-shader".into()],
            ordered_operations: vec![StaticMapOperation {
                operation_ordinal: 7,
                payload_sha256: "55".repeat(32),
                command: StaticMapCommand::Draw { draw },
            }],
            payload_sha256: "66".repeat(32),
        }],
        rail_ribbon: None,
        mapname_d80_state: replay_core::MapnameD80State::Missing,
        tree_runtime: None,
        tree_materialized_execution_sha256: None,
        s1_shadow: None,
        ordered_static_used_content: Vec::new(),
        ordered_render_operations: vec![RenderExecutionRef::Static {
            operation_ordinal: 7,
        }],
        snow_mud_texture: None,
        ordered_runtime_draws: Vec::new(),
        political_textures: Vec::new(),
        tiled_lights: Vec::new(),
        m02_combat: None,
        authenticated_ui_pass: None,
        m03_arrows: None,
        rail_supply_arrows: None,
        omissions: Vec::new(),
    };
    let receipt = {
        let resources = BTreeMap::new();
        let mut executor =
            NativeProductionD3dExecutor::new(&mut backend, &resolver, &resources).unwrap();
        execute_production_d3d_frame(&frame, &mut executor).unwrap()
    };
    assert_eq!(receipt.ordered_draws.len(), 1);
    assert_eq!(receipt.ordered_draws[0].global_ordinal, 0);
    assert_eq!(
        receipt.ordered_draws[0].draw_token,
        "test.portable.solid_red"
    );
    let readback = backend.readback().unwrap();
    assert!(readback
        .bytes
        .chunks_exact(4)
        .all(|pixel| pixel == [255, 0, 0, 255]));
    let mut substituted = frame.clone();
    let StaticMapCommand::Draw { draw } =
        &mut substituted.ordered_static_programs[0].ordered_operations[0].command
    else {
        unreachable!()
    };
    draw.output_state.ordered_color_targets[0].prepared_target =
        replay_core::PreparedRenderTarget::OpaqueRenderTarget {
            attachment_ordinal: FIXTURE_COLOR_ATTACHMENT_ORDINAL + 1,
        };
    {
        let resources = BTreeMap::new();
        let mut executor =
            NativeProductionD3dExecutor::new(&mut backend, &resolver, &resources).unwrap();
        let carried = execute_production_d3d_frame(&substituted, &mut executor).unwrap();
        assert_eq!(carried.ordered_draws.len(), 1);
        assert_eq!(
            carried.ordered_draws[0].draw_token,
            "test.portable.solid_red"
        );
    }
    let StaticMapCommand::Draw { draw } =
        &mut substituted.ordered_static_programs[0].ordered_operations[0].command
    else {
        unreachable!()
    };
    let initial = vec![0x5a; 8 * 4 * 4];
    draw.output_state.ordered_color_targets[0].exact_resource =
        Some(replay_core::RenderTargetExactResource {
            resource: replay_core::RenderTargetResourceDesc {
                dimension: replay_core::TextureResourceDimension::Texture2d,
                width: 8,
                height: 4,
                depth: 1,
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
            ordered_views: vec![replay_core::RenderTargetViewDesc::RenderTarget {
                dxgi_format_u32: 28,
                view: replay_core::RenderTargetAttachmentView::Texture2d { mip_slice: 0 },
            }],
            ordered_initial_subresources: vec![replay_core::RenderTargetInitialSubresource {
                subresource_ordinal: 0,
                row_pitch: 32,
                slice_pitch: 128,
                source: replay_core::RenderTargetInitialSource::Inline {
                    sha256: snapshot::sha256::Sha256::hex(&initial),
                    bytes: initial,
                },
            }],
        });
    {
        let resources = BTreeMap::new();
        let mut executor =
            NativeProductionD3dExecutor::new(&mut backend, &resolver, &resources).unwrap();
        let exact = execute_production_d3d_frame(&substituted, &mut executor).unwrap();
        assert_eq!(exact.ordered_draws.len(), 1);
    }
    unsafe {
        backend
            .attach_presentation_target(backend.color_rtv(), 8, 4, 28, 1, 0)
            .unwrap();
    }
    backend.clear([0.0, 0.0, 0.0, 1.0], 1.0, 0).unwrap();
    let mut presented = frame.clone();
    let StaticMapCommand::Draw { draw } =
        &mut presented.ordered_static_programs[0].ordered_operations[0].command
    else {
        unreachable!()
    };
    draw.output_state.ordered_color_targets[0].prepared_target =
        replay_core::PreparedRenderTarget::SwapchainBackBuffer;
    draw.output_state.depth_target = None;
    {
        let resources = BTreeMap::new();
        let mut executor =
            NativeProductionD3dExecutor::new(&mut backend, &resolver, &resources).unwrap();
        execute_production_d3d_frame(&presented, &mut executor).unwrap();
    }
    let presented_readback = backend.readback().unwrap();
    assert!(presented_readback
        .bytes
        .chunks_exact(4)
        .all(|pixel| pixel == [255, 0, 0, 255]));
}
