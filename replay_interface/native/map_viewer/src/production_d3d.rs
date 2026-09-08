
use crate::{
    production_main_scene_desc_from_outputs,
    production_map_interpreter::{materialize_static_program, unsupported_reason},
    validate_c03_camera_replay_binding, C03CameraReplayBinding, ExactRailD80Binding,
    FoundationContentExecutionReceipt, PreparedProductionMapFrame, PreparedRuntimeDraw,
    ProductionMainSceneTargetDesc, ProductionMapResource, Result,
    FOUNDATION_CONTENT_EXECUTION_RECEIPT_SCHEMA,
};
use replay_core::{
    BufferSource, C07TreeRuntimeCarrier, ConstantBufferBinding, M03ArrowDraw, M03ArrowFrame,
    MapnameD80State, RailSupplyArrowFrame, RenderExecutionRef, RenderTargetDesc,
    RenderTargetInitialSource, StaticGeometrySource, StaticMapCommand, StaticMapOperation,
    StaticMapPassProgram, StaticShaderResourceSource, UsedContentKey, ViewStatus,
};
use serde::{Deserialize, Serialize};
use snapshot::sha256::Sha256;
use std::collections::BTreeMap;

#[cfg(windows)]
pub fn execute_foundation_content_operation(
    resolver: &crate::GameFileResolver,
    resolved: &portable_content_manifest::foundation::ResolvedFoundationContent,
    content_authority_sha256: &str,
) -> Result<FoundationContentExecutionReceipt> {
    use crate::shader_compile::{
        build_stage_hlsl_with_defines, d3d_compile_stage, derive_a729_engine_map_shader_constants,
        engine_map_shader_defines, ShaderStage, ENGINE_COMPILE_FLAGS,
    };

    resolved
        .validate()
        .map_err(|error| format!("invalid resolved foundation content: {error}"))?;
    if !resolver.has_content_admission() {
        return Err("foundation content execution requires an admission-bound VFS".into());
    }
    let request = &resolved.request;
    let constants = derive_a729_engine_map_shader_constants(
        request.map_width,
        request.map_height,
        request.pow2_ratio_branch,
        request.max_texture_dimension,
    )?;
    let defines = engine_map_shader_defines(constants);
    let vertex_hlsl = build_stage_hlsl_with_defines(
        resolver,
        &request.effect_file,
        &request.effect_name,
        ShaderStage::Vertex,
        &defines,
    )?;
    let pixel_hlsl = build_stage_hlsl_with_defines(
        resolver,
        &request.effect_file,
        &request.effect_name,
        ShaderStage::Pixel,
        &defines,
    )?;
    if Sha256::hex(vertex_hlsl.as_bytes()) != resolved.vertex_shader.translation_unit_sha256
        || vertex_hlsl.len() as u64 != resolved.vertex_shader.translation_unit_bytes
        || Sha256::hex(pixel_hlsl.as_bytes()) != resolved.pixel_shader.translation_unit_sha256
        || pixel_hlsl.len() as u64 != resolved.pixel_shader.translation_unit_bytes
    {
        return Err(
            "foundation shader translation unit differs from the XMAP02-resolved identity".into(),
        );
    }
    let vertex_bytecode =
        d3d_compile_stage(&vertex_hlsl, ShaderStage::Vertex, ENGINE_COMPILE_FLAGS)?;
    let pixel_bytecode = d3d_compile_stage(&pixel_hlsl, ShaderStage::Pixel, ENGINE_COMPILE_FLAGS)?;

    let texture_key = resolved
        .ordered_used_content
        .iter()
        .find(|row| row.normalized_game_relative_path == request.texture_file)
        .ok_or("resolved foundation closure has no requested texture")?;
    let texture_bytes = resolver.read_admitted_used_content(texture_key)?;
    execute_foundation_d3d_objects(
        &vertex_bytecode,
        &pixel_bytecode,
        &texture_bytes,
        &resolved.texture,
    )?;

    FoundationContentExecutionReceipt {
        schema: FOUNDATION_CONTENT_EXECUTION_RECEIPT_SCHEMA.into(),
        content_manifest_sha256: resolved.manifest_digest_sha256.clone(),
        product_modset_sha256: resolved.product_modset_sha256.clone(),
        effective_vfs_manifest_sha256: resolved.effective_vfs_manifest_sha256.clone(),
        content_authority_sha256: content_authority_sha256.to_string(),
        resolved_operation_sha256: resolved.operation_identity_sha256.clone(),
        ordered_program_stage_sha256: vec![
            Sha256::hex(&vertex_bytecode),
            Sha256::hex(&pixel_bytecode),
        ],
        resource_identity_sha256: resolved.texture.resource_identity_sha256.clone(),
        descriptor_identity_sha256: resolved
            .texture
            .shader_resource_view_identity_sha256
            .clone(),
        compiled_or_loaded_programs: 2,
        created_resources: 1,
        created_descriptors: 1,
        deterministic_sha256: String::new(),
    }
    .seal()
}

