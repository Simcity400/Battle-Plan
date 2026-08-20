use crate::{
    decode_bmp_indexed, decode_definition_csv, decode_provinces_bmp, map_wrap_offsets,
    synthesize_river_world_mesh_with_defines, FoundationFeatureContext, FoundationFeatureOutput,
    GameFileResolver, ProductionMapCameraProjection, ProductionMapFeatureAdapter, Result,
    RiverFadeDefines, RiverPaletteDefines, ShaderStage, TerrainRenderViewFrustum,
    ViewerMapCameraState,
};
use portable_content_manifest::UsedContentKey;
use portable_render_stream::{
    AssignedStage, AssignedStageEdge, AssignedStageIdentity, FamilyDisposition, MapVisualFamily,
};
use replay_core::ValidatedContentAdmission;
use snapshot::sha256::Sha256;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;
use std::sync::Arc;

pub const XMAP08_RIVERS_STRAITS_SCHEMA: &str = "hoi4.xmap08.rivers-straits.a729.v2";
pub const XMAP08_RIVER_STAGE_NAME: &str = "rivers";
pub const XMAP08_RIVER_OPERATION_ID: &str = "rivers.prepare-and-draw";
pub const XMAP08_STRAIT_STAGE_NAME: &str = "straits";
pub const XMAP08_STRAIT_OPERATION_ID: &str = "straits.prepare-and-draw";

pub fn xmap08_assigned_stage_identities() -> [AssignedStageIdentity; 2] {
    [
        AssignedStageIdentity::new(XMAP08_RIVER_STAGE_NAME, XMAP08_RIVER_OPERATION_ID),
        AssignedStageIdentity::new(XMAP08_STRAIT_STAGE_NAME, XMAP08_STRAIT_OPERATION_ID),
    ]
}

pub fn xmap08_assigned_stage_edges() -> [AssignedStageEdge; 1] {
    let [rivers, straits] = xmap08_assigned_stage_identities();
    [AssignedStageEdge::new(rivers, straits)]
}
const RIVER_EFFECT: &str = "gfx/FX/river.shader";
const STRAIT_EFFECT: &str = "gfx/FX/strait.shader";
const PLATFORM_HEADER: &str = "gfx/FX/defines_hlsl_dx11.fxh";

const CORE_FILE_CONTENT: [&str; 12] = [
    "map/rivers.bmp",
    "map/heightmap.bmp",
    "map/provinces.bmp",
    "map/definition.csv",
    "map/adjacencies.csv",
    "map/adjacency_rules.txt",
    "common/defines/00_defines.lua",
    "common/defines/00_graphics.lua",
    "map/terrain/RiverSurface_masks.dds",
    "map/terrain/citylights_rgb_snowmask_a_0.dds",
    "map/terrain/strait.dds",
    PLATFORM_HEADER,
];

const RIVER_TEXTURE_VARIANTS: [(&str, &str); 2] = [
    ("Diffuse", "map/terrain/RiverSurface_diffuse_{}.dds"),
    ("NormalMap", "map/terrain/RiverSurface_normal_{}.dds"),
];

const RIVER_DOWNSTREAM_SAMPLERS: [(&str, &str); 12] = [
    ("WaterColor", "XMAP07"),
    ("LeanTexture1", "XMAP07"),
    ("LeanTexture2", "XMAP07"),
    ("ProvinceSecondaryColorMap", "XMAP05"),
    ("HeightNormal", "XMAP04"),
    ("SnowMudTexture", "XMAP07"),
    ("LightIndexMap", "XMAP09"),
    ("LightDataMap", "XMAP09"),
    ("GradientBorderChannel1", "XMAP05"),
    ("GradientBorderChannel2", "XMAP05"),
    ("ReflectionCubeMap", "XMAP07"),
    ("ShadowMap", "XMAP10"),
];

const STRAIT_DOWNSTREAM_SAMPLERS: [(&str, &str); 2] =
    [("FoWTexture", "XMAP05"), ("FoWDiffuse", "XMAP05")];

