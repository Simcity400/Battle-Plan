
use crate::{
    PreparedXmap12RenderFrame, PreparedXmap13Frame, Result, Xmap12PdxMeshMaterialization,
    Xmap13NativeComponent, Xmap13NativeComponentReceipt, Xmap13NativeDrawClosure,
    Xmap13TargetExecutionIdentity, XMAP13_NATIVE_RECEIPT_SCHEMA,
};
use replay_core::{UsedContentKey, ValidatedContentAdmission};
use serde::{Deserialize, Serialize};
use snapshot::sha256::Sha256;
use std::collections::{BTreeMap, BTreeSet};

pub const XMAP12_NATIVE_EXECUTION_SCHEMA: &str = "hoi4.replay.xmap12.native-execution.a729.v1";

#[allow(clippy::too_many_arguments)]
pub fn xmap13_native_bridge_frame(
    prepared: &PreparedXmap13Frame,
    content_manifest_sha256: &str,
    effective_vfs_manifest_sha256: &str,
    replay_frame_identity_sha256: &str,
    viewer_state_sha256: &str,
    fow_noise_content: UsedContentKey,
    fow_height_content: UsedContentKey,
    environment_map_content: UsedContentKey,
) -> Result<PreparedXmap12RenderFrame> {
    prepared.validate()?;
    let ordered_l03_rows = prepared
        .native_entity_draws
        .iter()
        .map(Xmap12PdxMeshMaterialization::l03_caster_row)
        .collect::<Vec<_>>();
    let mut bridge = PreparedXmap12RenderFrame {
        schema: crate::XMAP12_RENDER_FRAME_SCHEMA.into(),
        content_manifest_sha256: content_manifest_sha256.into(),
        effective_vfs_manifest_sha256: effective_vfs_manifest_sha256.into(),
        replay_frame_identity_sha256: replay_frame_identity_sha256.into(),
        viewer_state_sha256: viewer_state_sha256.into(),
        city_frame_sha256: prepared.semantic_input_sha256.clone(),
        building_frame_sha256: prepared.unit_entity_assignment.assignment_sha256.clone(),
        fixed_frame_sha256: prepared.counter_quad_assignment.assignment_sha256.clone(),
        fow_noise_content,
        fow_height_content,
        environment_map_content,
        ordered_draws: prepared.native_entity_draws.clone(),
        ordered_l03_rows,
        deterministic_sha256: String::new(),
    };
    bridge.deterministic_sha256 = Sha256::hex(&serde_json::to_vec(&bridge)?);
    bridge.validate()?;
    Ok(bridge)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap12NativeExecutionReceipt {
    pub schema: String,
    pub prepared_frame_sha256: String,
    pub content_manifest_sha256: String,
    pub replay_frame_identity_sha256: String,
    pub viewer_state_sha256: String,
    pub main_scene_target_sha256: String,
    pub program_set_sha256: String,
    pub resource_set_sha256: String,
    pub issued_draw_count: u32,
    pub ordered_draw_sha256: Vec<String>,
    pub deterministic_sha256: String,
}

impl Xmap12NativeExecutionReceipt {
    pub fn validate(&self) -> Result<()> {
        if self.schema != XMAP12_NATIVE_EXECUTION_SCHEMA
            || self.issued_draw_count as usize != self.ordered_draw_sha256.len()
        {
            return Err("XMAP12 native receipt shape is invalid".into());
        }
        for (value, label) in [
            (&self.prepared_frame_sha256, "XMAP12 native prepared frame"),
            (
                &self.content_manifest_sha256,
                "XMAP12 native content manifest",
            ),
            (
                &self.replay_frame_identity_sha256,
                "XMAP12 native replay frame",
            ),
            (&self.viewer_state_sha256, "XMAP12 native viewer state"),
            (
                &self.main_scene_target_sha256,
                "XMAP12 native MainScene target",
            ),
            (&self.program_set_sha256, "XMAP12 native program set"),
            (&self.resource_set_sha256, "XMAP12 native resource set"),
            (&self.deterministic_sha256, "XMAP12 native receipt"),
        ] {
            validate_sha256(value, label)?;
        }
        for value in &self.ordered_draw_sha256 {
            validate_sha256(value, "XMAP12 native issued draw")?;
        }
        let mut unsigned = self.clone();
        unsigned.deterministic_sha256.clear();
        if self.deterministic_sha256 != Sha256::hex(&serde_json::to_vec(&unsigned)?) {
            return Err("XMAP12 native receipt digest mismatch".into());
        }
        Ok(())
    }
}

#[cfg(windows)]
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct NativeXmap12ReflectedSharedResource {
    pub semantic: String,
    pub shader_register: u32,
    pub view_dimension: u32,
}

#[cfg(windows)]
pub struct NativeXmap12ProgramBinding {
    pub effect_name: String,
    pub program_sha256: String,
    pub required_shared_resource_registers: Vec<u32>,
    pub required_material_resource_registers: Vec<u32>,
    pub required_shared_resources: Vec<NativeXmap12ReflectedSharedResource>,
    pub required_sampler_registers: Vec<u32>,
    pub vertex_shader: *mut winapi::um::d3d11::ID3D11VertexShader,
    pub pixel_shader: *mut winapi::um::d3d11::ID3D11PixelShader,
    pub input_layout: *mut winapi::um::d3d11::ID3D11InputLayout,
    pub blend_state: *mut winapi::um::d3d11::ID3D11BlendState,
    pub depth_state: *mut winapi::um::d3d11::ID3D11DepthStencilState,
    pub rasterizer_state: *mut winapi::um::d3d11::ID3D11RasterizerState,
}

#[cfg(windows)]
pub struct NativeXmap12TextureBinding {
    pub content: UsedContentKey,
    pub shader_register: u32,
    pub view: *mut winapi::um::d3d11::ID3D11ShaderResourceView,
}

#[cfg(windows)]
pub struct NativeXmap12SharedResourceBinding {
    pub semantic: String,
    pub shader_register: u32,
    pub payload_sha256: String,
    pub descriptor_sha256: String,
    pub view_dimension: u32,
    pub view: *mut winapi::um::d3d11::ID3D11ShaderResourceView,
}

#[cfg(windows)]
pub struct NativeXmap12SamplerBinding {
    pub shader_register: u32,
    pub descriptor_sha256: String,
    pub sampler: *mut winapi::um::d3d11::ID3D11SamplerState,
}

#[cfg(windows)]
pub struct NativeXmap12PartitionSceneBinding {
    pub partition_mask: u8,
    pub viewer_state_sha256: String,
    pub payload_sha256: String,
    pub scene_cb0: *mut winapi::um::d3d11::ID3D11Buffer,
}

#[cfg(windows)]
pub struct NativeXmap12MainSceneBindings {
    pub color_rtv: *mut winapi::um::d3d11::ID3D11RenderTargetView,
    pub depth_dsv: *mut winapi::um::d3d11::ID3D11DepthStencilView,
    pub width: u32,
    pub height: u32,
    pub main_scene_target_sha256: String,
    pub programs: Vec<NativeXmap12ProgramBinding>,
    pub textures: Vec<NativeXmap12TextureBinding>,
    pub shared_resources: Vec<NativeXmap12SharedResourceBinding>,
    pub samplers: Vec<NativeXmap12SamplerBinding>,
    pub partition_scenes: Vec<NativeXmap12PartitionSceneBinding>,
    pub pixel_cb2_payload_sha256: String,
    pub pixel_cb2: *mut winapi::um::d3d11::ID3D11Buffer,
}

#[cfg(windows)]
pub struct NativeXmap12ShadowProgramBinding {
    pub pipeline: crate::L03ShadowPipeline,
    pub main_effect_name: String,
    pub program_sha256: String,
    pub required_material_resource_registers: Vec<u32>,
    pub required_sampler_registers: Vec<u32>,
    pub vertex_shader: *mut winapi::um::d3d11::ID3D11VertexShader,
    pub pixel_shader: *mut winapi::um::d3d11::ID3D11PixelShader,
    pub input_layout: *mut winapi::um::d3d11::ID3D11InputLayout,
    pub depth_state: *mut winapi::um::d3d11::ID3D11DepthStencilState,
    pub rasterizer_state: *mut winapi::um::d3d11::ID3D11RasterizerState,
}

#[cfg(windows)]
struct OwnedXmap12Program {
    effect_name: String,
    program_sha256: String,
    required_material_resource_registers: Vec<u32>,
    required_shared_resource_registers: Vec<u32>,
    required_shared_resources: Vec<NativeXmap12ReflectedSharedResource>,
    required_sampler_registers: Vec<u32>,
    vertex_shader: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11VertexShader>,
    pixel_shader: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11PixelShader>,
    input_layout: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11InputLayout>,
    blend_state: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11BlendState>,
    depth_state: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11DepthStencilState>,
    rasterizer_state: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11RasterizerState>,
}

#[cfg(windows)]
struct OwnedXmap12ShadowProgram {
    pipeline: crate::L03ShadowPipeline,
    main_effect_name: String,
    program_sha256: String,
    required_material_resource_registers: Vec<u32>,
    required_sampler_registers: Vec<u32>,
    vertex_shader: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11VertexShader>,
    pixel_shader: Option<crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11PixelShader>>,
    input_layout: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11InputLayout>,
    depth_state: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11DepthStencilState>,
    rasterizer_state: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11RasterizerState>,
}

#[cfg(windows)]
struct OwnedXmap12Texture {
    content: UsedContentKey,
    shader_register: u32,
    descriptor_sha256: String,
    _texture: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Texture2D>,
    view: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11ShaderResourceView>,
}

#[cfg(windows)]
struct OwnedXmap12SharedTexture {
    semantic: String,
    content: UsedContentKey,
    descriptor_sha256: String,
    view_dimension: u32,
    _texture: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Texture2D>,
    view: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11ShaderResourceView>,
}

#[cfg(windows)]
struct OwnedXmap12Sampler {
    shader_register: u32,
    descriptor_sha256: String,
    sampler: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11SamplerState>,
}

#[cfg(windows)]
struct OwnedXmap12Submesh {
    group_ordinal: u32,
    submesh_ordinal: u32,
    vertex: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Buffer>,
    index: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Buffer>,
}

#[cfg(windows)]
struct OwnedXmap12Draw {
    stable_typed_id: String,
    draw_sha256: String,
    object_cb1_by_partition:
        BTreeMap<u8, crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Buffer>>,
    skin_cb2: Option<crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Buffer>>,
    submeshes: Vec<OwnedXmap12Submesh>,
}

#[cfg(windows)]
pub struct NativeXmap12ResourceOwner {
    device_identity: usize,
    prepared_frame_sha256: String,
    content_manifest_sha256: String,
    effective_vfs_manifest_sha256: String,
    replay_frame_identity_sha256: String,
    viewer_state_sha256: String,
    map_width: u32,
    map_height: u32,
    viewport_width: u32,
    viewport_height: u32,
    programs: Vec<OwnedXmap12Program>,
    shadow_programs: Vec<OwnedXmap12ShadowProgram>,
    textures: Vec<OwnedXmap12Texture>,
    shared_textures: Vec<OwnedXmap12SharedTexture>,
    samplers: Vec<OwnedXmap12Sampler>,
    draws: Vec<OwnedXmap12Draw>,
    owned_resource_set_sha256: String,
}

#[cfg(windows)]
impl NativeXmap12ResourceOwner {
    pub unsafe fn create_xmap13_on_device(
        device: *mut winapi::um::d3d11::ID3D11Device,
        resolver: &crate::GameFileResolver,
        admission: &ValidatedContentAdmission,
        prepared: &PreparedXmap13Frame,
        content_manifest_sha256: &str,
        effective_vfs_manifest_sha256: &str,
        replay_frame_identity_sha256: &str,
        viewer_state_sha256: &str,
        fow_noise_content: UsedContentKey,
        fow_height_content: UsedContentKey,
        environment_map_content: UsedContentKey,
        map_width: u32,
        map_height: u32,
        viewport_width: u32,
        viewport_height: u32,
    ) -> Result<Self> {
        let bridge = xmap13_native_bridge_frame(
            prepared,
            content_manifest_sha256,
            effective_vfs_manifest_sha256,
            replay_frame_identity_sha256,
            viewer_state_sha256,
            fow_noise_content,
            fow_height_content,
            environment_map_content,
        )?;
        Self::create_on_device(
            device,
            resolver,
            admission,
            &bridge,
            map_width,
            map_height,
            viewport_width,
            viewport_height,
        )
    }

    pub unsafe fn create_on_device(
        device: *mut winapi::um::d3d11::ID3D11Device,
        resolver: &crate::GameFileResolver,
        admission: &ValidatedContentAdmission,
        frame: &PreparedXmap12RenderFrame,
        map_width: u32,
        map_height: u32,
        viewport_width: u32,
        viewport_height: u32,
    ) -> Result<Self> {
        use winapi::um::d3d11::{
            D3D11_BIND_CONSTANT_BUFFER, D3D11_BIND_INDEX_BUFFER, D3D11_BIND_VERTEX_BUFFER,
        };

        frame.validate()?;
        if device.is_null()
            || map_width == 0
            || map_height == 0
            || viewport_width == 0
            || viewport_height == 0
        {
            return Err("XMAP12 resource owner received null/zero device dimensions".into());
        }
        if !resolver.has_content_admission()
            || admission.manifest_digest_sha256() != frame.content_manifest_sha256
        {
            return Err(
                "XMAP12 resource owner requires the frame's admission-bound effective VFS".into(),
            );
        }
        for draw in &frame.ordered_draws {
            if draw.mesh_source.effective_vfs_manifest_sha256 != frame.effective_vfs_manifest_sha256
                || draw.shader_source.effective_vfs_manifest_sha256
                    != frame.effective_vfs_manifest_sha256
            {
                return Err("XMAP12 draw content belongs to another effective VFS".into());
            }
        }

        let constants = crate::shader_compile::derive_a729_engine_map_shader_constants(
            map_width, map_height, false, 0,
        )?;
        let engine_defines = crate::shader_compile::engine_map_shader_defines(constants);
        let selected_effects = frame
            .ordered_draws
            .iter()
            .flat_map(|draw| {
                draw.submeshes.iter().map(move |submesh| {
                    (
                        submesh.material_shader.clone(),
                        draw.shader_source.clone(),
                        draw.material_family,
                        draw.skin_palette_3200_bytes.is_some(),
                    )
                })
            })
            .collect::<BTreeSet<_>>();
        let mut effect_sources = BTreeMap::<String, UsedContentKey>::new();
        for (effect, source, _, _) in &selected_effects {
            if effect_sources
                .insert(effect.clone(), source.clone())
                .is_some_and(|prior| prior != *source)
            {
                return Err(format!(
                    "XMAP12 effect {effect} resolves from multiple admitted shader files"
                )
                .into());
            }
            require_admitted_content(resolver, admission, source)?;
        }

        let mut programs = Vec::with_capacity(effect_sources.len());
        let mut shadow_programs = Vec::with_capacity(effect_sources.len());
        let mut sampler_descriptors = BTreeMap::<u32, NativeSamplerDescriptor>::new();
        for (effect_name, shader_source) in &effect_sources {
            let effect_file = crate::effect_file_from_resolver(
                resolver,
                &shader_source.normalized_game_relative_path,
            )?;
            let main = effect_file
                .effect(effect_name)
                .ok_or_else(|| format!("XMAP12 shader lost selected effect {effect_name}"))?;
            let main_program = create_effect_program(
                device,
                resolver,
                &shader_source.normalized_game_relative_path,
                &effect_file,
                main,
                &engine_defines,
            )?;
            let reflected_samplers = main_program
                .required_sampler_registers
                .iter()
                .copied()
                .collect::<BTreeSet<_>>();
            merge_effect_samplers(
                resolver,
                &shader_source.normalized_game_relative_path,
                &reflected_samplers,
                &mut sampler_descriptors,
            )?;
            programs.push(main_program);

            let shadow_name = format!("{effect_name}Shadow");
            let shadow = effect_file.effect(&shadow_name).ok_or_else(|| {
                format!("XMAP12 selected effect {effect_name} lost exact shadow {shadow_name}")
            })?;
            let family_and_skin = selected_effects
                .iter()
                .filter(|(selected, _, _, _)| selected == effect_name)
                .map(|(_, _, family, skinned)| (*family, *skinned))
                .collect::<BTreeSet<_>>();
            if family_and_skin.len() != 1 {
                return Err(format!(
                    "XMAP12 effect {effect_name} is ambiguously assigned to material/skin families"
                )
                .into());
            }
            let (family, skinned) = *family_and_skin.iter().next().expect("one family");
            let shadow_program = create_shadow_program(
                device,
                resolver,
                &shader_source.normalized_game_relative_path,
                &effect_file,
                shadow,
                effect_name,
                xmap12_shadow_pipeline(family, skinned),
                &engine_defines,
            )?;
            let reflected_shadow_samplers = shadow_program
                .required_sampler_registers
                .iter()
                .copied()
                .collect::<BTreeSet<_>>();
            merge_effect_samplers(
                resolver,
                &shader_source.normalized_game_relative_path,
                &reflected_shadow_samplers,
                &mut sampler_descriptors,
            )?;
            shadow_programs.push(shadow_program);
        }
        programs.sort_by(|left, right| left.effect_name.cmp(&right.effect_name));
        shadow_programs.sort_by(|left, right| left.main_effect_name.cmp(&right.main_effect_name));
        validate_material_binding_contracts(frame, &programs, &shadow_programs)?;

        let mut samplers = Vec::with_capacity(sampler_descriptors.len());
        for descriptor in sampler_descriptors.into_values() {
            samplers.push(create_native_sampler(device, descriptor)?);
        }

        let mut texture_census = BTreeMap::<(u32, String), UsedContentKey>::new();
        for draw in &frame.ordered_draws {
            for submesh in &draw.submeshes {
                for (slot, register, authored) in [
                    ("diffuse", 0u32, submesh.diffuse.as_deref()),
                    ("specular", 1u32, submesh.specular.as_deref()),
                    ("normal", 2u32, submesh.normal_map.as_deref()),
                ] {
                    let Some(authored) = authored else {
                        continue;
                    };
                    let source = material_texture_source(draw, slot, authored)?;
                    require_admitted_content(resolver, admission, &source.content)?;
                    let key = (
                        register,
                        source.content.normalized_game_relative_path.clone(),
                    );
                    if texture_census
                        .insert(key, source.content.clone())
                        .is_some_and(|prior| prior != source.content)
                    {
                        return Err(
                            "XMAP12 material texture path has conflicting admitted identities"
                                .into(),
                        );
                    }
                }
            }
        }
        let mut textures = Vec::with_capacity(texture_census.len());
        for ((shader_register, _), content) in texture_census {
            let bytes = resolver.read_admitted_used_content(&content)?;
            let descriptor = parse_material_dds_descriptor(&content, &bytes)?;
            let (texture, view) = create_material_dds(device, &bytes, &descriptor)?;
            textures.push(OwnedXmap12Texture {
                content,
                shader_register,
                descriptor_sha256: descriptor.descriptor_sha256,
                _texture: texture,
                view,
            });
        }

        let mut shared_textures = Vec::with_capacity(3);
        for (semantic, content) in [
            ("FOWNoise_Texture", &frame.fow_noise_content),
            ("FOWHeight_Texture", &frame.fow_height_content),
        ] {
            require_admitted_content(resolver, admission, content)?;
            let bytes = resolver.read_admitted_used_content(content)?;
            let descriptor = parse_material_dds_descriptor(content, &bytes)?;
            let (texture, view) = create_material_dds(device, &bytes, &descriptor)?;
            shared_textures.push(OwnedXmap12SharedTexture {
                semantic: semantic.into(),
                content: content.clone(),
                descriptor_sha256: descriptor.descriptor_sha256,
                view_dimension: winapi::um::d3dcommon::D3D11_SRV_DIMENSION_TEXTURE2D as u32,
                _texture: texture,
                view,
            });
        }
        require_admitted_content(resolver, admission, &frame.environment_map_content)?;
        let environment_bytes =
            resolver.read_admitted_used_content(&frame.environment_map_content)?;
        let (environment_texture, environment_view, environment_descriptor_sha256) =
            create_environment_cube(device, &frame.environment_map_content, &environment_bytes)?;
        shared_textures.push(OwnedXmap12SharedTexture {
            semantic: "EnvironmentMap_Texture".into(),
            content: frame.environment_map_content.clone(),
            descriptor_sha256: environment_descriptor_sha256,
            view_dimension: winapi::um::d3dcommon::D3D11_SRV_DIMENSION_TEXTURECUBE as u32,
            _texture: environment_texture,
            view: environment_view,
        });

        let mut draws = Vec::with_capacity(frame.ordered_draws.len());
        for draw in &frame.ordered_draws {
            let mut object_cb1_by_partition = BTreeMap::new();
            for partition in [1u8, 2u8, 4u8] {
                if draw.partition_mask & partition != 0 {
                    object_cb1_by_partition.insert(
                        partition,
                        create_object_cb1(device, draw, partition, map_width)?,
                    );
                }
            }
            if object_cb1_by_partition.is_empty() {
                return Err("XMAP12 retained draw has no partition-specific object CB1".into());
            }
            let mut submeshes = Vec::with_capacity(draw.submeshes.len());
            for submesh in &draw.submeshes {
                let index_bytes = submesh
                    .indices
                    .iter()
                    .flat_map(|index| index.to_le_bytes())
                    .collect::<Vec<_>>();
                submeshes.push(OwnedXmap12Submesh {
                    group_ordinal: submesh.object_group_ordinal,
                    submesh_ordinal: submesh.object_submesh_ordinal,
                    vertex: create_immutable_buffer(
                        device,
                        &submesh.vertex_bytes,
                        D3D11_BIND_VERTEX_BUFFER,
                        "XMAP12 retained vertex buffer",
                    )?,
                    index: create_immutable_buffer(
                        device,
                        &index_bytes,
                        D3D11_BIND_INDEX_BUFFER,
                        "XMAP12 retained index buffer",
                    )?,
                });
            }
            draws.push(OwnedXmap12Draw {
                stable_typed_id: draw.stable_typed_id.clone(),
                draw_sha256: draw.deterministic_sha256.clone(),
                object_cb1_by_partition,
                skin_cb2: draw
                    .skin_palette_3200_bytes
                    .as_ref()
                    .map(|palette| {
                        create_immutable_buffer(
                            device,
                            palette,
                            D3D11_BIND_CONSTANT_BUFFER,
                            "XMAP13 retained skin CB2",
                        )
                    })
                    .transpose()?,
                submeshes,
            });
        }
        let owned_resource_set_sha256 = Sha256::hex(&serde_json::to_vec(&(
            frame.deterministic_sha256.as_str(),
            map_width,
            map_height,
            viewport_width,
            viewport_height,
            programs
                .iter()
                .map(|row| (&row.effect_name, &row.program_sha256))
                .collect::<Vec<_>>(),
            shadow_programs
                .iter()
                .map(|row| (&row.main_effect_name, &row.program_sha256))
                .collect::<Vec<_>>(),
            textures
                .iter()
                .map(|row| (row.shader_register, &row.content, &row.descriptor_sha256))
                .collect::<Vec<_>>(),
            shared_textures
                .iter()
                .map(|row| {
                    (
                        row.semantic.as_str(),
                        &row.content,
                        row.descriptor_sha256.as_str(),
                        row.view_dimension,
                    )
                })
                .collect::<Vec<_>>(),
            samplers
                .iter()
                .map(|row| (row.shader_register, &row.descriptor_sha256))
                .collect::<Vec<_>>(),
            draws
                .iter()
                .map(|row| (&row.stable_typed_id, &row.draw_sha256))
                .collect::<Vec<_>>(),
        ))?);

        Ok(Self {
            device_identity: device as usize,
            prepared_frame_sha256: frame.deterministic_sha256.clone(),
            content_manifest_sha256: frame.content_manifest_sha256.clone(),
            effective_vfs_manifest_sha256: frame.effective_vfs_manifest_sha256.clone(),
            replay_frame_identity_sha256: frame.replay_frame_identity_sha256.clone(),
            viewer_state_sha256: frame.viewer_state_sha256.clone(),
            map_width,
            map_height,
            viewport_width,
            viewport_height,
            programs,
            shadow_programs,
            textures,
            shared_textures,
            samplers,
            draws,
            owned_resource_set_sha256,
        })
    }

    pub fn matches(
        &self,
        device: *mut winapi::um::d3d11::ID3D11Device,
        frame: &PreparedXmap12RenderFrame,
        map_width: u32,
        map_height: u32,
        viewport_width: u32,
        viewport_height: u32,
    ) -> bool {
        self.device_identity == device as usize
            && self.prepared_frame_sha256 == frame.deterministic_sha256
            && self.content_manifest_sha256 == frame.content_manifest_sha256
            && self.effective_vfs_manifest_sha256 == frame.effective_vfs_manifest_sha256
            && self.replay_frame_identity_sha256 == frame.replay_frame_identity_sha256
            && self.viewer_state_sha256 == frame.viewer_state_sha256
            && self.map_width == map_width
            && self.map_height == map_height
            && self.viewport_width == viewport_width
            && self.viewport_height == viewport_height
    }

    #[allow(clippy::too_many_arguments)]
    pub fn matches_xmap13(
        &self,
        device: *mut winapi::um::d3d11::ID3D11Device,
        prepared: &PreparedXmap13Frame,
        content_manifest_sha256: &str,
        effective_vfs_manifest_sha256: &str,
        replay_frame_identity_sha256: &str,
        viewer_state_sha256: &str,
        fow_noise_content: UsedContentKey,
        fow_height_content: UsedContentKey,
        environment_map_content: UsedContentKey,
        map_width: u32,
        map_height: u32,
        viewport_width: u32,
        viewport_height: u32,
    ) -> Result<bool> {
        let bridge = xmap13_native_bridge_frame(
            prepared,
            content_manifest_sha256,
            effective_vfs_manifest_sha256,
            replay_frame_identity_sha256,
            viewer_state_sha256,
            fow_noise_content,
            fow_height_content,
            environment_map_content,
        )?;
        Ok(self.matches(
            device,
            &bridge,
            map_width,
            map_height,
            viewport_width,
            viewport_height,
        ))
    }

    pub fn owned_resource_set_sha256(&self) -> &str {
        &self.owned_resource_set_sha256
    }

    pub fn program_bindings(&self) -> Vec<NativeXmap12ProgramBinding> {
        self.programs
            .iter()
            .map(|row| NativeXmap12ProgramBinding {
                effect_name: row.effect_name.clone(),
                program_sha256: row.program_sha256.clone(),
                required_material_resource_registers: row
                    .required_material_resource_registers
                    .clone(),
                required_shared_resource_registers: row.required_shared_resource_registers.clone(),
                required_shared_resources: row.required_shared_resources.clone(),
                required_sampler_registers: row.required_sampler_registers.clone(),
                vertex_shader: row.vertex_shader.as_raw(),
                pixel_shader: row.pixel_shader.as_raw(),
                input_layout: row.input_layout.as_raw(),
                blend_state: row.blend_state.as_raw(),
                depth_state: row.depth_state.as_raw(),
                rasterizer_state: row.rasterizer_state.as_raw(),
            })
            .collect()
    }

    pub fn texture_bindings(&self) -> Vec<NativeXmap12TextureBinding> {
        self.textures
            .iter()
            .map(|row| NativeXmap12TextureBinding {
                content: row.content.clone(),
                shader_register: row.shader_register,
                view: row.view.as_raw(),
            })
            .collect()
    }

    pub fn static_shared_resource(
        &self,
        required: &NativeXmap12ReflectedSharedResource,
    ) -> Option<NativeXmap12SharedResourceBinding> {
        let row = self
            .shared_textures
            .iter()
            .find(|row| row.semantic == required.semantic)?;
        Some(NativeXmap12SharedResourceBinding {
            semantic: required.semantic.clone(),
            shader_register: required.shader_register,
            payload_sha256: row.content.payload_sha256.clone(),
            descriptor_sha256: row.descriptor_sha256.clone(),
            view_dimension: row.view_dimension,
            view: row.view.as_raw(),
        })
    }

    pub fn sampler_bindings(&self) -> Vec<NativeXmap12SamplerBinding> {
        self.samplers
            .iter()
            .map(|row| NativeXmap12SamplerBinding {
                shader_register: row.shader_register,
                descriptor_sha256: row.descriptor_sha256.clone(),
                sampler: row.sampler.as_raw(),
            })
            .collect()
    }

    pub fn shadow_program_bindings(&self) -> Vec<NativeXmap12ShadowProgramBinding> {
        self.shadow_programs
            .iter()
            .map(|row| NativeXmap12ShadowProgramBinding {
                pipeline: row.pipeline,
                main_effect_name: row.main_effect_name.clone(),
                program_sha256: row.program_sha256.clone(),
                required_material_resource_registers: row
                    .required_material_resource_registers
                    .clone(),
                required_sampler_registers: row.required_sampler_registers.clone(),
                vertex_shader: row.vertex_shader.as_raw(),
                pixel_shader: row
                    .pixel_shader
                    .as_ref()
                    .map_or(std::ptr::null_mut(), |shader| shader.as_raw()),
                input_layout: row.input_layout.as_raw(),
                depth_state: row.depth_state.as_raw(),
                rasterizer_state: row.rasterizer_state.as_raw(),
            })
            .collect()
    }

    fn retained_draw(&self, stable_typed_id: &str) -> Result<&OwnedXmap12Draw> {
        self.draws
            .iter()
            .find(|row| row.stable_typed_id == stable_typed_id)
            .ok_or_else(|| "XMAP12 retained draw disappeared".into())
    }
}

#[cfg(windows)]
pub struct NativeXmap12ShadowPartitionBinding {
    pub partition_mask: u8,
    pub selected_light_frame_sha256: String,
    pub payload_sha256: String,
    pub scene_cb0: *mut winapi::um::d3d11::ID3D11Buffer,
}

#[cfg(windows)]
pub struct NativeXmap12LightSceneConstantBuffer {
    device_identity: usize,
    selected_light_frame_sha256: String,
    payload_sha256: String,
    buffer: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Buffer>,
}

#[cfg(windows)]
impl NativeXmap12LightSceneConstantBuffer {
    pub unsafe fn create_on_device(
        device: *mut winapi::um::d3d11::ID3D11Device,
        selected_light: &crate::L03SelectedLightFrame,
    ) -> Result<Self> {
        if device.is_null() {
            return Err("XMAP12 L03 light SceneCB0 input is invalid".into());
        }
        let bytes = xmap12_light_scene_cb0_payload(selected_light.light_view_projection_f32_bits)?;
        Ok(Self {
            device_identity: device as usize,
            selected_light_frame_sha256: Sha256::hex(&serde_json::to_vec(selected_light)?),
            payload_sha256: Sha256::hex(&bytes),
            buffer: create_immutable_buffer(
                device,
                &bytes,
                winapi::um::d3d11::D3D11_BIND_CONSTANT_BUFFER,
                "XMAP12 selected-light SceneCB0",
            )?,
        })
    }

    pub fn partition_binding(
        &self,
        device: *mut winapi::um::d3d11::ID3D11Device,
        partition_mask: u8,
    ) -> Result<NativeXmap12ShadowPartitionBinding> {
        if self.device_identity != device as usize || !matches!(partition_mask, 1 | 2 | 4) {
            return Err("XMAP12 L03 light SceneCB0 binding is invalid".into());
        }
        Ok(NativeXmap12ShadowPartitionBinding {
            partition_mask,
            selected_light_frame_sha256: self.selected_light_frame_sha256.clone(),
            payload_sha256: self.payload_sha256.clone(),
            scene_cb0: self.buffer.as_raw(),
        })
    }
}

#[cfg(windows)]
fn xmap12_light_scene_cb0_payload(matrix: [u32; 16]) -> Result<[u8; 416]> {
    if matrix
        .iter()
        .map(|bits| f32::from_bits(*bits))
        .any(|value| !value.is_finite())
    {
        return Err("XMAP12 L03 light SceneCB0 matrix is invalid".into());
    }
    let mut bytes = [0u8; 416];
    for (lane, bits) in matrix.iter().enumerate() {
        bytes[lane * 4..lane * 4 + 4].copy_from_slice(&bits.to_le_bytes());
    }
    Ok(bytes)
}

#[cfg(windows)]
pub struct NativeXmap12LightDepthCaster<'a> {
    owner_packet: &'static str,
    device: *mut winapi::um::d3d11::ID3D11Device,
    owned: &'a NativeXmap12ResourceOwner,
    frame: &'a PreparedXmap12RenderFrame,
    camera_frame_sha256: String,
    replay_frame_identity_sha256: String,
    selected_light_frame_sha256: String,
    programs: &'a [NativeXmap12ShadowProgramBinding],
    partitions: &'a [NativeXmap12ShadowPartitionBinding],
    registered_submission_sha256s: Vec<String>,
}

