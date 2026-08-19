
use crate::{GameFileResolver, PreparedXmap15Frame, Result};
use portable_render_stream::{Xmap15PlanKey, Xmap15PreparedRow};
use serde::Serialize;
use snapshot::sha256::Sha256;
use std::collections::{BTreeMap, BTreeSet};

pub const XMAP15_MAPARROW_CB3_BYTES: u32 = 128;
pub const XMAP15_MOVEMENT_CB1_BYTES: u32 = 112;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Xmap15NativeRowReceipt {
    pub final_engine_draw_ordinal: u32,
    pub stable_key: String,
    pub definition_name: String,
    pub visible: bool,
    pub submitted_draw_count: u32,
    pub submitted_vertex_count: u32,
    pub submitted_index_count: u32,
    pub geometry_sha256: String,
    pub constant_payload_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Xmap15NativeExecutionReceipt {
    pub prepared_frame_sha256: String,
    pub assigned_stage_name: String,
    pub assigned_operation_name: String,
    pub assigned_stage_ordinal: u32,
    pub assigned_stage_assignment_sha256: String,
    pub lifecycle_update_sha256: String,
    pub target_sha256: String,
    pub scene_cb0_payload_sha256: String,
    pub point_lights_cb2_payload_sha256: String,
    pub ordered_rows: Vec<Xmap15NativeRowReceipt>,
    pub authenticated_empty: bool,
    pub deterministic_sha256: String,
}

impl Xmap15NativeExecutionReceipt {
    pub fn validate(&self) -> Result<()> {
        if self.prepared_frame_sha256.len() != 64
            || self.assigned_stage_name != crate::XMAP15_ASSIGNED_STAGE_NAME
            || self.assigned_operation_name != crate::XMAP15_ASSIGNED_OPERATION_NAME
            || self.assigned_stage_assignment_sha256.len() != 64
            || self.lifecycle_update_sha256.len() != 64
            || self.target_sha256.len() != 64
            || self.scene_cb0_payload_sha256.len() != 64
            || self.point_lights_cb2_payload_sha256.len() != 64
            || self.authenticated_empty != self.ordered_rows.is_empty()
            || self.deterministic_sha256 != execution_digest(self)?
        {
            return Err("XMAP15 native execution receipt is invalid".into());
        }
        let mut ordinals = BTreeSet::new();
        if self.ordered_rows.iter().any(|row| {
            !ordinals.insert(row.final_engine_draw_ordinal)
                || row.geometry_sha256.len() != 64
                || row.constant_payload_sha256.len() != 64
                || (!row.visible && row.submitted_draw_count != 0)
        }) {
            return Err("XMAP15 native row receipt order/identity is invalid".into());
        }
        Ok(())
    }
}

fn execution_digest(receipt: &Xmap15NativeExecutionReceipt) -> Result<String> {
    let mut unsigned = receipt.clone();
    unsigned.deterministic_sha256.clear();
    Ok(Sha256::hex(&serde_json::to_vec(&unsigned)?))
}

#[cfg(windows)]
pub use native::{NativeXmap15DrawInputs, NativeXmap15Handoff};

#[cfg(windows)]
mod native {
    use super::*;
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use crate::{
        NativeXmap16ExternalConstantBufferView, NativeXmap16ExternalResourceView,
        NativeXmap16MainSceneTargetView, NativeXmap16SceneConstantBufferView,
        PreparedXmap15MovementDefinition, PreparedXmap15PlanDefinition,
    };
    use std::{ffi::CString, ptr};
    use winapi::shared::dxgiformat::{
        DXGI_FORMAT_R16_UINT, DXGI_FORMAT_R32G32B32A32_FLOAT, DXGI_FORMAT_R32G32B32_FLOAT,
        DXGI_FORMAT_R32G32_FLOAT,
    };
    use winapi::shared::minwindef::{FALSE, TRUE};
    use winapi::um::d3d11::*;
    use winapi::um::d3dcommon::{
        D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP,
    };

    struct Program {
        vertex: D3dPtr<ID3D11VertexShader>,
        pixel: D3dPtr<ID3D11PixelShader>,
    }

    struct PlanTextures {
        masks: Vec<crate::xmap16_supply_arrow_handoff::NativeXmap16SupplyTexture>,
        patterns: Vec<crate::xmap16_supply_arrow_handoff::NativeXmap16SupplyTexture>,
    }

    struct MovementTextures {
        diffuse: crate::xmap16_supply_arrow_handoff::NativeXmap16SupplyTexture,
        normal: crate::xmap16_supply_arrow_handoff::NativeXmap16SupplyTexture,
        specular: crate::xmap16_supply_arrow_handoff::NativeXmap16SupplyTexture,
    }

    enum RowGeometry {
        Plan {
            buffer: D3dPtr<ID3D11Buffer>,
            vertex_count: u32,
            total_length_f32_bits: u32,
            wrap_offset_world: i32,
        },
        Movement {
            vertex_buffer: D3dPtr<ID3D11Buffer>,
            index_buffer: Option<D3dPtr<ID3D11Buffer>>,
        },
    }

    struct NativeRow {
        geometry: RowGeometry,
        geometry_sha256: String,
    }

    pub struct NativeXmap15Handoff {
        prepared_frame_sha256: String,
        plan_programs: BTreeMap<String, Program>,
        movement_program: Option<Program>,
        plan_layout: Option<D3dPtr<ID3D11InputLayout>>,
        movement_layout: Option<D3dPtr<ID3D11InputLayout>>,
        plan_textures: BTreeMap<String, PlanTextures>,
        movement_textures: BTreeMap<String, MovementTextures>,
        rows: Vec<NativeRow>,
        cb3: D3dPtr<ID3D11Buffer>,
        movement_cb1: D3dPtr<ID3D11Buffer>,
        blend: D3dPtr<ID3D11BlendState>,
        normal_depth: D3dPtr<ID3D11DepthStencilState>,
        see_through_depth: D3dPtr<ID3D11DepthStencilState>,
        depth_write: D3dPtr<ID3D11DepthStencilState>,
        depth_test_only: D3dPtr<ID3D11DepthStencilState>,
        no_depth: D3dPtr<ID3D11DepthStencilState>,
        rasterizer: D3dPtr<ID3D11RasterizerState>,
        plan_samplers: Vec<(u32, D3dPtr<ID3D11SamplerState>)>,
        movement_samplers: Vec<(u32, D3dPtr<ID3D11SamplerState>)>,
        required_external_resources: Vec<(String, u32)>,
    }

    pub struct NativeXmap15DrawInputs<'a, S>
    where
        S: NativeXmap16SceneConstantBufferView,
    {
        pub context: *mut ID3D11DeviceContext,
        pub target: &'a dyn NativeXmap16MainSceneTargetView,
        pub target_sha256: &'a str,
        pub prepared_frame: &'a PreparedXmap15Frame,
        pub lifecycle_update_sha256: &'a str,
        pub scene_cb0: &'a S,
        pub point_lights_cb2: &'a NativeXmap16ExternalConstantBufferView,
        pub external_resources: &'a [NativeXmap16ExternalResourceView],
        pub view_projection_matrix: [[f32; 4]; 4],
        pub application_elapsed_seconds: f32,
    }

    impl NativeXmap15Handoff {
        pub unsafe fn create_on_device(
            device: *mut ID3D11Device,
            resolver: &GameFileResolver,
            frame: &PreparedXmap15Frame,
        ) -> Result<Self> {
            frame.validate()?;
            if device.is_null() {
                return Err("XMAP15 native handoff received a null device".into());
            }
            verify_shader_closure(resolver, frame)?;

            let constants = crate::derive_a729_engine_map_shader_constants(
                frame.map_width,
                frame.map_height,
                false,
                0,
            )?;
            let defines = crate::engine_map_shader_defines(constants);
            let mut effects = frame
                .plan_definitions
                .iter()
                .map(|definition| definition.technique.clone())
                .collect::<BTreeSet<_>>();
            for row in &frame.portable.ordered_rows {
                if let Xmap15PreparedRow::Plan { semantic, .. } = row {
                    if semantic.see_through {
                        effects.insert(see_through_effect(
                            frame.plan_definition(&semantic.resolved_definition_name)?,
                        )?);
                    }
                }
            }
            crate::production_features::xmap15::validate_xmap15_plan_effect_contracts(
                resolver, &effects,
            )?;
            let mut compiled = BTreeMap::new();
            let mut first_plan_vs = None;
            let mut required_external_resources = BTreeSet::new();
            for effect in effects {
                let stage = crate::xmap16_supply_arrow_handoff::compile_effect(
                    resolver, &effect, &defines,
                )?;
                validate_plan_program_reflection(&stage.vertex_bytecode, &stage.pixel_bytecode)?;
                collect_plan_external_resources(
                    &stage.pixel_bytecode,
                    &mut required_external_resources,
                )?;
                if first_plan_vs.is_none() {
                    first_plan_vs = Some(stage.vertex_bytecode.clone());
                }
                compiled.insert(
                    effect,
                    create_program(
                        device,
                        &stage.vertex_bytecode,
                        &stage.pixel_bytecode,
                        "XMAP15 maparrow",
                    )?,
                );
            }
            let plan_layout = first_plan_vs
                .as_deref()
                .map(|bytecode| create_plan_layout(device, bytecode))
                .transpose()?;

            let has_movement = frame
                .portable
                .ordered_rows
                .iter()
                .any(|row| matches!(row, Xmap15PreparedRow::Movement { .. }));
            let (movement_program, movement_layout) = if has_movement {
                crate::production_features::xmap15::validate_xmap15_movement_effect_contract(
                    resolver,
                )?;
                let (vs, ps) = compile_movement_program(resolver, &defines)?;
                let layout = create_movement_layout(device, &vs)?;
                (
                    Some(create_program(device, &vs, &ps, "XMAP15 movement")?),
                    Some(layout),
                )
            } else {
                (None, None)
            };

            let plan_textures = upload_plan_textures(device, resolver, &frame.plan_definitions)?;
            let movement_textures =
                upload_movement_textures(device, resolver, &frame.movement_definitions)?;
            let rows = upload_rows(device, resolver, frame)?;
            let cb3 =
                create_constant_buffer(device, XMAP15_MAPARROW_CB3_BYTES, "XMAP15 maparrow CB3")?;
            let movement_cb1 =
                create_constant_buffer(device, XMAP15_MOVEMENT_CB1_BYTES, "XMAP15 movement CB1")?;
            let blend =
                crate::xmap16_supply_arrow_handoff::create_supply_arrow_blend_state(device)?;
            let (normal_depth, see_through_depth) =
                crate::xmap16_supply_arrow_handoff::create_supply_arrow_depth_states(device)?;
            let depth_write = create_depth_state(device, true, true, "XMAP15 depth write")?;
            let depth_test_only = create_depth_test_only_state(device)?;
            let no_depth = create_depth_state(device, false, false, "XMAP15 movement no-depth")?;
            let rasterizer =
                crate::xmap16_supply_arrow_handoff::create_supply_arrow_rasterizer_state(device)?;
            let plan_samplers = create_plan_samplers(device)?;
            let movement_samplers = create_movement_samplers(device)?;

            Ok(Self {
                prepared_frame_sha256: frame.portable.deterministic_sha256.clone(),
                plan_programs: compiled,
                movement_program,
                plan_layout,
                movement_layout,
                plan_textures,
                movement_textures,
                rows,
                cb3,
                movement_cb1,
                blend,
                normal_depth,
                see_through_depth,
                depth_write,
                depth_test_only,
                no_depth,
                rasterizer,
                plan_samplers,
                movement_samplers,
                required_external_resources: required_external_resources.into_iter().collect(),
            })
        }

        pub fn prepared_frame_sha256(&self) -> &str {
            &self.prepared_frame_sha256
        }

        pub unsafe fn draw<S>(
            &self,
            inputs: NativeXmap15DrawInputs<'_, S>,
        ) -> Result<Xmap15NativeExecutionReceipt>
        where
            S: NativeXmap16SceneConstantBufferView,
        {
            inputs.prepared_frame.validate()?;
            if self.prepared_frame_sha256 != inputs.prepared_frame.portable.deterministic_sha256
                || self.rows.len() != inputs.prepared_frame.portable.ordered_rows.len()
                || inputs.context.is_null()
                || inputs.target.color_rtv().is_null()
                || inputs.target.depth_dsv().is_null()
                || !inputs.application_elapsed_seconds.is_finite()
            {
                return Err("XMAP15 native draw inputs do not match the retained frame".into());
            }
            validate_sha(inputs.target_sha256, "XMAP15 target")?;
            validate_sha(inputs.lifecycle_update_sha256, "XMAP15 lifecycle")?;
            validate_scene_and_resources(&inputs)?;

            let context = inputs.context;
            let desc = inputs.target.desc();
            desc.validate()?;
            let mut rtv = inputs.target.color_rtv();
            (*context).OMSetRenderTargets(1, &mut rtv, inputs.target.depth_dsv());
            (*context).RSSetViewports(
                1,
                &D3D11_VIEWPORT {
                    TopLeftX: 0.0,
                    TopLeftY: 0.0,
                    Width: desc.color_width as f32,
                    Height: desc.color_height as f32,
                    MinDepth: 0.0,
                    MaxDepth: 1.0,
                },
            );
            (*context).RSSetState(self.rasterizer.as_raw());
            let external =
                ordered_external(inputs.external_resources, &self.required_external_resources)?;
            let mut base_srvs = [ptr::null_mut(); 11];
            for view in &external {
                base_srvs[view.identity.shader_register as usize] = view.as_raw();
            }
            let mut scene_cb0 = inputs.scene_cb0.raw_buffer();
            let mut point_cb2 = inputs.point_lights_cb2.as_raw();
            let mut receipts = Vec::with_capacity(self.rows.len());
            for (row, native) in inputs
                .prepared_frame
                .portable
                .ordered_rows
                .iter()
                .zip(&self.rows)
            {
                let receipt = match (row, &native.geometry) {
                    (
                        Xmap15PreparedRow::Plan { semantic, .. },
                        RowGeometry::Plan {
                            buffer,
                            vertex_count,
                            total_length_f32_bits,
                            wrap_offset_world,
                        },
                    ) => {
                        let definition = inputs
                            .prepared_frame
                            .plan_definition(&semantic.resolved_definition_name)?;
                        let effect = if semantic.see_through {
                            see_through_effect(definition)?
                        } else {
                            definition.technique.clone()
                        };
                        let program = self
                            .plan_programs
                            .get(&effect)
                            .ok_or("XMAP15 plan program vanished")?;
                        bind_program(context, program);
                        (*context).IASetInputLayout(
                            self.plan_layout
                                .as_ref()
                                .ok_or("XMAP15 plan layout vanished")?
                                .as_raw(),
                        );
                        (*context).IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP);
                        let mut raw = buffer.as_raw();
                        let stride = 32u32;
                        let offset = 0u32;
                        (*context).IASetVertexBuffers(0, 1, &mut raw, &stride, &offset);
                        bind_plan_state(self, context, &effect);
                        bind_plan_resources(
                            self,
                            context,
                            &mut base_srvs,
                            &semantic.resolved_definition_name,
                            semantic.texture_variant,
                        )?;
                        (*context).VSSetConstantBuffers(0, 1, &mut scene_cb0);
                        (*context).PSSetConstantBuffers(0, 1, &mut scene_cb0);
                        (*context).PSSetConstantBuffers(2, 1, &mut point_cb2);
                        let mut cb3 = self.cb3.as_raw();
                        (*context).VSSetConstantBuffers(3, 1, &mut cb3);
                        (*context).PSSetConstantBuffers(3, 1, &mut cb3);
                        let selected = inputs.prepared_frame.portable.selected_plan.as_ref()
                            == Some(&semantic.key);
                        let mut constant_hashes = Vec::new();
                        let mut draws = 0u32;
                        let mut submitted_vertices = 0u32;
                        if semantic.visible {
                            for outer in [false, true] {
                                let bytes = plan_cb3(
                                    definition,
                                    semantic,
                                    *total_length_f32_bits,
                                    *wrap_offset_world,
                                    inputs.application_elapsed_seconds,
                                    selected,
                                    outer,
                                )?;
                                (*context).UpdateSubresource(
                                    self.cb3.as_raw() as *mut _,
                                    0,
                                    ptr::null(),
                                    bytes.as_ptr() as *const _,
                                    0,
                                    0,
                                );
                                (*context).Draw(*vertex_count, 0);
                                constant_hashes.push(Sha256::hex(&bytes[..116]));
                                draws += 1;
                                submitted_vertices = submitted_vertices
                                    .checked_add(*vertex_count)
                                    .ok_or("XMAP15 vertex count overflow")?;
                            }
                        }
                        Xmap15NativeRowReceipt {
                            final_engine_draw_ordinal: semantic.final_engine_draw_ordinal,
                            stable_key: plan_key(&semantic.key),
                            definition_name: semantic.resolved_definition_name.clone(),
                            visible: semantic.visible,
                            submitted_draw_count: draws,
                            submitted_vertex_count: submitted_vertices,
                            submitted_index_count: 0,
                            geometry_sha256: native.geometry_sha256.clone(),
                            constant_payload_sha256: Sha256::hex(&serde_json::to_vec(
                                &constant_hashes,
                            )?),
                        }
                    }
                    (
                        Xmap15PreparedRow::Movement { semantic, .. },
                        RowGeometry::Movement {
                            vertex_buffer,
                            index_buffer,
                        },
                    ) => {
                        let program = self
                            .movement_program
                            .as_ref()
                            .ok_or("XMAP15 movement program vanished")?;
                        bind_program(context, program);
                        (*context).IASetInputLayout(
                            self.movement_layout
                                .as_ref()
                                .ok_or("XMAP15 movement layout vanished")?
                                .as_raw(),
                        );
                        (*context).IASetPrimitiveTopology(
                            match semantic.exact_primitive_topology_u32 {
                                4 => D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST,
                                5 => D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP,
                                _ => {
                                    return Err(
                                        "XMAP15 movement topology changed after validation".into()
                                    )
                                }
                            },
                        );
                        let mut raw = vertex_buffer.as_raw();
                        let stride = semantic.exact_vertex_stride_bytes;
                        let offset = 0u32;
                        (*context).IASetVertexBuffers(0, 1, &mut raw, &stride, &offset);
                        (*context).OMSetDepthStencilState(self.no_depth.as_raw(), 0);
                        let blend_factor = [0.0f32; 4];
                        (*context).OMSetBlendState(ptr::null_mut(), &blend_factor, u32::MAX);
                        bind_movement_resources(
                            self,
                            context,
                            semantic.definition.definition_name(),
                        )?;
                        let cb1 = movement_cb1(inputs.view_projection_matrix, semantic)?;
                        (*context).UpdateSubresource(
                            self.movement_cb1.as_raw() as *mut _,
                            0,
                            ptr::null(),
                            cb1.as_ptr() as *const _,
                            0,
                            0,
                        );
                        let mut raw_cb1 = self.movement_cb1.as_raw();
                        (*context).VSSetConstantBuffers(1, 1, &mut raw_cb1);
                        (*context).PSSetConstantBuffers(1, 1, &mut raw_cb1);
                        let draws = u32::from(semantic.visible);
                        if semantic.visible {
                            if let Some(index) = index_buffer {
                                (*context).IASetIndexBuffer(
                                    index.as_raw(),
                                    if semantic.exact_index_format_u32 == 57 {
                                        DXGI_FORMAT_R16_UINT
                                    } else {
                                        winapi::shared::dxgiformat::DXGI_FORMAT_R32_UINT
                                    },
                                    0,
                                );
                                (*context).DrawIndexed(semantic.exact_index_count, 0, 0);
                            } else {
                                (*context).Draw(semantic.exact_vertex_count, 0);
                            }
                        }
                        Xmap15NativeRowReceipt {
                            final_engine_draw_ordinal: semantic.final_engine_draw_ordinal,
                            stable_key: format!(
                                "movement:{}:{}:{}",
                                semantic.key.owner_country,
                                semantic.key.division_id,
                                semantic.key.movement_generation
                            ),
                            definition_name: semantic.definition.definition_name().into(),
                            visible: semantic.visible,
                            submitted_draw_count: draws,
                            submitted_vertex_count: if semantic.visible {
                                semantic.exact_vertex_count
                            } else {
                                0
                            },
                            submitted_index_count: if semantic.visible {
                                semantic.exact_index_count
                            } else {
                                0
                            },
                            geometry_sha256: native.geometry_sha256.clone(),
                            constant_payload_sha256: Sha256::hex(&cb1),
                        }
                    }
                    _ => return Err("XMAP15 retained geometry kind drifted".into()),
                };
                receipts.push(receipt);
            }
            let mut null_srvs = [ptr::null_mut(); 11];
            (*context).PSSetShaderResources(0, 11, null_srvs.as_mut_ptr());
            let mut receipt = Xmap15NativeExecutionReceipt {
                prepared_frame_sha256: inputs.prepared_frame.portable.deterministic_sha256.clone(),
                assigned_stage_name: inputs.prepared_frame.assigned_stage.stage_name.clone(),
                assigned_operation_name: inputs
                    .prepared_frame
                    .assigned_stage
                    .operation_name
                    .clone(),
                assigned_stage_ordinal: inputs.prepared_frame.assigned_stage.assigned_ordinal,
                assigned_stage_assignment_sha256: inputs
                    .prepared_frame
                    .assigned_stage
                    .assignment_sha256
                    .clone(),
                lifecycle_update_sha256: inputs.lifecycle_update_sha256.into(),
                target_sha256: inputs.target_sha256.into(),
                scene_cb0_payload_sha256: inputs.scene_cb0.payload_sha256().into(),
                point_lights_cb2_payload_sha256: inputs.point_lights_cb2.payload_sha256.clone(),
                authenticated_empty: receipts.is_empty(),
                ordered_rows: receipts,
                deterministic_sha256: String::new(),
            };
            receipt.deterministic_sha256 = execution_digest(&receipt)?;
            receipt.validate()?;
            Ok(receipt)
        }
    }

    fn verify_shader_closure(
        resolver: &GameFileResolver,
        frame: &PreparedXmap15Frame,
    ) -> Result<()> {
        let declared = frame
            .portable
            .ordered_used_content
            .iter()
            .map(|key| key.normalized_game_relative_path.to_ascii_lowercase())
            .collect::<BTreeSet<_>>();
        let shaders = [
            frame
                .heightmap
                .as_ref()
                .map(|_| crate::XMAP15_MAPARROW_SHADER_PATH),
            (!frame.movement_definitions.is_empty()).then_some(crate::XMAP15_MOVEMENT_SHADER_PATH),
        ];
        for shader in shaders.into_iter().flatten() {
            let needed = crate::shader_compile::effect_translation_unit_files(resolver, shader)?;
            if std::iter::once(crate::shader_compile::DX11_PLATFORM_HEADER)
                .chain(needed.iter().map(|(path, _)| path.as_str()))
                .any(|path| !declared.contains(&path.to_ascii_lowercase()))
            {
                return Err(format!(
                    "XMAP15 shader translation unit for {shader} escaped admitted content"
                )
                .into());
            }
        }
        match &frame.map_dimensions_content {
            Some(content) => {
                let bytes = resolver.read_admitted_used_content(content)?;
                let (width, height) = crate::production_foundation::bmp_dimensions(&bytes)?;
                if width != frame.map_width || height != frame.map_height {
                    return Err(
                        "XMAP15 shader dimensions drifted from admitted provinces.bmp".into(),
                    );
                }
            }
            None if frame.portable.ordered_rows.is_empty() => {}
            None => return Err("XMAP15 shader-backed frame omitted map dimensions".into()),
        }
        Ok(())
    }

    fn create_program(
        device: *mut ID3D11Device,
        vs: &[u8],
        ps: &[u8],
        role: &str,
    ) -> Result<Program> {
        unsafe {
            let mut raw_vs = ptr::null_mut();
            native_d3d11_hr(
                (*device).CreateVertexShader(
                    vs.as_ptr() as *const _,
                    vs.len(),
                    ptr::null_mut(),
                    &mut raw_vs,
                ),
                &format!("CreateVertexShader({role})"),
            )?;
            let mut raw_ps = ptr::null_mut();
            native_d3d11_hr(
                (*device).CreatePixelShader(
                    ps.as_ptr() as *const _,
                    ps.len(),
                    ptr::null_mut(),
                    &mut raw_ps,
                ),
                &format!("CreatePixelShader({role})"),
            )?;
            Ok(Program {
                vertex: D3dPtr::from_raw(raw_vs, "XMAP15 vertex shader")?,
                pixel: D3dPtr::from_raw(raw_ps, "XMAP15 pixel shader")?,
            })
        }
    }

    fn compile_movement_program(
        resolver: &GameFileResolver,
        defines: &[(String, String)],
    ) -> Result<(Vec<u8>, Vec<u8>)> {
        let vs = crate::shader_compile::build_stage_hlsl_with_defines(
            resolver,
            crate::XMAP15_MOVEMENT_SHADER_PATH,
            "ArrowEffect",
            crate::ShaderStage::Vertex,
            defines,
        )?;
        let ps = crate::shader_compile::build_stage_hlsl_with_defines(
            resolver,
            crate::XMAP15_MOVEMENT_SHADER_PATH,
            "ArrowEffect",
            crate::ShaderStage::Pixel,
            defines,
        )?;
        let compiled = (
            crate::d3d_compile_stage(
                &vs,
                crate::ShaderStage::Vertex,
                crate::shader_compile::ENGINE_COMPILE_FLAGS,
            )?,
            crate::d3d_compile_stage(
                &ps,
                crate::ShaderStage::Pixel,
                crate::shader_compile::ENGINE_COMPILE_FLAGS,
            )?,
        );
        validate_movement_program_reflection(&compiled.0, &compiled.1)?;
        Ok(compiled)
    }

    fn validate_plan_program_reflection(
        vertex_bytecode: &[u8],
        pixel_bytecode: &[u8],
    ) -> Result<()> {
        let vertex = crate::parse_dxbc_reflection(vertex_bytecode)?;
        let pixel = crate::parse_dxbc_reflection(pixel_bytecode)?;
        if vertex.target >> 16 != 0xfffe || pixel.target >> 16 != 0xffff {
            return Err("XMAP15 maparrow DXBC stage kind drifted".into());
        }
        let inputs = vertex
            .inputs
            .iter()
            .map(|input| {
                (
                    input.semantic_name.to_ascii_uppercase(),
                    input.semantic_index,
                    input.mask,
                )
            })
            .collect::<BTreeSet<_>>();
        if inputs
            != BTreeSet::from([
                ("POSITION".to_string(), 0, 0x07),
                ("TEXCOORD".to_string(), 0, 0x0f),
            ])
        {
            return Err("XMAP15 maparrow vertex reflection is not float3+float4".into());
        }
        let expected_cbuffers = crate::XMAP16_SUPPLY_ARROW_CONSTANT_BUFFERS
            .iter()
            .map(|(name, slot, _)| ((*name).to_string(), *slot))
            .collect::<BTreeSet<_>>();
        let actual_cbuffers = pixel
            .resources
            .iter()
            .filter(|resource| resource.resource_type == 0)
            .map(|resource| (resource.name.clone(), resource.bind_point))
            .collect::<BTreeSet<_>>();
        if actual_cbuffers != expected_cbuffers {
            return Err("XMAP15 maparrow reflected constant-buffer slots drifted".into());
        }
        let cb3 = pixel
            .cbuffers
            .iter()
            .find(|cbuffer| cbuffer.name == "dx11_cb3")
            .ok_or("XMAP15 maparrow pixel stage omitted dx11_cb3")?;
        let actual_cb3 = cb3
            .vars
            .iter()
            .map(|var| (var.name.as_str(), var.offset, var.size))
            .collect::<Vec<_>>();
        if cb3.size != XMAP15_MAPARROW_CB3_BYTES
            || actual_cb3 != crate::XMAP16_SUPPLY_ARROW_CB3_LAYOUT
        {
            return Err("XMAP15 maparrow CB3 declaration layout drifted".into());
        }
        Ok(())
    }

    fn validate_movement_program_reflection(
        vertex_bytecode: &[u8],
        pixel_bytecode: &[u8],
    ) -> Result<()> {
        let vertex = crate::parse_dxbc_reflection(vertex_bytecode)?;
        let pixel = crate::parse_dxbc_reflection(pixel_bytecode)?;
        if vertex.target >> 16 != 0xfffe || pixel.target >> 16 != 0xffff {
            return Err("XMAP15 movement DXBC stage kind drifted".into());
        }
        let inputs = vertex
            .inputs
            .iter()
            .map(|input| {
                (
                    input.semantic_name.to_ascii_uppercase(),
                    input.semantic_index,
                    input.mask,
                )
            })
            .collect::<BTreeSet<_>>();
        if inputs
            != BTreeSet::from([
                ("POSITION".to_string(), 0, 0x07),
                ("TEXCOORD".to_string(), 0, 0x03),
                ("TEXCOORD".to_string(), 1, 0x07),
            ])
        {
            return Err("XMAP15 movement vertex reflection is not float3+float2+float3".into());
        }
        for (label, reflection) in [("vertex", &vertex), ("pixel", &pixel)] {
            let cb = reflection
                .cbuffers
                .iter()
                .find(|cbuffer| cbuffer.name == "dx11_cb1")
                .ok_or_else(|| format!("XMAP15 movement {label} stage omitted dx11_cb1"))?;
            let actual = cb
                .vars
                .iter()
                .map(|var| (var.name.as_str(), var.offset, var.size))
                .collect::<Vec<_>>();
            let expected = [
                ("ViewProj", 0, 64),
                ("vProgress_MoveArmy", 64, 12),
                ("vDesaturationLengths", 80, 16),
                ("fOffsetX", 96, 4),
            ];
            if cb.size != XMAP15_MOVEMENT_CB1_BYTES || actual != expected {
                return Err(
                    format!("XMAP15 movement {label} CB1 declaration layout drifted").into(),
                );
            }
        }
        let expected_resources = BTreeSet::from([
            ("DiffuseTexture".to_string(), 0),
            ("NormalMap".to_string(), 1),
        ]);
        for resource_type in [2, 3] {
            let actual = pixel
                .resources
                .iter()
                .filter(|resource| resource.resource_type == resource_type)
                .map(|resource| {
                    let suffix = if resource_type == 2 {
                        "_Texture"
                    } else {
                        "_Sampler"
                    };
                    (
                        resource
                            .name
                            .strip_suffix(suffix)
                            .unwrap_or(&resource.name)
                            .to_string(),
                        resource.bind_point,
                    )
                })
                .collect::<BTreeSet<_>>();
            if actual != expected_resources {
                return Err("XMAP15 movement reflected texture/sampler slots drifted".into());
            }
        }
        Ok(())
    }

    fn collect_plan_external_resources(
        pixel_bytecode: &[u8],
        out: &mut BTreeSet<(String, u32)>,
    ) -> Result<()> {
        let allowed = [
            ("ShadowMap", 2u32),
            ("TerrainIDMap", 3),
            ("HeightMap", 4),
            ("TerrainNormal", 5),
            ("HeightNormal", 6),
            ("LeanTexture1", 7),
            ("LeanTexture2", 8),
            ("LightDataMap", 9),
            ("LightIndexMap", 10),
        ];
        let reflection = crate::parse_dxbc_reflection(pixel_bytecode)?;
        for resource in reflection
            .resources
            .iter()
            .filter(|resource| resource.resource_type == 2 && resource.bind_point >= 2)
        {
            let symbol = resource
                .name
                .strip_suffix("_Texture")
                .unwrap_or(&resource.name);
            if !allowed
                .iter()
                .any(|expected| expected.0 == symbol && expected.1 == resource.bind_point)
            {
                return Err(format!(
                    "XMAP15 maparrow reflected unsupported external resource {symbol}/t{}",
                    resource.bind_point
                )
                .into());
            }
            out.insert((symbol.to_string(), resource.bind_point));
        }
        Ok(())
    }

    unsafe fn create_plan_layout(
        device: *mut ID3D11Device,
        bytecode: &[u8],
    ) -> Result<D3dPtr<ID3D11InputLayout>> {
        if bytecode.is_empty() {
            return Err("XMAP15 plan vertex shader bytecode is empty".into());
        }
        let position = CString::new("POSITION")?;
        let texcoord = CString::new("TEXCOORD")?;
        let elements = [
            D3D11_INPUT_ELEMENT_DESC {
                SemanticName: position.as_ptr(),
                SemanticIndex: 0,
                Format: DXGI_FORMAT_R32G32B32_FLOAT,
                InputSlot: 0,
                AlignedByteOffset: 0,
                InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
                InstanceDataStepRate: 0,
            },
            D3D11_INPUT_ELEMENT_DESC {
                SemanticName: texcoord.as_ptr(),
                SemanticIndex: 0,
                Format: DXGI_FORMAT_R32G32B32A32_FLOAT,
                InputSlot: 0,
                AlignedByteOffset: 16,
                InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
                InstanceDataStepRate: 0,
            },
        ];
        create_layout(device, &elements, bytecode, "XMAP15 maparrow layout")
    }

    unsafe fn create_movement_layout(
        device: *mut ID3D11Device,
        bytecode: &[u8],
    ) -> Result<D3dPtr<ID3D11InputLayout>> {
        let position = CString::new("POSITION")?;
        let texcoord = CString::new("TEXCOORD")?;
        let elements = [
            D3D11_INPUT_ELEMENT_DESC {
                SemanticName: position.as_ptr(),
                SemanticIndex: 0,
                Format: DXGI_FORMAT_R32G32B32_FLOAT,
                InputSlot: 0,
                AlignedByteOffset: 0,
                InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
                InstanceDataStepRate: 0,
            },
            D3D11_INPUT_ELEMENT_DESC {
                SemanticName: texcoord.as_ptr(),
                SemanticIndex: 0,
                Format: DXGI_FORMAT_R32G32_FLOAT,
                InputSlot: 0,
                AlignedByteOffset: 12,
                InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
                InstanceDataStepRate: 0,
            },
            D3D11_INPUT_ELEMENT_DESC {
                SemanticName: texcoord.as_ptr(),
                SemanticIndex: 1,
                Format: DXGI_FORMAT_R32G32B32_FLOAT,
                InputSlot: 0,
                AlignedByteOffset: 20,
                InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
                InstanceDataStepRate: 0,
            },
        ];
        create_layout(device, &elements, bytecode, "XMAP15 movement layout")
    }

    unsafe fn create_layout(
        device: *mut ID3D11Device,
        elements: &[D3D11_INPUT_ELEMENT_DESC],
        bytecode: &[u8],
        role: &str,
    ) -> Result<D3dPtr<ID3D11InputLayout>> {
        let mut raw = ptr::null_mut();
        native_d3d11_hr(
            (*device).CreateInputLayout(
                elements.as_ptr(),
                elements.len() as u32,
                bytecode.as_ptr() as *const _,
                bytecode.len(),
                &mut raw,
            ),
            &format!("CreateInputLayout({role})"),
        )?;
        D3dPtr::from_raw(raw, role)
    }

    unsafe fn create_constant_buffer(
        device: *mut ID3D11Device,
        bytes: u32,
        role: &str,
    ) -> Result<D3dPtr<ID3D11Buffer>> {
        let desc = D3D11_BUFFER_DESC {
            ByteWidth: bytes,
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_CONSTANT_BUFFER,
            CPUAccessFlags: 0,
            MiscFlags: 0,
            StructureByteStride: 0,
        };
        let zero = vec![0u8; bytes as usize];
        let init = D3D11_SUBRESOURCE_DATA {
            pSysMem: zero.as_ptr() as *const _,
            SysMemPitch: 0,
            SysMemSlicePitch: 0,
        };
        let mut raw = ptr::null_mut();
        native_d3d11_hr(
            (*device).CreateBuffer(&desc, &init, &mut raw),
            &format!("CreateBuffer({role})"),
        )?;
        D3dPtr::from_raw(raw, role)
    }

    unsafe fn create_buffer(
        device: *mut ID3D11Device,
        bytes: &[u8],
        bind: u32,
        role: &str,
    ) -> Result<D3dPtr<ID3D11Buffer>> {
        if bytes.is_empty() {
            return Err(format!("{role} is empty").into());
        }
        let desc = D3D11_BUFFER_DESC {
            ByteWidth: u32::try_from(bytes.len())?,
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: bind,
            CPUAccessFlags: 0,
            MiscFlags: 0,
            StructureByteStride: 0,
        };
        let init = D3D11_SUBRESOURCE_DATA {
            pSysMem: bytes.as_ptr() as *const _,
            SysMemPitch: 0,
            SysMemSlicePitch: 0,
        };
        let mut raw = ptr::null_mut();
        native_d3d11_hr(
            (*device).CreateBuffer(&desc, &init, &mut raw),
            &format!("CreateBuffer({role})"),
        )?;
        D3dPtr::from_raw(raw, role)
    }

    unsafe fn upload_rows(
        device: *mut ID3D11Device,
        resolver: &GameFileResolver,
        frame: &PreparedXmap15Frame,
    ) -> Result<Vec<NativeRow>> {
        let heightmap = match &frame.heightmap {
            Some(key) => Some(crate::image_load::decode_bmp_indexed(
                &resolver.read_admitted_used_content(key)?,
                std::path::Path::new(crate::XMAP15_HEIGHTMAP_PATH),
            )?),
            None => None,
        };
        let mut out = Vec::with_capacity(frame.portable.ordered_rows.len());
        for row in &frame.portable.ordered_rows {
            match row {
                Xmap15PreparedRow::Plan { semantic, .. } => {
                    let definition = frame.plan_definition(&semantic.resolved_definition_name)?;
                    let profile = definition.geometry_profile()?;
                    let map = heightmap
                        .as_ref()
                        .ok_or("XMAP15 plan frame omitted admitted heightmap")?;
                    let mesh = semantic.build_map_arrow_mesh(profile, |x, z| {
                        Ok(crate::rail_files::sample_exact_supply_arrow_height(
                            map, x, z,
                        ))
                    })?;
                    let bytes = plan_mesh_bytes(&mesh);
                    out.push(NativeRow {
                        geometry_sha256: Sha256::hex(&bytes),
                        geometry: RowGeometry::Plan {
                            buffer: create_buffer(
                                device,
                                &bytes,
                                D3D11_BIND_VERTEX_BUFFER,
                                "XMAP15 plan geometry",
                            )?,
                            vertex_count: u32::try_from(mesh.vertices.len())?,
                            total_length_f32_bits: mesh.total_length_f32_bits,
                            wrap_offset_world: mesh.wrap_offset_world,
                        },
                    });
                }
                Xmap15PreparedRow::Movement { semantic, .. } => {
                    let vb = create_buffer(
                        device,
                        &semantic.exact_vertex_bytes,
                        D3D11_BIND_VERTEX_BUFFER,
                        "XMAP15 movement vertices",
                    )?;
                    let ib = if semantic.exact_index_bytes.is_empty() {
                        None
                    } else {
                        Some(create_buffer(
                            device,
                            &semantic.exact_index_bytes,
                            D3D11_BIND_INDEX_BUFFER,
                            "XMAP15 movement indices",
                        )?)
                    };
                    let mut identity = semantic.exact_vertex_bytes.clone();
                    identity.extend_from_slice(&semantic.exact_index_bytes);
                    identity
                        .extend_from_slice(&semantic.exact_primitive_topology_u32.to_le_bytes());
                    out.push(NativeRow {
                        geometry_sha256: Sha256::hex(&identity),
                        geometry: RowGeometry::Movement {
                            vertex_buffer: vb,
                            index_buffer: ib,
                        },
                    });
                }
            }
        }
        Ok(out)
    }

    fn plan_mesh_bytes(mesh: &portable_render_stream::Xmap15PlanMapArrowMesh) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(mesh.vertices.len() * 32);
        for vertex in &mesh.vertices {
            for value in vertex
                .position
                .into_iter()
                .chain(std::iter::once(vertex.padding))
                .chain(vertex.uv_is_head_variant)
            {
                bytes.extend_from_slice(&value.to_bits().to_le_bytes());
            }
        }
        bytes
    }

    unsafe fn upload_plan_textures(
        device: *mut ID3D11Device,
        resolver: &GameFileResolver,
        definitions: &[PreparedXmap15PlanDefinition],
    ) -> Result<BTreeMap<String, PlanTextures>> {
        let mut out = BTreeMap::new();
        for definition in definitions {
            let mut masks = Vec::new();
            let mut patterns = Vec::new();
            for (register, paths, destination) in [
                (0u32, &definition.ordered_texture_masks, &mut masks),
                (1u32, &definition.ordered_texture_patterns, &mut patterns),
            ] {
                for (variant, path) in paths.iter().enumerate() {
                    let key = definition
                        .ordered_used_content
                        .iter()
                        .find(|key| key.normalized_game_relative_path.eq_ignore_ascii_case(path))
                        .ok_or("XMAP15 plan texture escaped definition closure")?;
                    let bytes = resolver.read_admitted_used_content(key)?;
                    destination.push(
                        crate::xmap16_supply_arrow_handoff::create_supply_arrow_texture(
                            device,
                            &format!(
                                "{}.{}[{variant}]",
                                definition.name,
                                if register == 0 { "mask" } else { "pattern" }
                            ),
                            register,
                            &bytes,
                            &key.payload_sha256,
                            "XMAP15",
                        )?,
                    );
                }
            }
            if out
                .insert(definition.name.clone(), PlanTextures { masks, patterns })
                .is_some()
            {
                return Err("XMAP15 plan texture definition duplicated".into());
            }
        }
        Ok(out)
    }

    unsafe fn upload_movement_textures(
        device: *mut ID3D11Device,
        resolver: &GameFileResolver,
        definitions: &[PreparedXmap15MovementDefinition],
    ) -> Result<BTreeMap<String, MovementTextures>> {
        let mut out = BTreeMap::new();
        for definition in definitions {
            let load = |path: &str,
                        register: u32,
                        role: &str|
             -> Result<
                crate::xmap16_supply_arrow_handoff::NativeXmap16SupplyTexture,
            > {
                let key = definition
                    .ordered_used_content
                    .iter()
                    .find(|key| key.normalized_game_relative_path.eq_ignore_ascii_case(path))
                    .ok_or("XMAP15 movement texture escaped definition closure")?;
                let bytes = resolver.read_admitted_used_content(key)?;
                Ok(
                    crate::xmap16_supply_arrow_handoff::create_supply_arrow_texture(
                        device,
                        role,
                        register,
                        &bytes,
                        &key.payload_sha256,
                        "XMAP15",
                    )?,
                )
            };
            let textures = MovementTextures {
                diffuse: load(&definition.texture, 0, "movement.diffuse")?,
                normal: load(&definition.normal, 1, "movement.normal")?,
                specular: load(&definition.specular, 2, "movement.specular")?,
            };
            if out.insert(definition.name.clone(), textures).is_some() {
                return Err("XMAP15 movement texture definition duplicated".into());
            }
        }
        Ok(out)
    }

    fn see_through_effect(definition: &PreparedXmap15PlanDefinition) -> Result<String> {
        crate::production_features::xmap15::xmap15_see_through_effect(&definition.technique)
            .map(str::to_string)
            .map_err(|error| format!("XMAP15 definition {}: {error}", definition.name).into())
    }

    fn bind_program(context: *mut ID3D11DeviceContext, program: &Program) {
        unsafe {
            (*context).VSSetShader(program.vertex.as_raw(), ptr::null_mut(), 0);
            (*context).PSSetShader(program.pixel.as_raw(), ptr::null_mut(), 0);
        }
    }

    unsafe fn bind_plan_state(
        handoff: &NativeXmap15Handoff,
        context: *mut ID3D11DeviceContext,
        effect: &str,
    ) {
        let (state, stencil) = match effect {
            "MapArrowNoHeadSeethrough" => (handoff.see_through_depth.as_raw(), 8),
            "MapArrowNoHeadWidthDepthSeethrough" => (handoff.see_through_depth.as_raw(), 8),
            "MapArrowDefaultWithDepth" | "MapArrowNoHeadWidthDepth" => {
                (handoff.depth_write.as_raw(), 0)
            }
            "MapArrowDefaultWithDepthTestOnly" => (handoff.depth_test_only.as_raw(), 1),
            _ => (handoff.normal_depth.as_raw(), 1),
        };
        (*context).OMSetDepthStencilState(state, stencil);
        let blend_factor = [0.0f32; 4];
        let blend = if effect.ends_with("Seethrough") {
            handoff.blend.as_raw()
        } else {
            ptr::null_mut()
        };
        (*context).OMSetBlendState(blend, &blend_factor, u32::MAX);
    }

    unsafe fn bind_plan_resources(
        handoff: &NativeXmap15Handoff,
        context: *mut ID3D11DeviceContext,
        base: &mut [*mut ID3D11ShaderResourceView; 11],
        definition: &str,
        texture_variant: u8,
    ) -> Result<()> {
        let textures = handoff
            .plan_textures
            .get(definition)
            .ok_or("XMAP15 plan textures vanished")?;
        let variant = texture_variant as usize;
        base[0] = textures
            .masks
            .get(variant)
            .ok_or("XMAP15 plan mask variant is outside effective definition")?
            .view
            .as_raw();
        base[1] = textures
            .patterns
            .get(variant)
            .ok_or("XMAP15 plan pattern variant is outside effective definition")?
            .view
            .as_raw();
        (*context).PSSetShaderResources(0, 11, base.as_mut_ptr());
        for (slot, sampler) in &handoff.plan_samplers {
            let mut raw = sampler.as_raw();
            (*context).PSSetSamplers(*slot, 1, &mut raw);
        }
        Ok(())
    }

    unsafe fn bind_movement_resources(
        handoff: &NativeXmap15Handoff,
        context: *mut ID3D11DeviceContext,
        definition: &str,
    ) -> Result<()> {
        let textures = handoff
            .movement_textures
            .get(definition)
            .ok_or("XMAP15 movement textures vanished")?;
        let mut srvs = [
            textures.diffuse.view.as_raw(),
            textures.normal.view.as_raw(),
            textures.specular.view.as_raw(),
        ];
        (*context).PSSetShaderResources(0, 3, srvs.as_mut_ptr());
        for (slot, sampler) in &handoff.movement_samplers {
            let mut raw = sampler.as_raw();
            (*context).PSSetSamplers(*slot, 1, &mut raw);
        }
        Ok(())
    }

    fn plan_cb3(
        definition: &PreparedXmap15PlanDefinition,
        semantic: &portable_render_stream::Xmap15PlanSemanticRow,
        total_length_bits: u32,
        wrap: i32,
        elapsed: f32,
        selected: bool,
        outer: bool,
    ) -> Result<[u8; 128]> {
        let mut bytes = [0u8; 128];
        let mask = if outer {
            definition.mask_outer_f32_bits
        } else {
            definition.mask_inner_f32_bits
        };
        write_vec4_bits(&mut bytes, 0, [mask[0], mask[1], mask[2], 1.0f32.to_bits()]);
        write_vec4_bits(&mut bytes, 16, semantic.inner_rgba_f32_bits);
        write_vec4_bits(&mut bytes, 32, semantic.outer_rgba_f32_bits);
        write_vec4(
            &mut bytes,
            48,
            [
                elapsed,
                if selected { 1.0 } else { 0.0 },
                f32::from_bits(semantic.fade_in_f32_bits),
                f32::from_bits(semantic.fade_out_f32_bits),
            ],
        );
        write_vec4(
            &mut bytes,
            64,
            [
                wrap as f32,
                definition
                    .texture_anim_speed_f32_bits
                    .map(f32::from_bits)
                    .unwrap_or(0.0),
                0.0,
                0.0,
            ],
        );
        write_f32(
            &mut bytes,
            80,
            f32::from_bits(definition.body_uv_scale_f32_bits),
        );
        write_f32(
            &mut bytes,
            84,
            definition
                .body_uv_scale_2_f32_bits
                .map(f32::from_bits)
                .unwrap_or(0.0),
        );
        write_f32(
            &mut bytes,
            88,
            1.0 / definition.effective_texture_variant_count()? as f32,
        );
        write_f32(
            &mut bytes,
            92,
            f32::from_bits(semantic.culling_offset_f32_bits),
        );
        write_f32(&mut bytes, 96, 1.0);
        write_f32(&mut bytes, 100, 1.0);
        write_f32(&mut bytes, 104, 0.2);
        let total = f32::from_bits(total_length_bits);
        write_f32(&mut bytes, 108, total);
        write_f32(
            &mut bytes,
            112,
            definition
                .texture_loop_f32_bits
                .map(f32::from_bits)
                .unwrap_or(total),
        );
        if bytes[..116]
            .chunks_exact(4)
            .any(|chunk| !f32::from_bits(u32::from_le_bytes(chunk.try_into().unwrap())).is_finite())
        {
            return Err("XMAP15 plan CB3 contains non-finite values".into());
        }
        Ok(bytes)
    }

    fn movement_cb1(
        view_projection: [[f32; 4]; 4],
        semantic: &portable_render_stream::Xmap15MovementSemanticRow,
    ) -> Result<[u8; 112]> {
        let mut bytes = [0u8; 112];
        let mut offset = 0;
        for value in view_projection.into_iter().flatten() {
            write_f32(&mut bytes, offset, value);
            offset += 4;
        }
        write_vec3_bits(&mut bytes, 64, semantic.progress_move_army_f32_bits);
        write_vec4_bits(&mut bytes, 80, semantic.desaturation_lengths_f32_bits);
        write_f32(&mut bytes, 96, f32::from_bits(semantic.offset_x_f32_bits));
        Ok(bytes)
    }

    fn write_f32(bytes: &mut [u8], offset: usize, value: f32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_bits().to_le_bytes());
    }
    fn write_vec4(bytes: &mut [u8], offset: usize, values: [f32; 4]) {
        for (i, value) in values.into_iter().enumerate() {
            write_f32(bytes, offset + i * 4, value);
        }
    }
    fn write_vec3_bits(bytes: &mut [u8], offset: usize, values: [u32; 3]) {
        for (i, value) in values.into_iter().enumerate() {
            bytes[offset + i * 4..offset + i * 4 + 4].copy_from_slice(&value.to_le_bytes());
        }
    }
    fn write_vec4_bits(bytes: &mut [u8], offset: usize, values: [u32; 4]) {
        for (i, value) in values.into_iter().enumerate() {
            bytes[offset + i * 4..offset + i * 4 + 4].copy_from_slice(&value.to_le_bytes());
        }
    }

    unsafe fn create_depth_state(
        device: *mut ID3D11Device,
        enabled: bool,
        write: bool,
        role: &str,
    ) -> Result<D3dPtr<ID3D11DepthStencilState>> {
        let face = D3D11_DEPTH_STENCILOP_DESC {
            StencilFailOp: D3D11_STENCIL_OP_KEEP,
            StencilDepthFailOp: D3D11_STENCIL_OP_KEEP,
            StencilPassOp: D3D11_STENCIL_OP_KEEP,
            StencilFunc: D3D11_COMPARISON_ALWAYS,
        };
        let desc = D3D11_DEPTH_STENCIL_DESC {
            DepthEnable: if enabled { TRUE } else { FALSE },
            DepthWriteMask: if write {
                D3D11_DEPTH_WRITE_MASK_ALL
            } else {
                D3D11_DEPTH_WRITE_MASK_ZERO
            },
            DepthFunc: D3D11_COMPARISON_LESS_EQUAL,
            StencilEnable: FALSE,
            StencilReadMask: 0,
            StencilWriteMask: 0,
            FrontFace: face,
            BackFace: face,
        };
        let mut raw = ptr::null_mut();
        native_d3d11_hr(
            (*device).CreateDepthStencilState(&desc, &mut raw),
            &format!("CreateDepthStencilState({role})"),
        )?;
        D3dPtr::from_raw(raw, role)
    }

    unsafe fn create_depth_test_only_state(
        device: *mut ID3D11Device,
    ) -> Result<D3dPtr<ID3D11DepthStencilState>> {
        let back = D3D11_DEPTH_STENCILOP_DESC {
            StencilFailOp: D3D11_STENCIL_OP_KEEP,
            StencilDepthFailOp: D3D11_STENCIL_OP_KEEP,
            StencilPassOp: D3D11_STENCIL_OP_KEEP,
            StencilFunc: D3D11_COMPARISON_ALWAYS,
        };
        let desc = D3D11_DEPTH_STENCIL_DESC {
            DepthEnable: TRUE,
            DepthWriteMask: D3D11_DEPTH_WRITE_MASK_ZERO,
            DepthFunc: D3D11_COMPARISON_LESS_EQUAL,
            StencilEnable: TRUE,
            StencilReadMask: 1,
            StencilWriteMask: 1,
            FrontFace: D3D11_DEPTH_STENCILOP_DESC {
                StencilFailOp: D3D11_STENCIL_OP_KEEP,
                StencilDepthFailOp: D3D11_STENCIL_OP_KEEP,
                StencilPassOp: D3D11_STENCIL_OP_REPLACE,
                StencilFunc: D3D11_COMPARISON_NOT_EQUAL,
            },
            BackFace: back,
        };
        let mut raw = ptr::null_mut();
        native_d3d11_hr(
            (*device).CreateDepthStencilState(&desc, &mut raw),
            "CreateDepthStencilState(XMAP15 depth test only)",
        )?;
        D3dPtr::from_raw(raw, "XMAP15 depth test only")
    }

    unsafe fn create_movement_samplers(
        device: *mut ID3D11Device,
    ) -> Result<Vec<(u32, D3dPtr<ID3D11SamplerState>)>> {
        let mut out = Vec::new();
        for slot in 0..3u32 {
            let desc = D3D11_SAMPLER_DESC {
                Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
                AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
                AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
                AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
                MipLODBias: 0.0,
                MaxAnisotropy: 1,
                ComparisonFunc: D3D11_COMPARISON_NEVER,
                BorderColor: [0.0; 4],
                MinLOD: 0.0,
                MaxLOD: f32::MAX,
            };
            let mut raw = ptr::null_mut();
            native_d3d11_hr(
                (*device).CreateSamplerState(&desc, &mut raw),
                "CreateSamplerState(XMAP15 movement)",
            )?;
            out.push((slot, D3dPtr::from_raw(raw, "XMAP15 movement sampler")?));
        }
        Ok(out)
    }

    unsafe fn create_plan_samplers(
        device: *mut ID3D11Device,
    ) -> Result<Vec<(u32, D3dPtr<ID3D11SamplerState>)>> {
        let mut out = crate::xmap16_supply_arrow_handoff::create_supply_arrow_samplers(device)?;
        if !out.iter().any(|(slot, _)| *slot == 7) {
            let desc = D3D11_SAMPLER_DESC {
                Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
                AddressU: D3D11_TEXTURE_ADDRESS_WRAP,
                AddressV: D3D11_TEXTURE_ADDRESS_WRAP,
                AddressW: D3D11_TEXTURE_ADDRESS_WRAP,
                MipLODBias: 0.0,
                MaxAnisotropy: 1,
                ComparisonFunc: D3D11_COMPARISON_NEVER,
                BorderColor: [0.0; 4],
                MinLOD: 0.0,
                MaxLOD: f32::MAX,
            };
            let mut raw = ptr::null_mut();
            native_d3d11_hr(
                (*device).CreateSamplerState(&desc, &mut raw),
                "CreateSamplerState(XMAP15 LeanTexture1 s7)",
            )?;
            out.push((7, D3dPtr::from_raw(raw, "XMAP15 LeanTexture1 sampler")?));
            out.sort_by_key(|(slot, _)| *slot);
        }
        Ok(out)
    }

    fn ordered_external<'a>(
        resources: &'a [NativeXmap16ExternalResourceView],
        expected: &[(String, u32)],
    ) -> Result<Vec<&'a NativeXmap16ExternalResourceView>> {
        let mut out = Vec::new();
        for (symbol, register) in expected {
            let matches = resources
                .iter()
                .filter(|view| {
                    view.identity.symbol == *symbol && view.identity.shader_register == *register
                })
                .collect::<Vec<_>>();
            if matches.len() != 1 {
                return Err(format!(
                    "XMAP15 external resource {symbol}/t{register} is absent or duplicated"
                )
                .into());
            }
            let view = matches[0];
            if view.as_raw().is_null() {
                return Err(format!("XMAP15 external resource {symbol} is null").into());
            }
            out.push(view);
        }
        Ok(out)
    }

    fn validate_scene_and_resources<S>(inputs: &NativeXmap15DrawInputs<'_, S>) -> Result<()>
    where
        S: NativeXmap16SceneConstantBufferView,
    {
        if inputs.scene_cb0.symbol() != "SceneCB0"
            || inputs.scene_cb0.shader_register() != 0
            || inputs.scene_cb0.raw_buffer().is_null()
        {
            return Err("XMAP15 lacks exact SceneCB0".into());
        }
        if inputs.point_lights_cb2.symbol != "PointLightsCB2"
            || inputs.point_lights_cb2.shader_register != 2
            || inputs.point_lights_cb2.as_raw().is_null()
        {
            return Err("XMAP15 lacks exact PointLightsCB2".into());
        }
        Ok(())
    }

    fn plan_key(key: &Xmap15PlanKey) -> String {
        format!(
            "plan:{:?}:{}:{}:{}:{}",
            key.owner_kind,
            key.owner_id,
            key.engine_order_instance_id,
            key.generation,
            key.slot_index
        )
    }

    fn validate_sha(value: &str, label: &str) -> Result<()> {
        if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(format!("{label} SHA-256 is invalid").into());
        }
        Ok(())
    }
}