fn selected_river_static_content(sampler: &str, texture_quality: u8) -> Result<Option<String>> {
    if texture_quality > 2 {
        return Err(format!(
            "XMAP08 texture_quality must be in the exact engine domain 0..=2, got {texture_quality}"
        )
        .into());
    }
    if sampler == "RiverData" {
        return Ok(Some("map/terrain/RiverSurface_masks.dds".into()));
    }
    if sampler == "CityLightsAndSnowNoise" {
        return Ok(Some("map/terrain/citylights_rgb_snowmask_a_0.dds".into()));
    }
    Ok(RIVER_TEXTURE_VARIANTS
        .iter()
        .find(|(name, _)| *name == sampler)
        .map(|(_, pattern)| pattern.replace("{}", &texture_quality.to_string())))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum StraitAdjacencyKind {
    Unspecified = 0,
    Sea = 1,
    River = 2,
    RiverLarge = 3,
    Impassable = 4,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedStraitAdjacency {
    pub from: i32,
    pub to: i32,
    pub through: i32,
    pub kind: StraitAdjacencyKind,
    pub start: [i32; 2],
    pub stop: [i32; 2],
    pub rule: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedMapInputElement {
    pub semantic: String,
    pub semantic_index: u32,
    pub format: String,
    pub byte_offset: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedMapPipelineState {
    pub topology: String,
    pub index_format: String,
    pub vertex_stride: u32,
    pub input_layout: Vec<PreparedMapInputElement>,
    pub blend_state: Vec<(String, String)>,
    pub depth_stencil_state: Vec<(String, String)>,
    pub rasterizer_state: Vec<(String, String)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedMapSamplerRequirement {
    pub name: String,
    pub shader_register: u32,
    pub resource_type: String,
    pub mag_filter: String,
    pub min_filter: String,
    pub mip_filter: String,
    pub address_u: String,
    pub address_v: String,
    pub owner_packet: String,
    pub admitted_static_content: Vec<UsedContentKey>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedMapEffectProgram {
    pub effect_file: String,
    pub effect_name: String,
    pub ordered_shader_content: Vec<UsedContentKey>,
    pub vertex_translation_unit_sha256: String,
    pub pixel_translation_unit_sha256: String,
    pub vertex_bytecode: Arc<[u8]>,
    pub pixel_bytecode: Arc<[u8]>,
    pub vertex_bytecode_sha256: String,
    pub pixel_bytecode_sha256: String,
    pub samplers: Vec<PreparedMapSamplerRequirement>,
    pub pipeline: PreparedMapPipelineState,
    pub constant_buffer_owners: Vec<(u32, String)>,
    pub deterministic_sha256: String,
}

impl PreparedMapEffectProgram {
    fn validate(&self) -> Result<()> {
        if self.effect_file.is_empty()
            || self.effect_name.is_empty()
            || self.ordered_shader_content.is_empty()
            || self.vertex_bytecode.is_empty()
            || self.pixel_bytecode.is_empty()
            || self.vertex_bytecode_sha256 != Sha256::hex(&self.vertex_bytecode)
            || self.pixel_bytecode_sha256 != Sha256::hex(&self.pixel_bytecode)
            || self.pipeline.topology != "D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST"
            || self.pipeline.index_format != "DXGI_FORMAT_R16_UINT"
            || self.pipeline.vertex_stride != 32
            || self.pipeline.input_layout
                != [
                    PreparedMapInputElement {
                        semantic: "POSITION".into(),
                        semantic_index: 0,
                        format: "DXGI_FORMAT_R32G32B32A32_FLOAT".into(),
                        byte_offset: 0,
                    },
                    PreparedMapInputElement {
                        semantic: "TEXCOORD".into(),
                        semantic_index: 0,
                        format: "DXGI_FORMAT_R32G32B32A32_FLOAT".into(),
                        byte_offset: 16,
                    },
                ]
            || self.deterministic_sha256 != map_program_digest(self)
        {
            return Err("XMAP08 prepared shader/pipeline program is incomplete".into());
        }
        let vertex_reflection = crate::parse_dxbc_reflection(&self.vertex_bytecode)?;
        let pixel_reflection = crate::parse_dxbc_reflection(&self.pixel_bytecode)?;
        let vertex_inputs = vertex_reflection
            .inputs
            .iter()
            .map(|input| {
                (
                    input.semantic_name.to_ascii_uppercase(),
                    input.semantic_index,
                    input.register,
                    input.component_type,
                    input.mask,
                )
            })
            .collect::<BTreeSet<_>>();
        if vertex_reflection.target != 0xfffe_0500
            || pixel_reflection.target != 0xffff_0500
            || vertex_inputs
                != [
                    ("POSITION".to_string(), 0, 0, 3, 0x0f),
                    ("TEXCOORD".to_string(), 0, 1, 3, 0x0f),
                ]
                .into_iter()
                .collect()
            || vertex_reflection
                .resources
                .iter()
                .any(|resource| resource.resource_type != 0 || resource.bind_count != 1)
            || pixel_reflection.resources.iter().any(|resource| {
                !matches!(resource.resource_type, 0 | 2 | 3) || resource.bind_count != 1
            })
        {
            return Err("XMAP08 compiled bytecode has the wrong stage/input contract".into());
        }
        let mut registers = BTreeSet::new();
        for sampler in &self.samplers {
            if sampler.name.is_empty()
                || sampler.owner_packet.is_empty()
                || sampler.resource_type.is_empty()
                || sampler.mag_filter.is_empty()
                || sampler.min_filter.is_empty()
                || sampler.mip_filter.is_empty()
                || sampler.address_u.is_empty()
                || sampler.address_v.is_empty()
                || !registers.insert(sampler.shader_register)
                || (sampler.owner_packet == "XMAP08" && sampler.admitted_static_content.len() != 1)
                || (sampler.owner_packet != "XMAP08" && !sampler.admitted_static_content.is_empty())
            {
                return Err("XMAP08 sampler ownership/descriptor is incomplete".into());
            }
            for content in &sampler.admitted_static_content {
                content
                    .validate()
                    .map_err(|error| format!("XMAP08 sampler content identity: {error}"))?;
            }
        }
        if self
            .samplers
            .iter()
            .enumerate()
            .any(|(index, sampler)| sampler.shader_register != index as u32)
        {
            return Err("XMAP08 sampler registers are not contiguous from zero".into());
        }
        validate_fixed_pipeline_state(self)?;
        let mut reflected_textures = BTreeSet::new();
        let mut reflected_samplers = BTreeSet::new();
        for resource in &pixel_reflection.resources {
            let (suffix, output) = match resource.resource_type {
                2 => ("_Texture", &mut reflected_textures),
                3 => ("_Sampler", &mut reflected_samplers),
                _ => continue,
            };
            let Some(name) = resource.name.strip_suffix(suffix) else {
                return Err(format!(
                    "XMAP08 reflected pixel resource {} has no engine sampler suffix",
                    resource.name
                )
                .into());
            };
            if !self.samplers.iter().any(|sampler| {
                sampler.name == name && sampler.shader_register == resource.bind_point
            }) {
                return Err(format!(
                    "XMAP08 reflected pixel resource {} at register {} is undeclared",
                    resource.name, resource.bind_point
                )
                .into());
            }
            output.insert((name.to_string(), resource.bind_point));
        }
        if reflected_textures.is_empty() || reflected_textures != reflected_samplers {
            return Err("XMAP08 retained pixel texture/sampler reflection is incomplete".into());
        }
        Ok(())
    }

    pub fn retained_sampler_requirements(&self) -> Result<Vec<&PreparedMapSamplerRequirement>> {
        self.validate()?;
        Ok(reflected_resource_requirements(self)?
            .into_iter()
            .map(|(_, sampler)| sampler)
            .collect())
    }

    pub fn retains_constant_buffer_owner(&self, slot: u32, owner: &str) -> Result<bool> {
        self.validate()?;
        Ok(reflected_constant_buffer_requirements(self)?
            .into_iter()
            .any(|(_, retained_slot, retained_owner)| {
                retained_slot == slot && retained_owner == owner
            }))
    }
}

fn validate_fixed_pipeline_state(program: &PreparedMapEffectProgram) -> Result<()> {
    let state = |rows: &[(String, String)]| rows.iter().cloned().collect::<BTreeMap<_, _>>();
    let blend = state(&program.pipeline.blend_state);
    if blend.len() != program.pipeline.blend_state.len()
        || !blend.get("BlendEnable").is_some_and(|value| value == "yes")
        || !blend
            .get("SourceBlend")
            .is_some_and(|value| matches!(value.as_str(), "src_alpha" | "SRC_ALPHA"))
        || !blend
            .get("DestBlend")
            .is_some_and(|value| matches!(value.as_str(), "inv_src_alpha" | "INV_SRC_ALPHA"))
        || blend.get("AlphaTest").is_some_and(|value| value != "no")
        || blend
            .get("WriteMask")
            .is_some_and(|value| !matches!(value.as_str(), "RED|GREEN|BLUE"))
        || blend.keys().any(|key| {
            !matches!(
                key.as_str(),
                "BlendEnable" | "AlphaTest" | "SourceBlend" | "DestBlend" | "WriteMask"
            )
        })
    {
        return Err(format!(
            "XMAP08 {} blend state is outside the executable alpha-blend contract",
            program.effect_name
        )
        .into());
    }
    let depth = state(&program.pipeline.depth_stencil_state);
    if depth.len() != 1
        || !depth
            .get("DepthEnable")
            .is_some_and(|value| *value == "yes")
    {
        return Err(format!(
            "XMAP08 {} depth state is outside the exact depth-write contract",
            program.effect_name
        )
        .into());
    }
    let raster = state(&program.pipeline.rasterizer_state);
    if raster.len() != 3
        || !raster
            .get("FillMode")
            .is_some_and(|value| *value == "FILL_SOLID")
        || !raster
            .get("CullMode")
            .is_some_and(|value| *value == "CULL_BACK")
        || !raster.get("FrontCCW").is_some_and(|value| *value == "no")
    {
        return Err(format!(
            "XMAP08 {} rasterizer state is outside the exact solid/back-face contract",
            program.effect_name
        )
        .into());
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedRiverDraw {
    pub render_view_mask: u8,
    pub map_copy_offset_f32_bits: u32,
    pub first_vertex: u32,
    pub vertex_count: u32,
    pub index_count: u32,
    pub base_vertex: i32,
    pub render_group: u8,
    pub starts_at_source: bool,
    pub bounds_f32_bits: [[u32; 3]; 2],
    pub indices: Arc<[u8]>,
    pub indices_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedStraitDraw {
    pub render_view_mask: u8,
    pub map_copy_offset_f32_bits: u32,
    pub index_count: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MapDependentResourceBinding {
    pub effect_name: String,
    pub symbol: String,
    pub owner_packet: String,
    pub resource_sha256: String,
    pub descriptor_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MapConstantBufferBinding {
    pub effect_name: String,
    pub shader_register: u32,
    pub owner_packet: String,
    pub payload_sha256: String,
    pub descriptor_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiversStraitsVisibleDrawDependencies {
    pub resources: Vec<MapDependentResourceBinding>,
    pub constant_buffers: Vec<MapConstantBufferBinding>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadyRiversStraitsDrawContract {
    pub frame_sha256: String,
    pub ordered_resources: Vec<MapDependentResourceBinding>,
    pub ordered_constant_buffers: Vec<MapConstantBufferBinding>,
    pub deterministic_sha256: String,
}

fn validate_visible_resource_binding(
    effect_name: &str,
    sampler: &PreparedMapSamplerRequirement,
    binding: &MapDependentResourceBinding,
) -> Result<()> {
    if binding.effect_name != effect_name
        || binding.symbol != sampler.name
        || binding.owner_packet != sampler.owner_packet
    {
        return Err("XMAP08 visible resource dependency order/owner mismatch".into());
    }
    validate_sha256(&binding.resource_sha256, "XMAP08 resource")?;
    validate_sha256(&binding.descriptor_sha256, "XMAP08 resource descriptor")?;
    if sampler.owner_packet == "XMAP08" {
        let admitted = sampler
            .admitted_static_content
            .first()
            .ok_or("XMAP08 static sampler has no admitted content identity")?;
        if sampler.admitted_static_content.len() != 1
            || binding.resource_sha256 != admitted.payload_sha256
        {
            return Err(format!(
                "XMAP08 static sampler {} resource hash does not match its admitted payload",
                sampler.name
            )
            .into());
        }
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedRiversStraitsFrame {
    pub schema: String,
    pub map_width: u32,
    pub map_height: u32,
    pub camera_view_sha256: String,
    pub river_assignment: AssignedStage,
    pub strait_assignment: AssignedStage,
    pub ordered_used_content: Vec<UsedContentKey>,
    pub river_vertices: Arc<[u8]>,
    pub river_draws: Vec<PreparedRiverDraw>,
    pub river_enumerator_witness_sha256: String,
    pub strait_vertices: Arc<[u8]>,
    pub strait_indices: Arc<[u8]>,
    pub strait_draws: Vec<PreparedStraitDraw>,
    pub strait_enumerator_witness_sha256: String,
    pub strait_adjacencies: Vec<PreparedStraitAdjacency>,
    pub adjacency_rule_names: Vec<String>,
    pub river_program: PreparedMapEffectProgram,
    pub strait_program: PreparedMapEffectProgram,
    pub required_downstream_packets: Vec<String>,
    pub deterministic_sha256: String,
}

impl PreparedRiversStraitsFrame {
    pub fn validate(&self) -> Result<()> {
        if self.schema != XMAP08_RIVERS_STRAITS_SCHEMA
            || self.map_width == 0
            || self.map_height == 0
            || validate_sha256(&self.camera_view_sha256, "XMAP08 camera view").is_err()
            || self.ordered_used_content.is_empty()
            || self.required_downstream_packets
                != reflected_downstream_packets(&self.river_program)?
        {
            return Err("XMAP08 prepared rivers/straits payload is incomplete".into());
        }
        self.river_assignment
            .validate()
            .map_err(|error| format!("XMAP08 river assignment: {error}"))?;
        self.strait_assignment
            .validate()
            .map_err(|error| format!("XMAP08 strait assignment: {error}"))?;
        if self.river_assignment.stage_name != XMAP08_RIVER_STAGE_NAME
            || self.river_assignment.operation_name != XMAP08_RIVER_OPERATION_ID
            || self.strait_assignment.stage_name != XMAP08_STRAIT_STAGE_NAME
            || self.strait_assignment.operation_name != XMAP08_STRAIT_OPERATION_ID
            || self.river_assignment.topology_sha256 != self.strait_assignment.topology_sha256
            || self.river_assignment.assigned_ordinal >= self.strait_assignment.assigned_ordinal
        {
            return Err(
                "XMAP08 prepared frame does not bind the assigned rivers-before-straits topology"
                    .into(),
            );
        }
        self.river_program.validate()?;
        self.strait_program.validate()?;
        let mut paths = BTreeSet::new();
        for content in &self.ordered_used_content {
            content
                .validate()
                .map_err(|error| format!("XMAP08 used-content identity: {error}"))?;
            if !paths.insert(content.normalized_game_relative_path.as_str()) {
                return Err("XMAP08 used-content path is duplicated".into());
            }
        }
        if self.river_vertices.len() % 32 != 0 {
            return Err("XMAP08 river vertex payload is not stride-32".into());
        }
        let vertex_count = self.river_vertices.len() / 32;
        let mut prior_river_mask = 0u8;
        for draw in &self.river_draws {
            if !matches!(draw.render_view_mask, 1 | 2 | 4)
                || !map_copy_offset_matches_engine(
                    draw.render_view_mask,
                    draw.map_copy_offset_f32_bits,
                    self.map_width,
                )
                || draw.render_view_mask < prior_river_mask
                || draw.vertex_count < 4
                || draw.vertex_count % 2 != 0
                || draw.first_vertex as usize + draw.vertex_count as usize + 2 > vertex_count
                || draw.index_count != draw.vertex_count * 3
                || draw.base_vertex != draw.first_vertex as i32
                || draw.indices.len() != draw.index_count as usize * 2
                || draw.indices_sha256 != Sha256::hex(&draw.indices)
                || !river_index_payload_matches(draw)
                || draw.bounds_f32_bits[0]
                    .iter()
                    .zip(draw.bounds_f32_bits[1])
                    .any(|(min, max)| f32::from_bits(*min) > f32::from_bits(max))
            {
                return Err("XMAP08 river draw contract is invalid".into());
            }
            prior_river_mask = draw.render_view_mask;
        }
        if self.river_enumerator_witness_sha256 != component_enumerator_witness(self, true) {
            return Err("XMAP08 river enumerator witness mismatch".into());
        }
        if self.strait_vertices.len() % 32 != 0
            || self.strait_indices.len() % 2 != 0
            || (self.strait_indices.is_empty() != self.strait_vertices.is_empty())
            || !strait_index_payload_matches(&self.strait_indices, self.strait_vertices.len() / 32)
            || self.strait_draws.iter().enumerate().any(|(ordinal, draw)| {
                !matches!(draw.render_view_mask, 1 | 2 | 4)
                    || !map_copy_offset_matches_engine(
                        draw.render_view_mask,
                        draw.map_copy_offset_f32_bits,
                        self.map_width,
                    )
                    || ordinal > 0
                        && self.strait_draws[ordinal - 1].render_view_mask >= draw.render_view_mask
                    || draw.index_count as usize * 2 != self.strait_indices.len()
            })
            || (self.strait_vertices.is_empty() != self.strait_draws.is_empty())
        {
            return Err("XMAP08 strait draw contract is invalid".into());
        }
        if self.strait_enumerator_witness_sha256 != component_enumerator_witness(self, false) {
            return Err("XMAP08 strait enumerator witness mismatch".into());
        }
        let known_rules = self
            .adjacency_rule_names
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        if known_rules.len() != self.adjacency_rule_names.len()
            || self
                .strait_adjacencies
                .iter()
                .any(|row| !row.rule.is_empty() && !known_rules.contains(row.rule.as_str()))
        {
            return Err("XMAP08 adjacency rule reference is unresolved".into());
        }
        if self.deterministic_sha256 != prepared_frame_digest(self) {
            return Err("XMAP08 prepared frame digest mismatch".into());
        }
        Ok(())
    }

    pub fn admit_visible_draw_dependencies(
        &self,
        dependencies: RiversStraitsVisibleDrawDependencies,
    ) -> Result<ReadyRiversStraitsDrawContract> {
        self.validate()?;
        let expected_resources = reflected_resource_requirements(&self.river_program)?
            .into_iter()
            .chain(reflected_resource_requirements(&self.strait_program)?)
            .collect::<Vec<_>>();
        if dependencies.resources.len() != expected_resources.len() {
            return Err("XMAP08 visible resource dependencies are incomplete".into());
        }
        for (binding, expected) in dependencies.resources.iter().zip(&expected_resources) {
            validate_visible_resource_binding(expected.0, expected.1, binding)?;
        }
        let expected_cbuffers = reflected_constant_buffer_requirements(&self.river_program)?
            .into_iter()
            .chain(reflected_constant_buffer_requirements(
                &self.strait_program,
            )?)
            .collect::<Vec<_>>();
        if dependencies.constant_buffers.len() != expected_cbuffers.len() {
            return Err("XMAP08 visible constant-buffer dependencies are incomplete".into());
        }
        for (binding, expected) in dependencies.constant_buffers.iter().zip(&expected_cbuffers) {
            if (
                binding.effect_name.as_str(),
                binding.shader_register,
                binding.owner_packet.as_str(),
            ) != *expected
            {
                return Err("XMAP08 constant-buffer dependency order/owner mismatch".into());
            }
            validate_sha256(&binding.payload_sha256, "XMAP08 constant buffer")?;
            validate_sha256(
                &binding.descriptor_sha256,
                "XMAP08 constant-buffer descriptor",
            )?;
        }
        let mut identity = Vec::new();
        identity.extend_from_slice(self.deterministic_sha256.as_bytes());
        identity.extend_from_slice(format!("{:?}", dependencies.resources).as_bytes());
        identity.extend_from_slice(format!("{:?}", dependencies.constant_buffers).as_bytes());
        Ok(ReadyRiversStraitsDrawContract {
            frame_sha256: self.deterministic_sha256.clone(),
            ordered_resources: dependencies.resources,
            ordered_constant_buffers: dependencies.constant_buffers,
            deterministic_sha256: Sha256::hex(&identity),
        })
    }
}

fn map_copy_offset_matches_engine(render_view_mask: u8, offset_bits: u32, map_width: u32) -> bool {
    let map_width = map_width as f32;
    let expected = match render_view_mask {
        1 => 0.0f32.to_bits(),
        2 => map_width.to_bits(),
        4 => (-map_width).to_bits(),
        _ => return false,
    };
    offset_bits == expected
}

pub(crate) fn reflected_resource_requirements(
    program: &PreparedMapEffectProgram,
) -> Result<Vec<(&str, &PreparedMapSamplerRequirement)>> {
    let reflection = crate::parse_dxbc_reflection(&program.pixel_bytecode)?;
    let mut requirements = Vec::new();
    for resource in reflection
        .resources
        .iter()
        .filter(|resource| resource.resource_type == 2)
    {
        let name = resource.name.strip_suffix("_Texture").ok_or_else(|| {
            format!(
                "XMAP08 reflected texture {} has no engine texture suffix",
                resource.name
            )
        })?;
        let sampler = program
            .samplers
            .iter()
            .find(|sampler| sampler.name == name && sampler.shader_register == resource.bind_point)
            .ok_or_else(|| {
                format!(
                    "XMAP08 reflected texture {} at slot {} has no declared sampler",
                    resource.name, resource.bind_point
                )
            })?;
        requirements.push((program.effect_name.as_str(), sampler));
    }
    requirements.sort_by_key(|(_, sampler)| sampler.shader_register);
    Ok(requirements)
}

pub(crate) fn reflected_constant_buffer_requirements(
    program: &PreparedMapEffectProgram,
) -> Result<Vec<(&str, u32, &str)>> {
    let vertex = crate::parse_dxbc_reflection(&program.vertex_bytecode)?;
    let pixel = crate::parse_dxbc_reflection(&program.pixel_bytecode)?;
    let mut slots = BTreeSet::new();
    for reflection in [&vertex, &pixel] {
        for resource in reflection
            .resources
            .iter()
            .filter(|resource| resource.resource_type == 0)
        {
            slots.insert(resource.bind_point);
        }
    }
    slots
        .into_iter()
        .map(|slot| {
            let owner = program
                .constant_buffer_owners
                .iter()
                .find_map(|(candidate, owner)| (*candidate == slot).then_some(owner.as_str()))
                .ok_or_else(|| -> Box<dyn std::error::Error> {
                    format!("XMAP08 reflected constant buffer slot {slot} has no packet owner")
                        .into()
                })?;
            Ok((program.effect_name.as_str(), slot, owner))
        })
        .collect()
}

pub(crate) fn validate_program_draw_dependencies(
    program: &PreparedMapEffectProgram,
    resources: &[MapDependentResourceBinding],
    constant_buffers: &[MapConstantBufferBinding],
) -> Result<()> {
    let expected_resources = reflected_resource_requirements(program)?;
    if resources.len() != expected_resources.len() {
        return Err("XMAP08 executed program resource set is incomplete".into());
    }
    for (binding, expected) in resources.iter().zip(expected_resources) {
        validate_visible_resource_binding(expected.0, expected.1, binding)?;
    }
    let expected_cbuffers = reflected_constant_buffer_requirements(program)?;
    if constant_buffers.len() != expected_cbuffers.len() {
        return Err("XMAP08 executed program constant-buffer set is incomplete".into());
    }
    for (binding, expected) in constant_buffers.iter().zip(expected_cbuffers) {
        if (
            binding.effect_name.as_str(),
            binding.shader_register,
            binding.owner_packet.as_str(),
        ) != expected
        {
            return Err("XMAP08 executed constant-buffer order/owner mismatch".into());
        }
        validate_sha256(&binding.payload_sha256, "XMAP08 constant buffer")?;
        validate_sha256(
            &binding.descriptor_sha256,
            "XMAP08 constant-buffer descriptor",
        )?;
    }
    Ok(())
}

pub(crate) fn reflected_cbuffer_variable_is_used(
    program: &PreparedMapEffectProgram,
    slot: u32,
    variable_name: &str,
) -> Result<bool> {
    for reflection in [
        crate::parse_dxbc_reflection(&program.vertex_bytecode)?,
        crate::parse_dxbc_reflection(&program.pixel_bytecode)?,
    ] {
        let names = reflection
            .resources
            .iter()
            .filter(|resource| resource.resource_type == 0 && resource.bind_point == slot)
            .map(|resource| resource.name.as_str())
            .collect::<BTreeSet<_>>();
        if reflection
            .cbuffers
            .iter()
            .filter(|buffer| names.contains(buffer.name.as_str()))
            .flat_map(|buffer| &buffer.vars)
            .any(|variable| variable.name == variable_name && variable.used)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn reflected_downstream_packets(program: &PreparedMapEffectProgram) -> Result<Vec<String>> {
    let resource_owners = reflected_resource_requirements(program)?
        .into_iter()
        .map(|(_, sampler)| sampler.owner_packet.as_str());
    let constant_owners = reflected_constant_buffer_requirements(program)?
        .into_iter()
        .flat_map(|(_, _, owners)| owners.split('+'));
    Ok(resource_owners
        .chain(constant_owners)
        .filter(|owner| !matches!(*owner, "XMAP03" | "XMAP08"))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(str::to_owned)
        .collect())
}

struct Xmap08Content {
    map_width: u32,
    map_height: u32,
    ordered_used_content: Vec<UsedContentKey>,
    river_vertices: Arc<[u8]>,
    river_strips: Vec<BaseRiverStrip>,
    strait_vertices: Arc<[u8]>,
    strait_indices: Arc<[u8]>,
    strait_adjacencies: Vec<PreparedStraitAdjacency>,
    adjacency_rule_names: Vec<String>,
    river_program: PreparedMapEffectProgram,
    strait_program: PreparedMapEffectProgram,
}

#[derive(Clone)]
struct BaseRiverStrip {
    first_vertex: u32,
    vertex_count: u32,
    render_group: u8,
    starts_at_source: bool,
    bounds: [[f32; 3]; 2],
    indices: Arc<[u8]>,
}

impl Xmap08Content {
    fn admit(
        resolver: &GameFileResolver,
        admission: &ValidatedContentAdmission,
        high_gfx_shaders: bool,
        texture_quality: u8,
    ) -> Result<Self> {
        let river_tu =
            crate::shader_compile::effect_translation_unit_files(resolver, RIVER_EFFECT)?;
        let strait_tu =
            crate::shader_compile::effect_translation_unit_files(resolver, STRAIT_EFFECT)?;
        let mut ordered_used_content = Vec::new();
        let mut seen = BTreeSet::new();
        let mut bytes_for = BTreeMap::new();
        let selected_river_content = [
            "Diffuse",
            "NormalMap",
            "RiverData",
            "CityLightsAndSnowNoise",
        ]
        .into_iter()
        .map(|sampler| {
            selected_river_static_content(sampler, texture_quality)?
                .ok_or_else(|| format!("XMAP08 static sampler {sampler} has no content").into())
        })
        .collect::<Result<Vec<_>>>()?;
        for path in CORE_FILE_CONTENT
            .into_iter()
            .chain(selected_river_content.iter().map(String::as_str))
            .chain(river_tu.iter().map(|(path, _)| path.as_str()))
            .chain(strait_tu.iter().map(|(path, _)| path.as_str()))
        {
            if !seen.insert(path.to_ascii_lowercase()) {
                continue;
            }
            let content = admission
                .require_used_content(path)
                .map_err(|error| format!("XMAP08 requires admitted {path}: {error}"))?
                .clone();
            bytes_for.insert(
                path.to_ascii_lowercase(),
                resolver.read_admitted_used_content(&content)?,
            );
            ordered_used_content.push(content);
        }
        let rivers = decode_bmp_indexed(
            required_bytes(&bytes_for, "map/rivers.bmp")?,
            Path::new("map/rivers.bmp"),
        )?;
        let heightmap = decode_bmp_indexed(
            required_bytes(&bytes_for, "map/heightmap.bmp")?,
            Path::new("map/heightmap.bmp"),
        )?;
        let palette = RiverPaletteDefines::from_defines_source(&latin1(required_bytes(
            &bytes_for,
            "common/defines/00_defines.lua",
        )?))?;
        let fade = RiverFadeDefines::from_graphics_source(&latin1(required_bytes(
            &bytes_for,
            "common/defines/00_graphics.lua",
        )?))?;
        let mesh = synthesize_river_world_mesh_with_defines(&rivers, &heightmap, palette, fade)?;
        let river_vertices: Arc<[u8]> = mesh.to_captured_layout_bytes().into();
        let mut river_strips = Vec::with_capacity(mesh.strips.len());
        for strip in &mesh.strips {
            let first_vertex = u32::try_from(strip.pair_start * 2)?;
            let vertex_count = u32::try_from((strip.pair_end - strip.pair_start) * 2 - 2)?;
            let mut indices = Vec::with_capacity(vertex_count as usize * 3);
            let mut index = 0u32;
            for _ in 0..vertex_count / 2 {
                for value in [index, index + 2, index + 1, index + 2, index + 3, index + 1] {
                    indices.extend_from_slice(&(value as u16).to_le_bytes());
                }
                index += 2;
            }
            let vertices = &mesh.vertices[strip.pair_start * 2..strip.pair_end * 2];
            let mut min = [f32::INFINITY; 3];
            let mut max = [f32::NEG_INFINITY; 3];
            for vertex in vertices {
                for axis in 0..3 {
                    min[axis] = min[axis].min(vertex.position[axis]);
                    max[axis] = max[axis].max(vertex.position[axis]);
                }
            }
            river_strips.push(BaseRiverStrip {
                first_vertex,
                vertex_count,
                render_group: strip.render_group,
                starts_at_source: strip.starts_at_source,
                bounds: [min, max],
                indices: indices.into(),
            });
        }

        let definition_source = latin1(required_bytes(&bytes_for, "map/definition.csv")?);
        let (definitions, color_to_id) =
            decode_definition_csv(&definition_source, Path::new("map/definition.csv"))?;
        let provinces = decode_provinces_bmp(
            required_bytes(&bytes_for, "map/provinces.bmp")?,
            Path::new("map/provinces.bmp"),
            &color_to_id,
        )?;
        if provinces.width != rivers.width || provinces.height != rivers.height {
            return Err("XMAP08 provinces/rivers map dimensions disagree".into());
        }
        let strait_adjacencies =
            parse_adjacencies(&latin1(required_bytes(&bytes_for, "map/adjacencies.csv")?))?;
        let adjacency_rule_names = parse_adjacency_rule_names(&latin1(required_bytes(
            &bytes_for,
            "map/adjacency_rules.txt",
        )?))?;
        validate_adjacency_rule_references(&strait_adjacencies, &adjacency_rule_names)?;
        let (strait_vertices, strait_indices) =
            synthesize_strait_mesh(&strait_adjacencies, &definitions, &provinces)?;

        let defines =
            crate::engine_map_shader_defines(crate::derive_a729_engine_map_shader_constants(
                rivers.width as u32,
                rivers.height as u32,
                false,
                0,
            )?);
        let river_effect_name = if high_gfx_shaders {
            "river"
        } else {
            "river_low_gfx"
        };
        let river_program = prepare_program(
            resolver,
            admission,
            &river_tu,
            RIVER_EFFECT,
            river_effect_name,
            &defines,
            ProgramKind::River,
            texture_quality,
        )?;
        let strait_program = prepare_program(
            resolver,
            admission,
            &strait_tu,
            STRAIT_EFFECT,
            "strait",
            &defines,
            ProgramKind::Strait,
            texture_quality,
        )?;
        Ok(Self {
            map_width: rivers.width as u32,
            map_height: rivers.height as u32,
            ordered_used_content,
            river_vertices,
            river_strips,
            strait_vertices: strait_vertices.into(),
            strait_indices: strait_indices.into(),
            strait_adjacencies,
            adjacency_rule_names,
            river_program,
            strait_program,
        })
    }

    fn prepare(
        &self,
        camera: &ViewerMapCameraState,
        river_assignment: AssignedStage,
        strait_assignment: AssignedStage,
    ) -> Result<PreparedRiversStraitsFrame> {
        if camera.map_width != self.map_width || camera.map_height != self.map_height {
            return Err("XMAP08 camera/map dimensions disagree".into());
        }
        let views = enabled_render_views(camera)?;
        let mut river_draws = Vec::new();
        for (render_view_mask, map_copy_offset, frustum) in &views {
            for strip in &self.river_strips {
                if frustum.bounds_outside(strip.bounds[0], strip.bounds[1]) {
                    continue;
                }
                river_draws.push(PreparedRiverDraw {
                    render_view_mask: *render_view_mask,
                    map_copy_offset_f32_bits: map_copy_offset.to_bits(),
                    first_vertex: strip.first_vertex,
                    vertex_count: strip.vertex_count,
                    index_count: strip.vertex_count * 3,
                    base_vertex: i32::try_from(strip.first_vertex)?,
                    render_group: strip.render_group,
                    starts_at_source: strip.starts_at_source,
                    bounds_f32_bits: [
                        strip.bounds[0].map(f32::to_bits),
                        strip.bounds[1].map(f32::to_bits),
                    ],
                    indices: strip.indices.clone(),
                    indices_sha256: Sha256::hex(&strip.indices),
                });
            }
        }
        river_draws
            .sort_by_key(|draw| (draw.render_view_mask, draw.render_group, draw.first_vertex));
        let strait_index_count = u32::try_from(self.strait_indices.len() / 2)?;
        let strait_draws = if strait_index_count == 0 {
            Vec::new()
        } else {
            views
                .iter()
                .map(|(mask, offset, _)| PreparedStraitDraw {
                    render_view_mask: *mask,
                    map_copy_offset_f32_bits: offset.to_bits(),
                    index_count: strait_index_count,
                })
                .collect()
        };
        let mut prepared = PreparedRiversStraitsFrame {
            schema: XMAP08_RIVERS_STRAITS_SCHEMA.into(),
            map_width: self.map_width,
            map_height: self.map_height,
            camera_view_sha256: camera.deterministic_sha256.clone(),
            river_assignment,
            strait_assignment,
            ordered_used_content: self.ordered_used_content.clone(),
            river_vertices: self.river_vertices.clone(),
            river_draws,
            river_enumerator_witness_sha256: String::new(),
            strait_vertices: self.strait_vertices.clone(),
            strait_indices: self.strait_indices.clone(),
            strait_draws,
            strait_enumerator_witness_sha256: String::new(),
            strait_adjacencies: self.strait_adjacencies.clone(),
            adjacency_rule_names: self.adjacency_rule_names.clone(),
            river_program: self.river_program.clone(),
            strait_program: self.strait_program.clone(),
            required_downstream_packets: reflected_downstream_packets(&self.river_program)?,
            deterministic_sha256: String::new(),
        };
        prepared.river_enumerator_witness_sha256 = component_enumerator_witness(&prepared, true);
        prepared.strait_enumerator_witness_sha256 = component_enumerator_witness(&prepared, false);
        prepared.deterministic_sha256 = prepared_frame_digest(&prepared);
        prepared.validate()?;
        Ok(prepared)
    }
}

pub(crate) struct Xmap08RiversStraitsAdapter {
    content: Arc<Xmap08Content>,
}

impl Xmap08RiversStraitsAdapter {
    pub(crate) fn new(
        resolver: &GameFileResolver,
        admission: &ValidatedContentAdmission,
        high_gfx_shaders: bool,
        texture_quality: u8,
    ) -> Result<Self> {
        Ok(Self {
            content: Arc::new(Xmap08Content::admit(
                resolver,
                admission,
                high_gfx_shaders,
                texture_quality,
            )?),
        })
    }
}

impl ProductionMapFeatureAdapter for Xmap08RiversStraitsAdapter {
    fn family(&self) -> MapVisualFamily {
        MapVisualFamily::RiversStraits
    }

    fn prepare(&self, context: &FoundationFeatureContext<'_>) -> Result<FoundationFeatureOutput> {
        let camera = context
            .viewer
            .camera
            .as_ref()
            .ok_or("XMAP08 rivers/straits require the XMAP03 viewer camera")?;
        camera.validate()?;
        let river_assignment = context
            .require_assigned_stage(XMAP08_RIVER_STAGE_NAME, XMAP08_RIVER_OPERATION_ID)?
            .clone();
        let strait_assignment = context
            .require_assigned_stage(XMAP08_STRAIT_STAGE_NAME, XMAP08_STRAIT_OPERATION_ID)?
            .clone();
        let prepared =
            self.content
                .prepare(camera, river_assignment.clone(), strait_assignment.clone())?;
        let target_sha256 =
            Sha256::hex(format!("{}x{}", prepared.map_width, prepared.map_height).as_bytes());
        let river_operation = river_assignment
            .foundation_operation_identity(
                prepared.river_program.deterministic_sha256.clone(),
                Sha256::hex(format!("{:?}", prepared.river_draws).as_bytes()),
                target_sha256.clone(),
            )
            .map_err(|error| format!("XMAP08 river operation identity: {error}"))?;
        let strait_operation = strait_assignment
            .foundation_operation_identity(
                prepared.strait_program.deterministic_sha256.clone(),
                Sha256::hex(
                    format!(
                        "{:?}{:?}{:?}",
                        prepared.strait_draws, prepared.strait_vertices, prepared.strait_indices
                    )
                    .as_bytes(),
                ),
                target_sha256,
            )
            .map_err(|error| format!("XMAP08 strait operation identity: {error}"))?;
        Ok(FoundationFeatureOutput {
            disposition: FamilyDisposition::Present {
                payload_schema: XMAP08_RIVERS_STRAITS_SCHEMA.into(),
                producer_registration_sha256: Sha256::hex(
                    b"XMAP08\0FUN_141b0fc70\0FUN_141b0f440\0FUN_141b126a0\0FUN_141263a30\0FUN_141264830\0FUN_140a53290\0FUN_141b12d20\0FUN_141268c60",
                ),
                payload_sha256: prepared.deterministic_sha256.clone(),
                ordered_operations: vec![river_operation, strait_operation],
                ordered_used_content: prepared.ordered_used_content.clone(),
            },
            prepared_terrain: None,
            prepared_political: None,
            prepared_thin_borders: None,
            prepared_rivers_straits: Some(prepared),
            prepared_lighting: None,
            prepared_weather: None,
            prepared_water: None,
            prepared_effects: None,
            prepared_units: None,
            prepared_combat: None,
            prepared_orders: None,
            prepared_rail_supply: None,
            prepared_map_modes: None,
            prepared_map_furniture: None,
            element_diagnostics: Vec::new(),
            consumed_gap_ids: Vec::new(),
        })
    }
}

#[derive(Clone, Copy)]
enum ProgramKind {
    River,
    Strait,
}

fn prepare_program(
    resolver: &GameFileResolver,
    admission: &ValidatedContentAdmission,
    translation_unit: &[(String, crate::shader_compile::EffectFile)],
    effect_file: &str,
    effect_name: &str,
    defines: &[(String, String)],
    kind: ProgramKind,
    texture_quality: u8,
) -> Result<PreparedMapEffectProgram> {
    let parsed = translation_unit
        .last()
        .map(|(_, file)| file)
        .ok_or("XMAP08 effect translation unit is empty")?;
    let effect = parsed
        .effect(effect_name)
        .ok_or_else(|| format!("effective {effect_file} has no Effect named {effect_name}"))?;
    if effect.vertex_shader.is_none() || effect.pixel_shader.is_none() {
        return Err(format!("effective {effect_file}:{effect_name} omits a shader stage").into());
    }
    let vertex_hlsl = crate::shader_compile::build_stage_hlsl_with_defines(
        resolver,
        effect_file,
        effect_name,
        ShaderStage::Vertex,
        defines,
    )?;
    let pixel_hlsl = crate::shader_compile::build_stage_hlsl_with_defines(
        resolver,
        effect_file,
        effect_name,
        ShaderStage::Pixel,
        defines,
    )?;
    let (vertex_bytecode, pixel_bytecode) = compile_program_stages(&vertex_hlsl, &pixel_hlsl)?;
    let stage = crate::shader_compile::merged_effect_translation_unit_stage(
        translation_unit,
        ShaderStage::Pixel,
    );
    let mut samplers = Vec::new();
    for sampler in &stage.samplers {
        let (owner_packet, static_paths): (&str, Vec<String>) = match kind {
            ProgramKind::River => {
                if let Some(path) = selected_river_static_content(&sampler.name, texture_quality)? {
                    ("XMAP08", vec![path])
                } else if let Some((_, owner)) = RIVER_DOWNSTREAM_SAMPLERS
                    .iter()
                    .find(|(name, _)| *name == sampler.name)
                {
                    (*owner, Vec::new())
                } else {
                    return Err(format!(
                        "effective river program introduced unowned sampler {}",
                        sampler.name
                    )
                    .into());
                }
            }
            ProgramKind::Strait => {
                if sampler.name == "DiffuseTexture" {
                    ("XMAP08", vec!["map/terrain/strait.dds".into()])
                } else if let Some((_, owner)) = STRAIT_DOWNSTREAM_SAMPLERS
                    .iter()
                    .find(|(name, _)| *name == sampler.name)
                {
                    (*owner, Vec::new())
                } else {
                    return Err(format!(
                        "effective strait program introduced unowned sampler {}",
                        sampler.name
                    )
                    .into());
                }
            }
        };
        let required = |value: &Option<String>, field: &str| -> Result<String> {
            value.clone().ok_or_else(|| {
                format!("effective sampler {} omits required {field}", sampler.name).into()
            })
        };
        if sampler.mip_map_lod_bias.is_some()
            || sampler.mip_map_min_lod.is_some()
            || sampler.mip_map_max_lod.is_some()
            || sampler.max_anisotropy.is_some()
        {
            return Err(format!(
                "effective sampler {} uses an advanced LOD/anisotropy field outside XMAP08's exact executable descriptor",
                sampler.name
            )
            .into());
        }
        samplers.push(PreparedMapSamplerRequirement {
            name: sampler.name.clone(),
            shader_register: sampler.index,
            resource_type: sampler
                .sampler_type
                .clone()
                .unwrap_or_else(|| "Texture2D".into()),
            mag_filter: required(&sampler.mag_filter, "MagFilter")?,
            min_filter: required(&sampler.min_filter, "MinFilter")?,
            mip_filter: required(&sampler.mip_filter, "MipFilter")?,
            address_u: required(&sampler.address_u, "AddressU")?,
            address_v: required(&sampler.address_v, "AddressV")?,
            owner_packet: owner_packet.into(),
            admitted_static_content: static_paths
                .iter()
                .map(|path| {
                    admission
                        .require_used_content(path)
                        .map_err(|error| {
                            format!("XMAP08 sampler requires admitted {path}: {error}")
                        })
                        .cloned()
                })
                .collect::<std::result::Result<Vec<_>, _>>()?,
        });
    }
    samplers.sort_by_key(|sampler| sampler.shader_register);
    let mut ordered_shader_content = Vec::new();
    for path in translation_unit
        .iter()
        .map(|(path, _)| path.as_str())
        .chain([PLATFORM_HEADER])
    {
        ordered_shader_content.push(
            admission
                .require_used_content(path)
                .map_err(|error| format!("XMAP08 program requires admitted {path}: {error}"))?
                .clone(),
        );
    }
    let selected_state = |selected: &Option<String>,
                          conventional: &str,
                          map: fn(
        &crate::shader_compile::EffectFile,
    ) -> &BTreeMap<String, Vec<(String, String)>>,
                          engine_default: Vec<(String, String)>|
     -> Result<Vec<(String, String)>> {
        let name = selected.as_deref().unwrap_or(conventional);
        if let Some(state) = translation_unit
            .iter()
            .rev()
            .find_map(|(_, file)| map(file).get(name))
        {
            return Ok(state.clone());
        }
        if selected.is_some() {
            return Err(format!(
                "effective {effect_file}:{effect_name} selects missing state {name}"
            )
            .into());
        }
        Ok(engine_default)
    };
    let blend_state = selected_state(
        &effect.blend_state,
        "BlendState",
        |file| &file.blend_states,
        Vec::new(),
    )?;
    let depth_stencil_state = selected_state(
        &effect.depth_stencil_state,
        "DepthStencil",
        |file| &file.depth_stencil_states,
        vec![("DepthEnable".into(), "yes".into())],
    )?;
    let rasterizer_state = selected_state(
        &effect.rasterizer_state,
        "RasterizerState",
        |file| &file.rasterizer_states,
        vec![
            ("FillMode".into(), "FILL_SOLID".into()),
            ("CullMode".into(), "CULL_BACK".into()),
            ("FrontCCW".into(), "no".into()),
        ],
    )?;
    let constant_buffer_owners = match kind {
        ProgramKind::River => vec![
            (0, "XMAP03+XMAP05+XMAP07+XMAP09+XMAP10".into()),
            (1, "XMAP07".into()),
            (2, "XMAP09".into()),
        ],
        ProgramKind::Strait => vec![(0, "XMAP03".into())],
    };
    let mut program = PreparedMapEffectProgram {
        effect_file: effect_file.to_ascii_lowercase(),
        effect_name: effect_name.into(),
        ordered_shader_content,
        vertex_translation_unit_sha256: Sha256::hex(vertex_hlsl.as_bytes()),
        pixel_translation_unit_sha256: Sha256::hex(pixel_hlsl.as_bytes()),
        vertex_bytecode: vertex_bytecode.into(),
        pixel_bytecode: pixel_bytecode.into(),
        vertex_bytecode_sha256: String::new(),
        pixel_bytecode_sha256: String::new(),
        samplers,
        pipeline: PreparedMapPipelineState {
            topology: "D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST".into(),
            index_format: "DXGI_FORMAT_R16_UINT".into(),
            vertex_stride: 32,
            input_layout: vec![
                PreparedMapInputElement {
                    semantic: "POSITION".into(),
                    semantic_index: 0,
                    format: "DXGI_FORMAT_R32G32B32A32_FLOAT".into(),
                    byte_offset: 0,
                },
                PreparedMapInputElement {
                    semantic: "TEXCOORD".into(),
                    semantic_index: 0,
                    format: "DXGI_FORMAT_R32G32B32A32_FLOAT".into(),
                    byte_offset: 16,
                },
            ],
            blend_state,
            depth_stencil_state,
            rasterizer_state,
        },
        constant_buffer_owners,
        deterministic_sha256: String::new(),
    };
    if matches!(kind, ProgramKind::River)
        && reflected_cbuffer_variable_is_used(&program, 1, "ShadowMapTextureMatrix")?
    {
        program
            .constant_buffer_owners
            .iter_mut()
            .find(|(slot, _)| *slot == 1)
            .ok_or("XMAP08 river program omitted constant-buffer slot 1 ownership")?
            .1 = "XMAP07+XMAP09".into();
    }
    program.vertex_bytecode_sha256 = Sha256::hex(&program.vertex_bytecode);
    program.pixel_bytecode_sha256 = Sha256::hex(&program.pixel_bytecode);
    program.deterministic_sha256 = map_program_digest(&program);
    program.validate()?;
    Ok(program)
}

#[cfg(windows)]
fn compile_program_stages(vertex_hlsl: &str, pixel_hlsl: &str) -> Result<(Vec<u8>, Vec<u8>)> {
    Ok((
        crate::d3d_compile_stage(
            vertex_hlsl,
            ShaderStage::Vertex,
            crate::shader_compile::ENGINE_COMPILE_FLAGS,
        )?,
        crate::d3d_compile_stage(
            pixel_hlsl,
            ShaderStage::Pixel,
            crate::shader_compile::ENGINE_COMPILE_FLAGS,
        )?,
    ))
}

#[cfg(not(windows))]
fn compile_program_stages(_vertex_hlsl: &str, _pixel_hlsl: &str) -> Result<(Vec<u8>, Vec<u8>)> {
    Err("XMAP08 executable shader handoff requires Windows D3DCompile".into())
}

fn parse_adjacencies(source: &str) -> Result<Vec<PreparedStraitAdjacency>> {
    let mut rows = Vec::new();
    for (line_index, line) in source.lines().enumerate().skip(1) {
        let line = line.trim_end_matches('\r');
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        let fields = line.split(';').collect::<Vec<_>>();
        if fields.len() < 9 {
            return Err(format!("adjacencies.csv line {} is truncated", line_index + 1).into());
        }
        let parse = |field: usize, label: &str| -> Result<i32> {
            fields[field].trim().parse::<i32>().map_err(|error| {
                format!("adjacencies.csv line {} {label}: {error}", line_index + 1).into()
            })
        };
        let from = parse(0, "From")?;
        let to = parse(1, "To")?;
        if from < 0 {
            break;
        }
        let kind = match fields[2].trim() {
            "sea" => StraitAdjacencyKind::Sea,
            "river" => StraitAdjacencyKind::River,
            "river_large" => StraitAdjacencyKind::RiverLarge,
            "impassable" => StraitAdjacencyKind::Impassable,
            _ => StraitAdjacencyKind::Unspecified,
        };
        rows.push(PreparedStraitAdjacency {
            from,
            to,
            kind,
            through: parse(3, "Through")?,
            start: [parse(4, "start_x")?, parse(5, "start_y")?],
            stop: [parse(6, "stop_x")?, parse(7, "stop_y")?],
            rule: fields[8].trim().to_string(),
        });
    }
    Ok(rows)
}

fn parse_adjacency_rule_names(source: &str) -> Result<Vec<String>> {
    let stripped = crate::strip_comments(source);
    let mut names = Vec::new();
    let mut cursor = 0usize;
    while let Some(relative) = stripped[cursor..].find("adjacency_rule") {
        let start = cursor + relative + "adjacency_rule".len();
        let Some(open_relative) = stripped[start..].find('{') else {
            return Err("adjacency_rules.txt has an unterminated adjacency_rule".into());
        };
        let open = start + open_relative;
        let mut depth = 0i32;
        let mut close = None;
        for (relative, byte) in stripped.as_bytes()[open..].iter().enumerate() {
            match byte {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        close = Some(open + relative);
                        break;
                    }
                }
                _ => {}
            }
        }
        let close = close.ok_or("adjacency_rules.txt has an unbalanced adjacency_rule")?;
        let body = &stripped[open + 1..close];
        let assignment =
            crate::find_assignment(body, "name", 0).ok_or("adjacency_rule omits required name")?;
        let rest = body[assignment..].trim_start();
        let name = if let Some(rest) = rest.strip_prefix('"') {
            rest.split_once('"')
                .map(|(name, _)| name)
                .ok_or("adjacency_rule has unterminated quoted name")?
        } else {
            rest.split_whitespace()
                .next()
                .ok_or("adjacency_rule has blank name")?
        };
        if name.is_empty() || names.iter().any(|existing| existing == name) {
            return Err(format!("adjacency_rules.txt has blank or duplicate rule {name:?}").into());
        }
        names.push(name.to_string());
        cursor = close + 1;
    }
    names.sort();
    Ok(names)
}

fn validate_adjacency_rule_references(
    rows: &[PreparedStraitAdjacency],
    rule_names: &[String],
) -> Result<()> {
    let names = rule_names
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    for row in rows {
        if !row.rule.is_empty() && !names.contains(row.rule.as_str()) {
            return Err(format!("Invalid adjacency rule name: {}", row.rule).into());
        }
    }
    Ok(())
}

fn synthesize_strait_mesh(
    rows: &[PreparedStraitAdjacency],
    definitions: &BTreeMap<u32, crate::ProvinceDefinition>,
    provinces: &crate::BmpIds,
) -> Result<(Vec<u8>, Vec<u8>)> {
    let mut pixels_by_province: HashMap<u32, Vec<[i32; 2]>> = HashMap::new();
    for (index, province) in provinces.ids.iter().copied().enumerate() {
        pixels_by_province.entry(province).or_default().push([
            (index % provinces.width) as i32,
            (provinces.height - 1 - index / provinces.width) as i32,
        ]);
    }
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    for row in rows {
        if row.kind != StraitAdjacencyKind::Sea || row.through == 0 {
            continue;
        }
        let from = u32::try_from(row.from)?;
        let to = u32::try_from(row.to)?;
        let Some(from_definition) = definitions.get(&from) else {
            return Err(format!("strait adjacency references unknown province {from}").into());
        };
        let Some(to_definition) = definitions.get(&to) else {
            return Err(format!("strait adjacency references unknown province {to}").into());
        };
        if from_definition.province_type != "land" || to_definition.province_type != "land" {
            continue;
        }
        let (first_id, second_id, first_explicit, second_explicit) = if from >= to {
            (from, to, row.start, row.stop)
        } else {
            (to, from, row.stop, row.start)
        };
        let (mut first, mut second) = (
            valid_explicit_point(first_explicit),
            valid_explicit_point(second_explicit),
        );
        if first.is_none() || second.is_none() {
            let first_pixels = pixels_by_province
                .get(&first_id)
                .ok_or_else(|| format!("strait province {first_id} has no raster pixels"))?;
            let second_pixels = pixels_by_province
                .get(&second_id)
                .ok_or_else(|| format!("strait province {second_id} has no raster pixels"))?;
            let (nearest_first, nearest_second) = nearest_pixel_pair(first_pixels, second_pixels);
            first.get_or_insert(nearest_first);
            second.get_or_insert(nearest_second);
        }
        append_strait_quad(
            &mut vertices,
            &mut indices,
            first.ok_or("XMAP08 strait first endpoint remained unresolved")?,
            second.ok_or("XMAP08 strait second endpoint remained unresolved")?,
        )?;
    }
    Ok((vertices, indices))
}

fn valid_explicit_point(point: [i32; 2]) -> Option<[i32; 2]> {
    (point[0] >= 0 && point[1] >= 0).then_some(point)
}

fn nearest_pixel_pair(first: &[[i32; 2]], second: &[[i32; 2]]) -> ([i32; 2], [i32; 2]) {
    let mut best = (i64::MAX, first[0], second[0]);
    for &a in first {
        for &b in second {
            let dx = i64::from(a[0]) - i64::from(b[0]);
            let dy = i64::from(a[1]) - i64::from(b[1]);
            let distance = dx * dx + dy * dy;
            if distance < best.0 {
                best = (distance, a, b);
            }
        }
    }
    (best.1, best.2)
}

fn append_strait_quad(
    vertices: &mut Vec<u8>,
    indices: &mut Vec<u8>,
    first: [i32; 2],
    second: [i32; 2],
) -> Result<()> {
    let base = u16::try_from(vertices.len() / 32)
        .map_err(|_| "XMAP08 strait mesh exceeds the engine u16 vertex domain")?;
    let dx = (second[0] - first[0]) as f32;
    let dz = (second[1] - first[1]) as f32;
    let length = (dx * dx + dz * dz).sqrt();
    if !length.is_finite() || length == 0.0 {
        return Err("XMAP08 strait endpoints collapse to one pixel".into());
    }
    let normal = [-dz / length, dx / length];
    let half_width = 2.0f32;
    let repeat = (length / 4.0).ceil();
    let positions = [
        [
            first[0] as f32 - normal[0] * half_width,
            9.5,
            first[1] as f32 - normal[1] * half_width,
        ],
        [
            first[0] as f32 + normal[0] * half_width,
            9.5,
            first[1] as f32 + normal[1] * half_width,
        ],
        [
            second[0] as f32 - normal[0] * half_width,
            9.5,
            second[1] as f32 - normal[1] * half_width,
        ],
        [
            second[0] as f32 + normal[0] * half_width,
            9.5,
            second[1] as f32 + normal[1] * half_width,
        ],
    ];
    let uvs = [[0.0, 0.0], [1.0, 0.0], [0.0, repeat], [1.0, repeat]];
    for (position, uv) in positions.into_iter().zip(uvs) {
        for value in [
            position[0],
            position[1],
            position[2],
            0.0,
            uv[0],
            uv[1],
            0.0,
            0.0,
        ] {
            vertices.extend_from_slice(&value.to_le_bytes());
        }
    }
    for index in [base, base + 1, base + 3, base, base + 3, base + 2] {
        indices.extend_from_slice(&index.to_le_bytes());
    }
    Ok(())
}

fn enabled_render_views(
    camera: &ViewerMapCameraState,
) -> Result<Vec<(u8, f32, TerrainRenderViewFrustum)>> {
    let product_camera = camera.production_camera()?;
    let viewport_width = camera.viewport_width as f32;
    let viewport_height = camera.viewport_height as f32;
    let mut footprint_x = Vec::with_capacity(4);
    for (screen_x, screen_y) in [
        (0.0, 0.0),
        (viewport_width, 0.0),
        (0.0, viewport_height),
        (viewport_width, viewport_height),
    ] {
        let (world_x, _) = product_camera
            .screen_to_ground_y0(screen_x, screen_y, viewport_width, viewport_height)?
            .ok_or("XMAP08 camera corner misses the map plane")?;
        footprint_x.push(world_x);
    }
    let min_x = footprint_x.iter().copied().fold(f32::INFINITY, f32::min);
    let max_x = footprint_x
        .iter()
        .copied()
        .fold(f32::NEG_INFINITY, f32::max);
    let map_width = camera.map_width as f32;
    let wrap_offsets = map_wrap_offsets(min_x, max_x, map_width);
    let projection =
        ProductionMapCameraProjection::from_engine_constants(viewport_width / viewport_height)?;
    let shifted_frustum = |camera_x_shift: f32| -> Result<TerrainRenderViewFrustum> {
        let mut eye = camera.current_eye();
        let mut look_at = camera.current_look_at();
        eye[0] += camera_x_shift;
        look_at[0] += camera_x_shift;
        let shifted = projection.camera_from_eye_look_at(eye, look_at)?;
        TerrainRenderViewFrustum::from_view_projection_matrix(shifted.view_projection_matrix)
    };
    let mut out = vec![(
        1,
        0.0,
        TerrainRenderViewFrustum::from_view_projection_matrix(
            product_camera.view_projection_matrix,
        )?,
    )];
    if wrap_offsets.contains(&map_width) {
        out.push((2, map_width, shifted_frustum(-map_width)?));
    }
    if wrap_offsets.contains(&-map_width) {
        out.push((4, -map_width, shifted_frustum(map_width)?));
    }
    Ok(out)
}

fn required_bytes<'a>(bytes: &'a BTreeMap<String, Vec<u8>>, path: &str) -> Result<&'a [u8]> {
    bytes
        .get(&path.to_ascii_lowercase())
        .map(Vec::as_slice)
        .ok_or_else(|| format!("XMAP08 omitted admitted {path}").into())
}

fn latin1(bytes: &[u8]) -> String {
    crate::latin1_bytes_to_string(bytes)
}

fn validate_sha256(value: &str, label: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        || value.bytes().all(|byte| byte == b'0')
    {
        return Err(format!("{label} is not a canonical nonzero SHA-256 identity").into());
    }
    Ok(())
}

fn river_index_payload_matches(draw: &PreparedRiverDraw) -> bool {
    let segment_count = draw.vertex_count / 2;
    if segment_count > (u16::MAX as u32 - 3) / 2 {
        return false;
    }
    let mut actual = draw.indices.chunks_exact(2);
    for segment in 0..segment_count {
        let index = (segment * 2) as u16;
        for expected in [index, index + 2, index + 1, index + 2, index + 3, index + 1] {
            let Some(bytes) = actual.next() else {
                return false;
            };
            if u16::from_le_bytes([bytes[0], bytes[1]]) != expected {
                return false;
            }
        }
    }
    actual.remainder().is_empty() && actual.next().is_none()
}

fn strait_index_payload_matches(indices: &[u8], vertex_count: usize) -> bool {
    if vertex_count % 4 != 0 || vertex_count > u16::MAX as usize {
        return false;
    }
    let mut actual = indices.chunks_exact(2);
    for quad in 0..vertex_count / 4 {
        let base = (quad * 4) as u16;
        for expected in [base, base + 1, base + 3, base, base + 3, base + 2] {
            let Some(bytes) = actual.next() else {
                return false;
            };
            if u16::from_le_bytes([bytes[0], bytes[1]]) != expected {
                return false;
            }
        }
    }
    actual.remainder().is_empty() && actual.next().is_none()
}

fn map_program_digest(program: &PreparedMapEffectProgram) -> String {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(program.effect_file.as_bytes());
    bytes.extend_from_slice(program.effect_name.as_bytes());
    bytes.extend_from_slice(format!("{:?}", program.ordered_shader_content).as_bytes());
    bytes.extend_from_slice(program.vertex_translation_unit_sha256.as_bytes());
    bytes.extend_from_slice(program.pixel_translation_unit_sha256.as_bytes());
    bytes.extend_from_slice(&program.vertex_bytecode);
    bytes.extend_from_slice(&program.pixel_bytecode);
    bytes.extend_from_slice(format!("{:?}", program.samplers).as_bytes());
    bytes.extend_from_slice(format!("{:?}", program.pipeline).as_bytes());
    bytes.extend_from_slice(format!("{:?}", program.constant_buffer_owners).as_bytes());
    Sha256::hex(&bytes)
}

fn prepared_frame_digest(frame: &PreparedRiversStraitsFrame) -> String {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(frame.schema.as_bytes());
    bytes.extend_from_slice(&frame.map_width.to_le_bytes());
    bytes.extend_from_slice(&frame.map_height.to_le_bytes());
    bytes.extend_from_slice(frame.camera_view_sha256.as_bytes());
    bytes.extend_from_slice(frame.river_assignment.assignment_sha256.as_bytes());
    bytes.extend_from_slice(frame.strait_assignment.assignment_sha256.as_bytes());
    bytes.extend_from_slice(format!("{:?}", frame.ordered_used_content).as_bytes());
    bytes.extend_from_slice(&frame.river_vertices);
    bytes.extend_from_slice(format!("{:?}", frame.river_draws).as_bytes());
    bytes.extend_from_slice(frame.river_enumerator_witness_sha256.as_bytes());
    bytes.extend_from_slice(&frame.strait_vertices);
    bytes.extend_from_slice(&frame.strait_indices);
    bytes.extend_from_slice(format!("{:?}", frame.strait_draws).as_bytes());
    bytes.extend_from_slice(frame.strait_enumerator_witness_sha256.as_bytes());
    bytes.extend_from_slice(format!("{:?}", frame.strait_adjacencies).as_bytes());
    bytes.extend_from_slice(format!("{:?}", frame.adjacency_rule_names).as_bytes());
    bytes.extend_from_slice(frame.river_program.deterministic_sha256.as_bytes());
    bytes.extend_from_slice(frame.strait_program.deterministic_sha256.as_bytes());
    bytes.extend_from_slice(format!("{:?}", frame.required_downstream_packets).as_bytes());
    Sha256::hex(&bytes)
}

fn component_enumerator_witness(frame: &PreparedRiversStraitsFrame, river: bool) -> String {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(if river {
        b"hoi4.xmap08.river-enumerator.v1\0"
    } else {
        b"hoi4.xmap08.strait-enumerator.v1\0"
    });
    bytes.extend_from_slice(&frame.map_width.to_le_bytes());
    bytes.extend_from_slice(&frame.map_height.to_le_bytes());
    bytes.extend_from_slice(frame.camera_view_sha256.as_bytes());
    bytes.extend_from_slice(format!("{:?}", frame.ordered_used_content).as_bytes());
    if river {
        bytes.extend_from_slice(&frame.river_vertices);
        bytes.extend_from_slice(format!("{:?}", frame.river_draws).as_bytes());
        bytes.extend_from_slice(frame.river_program.deterministic_sha256.as_bytes());
    } else {
        bytes.extend_from_slice(&frame.strait_vertices);
        bytes.extend_from_slice(&frame.strait_indices);
        bytes.extend_from_slice(format!("{:?}", frame.strait_draws).as_bytes());
        bytes.extend_from_slice(format!("{:?}", frame.strait_adjacencies).as_bytes());
        bytes.extend_from_slice(frame.strait_program.deterministic_sha256.as_bytes());
    }
    Sha256::hex(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    use replay_core::{
        ContentLayerKind, ContentManifest, LayerStorageKind, RecordedContentLayer, RequiredContent,
        RequiredContentKind, SourceGameIdentity, SourceProfileIdentity,
    };

    #[test]
    fn river_texture_quality_selects_one_exact_engine_variant() {
        for quality in 0..=2 {
            assert_eq!(
                selected_river_static_content("Diffuse", quality).unwrap(),
                Some(format!("map/terrain/RiverSurface_diffuse_{quality}.dds"))
            );
            assert_eq!(
                selected_river_static_content("NormalMap", quality).unwrap(),
                Some(format!("map/terrain/RiverSurface_normal_{quality}.dds"))
            );
            assert_eq!(
                selected_river_static_content("RiverData", quality).unwrap(),
                Some("map/terrain/RiverSurface_masks.dds".into())
            );
        }
        assert!(selected_river_static_content("Diffuse", 3).is_err());
    }

    #[test]
    fn assigned_stage_contract_exports_only_the_evidence_owned_internal_edge() {
        let [rivers, straits] = xmap08_assigned_stage_identities();
        assert_eq!(
            (rivers.stage_name.as_str(), rivers.operation_name.as_str()),
            (XMAP08_RIVER_STAGE_NAME, XMAP08_RIVER_OPERATION_ID)
        );
        assert_eq!(
            (straits.stage_name.as_str(), straits.operation_name.as_str()),
            (XMAP08_STRAIT_STAGE_NAME, XMAP08_STRAIT_OPERATION_ID)
        );
        let [edge] = xmap08_assigned_stage_edges();
        assert_eq!(edge.before, rivers);
        assert_eq!(edge.after, straits);
    }

    #[test]
    fn visible_static_binding_rejects_substituted_texture_hash() {
        let admitted = UsedContentKey {
            effective_vfs_manifest_sha256: "a".repeat(64),
            normalized_game_relative_path: "map/terrain/riversurface_diffuse_0.dds".into(),
            winning_layer_ordinal: 0,
            layer_kind: portable_content_manifest::ContentLayerKind::Base,
            layer_id: "base".into(),
            source_sha256: "1".repeat(64),
            payload_sha256: "1".repeat(64),
            payload_bytes: 64,
        };
        let sampler = PreparedMapSamplerRequirement {
            name: "Diffuse".into(),
            shader_register: 1,
            resource_type: "Texture2D".into(),
            mag_filter: "Linear".into(),
            min_filter: "Linear".into(),
            mip_filter: "Linear".into(),
            address_u: "Wrap".into(),
            address_v: "Clamp".into(),
            owner_packet: "XMAP08".into(),
            admitted_static_content: vec![admitted],
        };
        let mut binding = MapDependentResourceBinding {
            effect_name: "river".into(),
            symbol: "Diffuse".into(),
            owner_packet: "XMAP08".into(),
            resource_sha256: "1".repeat(64),
            descriptor_sha256: "2".repeat(64),
        };
        validate_visible_resource_binding("river", &sampler, &binding).unwrap();
        binding.resource_sha256 = "3".repeat(64);
        assert!(
            validate_visible_resource_binding("river", &sampler, &binding)
                .unwrap_err()
                .to_string()
                .contains("does not match its admitted payload")
        );
    }

    #[cfg(windows)]
    fn content_layer_kind(kind: &str) -> ContentLayerKind {
        match kind {
            "integrated_dlc" => ContentLayerKind::IntegratedDlc,
            "dlc" => ContentLayerKind::Dlc,
            "mod" => ContentLayerKind::Mod,
            _ => ContentLayerKind::Base,
        }
    }

    #[cfg(windows)]
    fn admitted_program_inputs(
        resolver: &GameFileResolver,
        paths: &BTreeSet<String>,
    ) -> (GameFileResolver, ValidatedContentAdmission) {
        let effective_vfs_manifest_sha256 = "ab".repeat(32);
        let required_content = paths
            .iter()
            .enumerate()
            .map(|(index, path)| {
                let actual = resolver
                    .resolve_required_with_provenance(path, "XMAP08 executable program gate")
                    .unwrap();
                RequiredContent {
                    content_key: format!("xmap08-program-{index}"),
                    kind: if path.ends_with(".dds") {
                        RequiredContentKind::Texture
                    } else {
                        RequiredContentKind::ShaderSource
                    },
                    content: UsedContentKey {
                        effective_vfs_manifest_sha256: effective_vfs_manifest_sha256.clone(),
                        normalized_game_relative_path: actual.logical_path.to_ascii_lowercase(),
                        winning_layer_ordinal: actual.source.ordinal as u32,
                        layer_kind: content_layer_kind(&actual.source.kind),
                        layer_id: actual.source.source_id,
                        source_sha256: actual
                            .source
                            .archive_sha256
                            .unwrap_or_else(|| actual.sha256.clone()),
                        payload_sha256: actual.sha256,
                        payload_bytes: actual.byte_len,
                    },
                    observation_count: 1,
                    consumers: vec!["XMAP08".into()],
                }
            })
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
                layer_kind: content_layer_kind(&source.kind),
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
            .collect();
        let source_game = SourceGameIdentity {
            version: "1.19.2.0".into(),
            build: "a729".into(),
            executable_sha256: "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
                .into(),
            build_profile: "xmap08-executable-gate".into(),
        };
        let mut manifest = ContentManifest {
            schema: replay_core::PORTABLE_CONTENT_MANIFEST_SCHEMA.into(),
            product_modset_sha256: "99".repeat(32),
            source_game: source_game.clone(),
            source_profile: SourceProfileIdentity {
                profile_id: "xmap08-executable-gate".into(),
                playset_id: "vanilla".into(),
                selection_sha256: "00".repeat(32),
            },
            effective_vfs_manifest_sha256,
            ordered_layers,
            dlc_selection: Vec::new(),
            localisation_language: "l_english".into(),
            required_settings: BTreeMap::new(),
            recorder_local_settings: BTreeMap::new(),
            required_content,
            manifest_digest_sha256: None,
        };
        manifest.source_profile.selection_sha256 = manifest.computed_selection_sha256().unwrap();
        manifest = manifest.with_computed_digest().unwrap();
        let inventory = resolver
            .playback_inventory(
                &manifest,
                crate::PlaybackEnvironment {
                    source_game,
                    source_profile: manifest.source_profile.clone(),
                    dlc_selection: Vec::new(),
                    localisation_language: "l_english".into(),
                    settings: BTreeMap::new(),
                },
            )
            .unwrap();
        let admission = manifest.admit_playback(&inventory).unwrap();
        (
            resolver.clone().with_content_admission(&admission).unwrap(),
            admission,
        )
    }

    #[cfg(windows)]
    #[test]
    #[cfg_attr(
        not(gate_hoi4_install),
        ignore = "the game install is required for executable XMAP08 shader gates"
    )]
    fn gate_vanilla_xmap08_river_and_strait_program_variants_construct() {
        let resolver =
            GameFileResolver::vanilla(replay_test_gates::require_vanilla_game_root()).unwrap();
        let river_tu =
            crate::shader_compile::effect_translation_unit_files(&resolver, RIVER_EFFECT).unwrap();
        let strait_tu =
            crate::shader_compile::effect_translation_unit_files(&resolver, STRAIT_EFFECT).unwrap();
        let mut paths = river_tu
            .iter()
            .chain(&strait_tu)
            .map(|(path, _)| path.clone())
            .collect::<BTreeSet<_>>();
        paths.insert(PLATFORM_HEADER.into());
        for sampler in [
            "Diffuse",
            "NormalMap",
            "RiverData",
            "CityLightsAndSnowNoise",
        ] {
            paths.insert(selected_river_static_content(sampler, 0).unwrap().unwrap());
        }
        paths.insert("map/terrain/strait.dds".into());
        let (resolver, admission) = admitted_program_inputs(&resolver, &paths);
        let defines = crate::engine_map_shader_defines(
            crate::derive_a729_engine_map_shader_constants(5632, 2048, false, 0).unwrap(),
        );

        for effect_name in ["river", "river_low_gfx"] {
            let program = prepare_program(
                &resolver,
                &admission,
                &river_tu,
                RIVER_EFFECT,
                effect_name,
                &defines,
                ProgramKind::River,
                0,
            )
            .unwrap();
            program.validate().unwrap();
            let cb0_reflection = [
                crate::parse_dxbc_reflection(&program.vertex_bytecode).unwrap(),
                crate::parse_dxbc_reflection(&program.pixel_bytecode).unwrap(),
            ]
            .into_iter()
            .map(|reflection| {
                let names = reflection
                    .resources
                    .iter()
                    .filter(|resource| resource.resource_type == 0 && resource.bind_point == 0)
                    .map(|resource| resource.name.as_str())
                    .collect::<BTreeSet<_>>();
                reflection
                    .cbuffers
                    .into_iter()
                    .filter(|buffer| names.contains(buffer.name.as_str()))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
            let cb0_layouts = cb0_reflection
                .iter()
                .map(|stage| {
                    assert_eq!(stage.len(), 1);
                    let buffer = &stage[0];
                    assert_eq!(buffer.name, "dx11_cb0");
                    assert_eq!(buffer.size, 416);
                    (
                        buffer.name.as_str(),
                        buffer.size,
                        buffer
                            .vars
                            .iter()
                            .map(|variable| {
                                (variable.name.as_str(), variable.offset, variable.size)
                            })
                            .collect::<Vec<_>>(),
                    )
                })
                .collect::<Vec<_>>();
            assert_eq!(cb0_layouts[0], cb0_layouts[1]);
            let reflected = reflected_resource_requirements(&program)
                .unwrap()
                .into_iter()
                .map(|(_, sampler)| {
                    (
                        sampler.name.as_str(),
                        sampler.shader_register,
                        sampler.owner_packet.as_str(),
                    )
                })
                .collect::<Vec<_>>();
            let expected = if effect_name == "river" {
                vec![
                    ("WaterColor", 0, "XMAP07"),
                    ("Diffuse", 1, "XMAP08"),
                    ("NormalMap", 2, "XMAP08"),
                    ("RiverData", 3, "XMAP08"),
                    ("LeanTexture1", 4, "XMAP07"),
                    ("LeanTexture2", 5, "XMAP07"),
                    ("SnowMudTexture", 8, "XMAP07"),
                    ("CityLightsAndSnowNoise", 9, "XMAP08"),
                    ("LightIndexMap", 10, "XMAP09"),
                    ("LightDataMap", 11, "XMAP09"),
                    ("GradientBorderChannel1", 12, "XMAP05"),
                    ("GradientBorderChannel2", 13, "XMAP05"),
                    ("ReflectionCubeMap", 14, "XMAP07"),
                    ("ShadowMap", 15, "XMAP10"),
                ]
            } else {
                vec![
                    ("WaterColor", 0, "XMAP07"),
                    ("LeanTexture1", 4, "XMAP07"),
                    ("LeanTexture2", 5, "XMAP07"),
                    ("ReflectionCubeMap", 14, "XMAP07"),
                    ("ShadowMap", 15, "XMAP10"),
                ]
            };
            assert_eq!(reflected, expected);
            assert_eq!(
                reflected_constant_buffer_requirements(&program).unwrap(),
                if effect_name == "river" {
                    vec![
                        ("river", 0, "XMAP03+XMAP05+XMAP07+XMAP09+XMAP10"),
                        ("river", 1, "XMAP07"),
                        ("river", 2, "XMAP09"),
                    ]
                } else {
                    vec![
                        ("river_low_gfx", 0, "XMAP03+XMAP05+XMAP07+XMAP09+XMAP10"),
                        ("river_low_gfx", 1, "XMAP07"),
                    ]
                }
            );
            assert_eq!(
                reflected_downstream_packets(&program).unwrap(),
                ["XMAP05", "XMAP07", "XMAP09", "XMAP10"]
            );
            assert!(
                !reflected_cbuffer_variable_is_used(&program, 1, "ShadowMapTextureMatrix").unwrap()
            );
            assert_eq!(
                program
                    .samplers
                    .iter()
                    .find(|sampler| sampler.name == "Diffuse")
                    .unwrap()
                    .admitted_static_content[0]
                    .normalized_game_relative_path,
                "map/terrain/riversurface_diffuse_0.dds"
            );
        }
        let strait = prepare_program(
            &resolver,
            &admission,
            &strait_tu,
            STRAIT_EFFECT,
            "strait",
            &defines,
            ProgramKind::Strait,
            0,
        )
        .unwrap();
        strait.validate().unwrap();
        assert_eq!(
            reflected_resource_requirements(&strait)
                .unwrap()
                .into_iter()
                .map(|(_, sampler)| (
                    sampler.name.as_str(),
                    sampler.shader_register,
                    sampler.owner_packet.as_str(),
                ))
                .collect::<Vec<_>>(),
            [("DiffuseTexture", 0, "XMAP08")]
        );
        assert_eq!(
            reflected_constant_buffer_requirements(&strait).unwrap(),
            [("strait", 0, "XMAP03")]
        );
        assert!(
            strait
                .samplers
                .iter()
                .filter(|sampler| {
                    matches!(sampler.name.as_str(), "FoWTexture" | "FoWDiffuse")
                        && sampler.owner_packet == "XMAP05"
                })
                .count()
                == 2
        );
    }

    #[cfg(windows)]
    #[test]
    #[cfg_attr(
        not(gate_hoi4_install),
        ignore = "the game install is required for the executable XMAP08 D3D gate"
    )]
    fn gate_vanilla_xmap08_real_target_rejects_straits_before_rivers() {
        let resolver =
            GameFileResolver::vanilla(replay_test_gates::require_vanilla_game_root()).unwrap();
        let river_tu =
            crate::shader_compile::effect_translation_unit_files(&resolver, RIVER_EFFECT).unwrap();
        let strait_tu =
            crate::shader_compile::effect_translation_unit_files(&resolver, STRAIT_EFFECT).unwrap();
        let mut paths = CORE_FILE_CONTENT
            .into_iter()
            .map(str::to_owned)
            .chain(
                river_tu
                    .iter()
                    .chain(&strait_tu)
                    .map(|(path, _)| path.clone()),
            )
            .collect::<BTreeSet<_>>();
        for sampler in [
            "Diffuse",
            "NormalMap",
            "RiverData",
            "CityLightsAndSnowNoise",
        ] {
            paths.insert(selected_river_static_content(sampler, 0).unwrap().unwrap());
        }
        let (resolver, admission) = admitted_program_inputs(&resolver, &paths);
        let content = Xmap08Content::admit(&resolver, &admission, true, 0).unwrap();
        let camera = ViewerMapCameraState::from_admitted_content(
            &resolver,
            &admission,
            content.map_width,
            content.map_height,
            64,
            64,
            50.0,
        )
        .unwrap();
        let topology = portable_render_stream::AssignedStageTopology::seal(
            xmap08_assigned_stage_identities().into(),
            xmap08_assigned_stage_edges().into(),
        )
        .unwrap();
        let schedule =
            portable_render_stream::AssignedStageSchedule::from_topology(&topology).unwrap();
        let river_assignment = schedule
            .require(XMAP08_RIVER_STAGE_NAME, XMAP08_RIVER_OPERATION_ID)
            .unwrap()
            .clone();
        let strait_assignment = schedule
            .require(XMAP08_STRAIT_STAGE_NAME, XMAP08_STRAIT_OPERATION_ID)
            .unwrap()
            .clone();
        let frame = content
            .prepare(&camera, river_assignment, strait_assignment)
            .unwrap();
        assert!(!frame.strait_draws.is_empty());
        let viewer = crate::ViewerOwnedMapState {
            viewport_width: 64,
            viewport_height: 64,
            camera: Some(camera),
            observer_country_id: None,
            map_mode_id: "viewer.map_mode.unselected".into(),
            map_mode_selection_epoch: 0,
            selected_object_ids: Vec::new(),
            input_revision: 0,
            animation_time_f32_bits: 0.0f32.to_bits(),
            frame_delta_seconds_f32_bits: 0.0f32.to_bits(),
        };
        let envelope = crate::FoundationRenderEnvelope {
            schema: portable_render_stream::FOUNDATION_RENDER_SCHEMA.into(),
            hour_index: 1,
            content_manifest_sha256: "b".repeat(64),
            product_modset_sha256: "c".repeat(64),
            effective_vfs_manifest_sha256: "d".repeat(64),
            foundation_content_operation_sha256: "e".repeat(64),
            families: MapVisualFamily::ALL
                .into_iter()
                .map(|family| portable_render_stream::FamilyCoverageRow {
                    family,
                    owner_packet: family.owner_packet().into(),
                    disposition: FamilyDisposition::NotApplicable {
                        applicability_witness_sha256: "f".repeat(64),
                        reason: "XMAP08 hardware gate fixture".into(),
                    },
                })
                .collect(),
            element_diagnostics: Vec::new(),
        };

        unsafe {
            let device = crate::d3d11_device::native_d3d11_create_device().unwrap();
            let target_desc = crate::ProductionMainSceneTargetDesc {
                color_attachment_ordinal: 0,
                color_width: 64,
                color_height: 64,
                color_format: winapi::shared::dxgiformat::DXGI_FORMAT_R16G16B16A16_FLOAT,
                color_samples: 1,
                color_quality: 0,
                depth_attachment_ordinal: 1,
                depth_width: 64,
                depth_height: 64,
                depth_format: winapi::shared::dxgiformat::DXGI_FORMAT_D32_FLOAT,
                depth_samples: 1,
                depth_quality: 0,
            };
            let target = crate::NativeProductionMainSceneTarget::create_on_device(
                device.device.as_raw(),
                target_desc.clone(),
                frame.deterministic_sha256.clone(),
                1,
            )
            .unwrap();
            let authority = crate::Xmap08ExecutionFrameAuthority::from_test_identities(
                &envelope,
                &viewer,
                &target_desc,
                "8".repeat(64),
                "9".repeat(64),
                &frame,
            )
            .unwrap();
            let mut handoff = crate::NativeXmap08RiversStraitsHandoff::create_on_device(
                device.device.as_raw(),
                &resolver,
                &frame,
            )
            .unwrap();
            let error = handoff
                .execute_straits(device.context.as_raw(), &target, &viewer, &authority, None)
                .unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("straits cannot execute before the river pass completes"),
                "unexpected XMAP08 sequencing failure: {error}"
            );
            (*device.context.as_raw()).Flush();
            let removed = (*device.device.as_raw()).GetDeviceRemovedReason();
            assert!(
                removed >= 0,
                "XMAP08 D3D device removed: 0x{:08x}",
                removed as u32
            );
        }
    }

    #[test]
    fn adjacency_parser_preserves_type_zero_and_unknown_as_engine_zero() {
        let rows = parse_adjacencies(
            "From;To;Type;Through;start_x;start_y;stop_x;stop_y;adjacency_rule_name;Comment\n\
             1;2;;3;4;5;6;7;;blank\n\
             2;3;future_mod_type;4;-1;-1;-1;-1;;unknown\n\
             3;4;sea;5;1;2;3;4;;sea\n\
             -1;-1;;-1;-1;-1;-1;-1;;\n",
        )
        .unwrap();
        assert_eq!(rows[0].kind, StraitAdjacencyKind::Unspecified);
        assert_eq!(rows[1].kind, StraitAdjacencyKind::Unspecified);
        assert_eq!(rows[2].kind, StraitAdjacencyKind::Sea);
    }

    #[test]
    fn adjacency_rule_parser_validates_references() {
        let names = parse_adjacency_rule_names(
            "adjacency_rule = { name = \"SUEZ_CANAL\" required_provinces = { 1 2 } }\n\
             adjacency_rule = { name = GIBRALTAR_STRAIT }\n",
        )
        .unwrap();
        assert_eq!(names, ["GIBRALTAR_STRAIT", "SUEZ_CANAL"]);
        let rows = vec![PreparedStraitAdjacency {
            from: 1,
            to: 2,
            through: 3,
            kind: StraitAdjacencyKind::Sea,
            start: [-1, -1],
            stop: [-1, -1],
            rule: "SUEZ_CANAL".into(),
        }];
        validate_adjacency_rule_references(&rows, &names).unwrap();
    }

    #[test]
    fn strait_quad_matches_exact_a729_layout_and_indices() {
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        append_strait_quad(&mut vertices, &mut indices, [10, 20], [18, 20]).unwrap();
        assert_eq!(vertices.len(), 4 * 32);
        assert_eq!(indices.len(), 6 * 2);
        assert_eq!(
            indices,
            [0u16, 1, 3, 0, 3, 2]
                .into_iter()
                .flat_map(u16::to_le_bytes)
                .collect::<Vec<_>>()
        );
        let f32_at =
            |offset: usize| f32::from_le_bytes(vertices[offset..offset + 4].try_into().unwrap());
        assert_eq!(f32_at(4), 9.5);
        assert_eq!(f32_at(16), 0.0);
        assert_eq!(f32_at(48), 1.0);
        assert_eq!(f32_at(84), 2.0);
    }

    #[test]
    fn river_indices_are_relative_to_the_draw_base_vertex() {
        let indices = [0u16, 2, 1, 2, 3, 1, 2, 4, 3, 4, 5, 3]
            .into_iter()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>();
        let draw = PreparedRiverDraw {
            render_view_mask: 1,
            map_copy_offset_f32_bits: 0.0f32.to_bits(),
            first_vertex: 70_000,
            vertex_count: 4,
            index_count: 12,
            base_vertex: 70_000,
            render_group: 0,
            starts_at_source: true,
            bounds_f32_bits: [[0; 3]; 2],
            indices_sha256: Sha256::hex(&indices),
            indices: indices.into(),
        };
        assert!(river_index_payload_matches(&draw));
    }

    #[test]
    fn exact_empty_map_objects_are_representable() {
        let rows = parse_adjacencies(
            "From;To;Type;Through;start_x;start_y;stop_x;stop_y;adjacency_rule_name;Comment\n\
             -1;-1;;-1;-1;-1;-1;-1;;\n",
        )
        .unwrap();
        assert!(rows.is_empty());
    }
}