#[cfg(windows)]
impl<'a> NativeXmap12LightDepthCaster<'a> {
    pub unsafe fn new(
        device: *mut winapi::um::d3d11::ID3D11Device,
        owned: &'a NativeXmap12ResourceOwner,
        frame: &'a PreparedXmap12RenderFrame,
        projection: &crate::L03LightDepthProjection,
        selected_light_frame: &crate::L03SelectedLightFrame,
        programs: &'a [NativeXmap12ShadowProgramBinding],
        partitions: &'a [NativeXmap12ShadowPartitionBinding],
    ) -> Result<Self> {
        Self::new_for_owner(
            "XMAP12",
            device,
            owned,
            frame,
            projection,
            selected_light_frame,
            programs,
            partitions,
        )
    }

    pub unsafe fn new_xmap13(
        device: *mut winapi::um::d3d11::ID3D11Device,
        owned: &'a NativeXmap12ResourceOwner,
        frame: &'a PreparedXmap12RenderFrame,
        projection: &crate::L03LightDepthProjection,
        selected_light_frame: &crate::L03SelectedLightFrame,
        programs: &'a [NativeXmap12ShadowProgramBinding],
        partitions: &'a [NativeXmap12ShadowPartitionBinding],
    ) -> Result<Self> {
        Self::new_for_owner(
            "XMAP13",
            device,
            owned,
            frame,
            projection,
            selected_light_frame,
            programs,
            partitions,
        )
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn new_for_owner(
        owner_packet: &'static str,
        device: *mut winapi::um::d3d11::ID3D11Device,
        owned: &'a NativeXmap12ResourceOwner,
        frame: &'a PreparedXmap12RenderFrame,
        projection: &crate::L03LightDepthProjection,
        selected_light_frame: &crate::L03SelectedLightFrame,
        programs: &'a [NativeXmap12ShadowProgramBinding],
        partitions: &'a [NativeXmap12ShadowPartitionBinding],
    ) -> Result<Self> {
        frame.validate()?;
        if device.is_null() {
            return Err("XMAP12 L03 caster received a null D3D11 device".into());
        }
        if !owned.matches(
            device,
            frame,
            owned.map_width,
            owned.map_height,
            owned.viewport_width,
            owned.viewport_height,
        ) {
            return Err("XMAP12 L03 caster received resources for another frame/device".into());
        }
        let projection_selected_light = projection
            .selected_light
            .ok_or("XMAP12 L03 caster requires an active selected light")?;
        let projection_target = projection
            .light_depth_target
            .as_ref()
            .ok_or("XMAP12 L03 caster requires the selected light-depth target")?;
        if selected_light_frame.selected_light != projection_selected_light
            || &selected_light_frame.light_depth_target != projection_target
        {
            return Err("XMAP12 L03 selected-light frame differs from projection".into());
        }
        let selected_light_frame_sha256 = Sha256::hex(&serde_json::to_vec(selected_light_frame)?);
        let ids = frame
            .ordered_draws
            .iter()
            .map(|draw| draw.stable_typed_id.as_str())
            .collect::<BTreeSet<_>>();
        let owned_submissions = projection
            .submissions
            .iter()
            .filter(|submission| match submission {
                crate::L03LightDepthSubmission::Pdxmesh {
                    stable_typed_upstream_id,
                    ..
                } => ids.contains(stable_typed_upstream_id.as_str()),
                crate::L03LightDepthSubmission::Tree { .. } => false,
            })
            .collect::<Vec<_>>();
        if owned_submissions.is_empty() {
            return Err("XMAP12 L03 caster has no projected XMAP12 submissions".into());
        }
        let mut required_programs = Vec::new();
        for submission in &owned_submissions {
            if let crate::L03LightDepthSubmission::Pdxmesh {
                pipeline,
                selected_submesh_material,
                ..
            } = submission
            {
                let required = (*pipeline, selected_submesh_material.as_str());
                if !required_programs.contains(&required) {
                    required_programs.push(required);
                }
            }
        }
        for (pipeline, effect_name) in &required_programs {
            let matches = programs
                .iter()
                .filter(|program| {
                    program.pipeline == *pipeline && program.main_effect_name == *effect_name
                })
                .count();
            if matches != 1 {
                return Err(
                    "XMAP12 L03 effect/pipeline has no unique native shadow program".into(),
                );
            }
        }
        for program in programs {
            validate_sha256(&program.program_sha256, "XMAP12 L03 shadow program")?;
            let material_registers = program
                .required_material_resource_registers
                .iter()
                .copied()
                .collect::<BTreeSet<_>>();
            let sampler_registers = program
                .required_sampler_registers
                .iter()
                .copied()
                .collect::<BTreeSet<_>>();
            if program.vertex_shader.is_null()
                || program.input_layout.is_null()
                || program.depth_state.is_null()
                || program.rasterizer_state.is_null()
                || material_registers.len() != program.required_material_resource_registers.len()
                || sampler_registers.len() != program.required_sampler_registers.len()
                || material_registers.iter().any(|register| *register != 0)
                || sampler_registers.iter().any(|register| *register != 0)
                || !sampler_registers.iter().all(|register| {
                    owned
                        .samplers
                        .iter()
                        .any(|sampler| sampler.shader_register == *register)
                })
            {
                return Err("XMAP12 L03 shadow program contains null required state".into());
            }
        }
        let required_partitions = owned_submissions
            .iter()
            .filter_map(|submission| match submission {
                crate::L03LightDepthSubmission::Pdxmesh { partition_mask, .. } => {
                    Some(*partition_mask)
                }
                crate::L03LightDepthSubmission::Tree { .. } => None,
            })
            .collect::<BTreeSet<_>>();
        let mut bound_partitions = BTreeSet::new();
        for partition in partitions {
            validate_sha256(&partition.payload_sha256, "XMAP12 L03 partition CB0")?;
            if !matches!(partition.partition_mask, 1 | 2 | 4)
                || partition.scene_cb0.is_null()
                || partition.selected_light_frame_sha256 != selected_light_frame_sha256
                || !bound_partitions.insert(partition.partition_mask)
            {
                return Err("XMAP12 L03 partition binding is invalid".into());
            }
        }
        if required_partitions != bound_partitions {
            return Err("XMAP12 L03 partition CB0 census differs from projection".into());
        }
        let registered_submission_sha256s = owned_submissions
            .iter()
            .map(|submission| {
                serde_json::to_vec(*submission)
                    .map(|bytes| Sha256::hex(&bytes))
                    .map_err(Into::into)
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            owner_packet,
            device,
            owned,
            frame,
            camera_frame_sha256: frame.viewer_state_sha256.clone(),
            replay_frame_identity_sha256: frame.replay_frame_identity_sha256.clone(),
            selected_light_frame_sha256,
            programs,
            partitions,
            registered_submission_sha256s,
        })
    }
}

#[cfg(windows)]
impl crate::NativeXmap09LightDepthCaster for NativeXmap12LightDepthCaster<'_> {
    fn owner_packet(&self) -> &str {
        self.owner_packet
    }