#[cfg(windows)]
fn execute_foundation_d3d_objects(
    vertex_bytecode: &[u8],
    pixel_bytecode: &[u8],
    dds: &[u8],
    descriptor: &portable_content_manifest::foundation::FoundationTextureDescriptor,
) -> Result<()> {
    use crate::d3d11_device::{native_d3d11_create_device, native_d3d11_hr, D3dPtr};
    use std::ptr;
    use winapi::shared::dxgitype::DXGI_SAMPLE_DESC;
    use winapi::um::d3d11::{
        ID3D11PixelShader, ID3D11ShaderResourceView, ID3D11Texture2D, ID3D11VertexShader,
        D3D11_BIND_SHADER_RESOURCE, D3D11_SUBRESOURCE_DATA, D3D11_TEXTURE2D_DESC,
        D3D11_USAGE_DEFAULT,
    };

    if descriptor.dimension != "texture2d"
        || descriptor.depth != 1
        || descriptor.array_size != 1
        || descriptor.cube
    {
        return Err(
            "foundation TerrainDiffuse must resolve to one non-array, non-cube Texture2D".into(),
        );
    }
    if dds.len() < 128 || dds.get(..4) != Some(b"DDS ") {
        return Err("foundation texture bytes are not a complete DDS".into());
    }
    let read_u32 = |offset: usize| -> Result<u32> {
        Ok(u32::from_le_bytes(
            dds.get(offset..offset + 4)
                .ok_or("foundation DDS header is truncated")?
                .try_into()?,
        ))
    };
    let (dxgi_format, block_bytes, bytes_per_pixel, payload_offset) =
        foundation_native_format(&descriptor.format, dds)?;
    let mut offset = payload_offset;
    let mut initial = Vec::with_capacity(descriptor.mip_levels as usize);
    for mip in 0..descriptor.mip_levels {
        let width = (descriptor.width >> mip).max(1);
        let height = (descriptor.height >> mip).max(1);
        let (row_pitch, slice_pitch) = if block_bytes != 0 {
            let block_width = ((width + 3) / 4).max(1);
            let block_height = ((height + 3) / 4).max(1);
            let row = block_width
                .checked_mul(block_bytes)
                .ok_or("foundation DDS row pitch overflow")?;
            let slice = row
                .checked_mul(block_height)
                .ok_or("foundation DDS slice pitch overflow")?;
            (row, slice)
        } else {
            let row = width
                .checked_mul(bytes_per_pixel)
                .ok_or("foundation DDS row pitch overflow")?;
            let slice = row
                .checked_mul(height)
                .ok_or("foundation DDS slice pitch overflow")?;
            (row, slice)
        };
        let end = offset
            .checked_add(slice_pitch as usize)
            .ok_or("foundation DDS payload range overflow")?;
        let bytes = dds
            .get(offset..end)
            .ok_or("foundation DDS mip payload is truncated")?;
        initial.push(D3D11_SUBRESOURCE_DATA {
            pSysMem: bytes.as_ptr() as *const _,
            SysMemPitch: row_pitch,
            SysMemSlicePitch: slice_pitch,
        });
        offset = end;
    }
    if read_u32(16)? != descriptor.width
        || read_u32(12)? != descriptor.height
        || read_u32(28)?.max(1) != descriptor.mip_levels
    {
        return Err("foundation DDS header differs from resolved texture descriptor".into());
    }

    unsafe {
        let device = native_d3d11_create_device()?;
        let mut vertex = ptr::null_mut::<ID3D11VertexShader>();
        native_d3d11_hr(
            (*device.device.as_raw()).CreateVertexShader(
                vertex_bytecode.as_ptr() as *const _,
                vertex_bytecode.len(),
                ptr::null_mut(),
                &mut vertex,
            ),
            "CreateVertexShader(foundation content operation)",
        )?;
        let _vertex = D3dPtr::from_raw(vertex, "foundation vertex shader")?;

        let mut pixel = ptr::null_mut::<ID3D11PixelShader>();
        native_d3d11_hr(
            (*device.device.as_raw()).CreatePixelShader(
                pixel_bytecode.as_ptr() as *const _,
                pixel_bytecode.len(),
                ptr::null_mut(),
                &mut pixel,
            ),
            "CreatePixelShader(foundation content operation)",
        )?;
        let _pixel = D3dPtr::from_raw(pixel, "foundation pixel shader")?;

        let native = D3D11_TEXTURE2D_DESC {
            Width: descriptor.width,
            Height: descriptor.height,
            MipLevels: descriptor.mip_levels,
            ArraySize: 1,
            Format: dxgi_format,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_SHADER_RESOURCE,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let mut texture = ptr::null_mut::<ID3D11Texture2D>();
        native_d3d11_hr(
            (*device.device.as_raw()).CreateTexture2D(&native, initial.as_ptr(), &mut texture),
            "CreateTexture2D(foundation content operation)",
        )?;
        let texture = D3dPtr::from_raw(texture, "foundation texture")?;
        let mut view = ptr::null_mut::<ID3D11ShaderResourceView>();
        native_d3d11_hr(
            (*device.device.as_raw()).CreateShaderResourceView(
                texture.as_raw() as *mut _,
                ptr::null(),
                &mut view,
            ),
            "CreateShaderResourceView(foundation content operation)",
        )?;
        let _view = D3dPtr::from_raw(view, "foundation texture SRV")?;
    }
    Ok(())
}

#[cfg(windows)]
pub(crate) fn foundation_native_format(format: &str, dds: &[u8]) -> Result<(u32, u32, u32, usize)> {
    if let Some(value) = format.strip_prefix("dxgi:") {
        let dxgi = value.parse::<u32>()?;
        let block_bytes = match dxgi {
            70..=72 | 79..=81 => 8,
            73..=78 | 82..=84 => 16,
            _ => 0,
        };
        let bytes_per_pixel = match dxgi {
            28 | 29 | 87 | 91 => 4,
            _ if block_bytes != 0 => 0,
            _ => {
                return Err(format!(
                    "foundation DDS DXGI format {dxgi} needs an exact subresource layout implementation"
                )
                .into())
            }
        };
        return Ok((dxgi, block_bytes, bytes_per_pixel, 148));
    }
    match format {
        "fourcc:DXT1" => Ok((71, 8, 0, 128)),
        "fourcc:DXT3" => Ok((74, 16, 0, 128)),
        "fourcc:DXT5" => Ok((77, 16, 0, 128)),
        _ if format.starts_with("rgb:32:") => {
            let masks = format.split(':').skip(2).collect::<Vec<_>>();
            if masks.len() != 4 {
                return Err("foundation RGB DDS descriptor is malformed".into());
            }
            let dxgi = if masks == ["000000ff", "0000ff00", "00ff0000", "ff000000"] {
                28
            } else if masks == ["00ff0000", "0000ff00", "000000ff", "ff000000"] {
                87
            } else {
                return Err(
                    format!("foundation RGB DDS channel masks are unsupported: {format}").into(),
                );
            };
            Ok((dxgi, 0, 4, 128))
        }
        _ => {
            let fourcc = String::from_utf8_lossy(
                dds.get(84..88)
                    .ok_or("foundation DDS pixel format is truncated")?,
            );
            Err(
                format!("foundation DDS format {format:?} (fourcc {fourcc:?}) is unsupported")
                    .into(),
            )
        }
    }
}

pub type ProductionD3dStaticPassProgram = StaticMapPassProgram;
pub const PRODUCTION_D3D_EXECUTION_RECEIPT_SCHEMA: &str = "hoi4.production_d3d_execution.v7";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductionD3dOmission {
    pub pass: String,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnowMudD3dExecutionProof {
    pub tight_readback_bytes: Vec<u8>,
    pub readback_sha256: String,
    pub mapped_row_pitch: u32,
    pub verified_row_count: u32,
    pub verified_tight_byte_count: u64,
    pub replaced_binding_count: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OccupationSecondaryD3dExecutionProof {
    pub tight_readback_bytes: Vec<u8>,
    pub readback_sha256: String,
    pub mapped_row_pitch: u32,
    pub verified_row_count: u32,
    pub verified_tight_byte_count: u64,
    pub terrain_t6_binding_count: u32,
    pub water_t3_binding_count: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreparedProductionD3dFrame {
    pub content_manifest_sha256: String,
    pub product_modset_sha256: String,
    pub effective_vfs_manifest_sha256: String,
    pub c03_camera_replay: Option<C03CameraReplayBinding>,
    pub target: Option<ProductionMainSceneTargetDesc>,
    pub ordered_scene_constant_buffers: Vec<ConstantBufferBinding>,
    pub ordered_render_operations: Vec<RenderExecutionRef>,
    pub snow_mud_texture: Option<replay_core::SnowMudTexturePacket>,
    pub political_textures: Vec<replay_core::PoliticalTexturePacket>,
    pub ordered_static_used_content: Vec<UsedContentKey>,
    pub ordered_static_programs: Vec<StaticMapPassProgram>,
    pub authenticated_texture_snapshots: Vec<replay_core::AuthenticatedTextureSnapshotPacket>,
    pub rail_ribbon: Option<ExactRailD80Binding>,
    pub mapname_d80_state: MapnameD80State,
    pub tree_runtime: Option<C07TreeRuntimeCarrier>,
    pub tree_materialized_execution_sha256: Option<String>,
    pub s1_shadow: Option<PreparedS1ShadowExecution>,
    pub ordered_runtime_draws: Vec<PreparedRuntimeDraw>,
    pub authenticated_ui_pass: Option<replay_core::AuthenticatedUiPass>,
    pub m03_arrows: Option<M03ArrowFrame>,
    pub rail_supply_arrows: Option<RailSupplyArrowFrame>,
    pub omissions: Vec<ProductionD3dOmission>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuildingPresentationPriority {
    AuthenticatedD80Operations,
    TypedSemanticStateOnly,
    Unavailable,
}

pub fn has_authenticated_d80_map_operations(frame: &PreparedProductionD3dFrame) -> bool {
    frame.target.is_some()
        && (!frame.ordered_static_programs.is_empty()
            || !frame.ordered_runtime_draws.is_empty()
            || frame
                .m03_arrows
                .as_ref()
                .is_some_and(|m03| !m03.draws.is_empty()))
}

pub fn building_presentation_priority(
    frame: Option<&PreparedProductionD3dFrame>,
    buildings_status: ViewStatus,
) -> BuildingPresentationPriority {
    if frame.is_some_and(has_authenticated_d80_map_operations) {
        BuildingPresentationPriority::AuthenticatedD80Operations
    } else if buildings_status == ViewStatus::Present {
        BuildingPresentationPriority::TypedSemanticStateOnly
    } else {
        BuildingPresentationPriority::Unavailable
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductionD3dExecutedDraw {
    pub global_ordinal: u32,
    pub pass: String,
    pub draw_token: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductionD3dExecutionReceipt {
    pub schema: String,
    pub content_manifest_sha256: String,
    pub product_modset_sha256: String,
    pub effective_vfs_manifest_sha256: String,
    pub c03_camera_replay: Option<C03CameraReplayBinding>,
    pub tree_runtime_payload_sha256: Option<String>,
    pub tree_materialized_execution_sha256: Option<String>,
    pub s1_shadow_payload_sha256: Option<String>,
    pub s1_materialized_execution_sha256: Option<String>,
    pub mapname_d80_state: MapnameD80State,
    pub target: Option<ProductionMainSceneTargetDesc>,
    pub ordered_draws: Vec<ProductionD3dExecutedDraw>,
    pub ordered_used_content: Vec<UsedContentKey>,
    pub omissions: Vec<ProductionD3dOmission>,
    pub deterministic_sha256: String,
}

pub trait ProductionD3dExecutionBridge {
    fn begin_frame(&mut self, frame: &PreparedProductionD3dFrame) -> Result<()>;
    fn execute_static_operation(
        &mut self,
        global_ordinal: u32,
        pass: &StaticMapPassProgram,
        ordered_scene_constant_buffers: &[ConstantBufferBinding],
        operation: &StaticMapOperation,
    ) -> Result<()>;
    fn execute_runtime_draw(
        &mut self,
        global_ordinal: u32,
        ordered_scene_constant_buffers: &[ConstantBufferBinding],
        draw: &PreparedRuntimeDraw,
    ) -> Result<()>;
    fn execute_m03_draw(
        &mut self,
        global_ordinal: u32,
        ordered_scene_constant_buffers: &[ConstantBufferBinding],
        draw: &M03ArrowDraw,
    ) -> Result<()>;
    fn finish_frame(&mut self) -> Result<()>;
}

fn validate_provenance_sha256(value: &str, label: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!("{label} is not canonical lowercase SHA-256").into());
    }
    Ok(())
}

/// Join the production-map plan with independently generated static-pass
pub fn prepare_production_d3d_frame(
    frame: &PreparedProductionMapFrame,
    resources: &BTreeMap<String, ProductionMapResource>,
) -> Result<PreparedProductionD3dFrame> {
    validate_provenance_sha256(&frame.content_manifest_sha256, "content manifest digest")?;
    validate_provenance_sha256(&frame.product_modset_sha256, "product modset digest")?;
    validate_provenance_sha256(
        &frame.effective_vfs_manifest_sha256,
        "effective VFS manifest digest",
    )?;
    if let Some(binding) = &frame.c03_camera_replay {
        validate_c03_camera_replay_binding(binding)?;
        if frame.selected_submit_index != Some(binding.submit_index)
            || frame.selected_submit_global_order != Some(binding.submit_global_order)
        {
            return Err("C03 camera replay binding belongs to a different selected submit".into());
        }
    }
    let mut ordered_static_programs = Vec::new();
    let mut omissions = Vec::new();
    let mut mapname_d80_state = MapnameD80State::Missing;
    if frame.s1_shadow.is_none() {
        omissions.push(ProductionD3dOmission {
            pass: "c07_s1_shadow".into(),
            reason: "selected submit has no authenticated portable S1 carrier; light-depth, terrain-unlit, blur, and shadow-consumer execution remains omitted".into(),
        });
    }
    let mut omitted_static_ordinals = std::collections::BTreeSet::new();
    for program in &frame.ordered_static_map_programs {
        let program_mapname_state = program.mapname_d80_state()?;
        mapname_d80_state = mapname_d80_state.merge(program_mapname_state)?;
        let mut seen = std::collections::BTreeSet::new();
        if program
            .ordered_content_keys
            .iter()
            .any(|key| !seen.insert(key))
        {
            return Err("opaque D3D program content keys are duplicated".into());
        }
        if program.ordered_operations.is_empty() {
            omissions.push(ProductionD3dOmission {
                pass: program.pass_token.clone(),
                reason: "typed static-pass program contains no authenticated draws".into(),
            });
            continue;
        }
        if frame.tree_runtime.is_none()
            && program.ordered_operations.iter().any(|operation| {
                matches!(
                    &operation.command,
                    StaticMapCommand::Draw { draw }
                        if C07TreeRuntimeCarrier::is_retained_tree_shader_draw(draw)
                )
            })
        {
            omitted_static_ordinals.extend(
                program
                    .ordered_operations
                    .iter()
                    .map(|operation| operation.operation_ordinal),
            );
            omissions.push(ProductionD3dOmission {
                pass: program.pass_token.clone(),
                reason: "authenticated a729 tree shader program omitted because the selected submit has no C07 runtime visibility/model/LOD carrier".into(),
            });
            continue;
        }
        if program_mapname_state == MapnameD80State::LegacyUnstamped {
            omitted_static_ordinals.extend(
                program
                    .ordered_operations
                    .iter()
                    .map(|operation| operation.operation_ordinal),
            );
            omissions.push(ProductionD3dOmission {
                pass: program.pass_token.clone(),
                reason: "B10 legacy mapname program omitted: the draw lacks the authenticated reflected Transp_OffsetX CB1 semantic; reconstructed labels and slot/size inference are forbidden".into(),
            });
            continue;
        }
        let program = match materialize_static_program(program, resources) {
            Ok(program) => program,
            Err(error) => {
                if let Some(reason) = unsupported_reason(&error) {
                    if program_mapname_state == MapnameD80State::Authenticated {
                        return Err(format!(
                            "authenticated mapname program cannot be omitted during materialization: {reason}"
                        )
                        .into());
                    }
                    if frame.rail_ribbon.as_ref().is_some_and(|rail| {
                        program.ordered_operations.iter().any(|operation| {
                            rail.ordered_draws.iter().any(|binding| {
                                binding.operation_ordinal == operation.operation_ordinal
                            })
                        })
                    }) {
                        return Err(format!(
                            "authenticated D81/D80 railroad program cannot be omitted during materialization: {reason}"
                        )
                        .into());
                    }
                    if frame.rail_supply_arrows.as_ref().is_some_and(|rail| {
                        program.ordered_operations.iter().any(|operation| {
                            rail.ordered_draws.iter().any(|binding| {
                                binding.operation_ordinal == operation.operation_ordinal
                            })
                        })
                    }) {
                        return Err(format!(
                            "authenticated railway supply-map arrow program cannot be omitted during materialization: {reason}"
                        )
                        .into());
                    }
                    omitted_static_ordinals.extend(
                        program
                            .ordered_operations
                            .iter()
                            .map(|operation| operation.operation_ordinal),
                    );
                    omissions.push(ProductionD3dOmission {
                        pass: program.pass_token.clone(),
                        reason,
                    });
                    continue;
                }
                return Err(error.into());
            }
        };
        ordered_static_programs.push(program);
    }

    let mut omit_m03 = false;
    loop {
        let mut changed = false;
        if !omit_m03
            && frame.m03_arrows.as_ref().is_some_and(|m03| {
                m03.draws.iter().any(|packet| {
                    static_draw_omitted_dependency(&packet.draw, &omitted_static_ordinals, false)
                        .is_some()
                })
            })
        {
            omit_m03 = true;
            changed = true;
            omissions.push(ProductionD3dOmission {
                pass: "m03_order_movement_arrows".into(),
                reason: "M03 consumes a render-graph resource whose authenticated static producer was omitted; the entire dependent M03 feature is transitively omitted".into(),
            });
        }

        let mut retained = Vec::with_capacity(ordered_static_programs.len());
        for program in ordered_static_programs {
            if let Some(dependency) =
                static_program_omitted_dependency(&program, &omitted_static_ordinals, omit_m03)
            {
                omitted_static_ordinals.extend(
                    program
                        .ordered_operations
                        .iter()
                        .map(|operation| operation.operation_ordinal),
                );
                omissions.push(ProductionD3dOmission {
                    pass: program.pass_token.clone(),
                    reason: format!(
                        "static pass consumes {dependency}, whose authenticated producer was omitted; this dependent pass is transitively omitted"
                    ),
                });
                changed = true;
            } else {
                retained.push(program);
            }
        }
        ordered_static_programs = retained;
        if !changed {
            break;
        }
    }

    let retained_mapname_state = ordered_static_programs
        .iter()
        .try_fold(MapnameD80State::Missing, |state, program| {
            state.merge(program.mapname_d80_state()?)
        })?;
    match (mapname_d80_state, retained_mapname_state) {
        (MapnameD80State::Missing, MapnameD80State::Missing)
        | (MapnameD80State::LegacyUnstamped, MapnameD80State::Missing)
        | (MapnameD80State::Authenticated, MapnameD80State::Authenticated) => {}
        (MapnameD80State::Authenticated, _) => {
            return Err(
                "authenticated mapname program was omitted before production execution".into(),
            )
        }
        _ => return Err("prepared mapname D80 state disagrees with retained programs".into()),
    }

    let retained_authenticated_texture_snapshots = ordered_static_programs
        .iter()
        .flat_map(|program| &program.ordered_operations)
        .filter_map(|operation| match &operation.command {
            StaticMapCommand::Draw { draw } => Some(draw),
            StaticMapCommand::ClearColor { .. } | StaticMapCommand::ClearDepthStencil { .. } => {
                None
            }
        })
        .flat_map(|draw| &draw.ordered_shader_resources)
        .filter_map(|binding| match &binding.source {
            StaticShaderResourceSource::AuthenticatedSnapshot {
                snapshot_sha256, ..
            } => Some(snapshot_sha256.as_str()),
            _ => None,
        })
        .collect::<std::collections::BTreeSet<_>>();
    let authenticated_texture_snapshots = frame
        .authenticated_texture_snapshots
        .iter()
        .filter(|packet| {
            retained_authenticated_texture_snapshots.contains(packet.snapshot_sha256.as_str())
        })
        .cloned()
        .collect();

    let mut ordered_static_used_content = Vec::new();
    let mut output_states = Vec::new();
    for program in &ordered_static_programs {
        let expected_consumer = format!("production_map.static_program/{}", program.payload_sha256);
        for key in &program.ordered_content_keys {
            let resource = resources
                .get(key)
                .ok_or_else(|| format!("opaque program references absent content key {key}"))?;
            ordered_static_used_content.push(resource.content.clone());
        }
        for operation in &program.ordered_operations {
            validate_static_operation(operation, resources, &expected_consumer)?;
            if let StaticMapCommand::Draw { draw } = &operation.command {
                output_states.push(draw.output_state.clone());
            }
        }
    }
    let m03_arrows = if omit_m03 {
        None
    } else {
        frame.m03_arrows.clone()
    };
    if let Some(m03) = &m03_arrows {
        for content in &m03.used_content {
            ordered_static_used_content.push(content.clone());
        }
        output_states.extend(m03.draws.iter().map(|draw| draw.draw.output_state.clone()));
    }
    let rail_supply_arrows = if let Some(rail) = &frame.rail_supply_arrows {
        rail.validate()
            .map_err(|error| format!("invalid rail supply-arrow carrier: {error}"))?;
        rail.validate_against_static_programs(&frame.ordered_static_map_programs)
            .map_err(|error| format!("rail supply-arrow source join is invalid: {error}"))?;
        rail.validate_against_static_programs(&ordered_static_programs)
            .map_err(|error| {
                format!(
                    "authenticated railway supply-map arrow program cannot be partially omitted: {error}"
                )
            })?;
        extend_receipt_content_closure(&mut ordered_static_used_content, &rail.used_content);
        Some(rail.clone())
    } else {
        None
    };
    output_states.extend(frame.ordered_runtime_draws.iter().map(|draw| match draw {
        PreparedRuntimeDraw::Particle { packet, .. } => packet.output_state.clone(),
        PreparedRuntimeDraw::Entity { submission, .. }
        | PreparedRuntimeDraw::O04City { submission, .. }
        | PreparedRuntimeDraw::O05Building { submission, .. } => submission.output_state.clone(),
    }));
    let target = production_main_scene_desc_from_outputs(output_states.iter())?;
    if let Some(tree_runtime) = &frame.tree_runtime {
        validate_c07_executor_operations(
            tree_runtime,
            &ordered_static_programs,
            &frame.ordered_render_operations,
        )?;
    }

    let mut prepared = PreparedProductionD3dFrame {
        content_manifest_sha256: frame.content_manifest_sha256.clone(),
        product_modset_sha256: frame.product_modset_sha256.clone(),
        effective_vfs_manifest_sha256: frame.effective_vfs_manifest_sha256.clone(),
        c03_camera_replay: frame.c03_camera_replay.clone(),
        target,
        ordered_scene_constant_buffers: frame.ordered_scene_constant_buffers.clone(),
        ordered_render_operations: frame
            .ordered_render_operations
            .iter()
            .filter(|reference| match reference {
                RenderExecutionRef::Static { operation_ordinal } => {
                    !omitted_static_ordinals.contains(operation_ordinal)
                }
                RenderExecutionRef::Runtime { .. } => true,
                RenderExecutionRef::M03 { .. } => !omit_m03,
            })
            .cloned()
            .collect(),
        snow_mud_texture: frame.snow_mud_texture.clone(),
        political_textures: frame.political_textures.clone(),
        ordered_static_used_content,
        ordered_static_programs,
        authenticated_texture_snapshots,
        rail_ribbon: frame.rail_ribbon.clone(),
        mapname_d80_state,
        tree_runtime: frame.tree_runtime.clone(),
        tree_materialized_execution_sha256: None,
        s1_shadow: None,
        ordered_runtime_draws: frame.ordered_runtime_draws.clone(),
        authenticated_ui_pass: frame.authenticated_ui_pass.clone(),
        m03_arrows,
        rail_supply_arrows,
        omissions,
    };
    if let Some(carrier) = &frame.s1_shadow {
        prepared.s1_shadow = Some(prepare_s1_shadow_execution(carrier, &prepared)?);
    }
    prepared.tree_materialized_execution_sha256 = prepared
        .tree_runtime
        .as_ref()
        .map(|carrier| {
            c07_materialized_execution_sha256(
                carrier,
                &prepared.ordered_static_programs,
                &prepared.ordered_render_operations,
            )
        })
        .transpose()?;
    validate_exact_rail_d80_execution(&prepared)?;
    if let Some(rail) = &prepared.rail_supply_arrows {
        rail.validate_against_static_programs(&prepared.ordered_static_programs)
            .map_err(|error| format!("prepared rail supply-arrow join is invalid: {error}"))?;
    }
    Ok(prepared)
}

fn validate_exact_rail_d80_execution(frame: &PreparedProductionD3dFrame) -> Result<()> {
    let Some(binding) = &frame.rail_ribbon else {
        return Ok(());
    };
    if binding.authority != "exact_a729_d81_d80_railroad_registration_join"
        || binding.ordered_draws.is_empty()
    {
        return Err("D81/D80 railroad binding authority is incomplete".into());
    }
    let mut operations = BTreeMap::new();
    for program in &frame.ordered_static_programs {
        for operation in &program.ordered_operations {
            operations.insert(operation.operation_ordinal, (program, operation));
        }
    }
    for expected in &binding.ordered_draws {
        let (program, operation) = operations
            .get(&expected.operation_ordinal)
            .copied()
            .ok_or("D81/D80 railroad binding names an omitted static operation")?;
        if program.program_ordinal != expected.program_ordinal {
            return Err("D81/D80 railroad binding program ordinal drifted".into());
        }
        let StaticMapCommand::Draw { draw } = &operation.command else {
            return Err("D81/D80 railroad binding names a non-draw operation".into());
        };
        let StaticGeometrySource::Exact { geometry } = &draw.geometry else {
            return Err("D81/D80 railroad binding lost exact engine geometry".into());
        };
        let Some(vertex) = geometry.ordered_vertex_buffers.first() else {
            return Err("D81/D80 railroad binding lost its vertex buffer".into());
        };
        let BufferSource::Inline { bytes, sha256 } = &vertex.source else {
            return Err("D81/D80 railroad binding vertex source is no longer inline".into());
        };
        if draw.payload_sha256 != expected.draw_payload_sha256
            || sha256 != &expected.vertex_payload_sha256
            || u64::try_from(bytes.len()).ok() != Some(expected.vertex_byte_count)
        {
            return Err("D81/D80 railroad binding payload identity drifted".into());
        }
        let occurrences = frame
            .ordered_render_operations
            .iter()
            .filter(|reference| {
                matches!(
                    reference,
                    RenderExecutionRef::Static { operation_ordinal }
                        if *operation_ordinal == expected.operation_ordinal
                )
            })
            .count();
        if occurrences != 1 {
            return Err("D81/D80 railroad draw lacks one exact global interleave position".into());
        }
    }
    Ok(())
}

pub fn execute_production_d3d_frame(
    frame: &PreparedProductionD3dFrame,
    bridge: &mut impl ProductionD3dExecutionBridge,
) -> Result<ProductionD3dExecutionReceipt> {
    validate_provenance_sha256(&frame.content_manifest_sha256, "content manifest digest")?;
    validate_provenance_sha256(&frame.product_modset_sha256, "product modset digest")?;
    validate_provenance_sha256(
        &frame.effective_vfs_manifest_sha256,
        "effective VFS manifest digest",
    )?;
    validate_exact_rail_d80_execution(frame)?;
    if let Some(rail) = &frame.rail_supply_arrows {
        rail.validate_against_static_programs(&frame.ordered_static_programs)
            .map_err(|error| format!("executable rail supply-arrow join is invalid: {error}"))?;
        validate_receipt_content_closure(
            &frame.ordered_static_used_content,
            &rail.used_content,
            "rail authority",
        )?;
    }
    let retained_mapname_state = frame
        .ordered_static_programs
        .iter()
        .try_fold(MapnameD80State::Missing, |state, program| {
            state.merge(program.mapname_d80_state()?)
        })?;
    let legacy_omission = frame.omissions.iter().any(|omission| {
        omission
            .reason
            .starts_with("B10 legacy mapname program omitted:")
    });
    match (frame.mapname_d80_state, retained_mapname_state) {
        (MapnameD80State::Missing, MapnameD80State::Missing) if !legacy_omission => {}
        (MapnameD80State::LegacyUnstamped, MapnameD80State::Missing) if legacy_omission => {}
        (MapnameD80State::Authenticated, MapnameD80State::Authenticated) if !legacy_omission => {}
        _ => {
            return Err(
                "prepared mapname D80 state/omission disagrees with executable programs".into(),
            )
        }
    }
    if let Some(binding) = &frame.c03_camera_replay {
        validate_c03_camera_replay_binding(binding)?;
    }
    if let Some(s1_shadow) = &frame.s1_shadow {
        validate_s1_shadow_execution(s1_shadow, frame)?;
    }
    match (
        &frame.tree_runtime,
        &frame.tree_materialized_execution_sha256,
    ) {
        (Some(tree_runtime), Some(expected)) => {
            let actual = c07_materialized_execution_sha256(
                tree_runtime,
                &frame.ordered_static_programs,
                &frame.ordered_render_operations,
            )?;
            if &actual != expected {
                return Err("prepared C07 materialized execution stream drifted".into());
            }
        }
        (None, None) => {}
        _ => return Err("prepared C07 carrier/execution seal nullability mismatch".into()),
    }
    bridge.begin_frame(frame)?;
    let mut ordinal = 0u32;
    let mut ordered_draws = Vec::new();
    let mut used = BTreeMap::<String, UsedContentKey>::new();
    for content in &frame.ordered_static_used_content {
        insert_content(&mut used, content);
    }
    if frame.ordered_render_operations.is_empty() {
        if !frame.ordered_static_programs.is_empty() {
            return Err(
                "static map programs have no authoritative cross-family execution order".into(),
            );
        }
        if frame
            .m03_arrows
            .as_ref()
            .is_some_and(|m03| !m03.draws.is_empty())
        {
            return Err(
                "M03 arrow draws have no authoritative cross-family execution order".into(),
            );
        }
        for draw in &frame.ordered_runtime_draws {
            bridge.execute_runtime_draw(ordinal, &frame.ordered_scene_constant_buffers, draw)?;
            collect_runtime_draw_content(draw, &mut used);
            let pass = runtime_draw_pass(draw);
            ordered_draws.push(ProductionD3dExecutedDraw {
                global_ordinal: ordinal,
                pass: pass.into(),
                draw_token: runtime_draw_token(draw),
            });
            ordinal = ordinal
                .checked_add(1)
                .ok_or("production draw ordinal overflow")?;
        }
    } else {
        let static_count = frame
            .ordered_static_programs
            .iter()
            .map(|program| program.ordered_operations.len())
            .sum::<usize>();
        let m03_count = frame.m03_arrows.as_ref().map_or(0, |m03| m03.draws.len());
        if frame.ordered_render_operations.len()
            != static_count + frame.ordered_runtime_draws.len() + m03_count
        {
            return Err(
                "cross-family execution order does not cover every static/runtime operation".into(),
            );
        }
        let mut static_operations = BTreeMap::new();
        for program in &frame.ordered_static_programs {
            for operation in &program.ordered_operations {
                if static_operations
                    .insert(operation.operation_ordinal, (program, operation))
                    .is_some()
                {
                    return Err("static map programs repeat an operation ordinal".into());
                }
            }
        }
        let mut runtime_draws = BTreeMap::new();
        for draw in &frame.ordered_runtime_draws {
            let (PreparedRuntimeDraw::Particle { ordered_draw, .. }
            | PreparedRuntimeDraw::Entity { ordered_draw, .. }
            | PreparedRuntimeDraw::O04City { ordered_draw, .. }
            | PreparedRuntimeDraw::O05Building { ordered_draw, .. }) = draw;
            if runtime_draws.insert(ordered_draw, draw).is_some() {
                return Err("runtime draws repeat an ordered-draw reference".into());
            }
        }
        let mut seen_static = std::collections::BTreeSet::new();
        let mut seen_runtime = std::collections::BTreeSet::new();
        let mut seen_m03 = std::collections::BTreeSet::new();
        for reference in &frame.ordered_render_operations {
            match reference {
                RenderExecutionRef::Static { operation_ordinal } => {
                    if !seen_static.insert(*operation_ordinal) {
                        return Err("cross-family order repeats a static operation".into());
                    }
                    let (program, operation) = static_operations
                        .get(operation_ordinal)
                        .copied()
                        .ok_or("cross-family order references an absent static operation")?;
                    bridge.execute_static_operation(
                        ordinal,
                        program,
                        &frame.ordered_scene_constant_buffers,
                        operation,
                    )?;
                    collect_static_operation_content(operation, &mut used);
                    ordered_draws.push(ProductionD3dExecutedDraw {
                        global_ordinal: ordinal,
                        pass: program.pass_token.clone(),
                        draw_token: static_operation_token(operation),
                    });
                }
                RenderExecutionRef::Runtime { output } => {
                    if !seen_runtime.insert(output) {
                        return Err("cross-family order repeats a runtime draw".into());
                    }
                    let draw = *runtime_draws
                        .get(output)
                        .ok_or("cross-family order references an absent runtime draw")?;
                    bridge.execute_runtime_draw(
                        ordinal,
                        &frame.ordered_scene_constant_buffers,
                        draw,
                    )?;
                    collect_runtime_draw_content(draw, &mut used);
                    let pass = runtime_draw_pass(draw);
                    ordered_draws.push(ProductionD3dExecutedDraw {
                        global_ordinal: ordinal,
                        pass: pass.into(),
                        draw_token: runtime_draw_token(draw),
                    });
                }
                RenderExecutionRef::M03 { output } => {
                    if !seen_m03.insert(output.clone()) {
                        return Err("cross-family order repeats an M03 draw".into());
                    }
                    let draw = frame
                        .m03_arrows
                        .as_ref()
                        .and_then(|m03| m03.draw(output))
                        .ok_or("cross-family order references an absent M03 draw")?;
                    bridge.execute_m03_draw(
                        ordinal,
                        &frame.ordered_scene_constant_buffers,
                        draw,
                    )?;
                    collect_m03_draw_content(draw, &mut used);
                    ordered_draws.push(ProductionD3dExecutedDraw {
                        global_ordinal: ordinal,
                        pass: "m03_order_movement_arrows".into(),
                        draw_token: draw.draw.draw_token.clone(),
                    });
                }
            }
            ordinal = ordinal
                .checked_add(1)
                .ok_or("production draw ordinal overflow")?;
        }
        if seen_static.len() != static_count
            || seen_runtime.len() != frame.ordered_runtime_draws.len()
            || seen_m03.len() != m03_count
        {
            return Err("cross-family execution order omits a static/runtime operation".into());
        }
    }
    bridge.finish_frame()?;

    let ordered_used_content = used.into_values().collect::<Vec<_>>();
    let mut receipt = ProductionD3dExecutionReceipt {
        schema: PRODUCTION_D3D_EXECUTION_RECEIPT_SCHEMA.into(),
        content_manifest_sha256: frame.content_manifest_sha256.clone(),
        product_modset_sha256: frame.product_modset_sha256.clone(),
        effective_vfs_manifest_sha256: frame.effective_vfs_manifest_sha256.clone(),
        c03_camera_replay: frame.c03_camera_replay.clone(),
        tree_runtime_payload_sha256: frame
            .tree_runtime
            .as_ref()
            .map(|carrier| carrier.payload_sha256.clone()),
        tree_materialized_execution_sha256: frame.tree_materialized_execution_sha256.clone(),
        s1_shadow_payload_sha256: frame
            .s1_shadow
            .as_ref()
            .map(|prepared| prepared.carrier.payload_sha256.clone()),
        s1_materialized_execution_sha256: frame
            .s1_shadow
            .as_ref()
            .and_then(|prepared| prepared.materialized_execution_sha256.clone()),
        mapname_d80_state: frame.mapname_d80_state,
        target: frame.target.clone(),
        ordered_draws,
        ordered_used_content,
        omissions: frame.omissions.clone(),
        deterministic_sha256: String::new(),
    };
    receipt.deterministic_sha256 = Sha256::hex(&serde_json::to_vec(&receipt)?);
    Ok(receipt)
}

fn validate_c07_executor_operations(
    carrier: &C07TreeRuntimeCarrier,
    programs: &[StaticMapPassProgram],
    ordered_operations: &[RenderExecutionRef],
) -> Result<()> {
    let available = programs
        .iter()
        .flat_map(|program| {
            program
                .ordered_operations
                .iter()
                .map(|operation| operation.operation_ordinal)
        })
        .collect::<std::collections::BTreeSet<_>>();
    let mut execution_positions = BTreeMap::new();
    for (position, reference) in ordered_operations.iter().enumerate() {
        let RenderExecutionRef::Static { operation_ordinal } = reference else {
            continue;
        };
        if execution_positions
            .insert(*operation_ordinal, position)
            .is_some()
        {
            return Err("C07 materialized execution order repeats a static operation".into());
        }
    }
    let required = carrier.required_operation_ordinals();
    let mut unique = std::collections::BTreeSet::new();
    let mut previous_position = None;
    for operation_ordinal in required {
        if !unique.insert(operation_ordinal) || !available.contains(&operation_ordinal) {
            return Err(
                "C07 tree operation was duplicated or omitted during D3D preparation".into(),
            );
        }
        let position = *execution_positions
            .get(&operation_ordinal)
            .ok_or("C07 tree operation is absent from executor order")?;
        if previous_position.is_some_and(|previous| position <= previous) {
            return Err(
                "C07 tree executor order differs from the retained three-pass contract".into(),
            );
        }
        previous_position = Some(position);
    }
    Ok(())
}

#[derive(Serialize)]
struct C07MaterializedExecutionRow<'a> {
    operation_ordinal: u32,
    execution_position: usize,
    operation: &'a StaticMapOperation,
}

fn c07_materialized_execution_sha256(
    carrier: &C07TreeRuntimeCarrier,
    programs: &[StaticMapPassProgram],
    ordered_operations: &[RenderExecutionRef],
) -> Result<String> {
    if carrier.computed_payload_sha256()? != carrier.payload_sha256 {
        return Err("prepared C07 carrier payload SHA-256 drifted after admission".into());
    }
    validate_c07_executor_operations(carrier, programs, ordered_operations)?;
    let mut materialized = BTreeMap::new();
    for operation in programs
        .iter()
        .flat_map(|program| program.ordered_operations.iter())
    {
        if materialized
            .insert(operation.operation_ordinal, operation)
            .is_some()
        {
            return Err("C07 materialized programs repeat an operation ordinal".into());
        }
    }
    let execution_positions = ordered_operations
        .iter()
        .enumerate()
        .filter_map(|(position, reference)| match reference {
            RenderExecutionRef::Static { operation_ordinal } => {
                Some((*operation_ordinal, position))
            }
            _ => None,
        })
        .collect::<BTreeMap<_, _>>();
    let rows = carrier
        .required_operation_ordinals()
        .into_iter()
        .map(|operation_ordinal| {
            Ok(C07MaterializedExecutionRow {
                operation_ordinal,
                execution_position: *execution_positions
                    .get(&operation_ordinal)
                    .ok_or("C07 materialized operation has no execution position")?,
                operation: materialized
                    .get(&operation_ordinal)
                    .copied()
                    .ok_or("C07 materialized operation payload is absent")?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Sha256::hex(&serde_json::to_vec(&rows)?))
}

fn collect_m03_draw_content(packet: &M03ArrowDraw, out: &mut BTreeMap<String, UsedContentKey>) {
    let operation = StaticMapOperation {
        operation_ordinal: packet.engine_draw_ordinal,
        payload_sha256: packet.draw.payload_sha256.clone(),
        command: StaticMapCommand::Draw {
            draw: packet.draw.clone(),
        },
    };
    collect_static_operation_content(&operation, out);
}

fn static_program_omitted_dependency(
    program: &StaticMapPassProgram,
    omitted_static_ordinals: &std::collections::BTreeSet<u32>,
    m03_omitted: bool,
) -> Option<String> {
    program.ordered_operations.iter().find_map(|operation| {
        let StaticMapCommand::Draw { draw } = &operation.command else {
            return None;
        };
        static_draw_omitted_dependency(draw, omitted_static_ordinals, m03_omitted)
    })
}

fn static_draw_omitted_dependency(
    draw: &replay_core::StaticMapDraw,
    omitted_static_ordinals: &std::collections::BTreeSet<u32>,
    m03_omitted: bool,
) -> Option<String> {
    draw.ordered_shader_resources.iter().find_map(|binding| {
        let StaticShaderResourceSource::PreparedTarget { producer, .. } = &binding.source else {
            return None;
        };
        match producer {
            RenderExecutionRef::Static { operation_ordinal }
                if omitted_static_ordinals.contains(operation_ordinal) =>
            {
                Some(format!("omitted static operation {operation_ordinal}"))
            }
            RenderExecutionRef::M03 { output } if m03_omitted => Some(format!(
                "omitted M03 draw {}",
                serde_json::to_string(output).expect("M03ArrowDrawRef serialization cannot fail")
            )),
            _ => None,
        }
    })
}

fn validate_static_operation(
    operation: &StaticMapOperation,
    resources: &BTreeMap<String, ProductionMapResource>,
    expected_consumer: &str,
) -> Result<()> {
    let StaticMapCommand::Draw { draw } = &operation.command else {
        let target = match &operation.command {
            StaticMapCommand::ClearColor { target, .. }
            | StaticMapCommand::ClearDepthStencil { target, .. } => target,
            StaticMapCommand::Draw { .. } => unreachable!(),
        };
        validate_render_target_content(target, resources, expected_consumer)?;
        return Ok(());
    };
    if draw.draw_token.trim().is_empty() {
        return Err("production D3D draw token is empty".into());
    }
    validate_shader_payload(
        &draw.shader_program.compiled_vertex_shader.bytecode,
        &draw.shader_program.compiled_vertex_shader.sha256,
        "vertex shader",
    )?;
    validate_shader_payload(
        &draw.shader_program.compiled_pixel_shader.bytecode,
        &draw.shader_program.compiled_pixel_shader.sha256,
        "pixel shader",
    )?;
    for (label, stage) in [
        (
            "geometry shader",
            draw.shader_program.geometry_shader.as_ref(),
        ),
        ("hull shader", draw.shader_program.hull_shader.as_ref()),
        ("domain shader", draw.shader_program.domain_shader.as_ref()),
        (
            "compute shader",
            draw.shader_program.compute_shader.as_ref(),
        ),
    ] {
        if let Some(stage) = stage {
            validate_shader_payload(&stage.compiled.bytecode, &stage.compiled.sha256, label)?;
        }
    }
    require_content(&draw.shader_program.source, resources)?;
    for binding in &draw.ordered_shader_resources {
        if let StaticShaderResourceSource::UsedContent {
            content, resource, ..
        } = &binding.source
        {
            require_content(content, resources)?;
            for subresource in &resource.ordered_subresources {
                validate_buffer_source(&subresource.source, resources)?;
            }
        }
    }
    match &draw.geometry {
        StaticGeometrySource::Exact { geometry } => {
            for vertex in &geometry.ordered_vertex_buffers {
                validate_buffer_source(&vertex.source, resources)?;
            }
            if let Some(index) = &geometry.index_buffer {
                validate_buffer_source(&index.source, resources)?;
            }
        }
        StaticGeometrySource::InterpreterOutput {
            ordered_input_content_keys,
            ..
        } => {
            for key in ordered_input_content_keys {
                if !resources.contains_key(key) {
                    return Err(format!(
                        "static geometry interpreter input key {key} is not admitted"
                    )
                    .into());
                }
            }
        }
    }
    for constant in &draw.ordered_constant_buffers {
        validate_shader_payload(&constant.raw_bytes, &constant.sha256, "constant buffer")?;
        if constant.byte_count == 0 {
            return Err("constant-buffer selected range is empty".into());
        }
        let end = usize::try_from(constant.byte_offset)?
            .checked_add(usize::try_from(constant.byte_count)?)
            .ok_or("constant-buffer selected range overflow")?;
        if end > constant.raw_bytes.len() {
            return Err("constant-buffer selected range exceeds carried bytes".into());
        }
    }
    for target in &draw.output_state.ordered_color_targets {
        validate_render_target_content(target, resources, expected_consumer)?;
    }
    if let Some(target) = &draw.output_state.depth_target {
        validate_render_target_content(target, resources, expected_consumer)?;
    }
    Ok(())
}

fn validate_render_target_content(
    target: &RenderTargetDesc,
    resources: &BTreeMap<String, ProductionMapResource>,
    expected_consumer: &str,
) -> Result<()> {
    if let Some(content) = &target.persistent_content {
        require_content_consumer(content, resources, expected_consumer)?;
    }
    let Some(exact) = &target.exact_resource else {
        return Ok(());
    };
    for initial in &exact.ordered_initial_subresources {
        match &initial.source {
            RenderTargetInitialSource::Inline { bytes, sha256 } => {
                if Sha256::hex(bytes) != *sha256 {
                    return Err("exact render-target inline initialization SHA-256 mismatch".into());
                }
            }
            RenderTargetInitialSource::UsedContentRange {
                content,
                source_byte_offset,
                byte_count,
                range_sha256,
            } => {
                let resource = require_content_consumer(content, resources, expected_consumer)?;
                let start = usize::try_from(*source_byte_offset)
                    .map_err(|_| "exact render-target content offset does not fit usize")?;
                let count = usize::try_from(*byte_count)
                    .map_err(|_| "exact render-target content length does not fit usize")?;
                let end = start
                    .checked_add(count)
                    .ok_or("exact render-target content range overflows")?;
                let bytes = resource
                    .bytes
                    .get(start..end)
                    .ok_or("exact render-target content range exceeds admitted payload")?;
                if Sha256::hex(bytes) != *range_sha256 {
                    return Err("exact render-target content-range SHA-256 mismatch".into());
                }
            }
        }
    }
    Ok(())
}

fn require_content_consumer<'a>(
    content: &UsedContentKey,
    resources: &'a BTreeMap<String, ProductionMapResource>,
    expected_consumer: &str,
) -> Result<&'a ProductionMapResource> {
    let resource = require_content(content, resources)?;
    if !resource
        .consumers
        .iter()
        .any(|consumer| consumer == expected_consumer)
    {
        return Err(format!(
            "exact render-target content {} is not bound to sealed consumer {expected_consumer}",
            content.normalized_game_relative_path
        )
        .into());
    }
    Ok(resource)
}

fn validate_shader_payload(bytes: &[u8], expected: &str, role: &str) -> Result<()> {
    if bytes.is_empty() {
        return Err(format!("{role} bytecode/payload is empty").into());
    }
    let actual = Sha256::hex(bytes);
    if actual != expected {
        return Err(format!("{role} SHA-256 mismatch: expected {expected}, got {actual}").into());
    }
    Ok(())
}

fn validate_buffer_source(
    source: &BufferSource,
    resources: &BTreeMap<String, ProductionMapResource>,
) -> Result<()> {
    match source {
        BufferSource::Inline { bytes, sha256 } => {
            validate_shader_payload(bytes, sha256, "inline buffer")
        }
        BufferSource::ParticleInstanceRecords => Err(
            "ParticleInstanceRecords buffer source is not valid in static map validation".into(),
        ),
        BufferSource::UsedContentRange {
            content,
            source_byte_offset,
            byte_count,
            range_sha256,
        } => {
            let resource = require_content(content, resources)?;
            let start = usize::try_from(*source_byte_offset)
                .map_err(|_| "used-content range offset does not fit usize")?;
            let len = usize::try_from(*byte_count)
                .map_err(|_| "used-content range length does not fit usize")?;
            let end = start
                .checked_add(len)
                .ok_or("used-content range overflow")?;
            let bytes = resource
                .bytes
                .get(start..end)
                .ok_or("used-content range is outside admitted payload")?;
            validate_shader_payload(bytes, range_sha256, "used-content range")
        }
    }
}

fn require_content<'a>(
    content: &UsedContentKey,
    resources: &'a BTreeMap<String, ProductionMapResource>,
) -> Result<&'a ProductionMapResource> {
    resources
        .values()
        .find(|resource| &resource.content == content)
        .ok_or_else(|| {
            format!(
                "draw references non-admitted content {}",
                content.normalized_game_relative_path
            )
            .into()
        })
}

fn collect_static_operation_content(
    operation: &StaticMapOperation,
    out: &mut BTreeMap<String, UsedContentKey>,
) {
    let StaticMapCommand::Draw { draw } = &operation.command else {
        let target = match &operation.command {
            StaticMapCommand::ClearColor { target, .. }
            | StaticMapCommand::ClearDepthStencil { target, .. } => target,
            StaticMapCommand::Draw { .. } => unreachable!(),
        };
        collect_render_target_content(target, out);
        return;
    };
    let mut insert = |value: &UsedContentKey| insert_content(out, value);
    insert(&draw.shader_program.source);
    for binding in &draw.ordered_shader_resources {
        if let StaticShaderResourceSource::UsedContent {
            content, resource, ..
        } = &binding.source
        {
            insert(content);
            for subresource in &resource.ordered_subresources {
                collect_buffer_source_content(&subresource.source, &mut insert);
            }
        }
    }
    if let StaticGeometrySource::Exact { geometry } = &draw.geometry {
        for vertex in &geometry.ordered_vertex_buffers {
            collect_buffer_source_content(&vertex.source, &mut insert);
        }
        if let Some(index) = &geometry.index_buffer {
            collect_buffer_source_content(&index.source, &mut insert);
        }
    }
    for target in &draw.output_state.ordered_color_targets {
        collect_render_target_content_with(target, &mut insert);
    }
    if let Some(target) = &draw.output_state.depth_target {
        collect_render_target_content_with(target, &mut insert);
    }
}

fn collect_runtime_draw_content(
    draw: &PreparedRuntimeDraw,
    out: &mut BTreeMap<String, UsedContentKey>,
) {
    match draw {
        PreparedRuntimeDraw::Particle { packet, .. } => {
            collect_runtime_parts(
                &packet.shader_program.source,
                &packet.ordered_texture_bindings,
                &packet.geometry,
                &packet.output_state,
                out,
            );
        }
        PreparedRuntimeDraw::Entity { submission, .. }
        | PreparedRuntimeDraw::O04City { submission, .. }
        | PreparedRuntimeDraw::O05Building { submission, .. } => {
            collect_runtime_parts(
                &submission.shader_program.source,
                &submission.ordered_texture_bindings,
                &submission.geometry,
                &submission.output_state,
                out,
            );
            insert_content(out, &submission.content_identity.resolved_entity_asset);
            insert_content(out, &submission.content_identity.mesh);
        }
    }
}

fn collect_runtime_parts(
    shader: &UsedContentKey,
    textures: &[replay_core::TextureBinding],
    geometry: &replay_core::GeometryBinding,
    output: &replay_core::RenderOutputState,
    out: &mut BTreeMap<String, UsedContentKey>,
) {
    insert_content(out, shader);
    for texture in textures {
        insert_content(out, &texture.content);
        for subresource in &texture.resource.ordered_subresources {
            if let BufferSource::UsedContentRange { content, .. } = &subresource.source {
                insert_content(out, content);
            }
        }
    }
    for vertex in &geometry.ordered_vertex_buffers {
        if let BufferSource::UsedContentRange { content, .. } = &vertex.source {
            insert_content(out, content);
        }
    }
    if let Some(index) = &geometry.index_buffer {
        if let BufferSource::UsedContentRange { content, .. } = &index.source {
            insert_content(out, content);
        }
    }
    for target in &output.ordered_color_targets {
        collect_render_target_content(target, out);
    }
    if let Some(target) = &output.depth_target {
        collect_render_target_content(target, out);
    }
}

fn collect_render_target_content(
    target: &RenderTargetDesc,
    out: &mut BTreeMap<String, UsedContentKey>,
) {
    collect_render_target_content_with(target, &mut |content| insert_content(out, content));
}

fn collect_render_target_content_with(
    target: &RenderTargetDesc,
    insert: &mut impl FnMut(&UsedContentKey),
) {
    if let Some(content) = &target.persistent_content {
        insert(content);
    }
    if let Some(exact) = &target.exact_resource {
        for initial in &exact.ordered_initial_subresources {
            if let RenderTargetInitialSource::UsedContentRange { content, .. } = &initial.source {
                insert(content);
            }
        }
    }
}

fn static_operation_token(operation: &StaticMapOperation) -> String {
    match &operation.command {
        StaticMapCommand::ClearColor { .. } => "clear_color".into(),
        StaticMapCommand::ClearDepthStencil { .. } => "clear_depth_stencil".into(),
        StaticMapCommand::Draw { draw } => draw.draw_token.clone(),
    }
}

fn collect_buffer_source_content(source: &BufferSource, insert: &mut impl FnMut(&UsedContentKey)) {
    if let BufferSource::UsedContentRange { content, .. } = source {
        insert(content);
    }
}

fn insert_content(out: &mut BTreeMap<String, UsedContentKey>, value: &UsedContentKey) {
    let key = format!(
        "{}:{}:{}:{}",
        value.winning_layer_ordinal,
        value.layer_id,
        value.normalized_game_relative_path,
        value.payload_sha256
    );
    out.insert(key, value.clone());
}

fn extend_receipt_content_closure(
    receipt_content: &mut Vec<UsedContentKey>,
    required_content: &[UsedContentKey],
) {
    for content in required_content {
        if !receipt_content.contains(content) {
            receipt_content.push(content.clone());
        }
    }
}

fn validate_receipt_content_closure(
    receipt_content: &[UsedContentKey],
    required_content: &[UsedContentKey],
    label: &str,
) -> Result<()> {
    for required in required_content {
        if !receipt_content.contains(required) {
            return Err(format!(
                "prepared {label} content is absent from the execution receipt closure: {}",
                required.normalized_game_relative_path
            )
            .into());
        }
    }
    Ok(())
}

fn runtime_draw_token(draw: &PreparedRuntimeDraw) -> String {
    match draw {
        PreparedRuntimeDraw::Particle { ordered_draw, .. }
        | PreparedRuntimeDraw::Entity { ordered_draw, .. }
        | PreparedRuntimeDraw::O04City { ordered_draw, .. }
        | PreparedRuntimeDraw::O05Building { ordered_draw, .. } => {
            serde_json::to_string(ordered_draw).expect("RenderOutputRef serialization cannot fail")
        }
    }
}

fn runtime_draw_pass(draw: &PreparedRuntimeDraw) -> &'static str {
    match draw {
        PreparedRuntimeDraw::O04City { .. } => "o04_city",
        PreparedRuntimeDraw::O05Building { .. } => "o05_building",
        PreparedRuntimeDraw::Particle { .. } | PreparedRuntimeDraw::Entity { .. } => {
            "runtime_submit"
        }
    }
}

pub mod ui;

#[cfg(windows)]
mod native;
#[cfg(windows)]
mod native_draw;
mod s1_shadow;

pub use s1_shadow::{
    prepare_s1_shadow_execution, validate_s1_shadow_execution, PreparedS1ShadowExecution,
};

#[cfg(windows)]
pub use native::{NativeProductionD3dBackend, NativeProductionD3dExecutor, ProductionD3dReadback};

#[cfg(test)]
mod tests;