impl PreparedXmap15Frame {
    pub(crate) fn plan_definition(
        &self,
        name: &str,
    ) -> Result<&crate::PreparedXmap15PlanDefinition> {
        self.plan_definitions
            .iter()
            .find(|definition| definition.name == name)
            .ok_or_else(|| format!("XMAP15 plan definition {name} is absent").into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn execution_receipt_rejects_empty_mismatch() {
        let mut receipt = Xmap15NativeExecutionReceipt {
            prepared_frame_sha256: "a".repeat(64),
            assigned_stage_name: crate::XMAP15_ASSIGNED_STAGE_NAME.into(),
            assigned_operation_name: crate::XMAP15_ASSIGNED_OPERATION_NAME.into(),
            assigned_stage_ordinal: 4,
            assigned_stage_assignment_sha256: "f".repeat(64),
            lifecycle_update_sha256: "b".repeat(64),
            target_sha256: "c".repeat(64),
            scene_cb0_payload_sha256: "d".repeat(64),
            point_lights_cb2_payload_sha256: "e".repeat(64),
            ordered_rows: Vec::new(),
            authenticated_empty: false,
            deterministic_sha256: String::new(),
        };
        receipt.deterministic_sha256 = execution_digest(&receipt).unwrap();
        assert!(receipt.validate().is_err());
    }
}