    fn camera_frame_sha256(&self) -> &str {
        &self.camera_frame_sha256
    }

    fn replay_frame_identity_sha256(&self) -> &str {
        &self.replay_frame_identity_sha256
    }

    fn registered_submission_sha256s(&self) -> &[String] {
        &self.registered_submission_sha256s
    }

    unsafe fn draw_light_depth_submission(
        &mut self,
        context: *mut winapi::um::d3d11::ID3D11DeviceContext,
        selected_light: &crate::L03SelectedLightFrame,
        submission: &crate::L03LightDepthSubmission,
    ) -> Result<()> {
        use winapi::shared::dxgiformat::DXGI_FORMAT_R32_UINT;
        use winapi::um::d3dcommon::D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST;

        validate_context_device(self.device, context)?;
        let expected_selected_light_sha256 = Sha256::hex(&serde_json::to_vec(selected_light)?);
        if expected_selected_light_sha256 != self.selected_light_frame_sha256 {
            return Err("XMAP12 L03 selected-light identity changed after registration".into());
        }
        let submission_sha256 = Sha256::hex(&serde_json::to_vec(submission)?);
        if !self
            .registered_submission_sha256s
            .iter()
            .any(|registered| registered == &submission_sha256)
        {
            return Err("XMAP12 L03 draw is not in the registered submission census".into());
        }
        let crate::L03LightDepthSubmission::Pdxmesh {
            partition_mask,
            stable_typed_upstream_id,
            owner,
            world_matrix_f32_bits,
            selected_mesh,
            selected_submesh_material,
            group_ordinal,
            submesh_ordinal,
            pipeline,
            draw,
        } = submission
        else {
            return Err("XMAP12 L03 caster cannot execute a tree submission".into());
        };
        let materialization = self
            .frame
            .ordered_draws
            .iter()
            .find(|row| row.stable_typed_id == *stable_typed_upstream_id)
            .ok_or("XMAP12 L03 submission has no materialized object")?;
        let submesh = materialization
            .submeshes
            .iter()
            .find(|row| {
                row.object_group_ordinal == *group_ordinal
                    && row.object_submesh_ordinal == *submesh_ordinal
            })
            .ok_or("XMAP12 L03 submission has no materialized submesh")?;
        if materialization.owner != *owner
            || materialization.world_matrix_f32_bits != *world_matrix_f32_bits
            || materialization.object_draw.render.selected_mesh != *selected_mesh
            || submesh.material_shader != *selected_submesh_material
            || *draw
                != (crate::L03DrawArguments::DrawIndexed {
                    index_count: submesh.index_count,
                    start_index: 0,
                    base_vertex: 0,
                })
        {
            return Err("XMAP12 L03 submission differs from materialized geometry".into());
        }
        let program = self
            .programs
            .iter()
            .find(|row| {
                row.pipeline == *pipeline && row.main_effect_name == *selected_submesh_material
            })
            .ok_or("XMAP12 L03 shadow program disappeared")?;
        let partition = self
            .partitions
            .iter()
            .find(|row| row.partition_mask == *partition_mask)
            .ok_or("XMAP12 L03 partition CB0 disappeared")?;
        let retained = self.owned.retained_draw(stable_typed_upstream_id)?;
        if retained.draw_sha256 != materialization.deterministic_sha256 {
            return Err("XMAP12 L03 retained draw identity changed".into());
        }
        let geometry = retained
            .submeshes
            .iter()
            .find(|row| {
                row.group_ordinal == *group_ordinal && row.submesh_ordinal == *submesh_ordinal
            })
            .ok_or("XMAP12 L03 retained submesh disappeared")?;
        let scene = [partition.scene_cb0];
        let object_cb1 = retained
            .object_cb1_by_partition
            .get(partition_mask)
            .ok_or("XMAP12 L03 partition-specific object CB1 disappeared")?;
        let object = [object_cb1.as_raw()];
        let buffers = [geometry.vertex.as_raw()];
        let strides = [submesh.vertex_stride_bytes];
        let offsets = [0u32];
        (*context).VSSetShader(program.vertex_shader, std::ptr::null(), 0);
        (*context).PSSetShader(program.pixel_shader, std::ptr::null(), 0);
        (*context).IASetInputLayout(program.input_layout);
        (*context).IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
        (*context).RSSetState(program.rasterizer_state);
        (*context).OMSetDepthStencilState(program.depth_state, 0);
        (*context).VSSetConstantBuffers(0, 1, scene.as_ptr());
        (*context).VSSetConstantBuffers(1, 1, object.as_ptr());
        let skin_cb2 = retained
            .skin_cb2
            .as_ref()
            .map(|buffer| buffer.as_raw())
            .unwrap_or(std::ptr::null_mut());
        (*context).VSSetConstantBuffers(2, 1, &skin_cb2);
        (*context).PSSetConstantBuffers(0, 1, scene.as_ptr());
        (*context).PSSetConstantBuffers(1, 1, object.as_ptr());
        bind_shadow_material_resources(context, self.owned, materialization, submesh, program)?;
        (*context).IASetVertexBuffers(0, 1, buffers.as_ptr(), strides.as_ptr(), offsets.as_ptr());
        (*context).IASetIndexBuffer(geometry.index.as_raw(), DXGI_FORMAT_R32_UINT, 0);
        (*context).DrawIndexed(submesh.index_count, 0, 0);
        clear_shadow_material_bindings(context);
        Ok(())
    }
}

#[cfg(windows)]
pub unsafe fn execute_xmap12_main_scene(
    device: *mut winapi::um::d3d11::ID3D11Device,
    context: *mut winapi::um::d3d11::ID3D11DeviceContext,
    owned: &NativeXmap12ResourceOwner,
    frame: &PreparedXmap12RenderFrame,
    bindings: &NativeXmap12MainSceneBindings,
) -> Result<Xmap12NativeExecutionReceipt> {
    use winapi::shared::dxgiformat::DXGI_FORMAT_R32_UINT;
    use winapi::shared::windef::RECT;
    use winapi::um::d3d11::D3D11_VIEWPORT;
    use winapi::um::d3dcommon::D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST;

    frame.validate()?;
    if !owned.matches(
        device,
        frame,
        owned.map_width,
        owned.map_height,
        bindings.width,
        bindings.height,
    ) {
        return Err("XMAP12 MainScene resources do not match frame/device/viewport".into());
    }
    validate_native_bindings(device, context, frame, bindings)?;
    let program_by_effect = bindings
        .programs
        .iter()
        .map(|row| (row.effect_name.as_str(), row))
        .collect::<BTreeMap<_, _>>();
    let scene_by_partition = bindings
        .partition_scenes
        .iter()
        .map(|row| (row.partition_mask, row))
        .collect::<BTreeMap<_, _>>();
    let texture_by_identity = bindings
        .textures
        .iter()
        .map(|row| {
            (
                (
                    row.shader_register,
                    row.content.normalized_game_relative_path.as_str(),
                ),
                row,
            )
        })
        .collect::<BTreeMap<_, _>>();

    let viewport = D3D11_VIEWPORT {
        TopLeftX: 0.0,
        TopLeftY: 0.0,
        Width: bindings.width as f32,
        Height: bindings.height as f32,
        MinDepth: 0.0,
        MaxDepth: 1.0,
    };
    (*context).RSSetViewports(1, &viewport);
    let rect = RECT {
        left: 0,
        top: 0,
        right: bindings.width as i32,
        bottom: bindings.height as i32,
    };
    (*context).RSSetScissorRects(1, &rect);
    (*context).OMSetRenderTargets(1, &bindings.color_rtv, bindings.depth_dsv);
    (*context).IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
    bind_samplers(context, bindings);

    let mut ordered_draw_sha256 = Vec::new();
    for draw in &frame.ordered_draws {
        let retained = owned.retained_draw(&draw.stable_typed_id)?;
        if retained.draw_sha256 != draw.deterministic_sha256 {
            return Err("XMAP12 retained MainScene draw identity changed".into());
        }
        for partition in [1u8, 2u8, 4u8] {
            if draw.partition_mask & partition == 0 {
                continue;
            }
            let scene = scene_by_partition
                .get(&partition)
                .ok_or("XMAP12 draw has no exact partition SceneCB0")?;
            let object_cb1 = retained
                .object_cb1_by_partition
                .get(&partition)
                .ok_or("XMAP12 draw has no exact partition object CB1")?;
            (*context).VSSetConstantBuffers(0, 1, &scene.scene_cb0);
            (*context).PSSetConstantBuffers(0, 1, &scene.scene_cb0);
            (*context).VSSetConstantBuffers(1, 1, &object_cb1.as_raw());
            let skin_cb2 = retained
                .skin_cb2
                .as_ref()
                .map(|buffer| buffer.as_raw())
                .unwrap_or(std::ptr::null_mut());
            (*context).VSSetConstantBuffers(2, 1, &skin_cb2);
            (*context).PSSetConstantBuffers(1, 1, &object_cb1.as_raw());
            (*context).PSSetConstantBuffers(2, 1, &bindings.pixel_cb2);
            for submesh in &draw.submeshes {
                let program = program_by_effect
                    .get(submesh.material_shader.as_str())
                    .ok_or_else(|| {
                        format!(
                            "XMAP12 has no exact native program for effect {}",
                            submesh.material_shader
                        )
                    })?;
                bind_program(context, program);
                bind_program_shared_resources(context, program, bindings)?;
                bind_material_textures(context, draw, submesh, &texture_by_identity)?;
                let geometry = retained
                    .submeshes
                    .iter()
                    .find(|row| {
                        row.group_ordinal == submesh.object_group_ordinal
                            && row.submesh_ordinal == submesh.object_submesh_ordinal
                    })
                    .ok_or("XMAP12 retained MainScene submesh disappeared")?;
                let stride = submesh.vertex_stride_bytes;
                let offset = 0u32;
                (*context).IASetVertexBuffers(0, 1, &geometry.vertex.as_raw(), &stride, &offset);
                (*context).IASetIndexBuffer(geometry.index.as_raw(), DXGI_FORMAT_R32_UINT, 0);
                (*context).DrawIndexed(submesh.index_count, 0, 0);
                ordered_draw_sha256.push(Sha256::hex(&serde_json::to_vec(&(
                    draw.deterministic_sha256.as_str(),
                    partition,
                    submesh.object_group_ordinal,
                    submesh.object_submesh_ordinal,
                    program.program_sha256.as_str(),
                ))?));
            }
        }
    }

    let null_cb = std::ptr::null_mut();
    (*context).VSSetConstantBuffers(2, 1, &null_cb);
    clear_native_bindings(context);
    let program_set_sha256 = Sha256::hex(&serde_json::to_vec(
        &bindings
            .programs
            .iter()
            .map(|row| (&row.effect_name, &row.program_sha256))
            .collect::<Vec<_>>(),
    )?);
    let resource_set_sha256 = resource_set_sha256(bindings)?;
    let mut receipt = Xmap12NativeExecutionReceipt {
        schema: XMAP12_NATIVE_EXECUTION_SCHEMA.into(),
        prepared_frame_sha256: frame.deterministic_sha256.clone(),
        content_manifest_sha256: frame.content_manifest_sha256.clone(),
        replay_frame_identity_sha256: frame.replay_frame_identity_sha256.clone(),
        viewer_state_sha256: frame.viewer_state_sha256.clone(),
        main_scene_target_sha256: bindings.main_scene_target_sha256.clone(),
        program_set_sha256,
        resource_set_sha256,
        issued_draw_count: u32::try_from(ordered_draw_sha256.len())?,
        ordered_draw_sha256,
        deterministic_sha256: String::new(),
    };
    receipt.deterministic_sha256 = Sha256::hex(&serde_json::to_vec(&receipt)?);
    receipt.validate()?;
    Ok(receipt)
}

#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
pub unsafe fn execute_xmap13_unit_entities(
    device: *mut winapi::um::d3d11::ID3D11Device,
    context: *mut winapi::um::d3d11::ID3D11DeviceContext,
    owned: &NativeXmap12ResourceOwner,
    prepared: &PreparedXmap13Frame,
    identity: &Xmap13TargetExecutionIdentity,
    content_manifest_sha256: &str,
    effective_vfs_manifest_sha256: &str,
    replay_frame_identity_sha256: &str,
    viewer_state_sha256: &str,
    fow_noise_content: UsedContentKey,
    fow_height_content: UsedContentKey,
    environment_map_content: UsedContentKey,
    bindings: &NativeXmap12MainSceneBindings,
) -> Result<Xmap13NativeComponentReceipt> {
    if identity.hour_index != prepared.hour_index
        || identity.content_manifest_sha256 != content_manifest_sha256
        || identity.main_scene_target_sha256 != bindings.main_scene_target_sha256
    {
        return Err("XMAP13 unit execution identity differs from current frame/target".into());
    }
    let bridge = xmap13_native_bridge_frame(
        prepared,
        content_manifest_sha256,
        effective_vfs_manifest_sha256,
        replay_frame_identity_sha256,
        viewer_state_sha256,
        fow_noise_content,
        fow_height_content,
        environment_map_content,
    )?;
    let native = execute_xmap12_main_scene(device, context, owned, &bridge, bindings)?;
    let draw = if native.issued_draw_count == 0 {
        if !prepared.native_entity_draws.is_empty() {
            return Err(
                "XMAP13 native unit executor issued no draws for a non-empty materialized closure"
                    .into(),
            );
        }
        Xmap13NativeDrawClosure::AuthenticatedEmpty {
            enumerator_witness_sha256: prepared.identity_provider().census_sha256().into(),
        }
    } else {
        Xmap13NativeDrawClosure::Present {
            executor_target_sha256: bindings.main_scene_target_sha256.clone(),
            actual_draw_count: native.issued_draw_count,
            ordered_draw_receipt_sha256s: native.ordered_draw_sha256,
        }
    };
    let assignment = &prepared.unit_entity_assignment;
    let receipt = Xmap13NativeComponentReceipt {
        schema: XMAP13_NATIVE_RECEIPT_SCHEMA.into(),
        component: Xmap13NativeComponent::UnitEntities,
        target_execution_identity: identity.clone(),
        assigned_topology_sha256: assignment.topology_sha256.clone(),
        assigned_stage_id: assignment.stage_name.clone(),
        assigned_operation_id: assignment.operation_name.clone(),
        assigned_ordinal: assignment.assigned_ordinal,
        assigned_assignment_sha256: assignment.assignment_sha256.clone(),
        prepared_frame_sha256: prepared.deterministic_sha256.clone(),
        draw,
        packet_draw_receipt_sha256: String::new(),
    }
    .seal()?;
    receipt.validate_for(identity, prepared)?;
    Ok(receipt)
}

#[cfg(windows)]
unsafe fn validate_native_bindings(
    device: *mut winapi::um::d3d11::ID3D11Device,
    context: *mut winapi::um::d3d11::ID3D11DeviceContext,
    frame: &PreparedXmap12RenderFrame,
    bindings: &NativeXmap12MainSceneBindings,
) -> Result<()> {
    use winapi::um::unknwnbase::IUnknown;
    if device.is_null()
        || context.is_null()
        || bindings.color_rtv.is_null()
        || bindings.depth_dsv.is_null()
        || bindings.pixel_cb2.is_null()
        || bindings.width == 0
        || bindings.height == 0
    {
        return Err("XMAP12 native bindings contain null/zero required state".into());
    }
    let mut actual = std::ptr::null_mut();
    (*context).GetDevice(&mut actual);
    if actual.is_null() || actual != device {
        if !actual.is_null() {
            (*(actual as *mut IUnknown)).Release();
        }
        return Err("XMAP12 context belongs to a different D3D11 device".into());
    }
    (*(actual as *mut IUnknown)).Release();
    for (value, label) in [
        (
            &bindings.main_scene_target_sha256,
            "XMAP12 MainScene target",
        ),
        (&bindings.pixel_cb2_payload_sha256, "XMAP12 pixel CB2"),
    ] {
        validate_sha256(value, label)?;
    }
    let mut effects = BTreeSet::new();
    for program in &bindings.programs {
        validate_sha256(&program.program_sha256, "XMAP12 native program")?;
        let material = program
            .required_material_resource_registers
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        let shared = program
            .required_shared_resource_registers
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        let samplers = program
            .required_sampler_registers
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        let named_shared = program
            .required_shared_resources
            .iter()
            .map(|resource| {
                (
                    resource.shader_register,
                    resource.semantic.as_str(),
                    resource.view_dimension,
                )
            })
            .collect::<BTreeSet<_>>();
        if program.effect_name.trim().is_empty()
            || !effects.insert(program.effect_name.as_str())
            || shared.len() != program.required_shared_resource_registers.len()
            || material.len() != program.required_material_resource_registers.len()
            || material.iter().any(|register| *register > 2)
            || named_shared.len() != program.required_shared_resources.len()
            || named_shared.iter().any(|(register, semantic, dimension)| {
                *register <= 2
                    || semantic.trim().is_empty()
                    || *dimension == 0
                    || !shared.contains(register)
            })
            || named_shared.len() != shared.len()
            || samplers.len() != program.required_sampler_registers.len()
            || shared.iter().any(|register| *register <= 2)
            || program.vertex_shader.is_null()
            || program.pixel_shader.is_null()
            || program.input_layout.is_null()
            || program.blend_state.is_null()
            || program.depth_state.is_null()
            || program.rasterizer_state.is_null()
        {
            return Err("XMAP12 native program binding is duplicate/null".into());
        }
    }
    let required_effects = frame
        .ordered_draws
        .iter()
        .flat_map(|draw| &draw.submeshes)
        .map(|row| row.material_shader.as_str())
        .collect::<BTreeSet<_>>();
    if required_effects != effects {
        return Err("XMAP12 native program census differs from selected effects".into());
    }
    let mut partitions = BTreeSet::new();
    for scene in &bindings.partition_scenes {
        if !matches!(scene.partition_mask, 1 | 2 | 4)
            || !partitions.insert(scene.partition_mask)
            || scene.viewer_state_sha256 != frame.viewer_state_sha256
            || scene.scene_cb0.is_null()
        {
            return Err("XMAP12 partition SceneCB0 binding is invalid".into());
        }
        validate_sha256(&scene.payload_sha256, "XMAP12 partition SceneCB0")?;
    }
    let mut shared_keys = BTreeSet::new();
    for resource in &bindings.shared_resources {
        let live_dimension = xmap12_native_srv_view_dimension(resource.view)?;
        if resource.semantic.trim().is_empty()
            || resource.view.is_null()
            || resource.view_dimension == 0
            || resource.view_dimension != live_dimension
            || !shared_keys.insert((
                resource.shader_register,
                resource.semantic.as_str(),
                resource.view_dimension,
            ))
        {
            return Err("XMAP12 shared resource binding is invalid".into());
        }
        validate_sha256(&resource.payload_sha256, "XMAP12 shared resource")?;
        validate_sha256(&resource.descriptor_sha256, "XMAP12 shared descriptor")?;
    }
    let mut sampler_registers = BTreeSet::new();
    for sampler in &bindings.samplers {
        if sampler.sampler.is_null() || !sampler_registers.insert(sampler.shader_register) {
            return Err("XMAP12 sampler binding is null".into());
        }
        validate_sha256(&sampler.descriptor_sha256, "XMAP12 sampler descriptor")?;
    }
    for program in &bindings.programs {
        if !program.required_shared_resources.iter().all(|required| {
            shared_keys.contains(&(
                required.shader_register,
                required.semantic.as_str(),
                required.view_dimension,
            ))
        }) || !program
            .required_sampler_registers
            .iter()
            .all(|register| sampler_registers.contains(register))
        {
            return Err("XMAP12 native program is missing a reflected shared binding".into());
        }
    }
    let mut texture_bindings = BTreeSet::new();
    for texture in &bindings.textures {
        texture
            .content
            .validate()
            .map_err(|error| format!("XMAP12 native texture identity: {error}"))?;
        if texture.shader_register > 2
            || texture.view.is_null()
            || !texture_bindings.insert((
                texture.shader_register,
                texture.content.normalized_game_relative_path.as_str(),
            ))
        {
            return Err("XMAP12 native material texture binding is invalid".into());
        }
    }
    let mut required_textures = BTreeSet::new();
    for draw in &frame.ordered_draws {
        for submesh in &draw.submeshes {
            for (slot_name, register, authored) in [
                ("diffuse", 0u32, submesh.diffuse.as_deref()),
                ("specular", 1u32, submesh.specular.as_deref()),
                ("normal", 2u32, submesh.normal_map.as_deref()),
            ] {
                let Some(authored) = authored else {
                    continue;
                };
                let source = material_texture_source(draw, slot_name, authored)?;
                required_textures.insert((
                    register,
                    source.content.normalized_game_relative_path.as_str(),
                ));
            }
        }
    }
    if texture_bindings != required_textures {
        return Err("XMAP12 native material texture census differs from prepared draws".into());
    }
    Ok(())
}

#[cfg(windows)]
#[derive(Clone, Debug, PartialEq, Eq)]
struct NativeSamplerDescriptor {
    name: String,
    shader_register: u32,
    mag_filter: String,
    min_filter: String,
    mip_filter: String,
    address_u: String,
    address_v: String,
    sampler_type: Option<String>,
    mip_map_lod_bias: Option<String>,
    mip_map_min_lod: Option<String>,
    mip_map_max_lod: Option<String>,
    max_anisotropy: Option<String>,
}

#[cfg(windows)]
#[derive(Clone, Debug, Serialize)]
struct Xmap12DdsMipDescriptor {
    source_offset: u64,
    row_pitch: u32,
    slice_pitch: u32,
}

#[cfg(windows)]
#[derive(Clone, Debug, Serialize)]
struct Xmap12DdsDescriptor {
    width: u32,
    height: u32,
    mip_levels: u32,
    dxgi_format: u32,
    payload_sha256: String,
    mips: Vec<Xmap12DdsMipDescriptor>,
    descriptor_sha256: String,
}

#[cfg(windows)]
fn require_admitted_content(
    resolver: &crate::GameFileResolver,
    admission: &ValidatedContentAdmission,
    expected: &UsedContentKey,
) -> Result<()> {
    expected
        .validate()
        .map_err(|error| format!("invalid XMAP12 used-content key: {error}"))?;
    let admitted = admission
        .require_used_content(&expected.normalized_game_relative_path)
        .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
    if admitted != expected {
        return Err(format!(
            "XMAP12 admitted identity changed for {}",
            expected.normalized_game_relative_path
        )
        .into());
    }
    resolver.read_admitted_used_content(expected)?;
    Ok(())
}

#[cfg(windows)]
fn xmap12_shadow_pipeline(
    family: crate::Xmap12PdxMeshMaterialFamily,
    skinned: bool,
) -> crate::L03ShadowPipeline {
    match (family, skinned) {
        (crate::Xmap12PdxMeshMaterialFamily::Standard, false) => {
            crate::L03ShadowPipeline::StandardUnskinned
        }
        (crate::Xmap12PdxMeshMaterialFamily::Standard, true) => {
            crate::L03ShadowPipeline::StandardSkinned
        }
        (crate::Xmap12PdxMeshMaterialFamily::FixedMapFurnitureBorder, false) => {
            crate::L03ShadowPipeline::FixedMapFurnitureBorder
        }
        (crate::Xmap12PdxMeshMaterialFamily::FixedMapFurnitureBorder, true) => {
            unreachable!("fixed map furniture is never skinned")
        }
    }
}

#[cfg(windows)]
fn merge_effect_samplers(
    resolver: &crate::GameFileResolver,
    shader_path: &str,
    reflected_registers: &BTreeSet<u32>,
    census: &mut BTreeMap<u32, NativeSamplerDescriptor>,
) -> Result<()> {
    let files = crate::shader_compile::effect_translation_unit_files(resolver, shader_path)?;
    let stage = crate::shader_compile::merged_effect_translation_unit_stage(
        &files,
        crate::shader_compile::ShaderStage::Pixel,
    );
    for sampler in stage.samplers {
        if !reflected_registers.contains(&sampler.index) {
            continue;
        }
        let descriptor = NativeSamplerDescriptor {
            name: sampler.name,
            shader_register: sampler.index,
            mag_filter: required_sampler_field(sampler.mag_filter, "MagFilter")?,
            min_filter: required_sampler_field(sampler.min_filter, "MinFilter")?,
            mip_filter: required_sampler_field(sampler.mip_filter, "MipFilter")?,
            address_u: required_sampler_field(sampler.address_u, "AddressU")?,
            address_v: required_sampler_field(sampler.address_v, "AddressV")?,
            sampler_type: sampler.sampler_type,
            mip_map_lod_bias: sampler.mip_map_lod_bias,
            mip_map_min_lod: sampler.mip_map_min_lod,
            mip_map_max_lod: sampler.mip_map_max_lod,
            max_anisotropy: sampler.max_anisotropy,
        };
        if census
            .insert(descriptor.shader_register, descriptor.clone())
            .is_some_and(|prior| prior != descriptor)
        {
            return Err(format!(
                "XMAP12 shader sampler s{} has conflicting effective descriptors",
                descriptor.shader_register
            )
            .into());
        }
    }
    Ok(())
}

#[cfg(windows)]
fn required_sampler_field(value: Option<String>, field: &str) -> Result<String> {
    value
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("XMAP12 effective sampler omitted {field}").into())
}

#[cfg(windows)]
unsafe fn create_effect_program(
    device: *mut winapi::um::d3d11::ID3D11Device,
    resolver: &crate::GameFileResolver,
    shader_path: &str,
    file: &crate::EffectFile,
    effect: &crate::EffectDef,
    engine_defines: &[(String, String)],
) -> Result<OwnedXmap12Program> {
    use crate::shader_compile::{
        build_stage_hlsl_with_defines, d3d_compile_stage, parse_dxbc_reflection, ShaderStage,
        ENGINE_COMPILE_FLAGS,
    };
    let vertex_hlsl = build_stage_hlsl_with_defines(
        resolver,
        shader_path,
        &effect.name,
        ShaderStage::Vertex,
        engine_defines,
    )?;
    let pixel_hlsl = build_stage_hlsl_with_defines(
        resolver,
        shader_path,
        &effect.name,
        ShaderStage::Pixel,
        engine_defines,
    )?;
    let vertex_bytes = d3d_compile_stage(&vertex_hlsl, ShaderStage::Vertex, ENGINE_COMPILE_FLAGS)?;
    let pixel_bytes = d3d_compile_stage(&pixel_hlsl, ShaderStage::Pixel, ENGINE_COMPILE_FLAGS)?;
    let vertex_reflection = parse_dxbc_reflection(&vertex_bytes)?;
    let skinned = effect.name.contains("Skinned");
    validate_static_vertex_reflection(&vertex_reflection.inputs, skinned)?;
    let pixel_reflection = parse_dxbc_reflection(&pixel_bytes)?;
    crate::xmap10_scene_cb0_reflected_layout_sha256(&vertex_bytes, &pixel_bytes)
        .map_err(|error| format!("XMAP12 main effect SceneCB0 contract: {error}"))?;
    validate_xmap12_stage_contract(
        &vertex_reflection,
        "main vertex",
        if skinned { &[0, 1, 2] } else { &[0, 1] },
        &[],
        false,
        Some(&[]),
    )?;
    require_reflected_cbuffer(&vertex_reflection, "main vertex", 0)?;
    require_reflected_cbuffer(&vertex_reflection, "main vertex", 1)?;
    if skinned {
        require_reflected_cbuffer(&vertex_reflection, "main vertex", 2)?;
    }
    validate_xmap12_stage_contract(
        &pixel_reflection,
        "main pixel",
        &[0, 1, 2],
        &[0, 1, 2],
        true,
        None,
    )?;
    let required_material_resource_registers =
        reflected_registers(&pixel_reflection, 2, |register| register <= 2)?;
    let required_shared_resources =
        reflected_named_resources(&pixel_reflection, 2, |register| register > 2)?;
    let required_shared_resource_registers = required_shared_resources
        .iter()
        .map(|resource| resource.shader_register)
        .collect::<Vec<_>>();
    let required_sampler_registers = reflected_registers(&pixel_reflection, 3, |_| true)?;
    let (blend_state, depth_state, rasterizer_state, state_sha256) =
        create_effect_states(device, file, effect)?;
    let program_sha256 = Sha256::hex(&serde_json::to_vec(&(
        shader_path,
        effect.name.as_str(),
        Sha256::hex(vertex_hlsl.as_bytes()),
        Sha256::hex(pixel_hlsl.as_bytes()),
        Sha256::hex(&vertex_bytes),
        Sha256::hex(&pixel_bytes),
        &required_material_resource_registers,
        &required_shared_resource_registers,
        &required_shared_resources,
        &required_sampler_registers,
        state_sha256,
    ))?);
    Ok(OwnedXmap12Program {
        effect_name: effect.name.clone(),
        program_sha256,
        required_material_resource_registers,
        required_shared_resource_registers,
        required_shared_resources,
        required_sampler_registers,
        vertex_shader: create_vertex_shader(device, &vertex_bytes, &effect.name)?,
        pixel_shader: create_pixel_shader(device, &pixel_bytes, &effect.name)?,
        input_layout: create_static_input_layout(device, &vertex_bytes, &effect.name, skinned)?,
        blend_state,
        depth_state,
        rasterizer_state,
    })
}

#[cfg(windows)]
unsafe fn create_shadow_program(
    device: *mut winapi::um::d3d11::ID3D11Device,
    resolver: &crate::GameFileResolver,
    shader_path: &str,
    file: &crate::EffectFile,
    effect: &crate::EffectDef,
    main_effect_name: &str,
    pipeline: crate::L03ShadowPipeline,
    engine_defines: &[(String, String)],
) -> Result<OwnedXmap12ShadowProgram> {
    use crate::shader_compile::{
        build_stage_hlsl_with_defines, d3d_compile_stage, parse_dxbc_reflection, ShaderStage,
        ENGINE_COMPILE_FLAGS,
    };
    let vertex_hlsl = build_stage_hlsl_with_defines(
        resolver,
        shader_path,
        &effect.name,
        ShaderStage::Vertex,
        engine_defines,
    )?;
    let vertex_bytes = d3d_compile_stage(&vertex_hlsl, ShaderStage::Vertex, ENGINE_COMPILE_FLAGS)?;
    let vertex_reflection = parse_dxbc_reflection(&vertex_bytes)?;
    let skinned = effect.name.contains("Skinned");
    validate_static_vertex_reflection(&vertex_reflection.inputs, skinned)?;
    crate::xmap10_scene_cb0_stage_reflected_layout_sha256(&vertex_bytes, "XMAP12 shadow vertex")
        .map_err(|error| format!("XMAP12 shadow effect SceneCB0 contract: {error}"))?;
    validate_xmap12_stage_contract(
        &vertex_reflection,
        "shadow vertex",
        if skinned { &[0, 1, 2] } else { &[0, 1] },
        &[],
        false,
        Some(&[]),
    )?;
    require_reflected_cbuffer(&vertex_reflection, "shadow vertex", 0)?;
    require_reflected_cbuffer(&vertex_reflection, "shadow vertex", 1)?;
    if skinned {
        require_reflected_cbuffer(&vertex_reflection, "shadow vertex", 2)?;
    }
    let (pixel_hlsl_sha256, pixel_bytes, pixel_shader, pixel_reflection) = if effect
        .pixel_shader
        .is_some()
    {
        let hlsl = build_stage_hlsl_with_defines(
            resolver,
            shader_path,
            &effect.name,
            ShaderStage::Pixel,
            engine_defines,
        )?;
        let bytes = d3d_compile_stage(&hlsl, ShaderStage::Pixel, ENGINE_COMPILE_FLAGS)?;
        let reflection = parse_dxbc_reflection(&bytes)?;
        validate_xmap12_stage_contract(
            &reflection,
            "shadow pixel",
            &[0, 1],
            &[0],
            false,
            Some(&[0]),
        )?;
        if reflection
            .resources
            .iter()
            .any(|resource| resource.resource_type == 0 && resource.bind_point == 0)
        {
            crate::xmap10_scene_cb0_stage_reflected_layout_sha256(&bytes, "XMAP12 shadow pixel")
                .map_err(|error| format!("XMAP12 shadow pixel SceneCB0 contract: {error}"))?;
        }
        let shader = Some(create_pixel_shader(device, &bytes, &effect.name)?);
        (
            Some(Sha256::hex(hlsl.as_bytes())),
            bytes,
            shader,
            Some(reflection),
        )
    } else {
        (None, Vec::new(), None, None)
    };
    let required_material_resource_registers = pixel_reflection
        .as_ref()
        .map(|reflection| reflected_registers(reflection, 2, |_| true))
        .transpose()?
        .unwrap_or_default();
    let required_sampler_registers = pixel_reflection
        .as_ref()
        .map(|reflection| reflected_registers(reflection, 3, |_| true))
        .transpose()?
        .unwrap_or_default();
    let (blend, depth_state, rasterizer_state, state_sha256) =
        create_effect_states(device, file, effect)?;
    drop(blend);
    let program_sha256 = Sha256::hex(&serde_json::to_vec(&(
        shader_path,
        main_effect_name,
        effect.name.as_str(),
        Sha256::hex(vertex_hlsl.as_bytes()),
        Sha256::hex(&vertex_bytes),
        pixel_hlsl_sha256,
        (!pixel_bytes.is_empty()).then(|| Sha256::hex(&pixel_bytes)),
        &required_material_resource_registers,
        &required_sampler_registers,
        state_sha256,
    ))?);
    Ok(OwnedXmap12ShadowProgram {
        pipeline,
        main_effect_name: main_effect_name.to_string(),
        program_sha256,
        required_material_resource_registers,
        required_sampler_registers,
        vertex_shader: create_vertex_shader(device, &vertex_bytes, &effect.name)?,
        pixel_shader,
        input_layout: create_static_input_layout(device, &vertex_bytes, &effect.name, skinned)?,
        depth_state,
        rasterizer_state,
    })
}

#[cfg(windows)]
fn reflected_registers(
    reflection: &crate::shader_compile::DxbcReflection,
    resource_type: u32,
    include: impl Fn(u32) -> bool,
) -> Result<Vec<u32>> {
    let mut out = BTreeSet::new();
    for resource in reflection
        .resources
        .iter()
        .filter(|resource| resource.resource_type == resource_type)
    {
        if resource.bind_count != 1 {
            return Err("XMAP12 effect uses an unsupported resource array".into());
        }
        if include(resource.bind_point) && !out.insert(resource.bind_point) {
            return Err("XMAP12 reflected resource register is duplicate".into());
        }
    }
    Ok(out.into_iter().collect())
}

#[cfg(windows)]
fn reflected_named_resources(
    reflection: &crate::shader_compile::DxbcReflection,
    resource_type: u32,
    include: impl Fn(u32) -> bool,
) -> Result<Vec<NativeXmap12ReflectedSharedResource>> {
    let mut registers = BTreeSet::new();
    let mut names = BTreeSet::new();
    let mut out = Vec::new();
    for resource in reflection
        .resources
        .iter()
        .filter(|resource| resource.resource_type == resource_type)
    {
        if resource.bind_count != 1 {
            return Err("XMAP12 effect uses an unsupported resource array".into());
        }
        if !include(resource.bind_point) {
            continue;
        }
        if resource.name.trim().is_empty()
            || !registers.insert(resource.bind_point)
            || !names.insert(resource.name.as_str())
        {
            return Err("XMAP12 reflected shared resource is blank or duplicate".into());
        }
        out.push(NativeXmap12ReflectedSharedResource {
            semantic: resource.name.clone(),
            shader_register: resource.bind_point,
            view_dimension: resource.dimension,
        });
    }
    out.sort_by_key(|resource| resource.shader_register);
    Ok(out)
}

#[cfg(windows)]
fn validate_xmap12_stage_contract(
    reflection: &crate::shader_compile::DxbcReflection,
    stage: &str,
    allowed_cbuffer_slots: &[u32],
    allowed_material_texture_slots: &[u32],
    allow_shared_textures: bool,
    allowed_sampler_slots: Option<&[u32]>,
) -> Result<()> {
    let mut resources = BTreeSet::new();
    for resource in &reflection.resources {
        if resource.bind_count != 1
            || !resources.insert((resource.resource_type, resource.bind_point))
        {
            return Err(format!(
                "XMAP12 {stage} reflection contains an array or duplicate binding"
            )
            .into());
        }
        match resource.resource_type {
            0 => {
                if !allowed_cbuffer_slots.contains(&resource.bind_point) {
                    return Err(format!(
                        "XMAP12 {stage} requires unsupported constant buffer b{}",
                        resource.bind_point
                    )
                    .into());
                }
                validate_xmap12_cbuffer_layout(reflection, resource, stage)?;
            }
            2 => {
                let is_material = allowed_material_texture_slots.contains(&resource.bind_point);
                let supported = is_material || (allow_shared_textures && resource.bind_point > 2);
                if !supported {
                    return Err(format!(
                        "XMAP12 {stage} requires unsupported texture t{}",
                        resource.bind_point
                    )
                    .into());
                }
                if is_material
                    && resource.dimension
                        != winapi::um::d3dcommon::D3D11_SRV_DIMENSION_TEXTURE2D as u32
                {
                    return Err(format!(
                        "XMAP12 {stage} material t{} is not Texture2D",
                        resource.bind_point
                    )
                    .into());
                }
            }
            3 => {
                if allowed_sampler_slots.is_some_and(|slots| !slots.contains(&resource.bind_point))
                {
                    return Err(format!(
                        "XMAP12 {stage} requires unsupported sampler s{}",
                        resource.bind_point
                    )
                    .into());
                }
            }
            other => {
                return Err(format!(
                    "XMAP12 {stage} requires unsupported D3D resource type {other}"
                )
                .into())
            }
        }
    }
    Ok(())
}

#[cfg(windows)]
fn validate_xmap12_cbuffer_layout(
    reflection: &crate::shader_compile::DxbcReflection,
    resource: &crate::shader_compile::DxbcResource,
    stage: &str,
) -> Result<()> {
    let expected_name = format!("dx11_cb{}", resource.bind_point);
    let matches = reflection
        .cbuffers
        .iter()
        .filter(|cbuffer| cbuffer.name == resource.name)
        .collect::<Vec<_>>();
    if resource.name != expected_name || matches.len() != 1 {
        return Err(format!(
            "XMAP12 {stage} b{} has no unique canonical reflected layout",
            resource.bind_point
        )
        .into());
    }
    let cbuffer = matches[0];
    let actual_rows = cbuffer
        .vars
        .iter()
        .map(|var| (var.name.as_str(), var.offset, var.size))
        .collect::<Vec<_>>();
    let valid = match resource.bind_point {
        0 => cbuffer.size == crate::XMAP10_SCENE_CB0_BYTE_LEN as u32,
        1 => {
            cbuffer.size == 96
                && actual_rows
                    == [
                        ("WorldMatrix", 0, 64),
                        ("AtlasCoordinate", 64, 16),
                        ("vUVAnimSpeed", 80, 4),
                    ]
        }
        2 => cbuffer.size == 16 && actual_rows == [("GridStart_InvCellSize", 0, 16)],
        _ => false,
    };
    if !valid {
        return Err(format!(
            "XMAP12 {stage} b{} layout differs from the supported exact payload",
            resource.bind_point
        )
        .into());
    }
    Ok(())
}

#[cfg(windows)]
fn require_reflected_cbuffer(
    reflection: &crate::shader_compile::DxbcReflection,
    stage: &str,
    slot: u32,
) -> Result<()> {
    if reflection.resources.iter().any(|resource| {
        resource.resource_type == 0 && resource.bind_point == slot && resource.bind_count == 1
    }) {
        Ok(())
    } else {
        Err(format!("XMAP12 {stage} omitted required constant buffer b{slot}").into())
    }
}

#[cfg(windows)]
fn validate_material_binding_contracts(
    frame: &PreparedXmap12RenderFrame,
    programs: &[OwnedXmap12Program],
    shadow_programs: &[OwnedXmap12ShadowProgram],
) -> Result<()> {
    let main = programs
        .iter()
        .map(|program| (program.effect_name.as_str(), program))
        .collect::<BTreeMap<_, _>>();
    let shadow = shadow_programs
        .iter()
        .map(|program| (program.main_effect_name.as_str(), program))
        .collect::<BTreeMap<_, _>>();
    for draw in &frame.ordered_draws {
        for submesh in &draw.submeshes {
            let available = [
                submesh.diffuse.is_some(),
                submesh.specular.is_some(),
                submesh.normal_map.is_some(),
            ];
            let main_program = main
                .get(submesh.material_shader.as_str())
                .ok_or("XMAP12 material contract lost its main program")?;
            if main_program
                .required_material_resource_registers
                .iter()
                .any(|slot| {
                    usize::try_from(*slot)
                        .ok()
                        .and_then(|slot| available.get(slot))
                        .copied()
                        != Some(true)
                })
            {
                return Err(format!(
                    "XMAP12 effect {} reads a material texture absent from {}",
                    submesh.material_shader, draw.stable_typed_id
                )
                .into());
            }
            let shadow_program = shadow
                .get(submesh.material_shader.as_str())
                .ok_or("XMAP12 material contract lost its shadow program")?;
            if shadow_program
                .required_material_resource_registers
                .iter()
                .any(|slot| {
                    usize::try_from(*slot)
                        .ok()
                        .and_then(|slot| available.get(slot))
                        .copied()
                        != Some(true)
                })
            {
                return Err(format!(
                    "XMAP12 shadow effect for {} reads a material texture absent from {}",
                    submesh.material_shader, draw.stable_typed_id
                )
                .into());
            }
        }
    }
    Ok(())
}

#[cfg(windows)]
fn validate_static_vertex_reflection(
    inputs: &[crate::shader_compile::DxbcSignatureParam],
    skinned: bool,
) -> Result<()> {
    let actual = inputs
        .iter()
        .filter(|row| !row.semantic_name.eq_ignore_ascii_case("SV_VertexID"))
        .map(|row| {
            (
                row.semantic_name.to_ascii_uppercase(),
                row.semantic_index,
                row.component_type,
                row.mask,
            )
        })
        .collect::<BTreeSet<_>>();
    let mut required = [
        ("POSITION".to_string(), 0, 3, 0x7),
        ("NORMAL".to_string(), 0, 3, 0x7),
        ("TANGENT".to_string(), 0, 3, 0xf),
        ("TEXCOORD".to_string(), 0, 3, 0x3),
    ]
    .into_iter()
    .collect::<BTreeSet<_>>();
    if skinned {
        required.insert(("TEXCOORD".to_string(), 4, 1, 0xf));
        required.insert(("TEXCOORD".to_string(), 5, 3, 0x7));
    }
    if actual != required {
        return Err(format!(
            "selected effect input signature is not the exact admitted PdxMesh layout: {actual:?}"
        )
        .into());
    }
    Ok(())
}

#[cfg(windows)]
unsafe fn create_vertex_shader(
    device: *mut winapi::um::d3d11::ID3D11Device,
    bytes: &[u8],
    effect: &str,
) -> Result<crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11VertexShader>> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    let mut raw = std::ptr::null_mut();
    native_d3d11_hr(
        (*device).CreateVertexShader(
            bytes.as_ptr() as *const _,
            bytes.len(),
            std::ptr::null_mut(),
            &mut raw,
        ),
        &format!("CreateVertexShader(XMAP12 {effect})"),
    )?;
    D3dPtr::from_raw(raw, "XMAP12 retained vertex shader")
}

#[cfg(windows)]
unsafe fn create_pixel_shader(
    device: *mut winapi::um::d3d11::ID3D11Device,
    bytes: &[u8],
    effect: &str,
) -> Result<crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11PixelShader>> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    let mut raw = std::ptr::null_mut();
    native_d3d11_hr(
        (*device).CreatePixelShader(
            bytes.as_ptr() as *const _,
            bytes.len(),
            std::ptr::null_mut(),
            &mut raw,
        ),
        &format!("CreatePixelShader(XMAP12 {effect})"),
    )?;
    D3dPtr::from_raw(raw, "XMAP12 retained pixel shader")
}

#[cfg(windows)]
unsafe fn create_static_input_layout(
    device: *mut winapi::um::d3d11::ID3D11Device,
    vertex_bytes: &[u8],
    effect: &str,
    skinned: bool,
) -> Result<crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11InputLayout>> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use std::ffi::CString;
    use winapi::shared::dxgiformat::{
        DXGI_FORMAT_R32G32B32A32_FLOAT, DXGI_FORMAT_R32G32B32_FLOAT, DXGI_FORMAT_R32G32_FLOAT,
    };
    use winapi::um::d3d11::{D3D11_INPUT_ELEMENT_DESC, D3D11_INPUT_PER_VERTEX_DATA};
    let position = CString::new("POSITION")?;
    let normal = CString::new("NORMAL")?;
    let tangent = CString::new("TANGENT")?;
    let texcoord = CString::new("TEXCOORD")?;
    let element = |semantic, format, offset| D3D11_INPUT_ELEMENT_DESC {
        SemanticName: semantic,
        SemanticIndex: 0,
        Format: format,
        InputSlot: 0,
        AlignedByteOffset: offset,
        InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
        InstanceDataStepRate: 0,
    };
    let mut elements = vec![
        element(position.as_ptr(), DXGI_FORMAT_R32G32B32_FLOAT, 0),
        element(normal.as_ptr(), DXGI_FORMAT_R32G32B32_FLOAT, 12),
        element(tangent.as_ptr(), DXGI_FORMAT_R32G32B32A32_FLOAT, 24),
        element(texcoord.as_ptr(), DXGI_FORMAT_R32G32_FLOAT, 40),
    ];
    if skinned {
        use winapi::shared::dxgiformat::{
            DXGI_FORMAT_R32G32B32A32_UINT, DXGI_FORMAT_R32G32B32_FLOAT,
        };
        let mut indices = element(texcoord.as_ptr(), DXGI_FORMAT_R32G32B32A32_UINT, 48);
        indices.SemanticIndex = 4;
        elements.push(indices);
        let mut weights = element(texcoord.as_ptr(), DXGI_FORMAT_R32G32B32_FLOAT, 64);
        weights.SemanticIndex = 5;
        elements.push(weights);
    }
    let mut raw = std::ptr::null_mut();
    native_d3d11_hr(
        (*device).CreateInputLayout(
            elements.as_ptr(),
            elements.len() as u32,
            vertex_bytes.as_ptr() as *const _,
            vertex_bytes.len(),
            &mut raw,
        ),
        &format!("CreateInputLayout(XMAP12 {effect})"),
    )?;
    D3dPtr::from_raw(raw, "XMAP12 retained static input layout")
}

#[cfg(windows)]
unsafe fn create_effect_states(
    device: *mut winapi::um::d3d11::ID3D11Device,
    file: &crate::EffectFile,
    effect: &crate::EffectDef,
) -> Result<(
    crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11BlendState>,
    crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11DepthStencilState>,
    crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11RasterizerState>,
    String,
)> {
    let blend = state_rows(effect.blend_state.as_deref(), &file.blend_states, "blend")?;
    let depth = state_rows(
        effect.depth_stencil_state.as_deref(),
        &file.depth_stencil_states,
        "depth-stencil",
    )?;
    let raster = state_rows(
        effect.rasterizer_state.as_deref(),
        &file.rasterizer_states,
        "rasterizer",
    )?;
    let identity = Sha256::hex(&serde_json::to_vec(&(
        effect.blend_state.as_deref(),
        &blend,
        effect.depth_stencil_state.as_deref(),
        &depth,
        effect.rasterizer_state.as_deref(),
        &raster,
    ))?);
    Ok((
        create_blend_state(device, blend)?,
        create_depth_state(device, depth)?,
        create_rasterizer_state(device, raster)?,
        identity,
    ))
}

#[cfg(windows)]
fn state_rows<'a>(
    name: Option<&str>,
    states: &'a BTreeMap<String, Vec<(String, String)>>,
    role: &str,
) -> Result<&'a [(String, String)]> {
    name.map(|name| {
        states
            .get(name)
            .map(Vec::as_slice)
            .ok_or_else(|| format!("XMAP12 effect names missing {role} state {name}").into())
    })
    .transpose()
    .map(|rows| rows.unwrap_or(&[]))
}

#[cfg(windows)]
fn yes_no(value: &str, role: &str) -> Result<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "yes" | "true" | "1" => Ok(true),
        "no" | "false" | "0" => Ok(false),
        _ => Err(format!("XMAP12 {role} is not an exact yes/no value: {value}").into()),
    }
}

#[cfg(windows)]
fn normalized_state_value(value: &str) -> String {
    value
        .trim()
        .trim_matches('"')
        .replace('-', "_")
        .to_ascii_uppercase()
}

#[cfg(windows)]
unsafe fn create_blend_state(
    device: *mut winapi::um::d3d11::ID3D11Device,
    rows: &[(String, String)],
) -> Result<crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11BlendState>> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use winapi::um::d3d11::*;
    let mut desc: D3D11_BLEND_DESC = std::mem::zeroed();
    let target = &mut desc.RenderTarget[0];
    target.BlendEnable = 0;
    target.SrcBlend = D3D11_BLEND_ONE;
    target.DestBlend = D3D11_BLEND_ZERO;
    target.BlendOp = D3D11_BLEND_OP_ADD;
    target.SrcBlendAlpha = D3D11_BLEND_ONE;
    target.DestBlendAlpha = D3D11_BLEND_ZERO;
    target.BlendOpAlpha = D3D11_BLEND_OP_ADD;
    target.RenderTargetWriteMask = D3D11_COLOR_WRITE_ENABLE_ALL as u8;
    for (key, value) in rows {
        match key.as_str() {
            "BlendEnable" => target.BlendEnable = i32::from(yes_no(value, key)?),
            "SourceBlend" | "SrcBlend" => target.SrcBlend = blend_factor(value)?,
            "DestBlend" => target.DestBlend = blend_factor(value)?,
            "BlendOp" => target.BlendOp = blend_op(value)?,
            "SourceBlendAlpha" | "SrcBlendAlpha" => target.SrcBlendAlpha = blend_factor(value)?,
            "DestBlendAlpha" => target.DestBlendAlpha = blend_factor(value)?,
            "BlendOpAlpha" => target.BlendOpAlpha = blend_op(value)?,
            "WriteMask" | "RenderTargetWriteMask" => {
                target.RenderTargetWriteMask = color_write_mask(value)?
            }
            "AlphaToCoverageEnable" => desc.AlphaToCoverageEnable = i32::from(yes_no(value, key)?),
            "IndependentBlendEnable" => {
                desc.IndependentBlendEnable = i32::from(yes_no(value, key)?)
            }
            _ => {
                return Err(
                    format!("XMAP12 blend state key {key} has no exact D3D11 translation").into(),
                )
            }
        }
    }
    if desc.IndependentBlendEnable != 0 {
        return Err("XMAP12 independent blend state requires per-target authored rows".into());
    }
    let mut raw = std::ptr::null_mut();
    native_d3d11_hr(
        (*device).CreateBlendState(&desc, &mut raw),
        "CreateBlendState(XMAP12 effect)",
    )?;
    D3dPtr::from_raw(raw, "XMAP12 retained blend state")
}

#[cfg(windows)]
fn blend_factor(value: &str) -> Result<u32> {
    use winapi::um::d3d11::*;
    Ok(match normalized_state_value(value).as_str() {
        "ZERO" => D3D11_BLEND_ZERO,
        "ONE" => D3D11_BLEND_ONE,
        "SRC_COLOR" => D3D11_BLEND_SRC_COLOR,
        "INV_SRC_COLOR" => D3D11_BLEND_INV_SRC_COLOR,
        "SRC_ALPHA" => D3D11_BLEND_SRC_ALPHA,
        "INV_SRC_ALPHA" => D3D11_BLEND_INV_SRC_ALPHA,
        "DEST_ALPHA" => D3D11_BLEND_DEST_ALPHA,
        "INV_DEST_ALPHA" => D3D11_BLEND_INV_DEST_ALPHA,
        "DEST_COLOR" => D3D11_BLEND_DEST_COLOR,
        "INV_DEST_COLOR" => D3D11_BLEND_INV_DEST_COLOR,
        "SRC_ALPHA_SAT" => D3D11_BLEND_SRC_ALPHA_SAT,
        other => {
            return Err(
                format!("XMAP12 blend factor {other} has no exact D3D11 translation").into(),
            )
        }
    })
}

#[cfg(windows)]
fn blend_op(value: &str) -> Result<u32> {
    use winapi::um::d3d11::*;
    Ok(match normalized_state_value(value).as_str() {
        "ADD" => D3D11_BLEND_OP_ADD,
        "SUBTRACT" => D3D11_BLEND_OP_SUBTRACT,
        "REV_SUBTRACT" => D3D11_BLEND_OP_REV_SUBTRACT,
        "MIN" => D3D11_BLEND_OP_MIN,
        "MAX" => D3D11_BLEND_OP_MAX,
        other => {
            return Err(
                format!("XMAP12 blend operation {other} has no exact D3D11 translation").into(),
            )
        }
    })
}

#[cfg(windows)]
fn color_write_mask(value: &str) -> Result<u8> {
    let normalized = normalized_state_value(value);
    if normalized == "ALL" {
        return Ok(0xf);
    }
    if normalized == "NONE" || normalized == "0" {
        return Ok(0);
    }
    let mut mask = 0u8;
    for lane in normalized.split('|').map(str::trim) {
        mask |= match lane {
            "RED" | "R" => 0x1,
            "GREEN" | "G" => 0x2,
            "BLUE" | "B" => 0x4,
            "ALPHA" | "A" => 0x8,
            _ => return Err(format!("XMAP12 color write lane {lane} is unsupported").into()),
        };
    }
    Ok(mask)
}

#[cfg(windows)]
unsafe fn create_depth_state(
    device: *mut winapi::um::d3d11::ID3D11Device,
    rows: &[(String, String)],
) -> Result<crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11DepthStencilState>> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use winapi::um::d3d11::*;
    let stencil = D3D11_DEPTH_STENCILOP_DESC {
        StencilFailOp: D3D11_STENCIL_OP_KEEP,
        StencilDepthFailOp: D3D11_STENCIL_OP_KEEP,
        StencilPassOp: D3D11_STENCIL_OP_KEEP,
        StencilFunc: D3D11_COMPARISON_ALWAYS,
    };
    let mut desc = D3D11_DEPTH_STENCIL_DESC {
        DepthEnable: 1,
        DepthWriteMask: D3D11_DEPTH_WRITE_MASK_ALL,
        DepthFunc: D3D11_COMPARISON_LESS,
        StencilEnable: 0,
        StencilReadMask: u8::MAX,
        StencilWriteMask: u8::MAX,
        FrontFace: stencil,
        BackFace: stencil,
    };
    for (key, value) in rows {
        match key.as_str() {
            "DepthEnable" => desc.DepthEnable = i32::from(yes_no(value, key)?),
            "DepthWriteEnable" | "DepthWrite" => {
                desc.DepthWriteMask = if yes_no(value, key)? {
                    D3D11_DEPTH_WRITE_MASK_ALL
                } else {
                    D3D11_DEPTH_WRITE_MASK_ZERO
                }
            }
            "DepthWriteMask" => {
                desc.DepthWriteMask = match normalized_state_value(value).as_str() {
                    "ALL" => D3D11_DEPTH_WRITE_MASK_ALL,
                    "ZERO" | "NONE" => D3D11_DEPTH_WRITE_MASK_ZERO,
                    other => {
                        return Err(format!("XMAP12 depth write mask {other} is unsupported").into())
                    }
                }
            }
            "DepthFunc" | "DepthFunction" => desc.DepthFunc = comparison(value)?,
            "StencilEnable" => desc.StencilEnable = i32::from(yes_no(value, key)?),
            _ => {
                return Err(
                    format!("XMAP12 depth state key {key} has no exact D3D11 translation").into(),
                )
            }
        }
    }
    if desc.StencilEnable != 0 {
        return Err(
            "XMAP12 stencil-enabled effect state is not represented by its parsed rows".into(),
        );
    }
    let mut raw = std::ptr::null_mut();
    native_d3d11_hr(
        (*device).CreateDepthStencilState(&desc, &mut raw),
        "CreateDepthStencilState(XMAP12 effect)",
    )?;
    D3dPtr::from_raw(raw, "XMAP12 retained depth state")
}

#[cfg(windows)]
fn comparison(value: &str) -> Result<u32> {
    use winapi::um::d3d11::*;
    Ok(match normalized_state_value(value).as_str() {
        "NEVER" => D3D11_COMPARISON_NEVER,
        "LESS" => D3D11_COMPARISON_LESS,
        "EQUAL" => D3D11_COMPARISON_EQUAL,
        "LESS_EQUAL" => D3D11_COMPARISON_LESS_EQUAL,
        "GREATER" => D3D11_COMPARISON_GREATER,
        "NOT_EQUAL" => D3D11_COMPARISON_NOT_EQUAL,
        "GREATER_EQUAL" => D3D11_COMPARISON_GREATER_EQUAL,
        "ALWAYS" => D3D11_COMPARISON_ALWAYS,
        other => return Err(format!("XMAP12 comparison {other} is unsupported").into()),
    })
}

#[cfg(windows)]
unsafe fn create_rasterizer_state(
    device: *mut winapi::um::d3d11::ID3D11Device,
    rows: &[(String, String)],
) -> Result<crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11RasterizerState>> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use winapi::um::d3d11::*;
    let mut desc = D3D11_RASTERIZER_DESC {
        FillMode: D3D11_FILL_SOLID,
        CullMode: D3D11_CULL_BACK,
        FrontCounterClockwise: 0,
        DepthBias: 0,
        DepthBiasClamp: 0.0,
        SlopeScaledDepthBias: 0.0,
        DepthClipEnable: 1,
        ScissorEnable: 0,
        MultisampleEnable: 0,
        AntialiasedLineEnable: 0,
    };
    for (key, value) in rows {
        match key.as_str() {
            "FillMode" => {
                desc.FillMode = match normalized_state_value(value).as_str() {
                    "SOLID" => D3D11_FILL_SOLID,
                    "WIREFRAME" => D3D11_FILL_WIREFRAME,
                    other => return Err(format!("XMAP12 fill mode {other} is unsupported").into()),
                }
            }
            "CullMode" => {
                desc.CullMode = match normalized_state_value(value).as_str() {
                    "NONE" => D3D11_CULL_NONE,
                    "FRONT" => D3D11_CULL_FRONT,
                    "BACK" => D3D11_CULL_BACK,
                    other => return Err(format!("XMAP12 cull mode {other} is unsupported").into()),
                }
            }
            "FrontCounterClockwise" => desc.FrontCounterClockwise = i32::from(yes_no(value, key)?),
            "DepthBias" => desc.DepthBias = value.trim().parse()?,
            "DepthBiasClamp" => desc.DepthBiasClamp = value.trim().parse()?,
            "SlopeScaledDepthBias" => desc.SlopeScaledDepthBias = value.trim().parse()?,
            "DepthClipEnable" => desc.DepthClipEnable = i32::from(yes_no(value, key)?),
            "ScissorEnable" => desc.ScissorEnable = i32::from(yes_no(value, key)?),
            "MultisampleEnable" => desc.MultisampleEnable = i32::from(yes_no(value, key)?),
            "AntialiasedLineEnable" => desc.AntialiasedLineEnable = i32::from(yes_no(value, key)?),
            _ => {
                return Err(
                    format!("XMAP12 rasterizer key {key} has no exact D3D11 translation").into(),
                )
            }
        }
    }
    let mut raw = std::ptr::null_mut();
    native_d3d11_hr(
        (*device).CreateRasterizerState(&desc, &mut raw),
        "CreateRasterizerState(XMAP12 effect)",
    )?;
    D3dPtr::from_raw(raw, "XMAP12 retained rasterizer state")
}

#[cfg(windows)]
unsafe fn create_native_sampler(
    device: *mut winapi::um::d3d11::ID3D11Device,
    descriptor: NativeSamplerDescriptor,
) -> Result<OwnedXmap12Sampler> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use winapi::um::d3d11::*;
    let comparison_sampler = descriptor.sampler_type.as_deref() == Some("Shadow");
    let filter = native_sampler_filter(&descriptor)?;
    let address = |value: &str| -> Result<u32> {
        Ok(match normalized_state_value(value).as_str() {
            "WRAP" => D3D11_TEXTURE_ADDRESS_WRAP,
            "MIRROR" => D3D11_TEXTURE_ADDRESS_MIRROR,
            "CLAMP" => D3D11_TEXTURE_ADDRESS_CLAMP,
            "BORDER" => D3D11_TEXTURE_ADDRESS_BORDER,
            "MIRROR_ONCE" => D3D11_TEXTURE_ADDRESS_MIRROR_ONCE,
            other => return Err(format!("XMAP12 sampler address {other} is unsupported").into()),
        })
    };
    let parse_float = |value: Option<&String>, default: f32| -> Result<f32> {
        match value.map(|value| normalized_state_value(value)) {
            None => Ok(default),
            Some(value) if value == "MAX_FLOAT" || value == "FLT_MAX" => Ok(f32::MAX),
            Some(value) => Ok(value.parse()?),
        }
    };
    let max_anisotropy = descriptor
        .max_anisotropy
        .as_deref()
        .map(str::parse)
        .transpose()?
        .unwrap_or(1u32);
    if !(1..=16).contains(&max_anisotropy) {
        return Err("XMAP12 sampler MaxAnisotropy is outside D3D11 limits".into());
    }
    let native = D3D11_SAMPLER_DESC {
        Filter: filter,
        AddressU: address(&descriptor.address_u)?,
        AddressV: address(&descriptor.address_v)?,
        AddressW: address(&descriptor.address_u)?,
        MipLODBias: parse_float(descriptor.mip_map_lod_bias.as_ref(), 0.0)?,
        MaxAnisotropy: max_anisotropy,
        ComparisonFunc: if comparison_sampler {
            D3D11_COMPARISON_LESS_EQUAL
        } else {
            D3D11_COMPARISON_NEVER
        },
        BorderColor: [0.0; 4],
        MinLOD: parse_float(descriptor.mip_map_min_lod.as_ref(), 0.0)?,
        MaxLOD: parse_float(descriptor.mip_map_max_lod.as_ref(), f32::MAX)?,
    };
    let descriptor_sha256 = Sha256::hex(&serde_json::to_vec(&(
        descriptor.name.as_str(),
        descriptor.shader_register,
        descriptor.mag_filter.as_str(),
        descriptor.min_filter.as_str(),
        descriptor.mip_filter.as_str(),
        descriptor.address_u.as_str(),
        descriptor.address_v.as_str(),
        descriptor.sampler_type.as_deref(),
        descriptor.mip_map_lod_bias.as_deref(),
        descriptor.mip_map_min_lod.as_deref(),
        descriptor.mip_map_max_lod.as_deref(),
        descriptor.max_anisotropy.as_deref(),
    ))?);
    let mut raw = std::ptr::null_mut();
    native_d3d11_hr(
        (*device).CreateSamplerState(&native, &mut raw),
        &format!(
            "CreateSamplerState(XMAP12 {} s{})",
            descriptor.name, descriptor.shader_register
        ),
    )?;
    Ok(OwnedXmap12Sampler {
        shader_register: descriptor.shader_register,
        descriptor_sha256,
        sampler: D3dPtr::from_raw(raw, "XMAP12 retained sampler")?,
    })
}

#[cfg(windows)]
fn native_sampler_filter(descriptor: &NativeSamplerDescriptor) -> Result<u32> {
    use winapi::um::d3d11::*;
    let comparison_sampler = descriptor.sampler_type.as_deref() == Some("Shadow");
    let filter_tuple = (
        normalized_state_value(&descriptor.min_filter),
        normalized_state_value(&descriptor.mag_filter),
        normalized_state_value(&descriptor.mip_filter),
    );
    Ok(
        match (
            comparison_sampler,
            filter_tuple.0.as_str(),
            filter_tuple.1.as_str(),
            filter_tuple.2.as_str(),
        ) {
            (false, "POINT", "POINT", "POINT") => D3D11_FILTER_MIN_MAG_MIP_POINT,
            (false, "POINT", "POINT", "NONE") => D3D11_FILTER_MIN_MAG_MIP_POINT,
            (false, "LINEAR", "LINEAR", "POINT") => D3D11_FILTER_MIN_MAG_LINEAR_MIP_POINT,
            (false, "LINEAR", "LINEAR", "NONE") => D3D11_FILTER_MIN_MAG_LINEAR_MIP_POINT,
            (false, "LINEAR", "LINEAR", "LINEAR") => D3D11_FILTER_MIN_MAG_MIP_LINEAR,
            (true, "POINT", "POINT", "POINT") => D3D11_FILTER_COMPARISON_MIN_MAG_MIP_POINT,
            (true, "POINT", "POINT", "NONE") => D3D11_FILTER_COMPARISON_MIN_MAG_MIP_POINT,
            (true, "LINEAR", "LINEAR", "NONE") => D3D11_FILTER_COMPARISON_MIN_MAG_LINEAR_MIP_POINT,
            (true, "LINEAR", "LINEAR", "LINEAR") => D3D11_FILTER_COMPARISON_MIN_MAG_MIP_LINEAR,
            (_, "ANISOTROPIC", "ANISOTROPIC", "ANISOTROPIC") => {
                if comparison_sampler {
                    D3D11_FILTER_COMPARISON_ANISOTROPIC
                } else {
                    D3D11_FILTER_ANISOTROPIC
                }
            }
            _ => {
                return Err(format!(
                    "XMAP12 sampler {} has an unproven mixed filter {:?}",
                    descriptor.name, filter_tuple
                )
                .into())
            }
        },
    )
}

#[cfg(windows)]
fn parse_material_dds_descriptor(
    content: &UsedContentKey,
    bytes: &[u8],
) -> Result<Xmap12DdsDescriptor> {
    if bytes.len() < 128 || bytes.get(..4) != Some(b"DDS ") {
        return Err(format!(
            "XMAP12 material texture {} is not a complete DDS",
            content.normalized_game_relative_path
        )
        .into());
    }
    if Sha256::hex(bytes) != content.payload_sha256 {
        return Err(format!(
            "XMAP12 DDS {} differs from admitted payload",
            content.normalized_game_relative_path
        )
        .into());
    }
    let read_u32 = |offset: usize| -> Result<u32> {
        Ok(u32::from_le_bytes(
            bytes
                .get(offset..offset + 4)
                .ok_or("XMAP12 DDS header is truncated")?
                .try_into()?,
        ))
    };
    if read_u32(4)? != 124 || read_u32(76)? != 32 {
        return Err("XMAP12 DDS header sizes are noncanonical".into());
    }
    let width = read_u32(16)?;
    let height = read_u32(12)?;
    let mip_levels = read_u32(28)?.max(1);
    if width == 0 || height == 0 || read_u32(24)?.max(1) != 1 {
        return Err("XMAP12 material DDS is not a nonempty Texture2D".into());
    }
    let max_mips = u32::BITS - width.max(height).leading_zeros();
    if mip_levels > max_mips {
        return Err("XMAP12 material DDS has too many mip levels".into());
    }
    let pixel_flags = read_u32(80)?;
    let fourcc = bytes
        .get(84..88)
        .ok_or("XMAP12 DDS pixel format is truncated")?;
    let caps2 = read_u32(112)?;
    let (dxgi_format, block_bytes, bytes_per_pixel, payload_offset) = if fourcc == b"DX10" {
        if bytes.len() < 148
            || read_u32(132)? != 3
            || read_u32(140)? != 1
            || read_u32(136)? & 0x4 != 0
        {
            return Err("XMAP12 DDS DX10 header is not one non-cube Texture2D".into());
        }
        let format = read_u32(128)?;
        let (block, pixel) = dds_pitch_model(format)?;
        (format, block, pixel, 148usize)
    } else if pixel_flags & 0x4 != 0 {
        if caps2 & 0x0000_fe00 != 0 {
            return Err("XMAP12 legacy DDS is a cube or volume texture".into());
        }
        match fourcc {
            b"DXT1" => (71, 8, 0, 128),
            b"DXT3" => (74, 16, 0, 128),
            b"DXT5" => (77, 16, 0, 128),
            b"ATI1" | b"BC4U" => (80, 8, 0, 128),
            b"BC4S" => (81, 8, 0, 128),
            b"ATI2" | b"BC5U" => (83, 16, 0, 128),
            b"BC5S" => (84, 16, 0, 128),
            _ => {
                return Err(format!(
                    "XMAP12 DDS has unsupported legacy fourcc {:?}",
                    String::from_utf8_lossy(fourcc)
                )
                .into())
            }
        }
    } else {
        if caps2 & 0x0000_fe00 != 0 {
            return Err("XMAP12 legacy DDS is a cube or volume texture".into());
        }
        let bits = read_u32(88)?;
        let masks = (read_u32(92)?, read_u32(96)?, read_u32(100)?, read_u32(104)?);
        match (bits, masks) {
            (32, (0x000000ff, 0x0000ff00, 0x00ff0000, 0xff000000)) => (28, 0, 4, 128),
            (32, (0x00ff0000, 0x0000ff00, 0x000000ff, 0xff000000)) => (87, 0, 4, 128),
            (8, (0x000000ff, 0, 0, 0)) => (61, 0, 1, 128),
            _ => {
                return Err(
                    format!("XMAP12 DDS has unsupported RGB layout {bits}/{masks:08x?}").into(),
                )
            }
        }
    };
    let mut source_offset = payload_offset;
    let mut mips = Vec::with_capacity(mip_levels as usize);
    for mip in 0..mip_levels {
        let mip_width = (width >> mip).max(1);
        let mip_height = (height >> mip).max(1);
        let (row_pitch, slice_pitch) = if block_bytes != 0 {
            let row = mip_width
                .div_ceil(4)
                .max(1)
                .checked_mul(block_bytes)
                .ok_or("XMAP12 DDS row pitch overflow")?;
            let slice = row
                .checked_mul(mip_height.div_ceil(4).max(1))
                .ok_or("XMAP12 DDS slice pitch overflow")?;
            (row, slice)
        } else {
            let row = mip_width
                .checked_mul(bytes_per_pixel)
                .ok_or("XMAP12 DDS row pitch overflow")?;
            let slice = row
                .checked_mul(mip_height)
                .ok_or("XMAP12 DDS slice pitch overflow")?;
            (row, slice)
        };
        let end = source_offset
            .checked_add(slice_pitch as usize)
            .ok_or("XMAP12 DDS payload range overflow")?;
        bytes
            .get(source_offset..end)
            .ok_or_else(|| format!("XMAP12 DDS mip {mip} is truncated"))?;
        mips.push(Xmap12DdsMipDescriptor {
            source_offset: source_offset as u64,
            row_pitch,
            slice_pitch,
        });
        source_offset = end;
    }
    let mut descriptor = Xmap12DdsDescriptor {
        width,
        height,
        mip_levels,
        dxgi_format,
        payload_sha256: content.payload_sha256.clone(),
        mips,
        descriptor_sha256: String::new(),
    };
    descriptor.descriptor_sha256 = Sha256::hex(&serde_json::to_vec(&(
        "hoi4.replay.xmap12.material-dds.a729.v1",
        content,
        descriptor.width,
        descriptor.height,
        descriptor.mip_levels,
        descriptor.dxgi_format,
        &descriptor.payload_sha256,
        &descriptor.mips,
    ))?);
    Ok(descriptor)
}

#[cfg(windows)]
fn dds_pitch_model(format: u32) -> Result<(u32, u32)> {
    Ok(match format {
        71 | 72 | 80 | 81 => (8, 0),
        74 | 75 | 77 | 78 | 83 | 84 | 95 | 96 | 98 | 99 => (16, 0),
        28 | 29 | 87 | 91 => (0, 4),
        49 => (0, 2),
        61 => (0, 1),
        // Typeless formats need an authored SRV format that the XMAP12
        // content contract does not carry. Reject rather than choose one.
        27 | 70 | 73 | 76 | 79 | 82 | 94 | 97 | 90 => {
            return Err(format!(
                "XMAP12 DDS uses typeless DXGI format {format} without an admitted SRV descriptor"
            )
            .into())
        }
        other => return Err(format!("XMAP12 DDS DXGI format {other} is unsupported").into()),
    })
}

#[cfg(windows)]
unsafe fn create_material_dds(
    device: *mut winapi::um::d3d11::ID3D11Device,
    bytes: &[u8],
    descriptor: &Xmap12DdsDescriptor,
) -> Result<(
    crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Texture2D>,
    crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11ShaderResourceView>,
)> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use winapi::shared::dxgitype::DXGI_SAMPLE_DESC;
    use winapi::um::d3d11::*;
    let initial = descriptor
        .mips
        .iter()
        .map(|mip| {
            let offset = usize::try_from(mip.source_offset)?;
            let end = offset
                .checked_add(mip.slice_pitch as usize)
                .ok_or("XMAP12 DDS native range overflow")?;
            let payload = bytes
                .get(offset..end)
                .ok_or("XMAP12 DDS native mip is truncated")?;
            Ok(D3D11_SUBRESOURCE_DATA {
                pSysMem: payload.as_ptr() as *const _,
                SysMemPitch: mip.row_pitch,
                SysMemSlicePitch: mip.slice_pitch,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let native = D3D11_TEXTURE2D_DESC {
        Width: descriptor.width,
        Height: descriptor.height,
        MipLevels: descriptor.mip_levels,
        ArraySize: 1,
        Format: descriptor.dxgi_format as _,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_IMMUTABLE,
        BindFlags: D3D11_BIND_SHADER_RESOURCE,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let mut texture_raw = std::ptr::null_mut();
    native_d3d11_hr(
        (*device).CreateTexture2D(&native, initial.as_ptr(), &mut texture_raw),
        "CreateTexture2D(XMAP12 admitted material DDS)",
    )?;
    let texture = D3dPtr::from_raw(texture_raw, "XMAP12 retained material DDS")?;
    let mut view_raw = std::ptr::null_mut();
    native_d3d11_hr(
        (*device).CreateShaderResourceView(
            texture.as_raw() as *mut _,
            std::ptr::null(),
            &mut view_raw,
        ),
        "CreateShaderResourceView(XMAP12 admitted material DDS)",
    )?;
    Ok((
        texture,
        D3dPtr::from_raw(view_raw, "XMAP12 retained material DDS SRV")?,
    ))
}

#[cfg(windows)]
unsafe fn create_environment_cube(
    device: *mut winapi::um::d3d11::ID3D11Device,
    content: &UsedContentKey,
    bytes: &[u8],
) -> Result<(
    crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Texture2D>,
    crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11ShaderResourceView>,
    String,
)> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use winapi::shared::dxgiformat::DXGI_FORMAT_B8G8R8A8_UNORM_SRGB;
    use winapi::shared::dxgitype::DXGI_SAMPLE_DESC;
    use winapi::um::d3d11::*;

    if Sha256::hex(bytes) != content.payload_sha256 {
        return Err(format!(
            "XMAP12 environment cube {} differs from admitted payload",
            content.normalized_game_relative_path
        )
        .into());
    }
    let cube = crate::reflection_cube_law::parse_reflection_dds(
        bytes,
        &content.normalized_game_relative_path,
    )?;
    let width = u32::try_from(cube.width)?;
    let height = u32::try_from(cube.height)?;
    let mip_levels = u32::try_from(cube.mips)?;
    let array_size = u32::try_from(cube.faces)?;
    let mut initial = Vec::with_capacity(cube.faces * cube.mips);
    let mut source_offset = 0usize;
    for _face in 0..cube.faces {
        for mip in 0..cube.mips {
            let mip_width = (cube.width.checked_shr(mip as u32).unwrap_or(0)).max(1);
            let mip_height = (cube.height.checked_shr(mip as u32).unwrap_or(0)).max(1);
            let row_pitch = mip_width
                .checked_mul(4)
                .ok_or("XMAP12 environment cube row pitch overflow")?;
            let slice_pitch = row_pitch
                .checked_mul(mip_height)
                .ok_or("XMAP12 environment cube slice pitch overflow")?;
            let end = source_offset
                .checked_add(slice_pitch)
                .ok_or("XMAP12 environment cube payload range overflow")?;
            let payload = cube
                .payload
                .get(source_offset..end)
                .ok_or("XMAP12 environment cube mip is truncated")?;
            initial.push(D3D11_SUBRESOURCE_DATA {
                pSysMem: payload.as_ptr() as *const _,
                SysMemPitch: u32::try_from(row_pitch)?,
                SysMemSlicePitch: u32::try_from(slice_pitch)?,
            });
            source_offset = end;
        }
    }
    if source_offset != cube.payload.len() {
        return Err("XMAP12 environment cube has trailing payload bytes".into());
    }
    let descriptor_sha256 = Sha256::hex(&serde_json::to_vec(&(
        "hoi4.replay.xmap12.environment-cube.a729.v1",
        content,
        width,
        height,
        mip_levels,
        array_size,
        DXGI_FORMAT_B8G8R8A8_UNORM_SRGB,
    ))?);
    let native = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: mip_levels,
        ArraySize: array_size,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM_SRGB,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_IMMUTABLE,
        BindFlags: D3D11_BIND_SHADER_RESOURCE,
        CPUAccessFlags: 0,
        MiscFlags: D3D11_RESOURCE_MISC_TEXTURECUBE,
    };
    let mut texture_raw = std::ptr::null_mut();
    native_d3d11_hr(
        (*device).CreateTexture2D(&native, initial.as_ptr(), &mut texture_raw),
        "CreateTexture2D(XMAP12 admitted environment cube)",
    )?;
    let texture = D3dPtr::from_raw(texture_raw, "XMAP12 retained environment cube")?;
    let mut view_raw = std::ptr::null_mut();
    native_d3d11_hr(
        (*device).CreateShaderResourceView(
            texture.as_raw() as *mut _,
            std::ptr::null(),
            &mut view_raw,
        ),
        "CreateShaderResourceView(XMAP12 admitted environment cube)",
    )?;
    Ok((
        texture,
        D3dPtr::from_raw(view_raw, "XMAP12 retained environment cube SRV")?,
        descriptor_sha256,
    ))
}

/// Materialize an admission-bound 2D DDS for another map-object family on the
/// same device. The returned descriptor identity is derived from the exact
/// uploaded descriptor; callers retain both COM objects.
#[cfg(windows)]
pub(crate) unsafe fn create_admitted_material_dds_on_device(
    device: *mut winapi::um::d3d11::ID3D11Device,
    resolver: &crate::GameFileResolver,
    admission: &ValidatedContentAdmission,
    content: &UsedContentKey,
) -> Result<(
    crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Texture2D>,
    crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11ShaderResourceView>,
    String,
)> {
    require_admitted_content(resolver, admission, content)?;
    let bytes = resolver.read_admitted_used_content(content)?;
    let descriptor = parse_material_dds_descriptor(content, &bytes)?;
    let descriptor_sha256 = descriptor.descriptor_sha256.clone();
    let (texture, view) = create_material_dds(device, &bytes, &descriptor)?;
    Ok((texture, view, descriptor_sha256))
}

/// Materialize the exact admitted reflection cube for another family without
/// routing through an unrelated packet's optional resource owner.
#[cfg(windows)]
pub(crate) unsafe fn create_admitted_environment_cube_on_device(
    device: *mut winapi::um::d3d11::ID3D11Device,
    resolver: &crate::GameFileResolver,
    admission: &ValidatedContentAdmission,
    content: &UsedContentKey,
) -> Result<(
    crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Texture2D>,
    crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11ShaderResourceView>,
    String,
)> {
    require_admitted_content(resolver, admission, content)?;
    let bytes = resolver.read_admitted_used_content(content)?;
    create_environment_cube(device, content, &bytes)
}

#[cfg(windows)]
unsafe fn create_object_cb1(
    device: *mut winapi::um::d3d11::ID3D11Device,
    draw: &Xmap12PdxMeshMaterialization,
    partition_mask: u8,
    map_width: u32,
) -> Result<crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Buffer>> {
    // Exact retained PdxMesh CB1 layout is 24 f32 lanes (96 bytes): the
    // row-vector WorldMatrix followed by the engine-default CMeshUserData
    // AtlasCoordinate=(0,0,1,1), vUVAnimSpeed=1, and three padding lanes.
    // The retained O04 city-submit event 171 binds this same payload to VS/PS.
    let bytes = xmap12_object_cb1_payload(draw.world_matrix_f32_bits, partition_mask, map_width)?;
    create_immutable_buffer(
        device,
        &bytes,
        winapi::um::d3d11::D3D11_BIND_CONSTANT_BUFFER,
        "XMAP12 object CB1",
    )
}

#[cfg(windows)]
fn xmap12_object_cb1_payload(
    matrix: [u32; 16],
    partition_mask: u8,
    map_width: u32,
) -> Result<[u8; 96]> {
    if map_width == 0 {
        return Err("XMAP12 object CB1 received a zero map width".into());
    }
    let x_offset = match partition_mask {
        1 => 0.0,
        // Exact XMAP04 map-copy law: west-view objects are translated into the
        // eastern copy and east-view objects into the western copy.
        2 => map_width as f32,
        4 => -(map_width as f32),
        _ => return Err("XMAP12 object CB1 received an invalid partition".into()),
    };
    let mut matrix = matrix;
    let translated_x = f32::from_bits(matrix[12]) + x_offset;
    if !translated_x.is_finite() {
        return Err("XMAP12 partition object translation is non-finite".into());
    }
    matrix[12] = translated_x.to_bits();
    let mut bytes = [0u8; 96];
    for (lane, bits) in matrix.iter().enumerate() {
        bytes[lane * 4..lane * 4 + 4].copy_from_slice(&bits.to_le_bytes());
    }
    for (lane, value) in [0.0f32, 0.0, 1.0, 1.0, 1.0].into_iter().enumerate() {
        let offset = 64 + lane * 4;
        bytes[offset..offset + 4].copy_from_slice(&value.to_bits().to_le_bytes());
    }
    Ok(bytes)
}

#[cfg(windows)]
unsafe fn create_immutable_buffer(
    device: *mut winapi::um::d3d11::ID3D11Device,
    bytes: &[u8],
    bind_flags: u32,
    role: &str,
) -> Result<crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Buffer>> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use winapi::um::d3d11::{D3D11_BUFFER_DESC, D3D11_SUBRESOURCE_DATA, D3D11_USAGE_IMMUTABLE};
    if bytes.is_empty() || bytes.len() > u32::MAX as usize {
        return Err(format!("{role} payload size is invalid").into());
    }
    let desc = D3D11_BUFFER_DESC {
        ByteWidth: bytes.len() as u32,
        Usage: D3D11_USAGE_IMMUTABLE,
        BindFlags: bind_flags,
        CPUAccessFlags: 0,
        MiscFlags: 0,
        StructureByteStride: 0,
    };
    let initial = D3D11_SUBRESOURCE_DATA {
        pSysMem: bytes.as_ptr() as *const _,
        SysMemPitch: 0,
        SysMemSlicePitch: 0,
    };
    let mut raw = std::ptr::null_mut();
    native_d3d11_hr((*device).CreateBuffer(&desc, &initial, &mut raw), role)?;
    D3dPtr::from_raw(raw, role)
}

#[cfg(windows)]
unsafe fn bind_program(
    context: *mut winapi::um::d3d11::ID3D11DeviceContext,
    program: &NativeXmap12ProgramBinding,
) {
    let blend_factor = [0.0f32; 4];
    (*context).IASetInputLayout(program.input_layout);
    (*context).VSSetShader(program.vertex_shader, std::ptr::null(), 0);
    (*context).PSSetShader(program.pixel_shader, std::ptr::null(), 0);
    (*context).OMSetBlendState(program.blend_state, &blend_factor, u32::MAX);
    (*context).OMSetDepthStencilState(program.depth_state, 0);
    (*context).RSSetState(program.rasterizer_state);
}

#[cfg(windows)]
unsafe fn bind_samplers(
    context: *mut winapi::um::d3d11::ID3D11DeviceContext,
    bindings: &NativeXmap12MainSceneBindings,
) {
    for sampler in &bindings.samplers {
        (*context).PSSetSamplers(sampler.shader_register, 1, &sampler.sampler);
    }
}

#[cfg(windows)]
unsafe fn bind_program_shared_resources(
    context: *mut winapi::um::d3d11::ID3D11DeviceContext,
    program: &NativeXmap12ProgramBinding,
    bindings: &NativeXmap12MainSceneBindings,
) -> Result<()> {
    for required in &program.required_shared_resources {
        let resource = bindings
            .shared_resources
            .iter()
            .find(|resource| {
                resource.shader_register == required.shader_register
                    && resource.semantic == required.semantic
                    && resource.view_dimension == required.view_dimension
            })
            .ok_or("XMAP12 reflected shared resource disappeared before draw")?;
        (*context).PSSetShaderResources(resource.shader_register, 1, &resource.view);
    }
    Ok(())
}

#[cfg(windows)]
unsafe fn bind_material_textures(
    context: *mut winapi::um::d3d11::ID3D11DeviceContext,
    draw: &Xmap12PdxMeshMaterialization,
    submesh: &crate::Xmap12MaterializedSubmesh,
    bindings: &BTreeMap<(u32, &str), &NativeXmap12TextureBinding>,
) -> Result<()> {
    // Material registers are draw-local. Explicitly clear all three before
    // binding the authored set so absent textures cannot inherit the prior
    // submesh's SRVs or arbitrary caller state.
    let cleared = [std::ptr::null_mut(); 3];
    (*context).PSSetShaderResources(0, cleared.len() as u32, cleared.as_ptr());
    for (slot_name, register, authored) in [
        ("diffuse", 0u32, submesh.diffuse.as_deref()),
        ("specular", 1u32, submesh.specular.as_deref()),
        ("normal", 2u32, submesh.normal_map.as_deref()),
    ] {
        let Some(authored) = authored else {
            continue;
        };
        let source = material_texture_source(draw, slot_name, authored)?;
        let native = bindings
            .get(&(
                register,
                source.content.normalized_game_relative_path.as_str(),
            ))
            .ok_or("XMAP12 native material texture has no admitted GPU view")?;
        (*context).PSSetShaderResources(register, 1, &native.view);
    }
    Ok(())
}

#[cfg(windows)]
unsafe fn bind_shadow_material_resources(
    context: *mut winapi::um::d3d11::ID3D11DeviceContext,
    owned: &NativeXmap12ResourceOwner,
    draw: &Xmap12PdxMeshMaterialization,
    submesh: &crate::Xmap12MaterializedSubmesh,
    program: &NativeXmap12ShadowProgramBinding,
) -> Result<()> {
    clear_shadow_material_bindings(context);
    for register in &program.required_material_resource_registers {
        let (slot_name, authored) = match *register {
            0 => ("diffuse", submesh.diffuse.as_deref()),
            _ => return Err("XMAP12 shadow requested an unsupported material register".into()),
        };
        let authored = authored.ok_or("XMAP12 shadow material texture disappeared")?;
        let source = material_texture_source(draw, slot_name, authored)?;
        let native = owned
            .textures
            .iter()
            .find(|texture| {
                texture.shader_register == *register && texture.content == source.content
            })
            .ok_or("XMAP12 shadow material texture has no admitted GPU view")?;
        (*context).PSSetShaderResources(*register, 1, &native.view.as_raw());
    }
    for register in &program.required_sampler_registers {
        let sampler = owned
            .samplers
            .iter()
            .find(|sampler| sampler.shader_register == *register)
            .ok_or("XMAP12 shadow reflected sampler disappeared")?;
        (*context).PSSetSamplers(*register, 1, &sampler.sampler.as_raw());
    }
    Ok(())
}

#[cfg(windows)]
unsafe fn clear_shadow_material_bindings(context: *mut winapi::um::d3d11::ID3D11DeviceContext) {
    let null_srv = [std::ptr::null_mut(); 1];
    let null_sampler = [std::ptr::null_mut(); 1];
    (*context).PSSetShaderResources(0, 1, null_srv.as_ptr());
    (*context).PSSetSamplers(0, 1, null_sampler.as_ptr());
}

#[cfg(windows)]
fn material_texture_source<'a>(
    draw: &'a Xmap12PdxMeshMaterialization,
    slot_name: &str,
    authored: &str,
) -> Result<&'a crate::Xmap12TextureSource> {
    let normalized = authored.replace('\\', "/").to_ascii_lowercase();
    let authored_base = normalized.rsplit('/').next().unwrap_or("");
    let source = draw
        .ordered_texture_sources
        .iter()
        .filter(|row| {
            row.slot.eq_ignore_ascii_case(slot_name)
                && row
                    .content
                    .normalized_game_relative_path
                    .to_ascii_lowercase()
                    .rsplit('/')
                    .next()
                    == Some(authored_base)
        })
        .collect::<Vec<_>>();
    if source.len() != 1 {
        return Err("XMAP12 native material texture identity is ambiguous".into());
    }
    Ok(source[0])
}

#[cfg(windows)]
unsafe fn clear_native_bindings(context: *mut winapi::um::d3d11::ID3D11DeviceContext) {
    let null_srv = [std::ptr::null_mut(); 16];
    let null_cb = [std::ptr::null_mut(); 3];
    (*context).PSSetShaderResources(0, null_srv.len() as u32, null_srv.as_ptr());
    (*context).VSSetConstantBuffers(0, null_cb.len() as u32, null_cb.as_ptr());
    (*context).PSSetConstantBuffers(0, null_cb.len() as u32, null_cb.as_ptr());
    (*context).OMSetRenderTargets(0, std::ptr::null(), std::ptr::null_mut());
}

#[cfg(windows)]
fn resource_set_sha256(bindings: &NativeXmap12MainSceneBindings) -> Result<String> {
    Ok(Sha256::hex(&serde_json::to_vec(&(
        bindings
            .textures
            .iter()
            .map(|row| (row.shader_register, &row.content))
            .collect::<Vec<_>>(),
        bindings
            .shared_resources
            .iter()
            .map(|row| {
                (
                    row.shader_register,
                    &row.semantic,
                    &row.payload_sha256,
                    &row.descriptor_sha256,
                    row.view_dimension,
                )
            })
            .collect::<Vec<_>>(),
        bindings
            .samplers
            .iter()
            .map(|row| (row.shader_register, &row.descriptor_sha256))
            .collect::<Vec<_>>(),
        bindings
            .partition_scenes
            .iter()
            .map(|row| {
                (
                    row.partition_mask,
                    &row.viewer_state_sha256,
                    &row.payload_sha256,
                )
            })
            .collect::<Vec<_>>(),
        &bindings.pixel_cb2_payload_sha256,
    ))?))
}

#[cfg(windows)]
unsafe fn validate_context_device(
    expected: *mut winapi::um::d3d11::ID3D11Device,
    context: *mut winapi::um::d3d11::ID3D11DeviceContext,
) -> Result<()> {
    use winapi::um::unknwnbase::IUnknown;
    if expected.is_null() || context.is_null() {
        return Err("XMAP12 D3D11 device/context is null".into());
    }
    let mut actual = std::ptr::null_mut();
    (*context).GetDevice(&mut actual);
    if actual.is_null() {
        return Err("XMAP12 D3D11 context returned no device".into());
    }
    let same = actual == expected;
    (*(actual as *mut IUnknown)).Release();
    if !same {
        return Err("XMAP12 D3D11 context belongs to a different device".into());
    }
    Ok(())
}

/// Return the exact live SRV dimension used to join an external XMAP04/05/07/
/// 09/10 provider to effective XMAP12 reflection.
///
/// # Safety
/// `view` must be a live D3D11 shader-resource view for the duration of this
/// call.
#[cfg(windows)]
pub unsafe fn xmap12_native_srv_view_dimension(
    view: *mut winapi::um::d3d11::ID3D11ShaderResourceView,
) -> Result<u32> {
    if view.is_null() {
        return Err("XMAP12 cannot inspect a null shared SRV".into());
    }
    let mut descriptor: winapi::um::d3d11::D3D11_SHADER_RESOURCE_VIEW_DESC = std::mem::zeroed();
    (*view).GetDesc(&mut descriptor);
    Ok(descriptor.ViewDimension as u32)
}

fn validate_sha256(value: &str, label: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!("{label} is not canonical lowercase SHA-256").into());
    }
    Ok(())
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn object_cb1_matches_retained_pdxmesh_user_data_layout() {
        let matrix = std::array::from_fn(|lane| (lane as f32 + 0.25).to_bits());
        let bytes = xmap12_object_cb1_payload(matrix, 1, 5632).unwrap();
        assert_eq!(bytes.len(), 96);
        for (lane, expected) in matrix.iter().enumerate() {
            assert_eq!(
                u32::from_le_bytes(bytes[lane * 4..lane * 4 + 4].try_into().unwrap()),
                *expected
            );
        }
        let tail = bytes[64..]
            .chunks_exact(4)
            .map(|lane| f32::from_le_bytes(lane.try_into().unwrap()))
            .collect::<Vec<_>>();
        assert_eq!(tail, [0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn object_cb1_translates_exact_xmap04_wrap_copies() {
        let mut matrix = [0.0f32.to_bits(); 16];
        matrix[0] = 1.0f32.to_bits();
        matrix[5] = 1.0f32.to_bits();
        matrix[10] = 1.0f32.to_bits();
        matrix[15] = 1.0f32.to_bits();
        matrix[12] = 17.5f32.to_bits();
        let x = |partition| {
            let bytes = xmap12_object_cb1_payload(matrix, partition, 5632).unwrap();
            f32::from_le_bytes(bytes[48..52].try_into().unwrap())
        };
        assert_eq!(x(1), 17.5);
        assert_eq!(x(2), 17.5 + 5632.0);
        assert_eq!(x(4), 17.5 - 5632.0);
        assert!(xmap12_object_cb1_payload(matrix, 8, 5632).is_err());
        assert!(xmap12_object_cb1_payload(matrix, 1, 0).is_err());
    }

    #[test]
    fn shadow_reflection_rejects_unbound_mod_resource() {
        let reflection = crate::shader_compile::DxbcReflection {
            target: 0xffff_0500,
            creator: "test".into(),
            resources: vec![crate::shader_compile::DxbcResource {
                name: "UnsupportedShadowMap".into(),
                resource_type: 2,
                dimension: 4,
                bind_point: 3,
                bind_count: 1,
            }],
            cbuffers: Vec::new(),
            inputs: Vec::new(),
            outputs: Vec::new(),
        };
        let error = validate_xmap12_stage_contract(
            &reflection,
            "shadow pixel",
            &[0, 1],
            &[0],
            false,
            Some(&[0]),
        )
        .unwrap_err();
        assert!(error.to_string().contains("unsupported texture t3"));
    }

    #[test]
    fn material_reflection_rejects_non_2d_srv_dimension() {
        let reflection = crate::shader_compile::DxbcReflection {
            target: 0xffff_0500,
            creator: "test".into(),
            resources: vec![crate::shader_compile::DxbcResource {
                name: "DiffuseMap_Texture".into(),
                resource_type: 2,
                dimension: winapi::um::d3dcommon::D3D11_SRV_DIMENSION_TEXTURECUBE as u32,
                bind_point: 0,
                bind_count: 1,
            }],
            cbuffers: Vec::new(),
            inputs: Vec::new(),
            outputs: Vec::new(),
        };
        let error = validate_xmap12_stage_contract(
            &reflection,
            "main pixel",
            &[0, 1, 2],
            &[0, 1, 2],
            true,
            None,
        )
        .unwrap_err();
        assert!(error.to_string().contains("material t0 is not Texture2D"));
    }

    #[test]
    fn selected_light_cb0_contains_only_exact_light_view_projection() {
        let matrix = std::array::from_fn(|lane| (lane as f32 + 1.0).to_bits());
        let bytes = xmap12_light_scene_cb0_payload(matrix).unwrap();
        assert_eq!(bytes.len(), 416);
        for (lane, expected) in matrix.iter().enumerate() {
            assert_eq!(
                u32::from_le_bytes(bytes[lane * 4..lane * 4 + 4].try_into().unwrap()),
                *expected
            );
        }
        assert!(bytes[64..].iter().all(|byte| *byte == 0));
        let mut invalid = matrix;
        invalid[7] = f32::NAN.to_bits();
        assert!(xmap12_light_scene_cb0_payload(invalid).is_err());
    }

    #[test]
    fn engine_none_mip_filter_uses_the_proven_point_mip_native_filter() {
        let descriptor = NativeSamplerDescriptor {
            name: "FlagMap".into(),
            shader_register: 3,
            mag_filter: "Linear".into(),
            min_filter: "Linear".into(),
            mip_filter: "None".into(),
            address_u: "Wrap".into(),
            address_v: "Wrap".into(),
            sampler_type: None,
            mip_map_lod_bias: None,
            mip_map_min_lod: None,
            mip_map_max_lod: None,
            max_anisotropy: None,
        };
        assert_eq!(
            native_sampler_filter(&descriptor).unwrap(),
            winapi::um::d3d11::D3D11_FILTER_MIN_MAG_LINEAR_MIP_POINT
        );
    }
}
