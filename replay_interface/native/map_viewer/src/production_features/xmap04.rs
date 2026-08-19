use crate::{
    decode_bmp_indexed, generate_terrain_chunk_cb1, generate_terrain_chunk_index,
    generate_terrain_chunk_slot0, generate_terrain_chunk_slot1, map_wrap_offsets,
    terrain_lod_cull_update, FoundationFeatureContext, FoundationFeatureOutput, GameFileResolver,
    IndexImage, ProductionMapCameraProjection, ProductionMapFeatureAdapter, Result,
    TerrainLodCullOptions, TerrainLodCullState, TerrainRenderViewFrustum, TerrainRenderViews,
    ViewerMapCameraState, XMAP03_A729_EXECUTABLE_SHA256,
};
use portable_content_manifest::UsedContentKey;
use portable_render_stream::{FamilyDisposition, FoundationOperationIdentity, MapVisualFamily};
use replay_core::{ValidatedContentAdmission, WorldFrame};
use snapshot::sha256::Sha256;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::{Arc, Mutex};

pub(crate) const XMAP04_TERRAIN_LOD_SCHEMA: &str = "hoi4.xmap04.terrain-lod-cull.a729.v2";

const TERRAIN_FILE_CONTENT: [&str; 14] = [
    "map/provinces.bmp",
    "map/definition.csv",
    "map/default.map",
    "map/heightmap.bmp",
    "map/world_normal.bmp",
    "map/terrain.bmp",
    "common/terrain/00_terrain.txt",
    "map/terrain/atlas0.dds",
    "map/terrain/colormap_rgb_cityemissivemask_a.dds",
    "map/terrain/atlas_normal0.dds",
    "map/terrain/snow_normal_rgb_diffuse_a.dds",
    "map/terrain/citylights_rgb_snowmask_a_0.dds",
    "map/terrain/mud_diffuse_rgb_gloss_a_0.dds",
    "map/terrain/mud_normal_rgb_spec_a_0.dds",
];

const TERRAIN_EFFECT_FILE: &str = "gfx/FX/pdxmap.shader";
const TERRAIN_PLATFORM_HEADER: &str = "gfx/FX/defines_hlsl_dx11.fxh";
const TERRAIN_GRAPHICS_DEFINES: &str = "common/defines/00_graphics.lua";
const UNDERWATER_EFFECT_NAME: &str = "underwater";
const UNDERWATER_TERRAIN_DIFFUSE: &str = "map/terrain/underwater_terrain_0.dds";
const XMAP04_MAP_COPY_RECEIPT_SET_DOMAIN: &[u8] = b"hoi4.replay-map-copy-receipt-set.v1\0";
pub const XMAP04_REPLAY_FRAME_IDENTITY_DOMAIN: &[u8] = b"hoi4.xmap04.replay-frame-coordinate.v2\0";

pub fn xmap04_replay_frame_identity_sha256(world_frame: &WorldFrame) -> Result<String> {
    fn append_optional_text(identity: &mut Vec<u8>, value: Option<&str>) -> Result<()> {
        match value {
            Some(value) => {
                identity.push(1);
                identity.extend_from_slice(&u64::try_from(value.len())?.to_le_bytes());
                identity.extend_from_slice(value.as_bytes());
            }
            None => identity.push(0),
        }
        Ok(())
    }

    let mut identity = XMAP04_REPLAY_FRAME_IDENTITY_DOMAIN.to_vec();
    identity.extend_from_slice(&world_frame.hour_index().to_le_bytes());
    append_optional_text(&mut identity, world_frame.date())?;
    append_optional_text(&mut identity, world_frame.save_version())?;
    Ok(Sha256::hex(&identity))
}

const TERRAIN_DOWNSTREAM_RESOURCE_OWNERS: [(&str, &str); 7] = [
    ("ProvinceSecondaryColorMap", "XMAP05"),
    ("SnowMudData", "XMAP07"),
    ("LightIndexMap", "XMAP09"),
    ("LightDataMap", "XMAP09"),
    ("ShadowMap", "XMAP10"),
    ("GradientBorderChannel1", "XMAP05"),
    ("GradientBorderChannel2", "XMAP05"),
];

const TERRAIN_STATIC_SAMPLER_CONTENT: [(&str, &str); 8] = [
    ("TerrainDiffuse", "map/terrain/atlas0.dds"),
    ("HeightNormal", "map/world_normal.bmp"),
    (
        "TerrainColorTint",
        "map/terrain/colormap_rgb_cityemissivemask_a.dds",
    ),
    ("SnowTexture", "map/terrain/snow_normal_rgb_diffuse_a.dds"),
    ("TerrainNormal", "map/terrain/atlas_normal0.dds"),
    (
        "CityLightsAndSnowNoise",
        "map/terrain/citylights_rgb_snowmask_a_0.dds",
    ),
    ("MudNormalSpec", "map/terrain/mud_normal_rgb_spec_a_0.dds"),
    (
        "MudDiffuseGloss",
        "map/terrain/mud_diffuse_rgb_gloss_a_0.dds",
    ),
];

const UNDERWATER_RESOURCE_OWNERS: [(&str, u32, &str, Option<&str>); 6] = [
    (
        "TerrainDiffuse",
        0,
        "XMAP04",
        Some(UNDERWATER_TERRAIN_DIFFUSE),
    ),
    ("HeightNormal", 1, "XMAP04", Some("map/world_normal.bmp")),
    (
        "TerrainColorTint",
        2,
        "XMAP04",
        Some("map/terrain/colormap_rgb_cityemissivemask_a.dds"),
    ),
    ("LightIndexMap", 10, "XMAP09", None),
    ("LightDataMap", 11, "XMAP09", None),
    ("ShadowMap", 12, "XMAP10", None),
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerrainGeneratedResourceDescriptor {
    pub schema: String,
    pub width: u32,
    pub height: u32,
    pub dxgi_format: u32,
    pub row_pitch: u32,
    pub mip_levels: u32,
    pub payload_sha256: String,
    pub descriptor_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerrainSamplerRequirement {
    pub name: String,
    pub shader_register: u32,
    pub resource_type: String,
    pub mag_filter: String,
    pub min_filter: String,
    pub mip_filter: String,
    pub address_u: String,
    pub address_v: String,
    pub mip_map_lod_bias: Option<String>,
    pub mip_map_min_lod: Option<String>,
    pub mip_map_max_lod: Option<String>,
    pub max_anisotropy: Option<String>,
    pub owner_packet: String,
    pub admitted_static_content: Option<UsedContentKey>,
    pub generated_static_payload: Option<Arc<[u8]>>,
    pub generated_static_descriptor: Option<TerrainGeneratedResourceDescriptor>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedTerrainProgram {
    pub effect_file: String,
    pub effect_name: String,
    pub ordered_shader_content: Vec<UsedContentKey>,
    pub vertex_translation_unit_sha256: String,
    pub pixel_translation_unit_sha256: String,
    pub vertex_bytecode: Arc<[u8]>,
    pub pixel_bytecode: Arc<[u8]>,
    pub vertex_bytecode_sha256: String,
    pub pixel_bytecode_sha256: String,
    pub samplers: Vec<TerrainSamplerRequirement>,
    pub required_downstream_packets: Vec<String>,
    pub deterministic_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedUnderwaterTerrainProgram {
    pub effect_file: String,
    pub effect_name: String,
    pub ordered_shader_content: Vec<UsedContentKey>,
    pub vertex_translation_unit_sha256: String,
    pub pixel_translation_unit_sha256: String,
    pub vertex_bytecode: Arc<[u8]>,
    pub pixel_bytecode: Arc<[u8]>,
    pub vertex_bytecode_sha256: String,
    pub pixel_bytecode_sha256: String,
    pub samplers: Vec<TerrainSamplerRequirement>,
    pub required_downstream_packets: Vec<String>,
    pub reflection_contract_sha256: String,
    pub deterministic_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerrainDependentResourceBinding {
    pub symbol: String,
    pub owner_packet: String,
    pub resource_sha256: String,
    pub descriptor_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerrainVisibleDrawDependencies {
    pub scene_cb0_sha256: String,
    pub resources: Vec<TerrainDependentResourceBinding>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadyTerrainDrawContract {
    pub terrain_frame_sha256: String,
    pub terrain_program_sha256: String,
    pub scene_cb0_sha256: String,
    pub ordered_dependency_resources: Vec<TerrainDependentResourceBinding>,
    pub deterministic_sha256: String,
}

impl PreparedTerrainProgram {
    fn validate(&self) -> Result<()> {
        if self.effect_file != TERRAIN_EFFECT_FILE.to_ascii_lowercase()
            || !matches!(self.effect_name.as_str(), "terrain" | "terrain_low_gfx")
        {
            return Err("XMAP04 executable terrain program has the wrong effect identity".into());
        }
        if self.ordered_shader_content.is_empty()
            || self.vertex_bytecode.is_empty()
            || self.pixel_bytecode.is_empty()
        {
            return Err("XMAP04 executable terrain program has an empty compiled payload".into());
        }
        if self.vertex_bytecode_sha256 != Sha256::hex(&self.vertex_bytecode)
            || self.pixel_bytecode_sha256 != Sha256::hex(&self.pixel_bytecode)
        {
            return Err("XMAP04 executable terrain program bytecode identity is invalid".into());
        }
        let expected_sampler_count =
            TERRAIN_STATIC_SAMPLER_CONTENT.len() + TERRAIN_DOWNSTREAM_RESOURCE_OWNERS.len() + 1;
        if self.samplers.len() != expected_sampler_count {
            return Err(format!(
                "XMAP04 executable terrain program has {} samplers; exact a729 requires {expected_sampler_count}",
                self.samplers.len()
            )
            .into());
        }
        if self.required_downstream_packets != ["XMAP05", "XMAP07", "XMAP09", "XMAP10"] {
            return Err(
                "XMAP04 executable terrain program has the wrong downstream packet set".into(),
            );
        }
        if self.deterministic_sha256 != prepared_terrain_program_digest(self) {
            return Err(
                "XMAP04 executable terrain program deterministic identity is invalid".into(),
            );
        }
        validate_terrain_sha256(
            &self.vertex_translation_unit_sha256,
            "terrain vertex translation unit",
        )?;
        validate_terrain_sha256(
            &self.pixel_translation_unit_sha256,
            "terrain pixel translation unit",
        )?;
        let vertex_reflection = crate::parse_dxbc_reflection(&self.vertex_bytecode)?;
        let pixel_reflection = crate::parse_dxbc_reflection(&self.pixel_bytecode)?;
        if vertex_reflection.target >> 16 != 0xfffe
            || pixel_reflection.target >> 16 != 0xffff
            || !vertex_reflection.inputs.iter().any(|input| {
                input.semantic_name.eq_ignore_ascii_case("POSITION")
                    && input.semantic_index == 0
                    && input.component_type == 2
                    && input.mask == 0x0f
            })
            || !vertex_reflection.inputs.iter().any(|input| {
                input.semantic_name.eq_ignore_ascii_case("TEXCOORD")
                    && input.semantic_index == 0
                    && input.component_type == 2
                    && input.mask & 0x03 == 0x03
            })
        {
            return Err("XMAP04 terrain bytecode has the wrong stage/input contract".into());
        }
        let mut registers = BTreeSet::new();
        for sampler in &self.samplers {
            if sampler.name.is_empty()
                || sampler.resource_type.is_empty()
                || sampler.mag_filter.is_empty()
                || sampler.min_filter.is_empty()
                || sampler.mip_filter.is_empty()
                || sampler.address_u.is_empty()
                || sampler.address_v.is_empty()
                || [
                    &sampler.mip_map_lod_bias,
                    &sampler.mip_map_min_lod,
                    &sampler.mip_map_max_lod,
                    &sampler.max_anisotropy,
                ]
                .iter()
                .any(|value| value.as_ref().is_some_and(String::is_empty))
                || !registers.insert(sampler.shader_register)
            {
                return Err("XMAP04 terrain sampler descriptor is incomplete".into());
            }
            match &sampler.admitted_static_content {
                Some(content)
                    if sampler.owner_packet == "XMAP04"
                        && sampler.generated_static_payload.is_none()
                        && sampler.generated_static_descriptor.is_none() =>
                {
                    content.validate().map_err(|error| {
                        format!("XMAP04 terrain sampler used-content identity: {error}")
                    })?;
                }
                None if sampler.owner_packet == "XMAP04"
                    && sampler.generated_static_payload.is_some()
                    && sampler.generated_static_descriptor.is_some() =>
                {
                    let payload = sampler.generated_static_payload.as_ref().unwrap();
                    let descriptor = sampler.generated_static_descriptor.as_ref().unwrap();
                    if sampler.name != "TerrainIDMap"
                        || descriptor.schema != "hoi4.xmap04.terrain-id-map.generated.v1"
                        || descriptor.payload_sha256 != Sha256::hex(payload)
                        || descriptor.width == 0
                        || descriptor.height == 0
                        || descriptor.dxgi_format != crate::TERRAIN_ID_MAP_DXGI_FORMAT
                        || descriptor.row_pitch != descriptor.width.saturating_mul(4)
                        || descriptor.mip_levels != 1
                        || descriptor.descriptor_sha256
                            != generated_resource_descriptor_digest(descriptor)
                    {
                        return Err(
                            "XMAP04 generated TerrainIDMap resource descriptor is invalid".into(),
                        );
                    }
                }
                None if sampler.owner_packet != "XMAP04"
                    && sampler.generated_static_payload.is_none()
                    && sampler.generated_static_descriptor.is_none() => {}
                _ => {
                    return Err(
                        "XMAP04 terrain sampler owner/content boundary is inconsistent".into(),
                    )
                }
            }
            if !pixel_reflection.resources.iter().any(|resource| {
                resource.name == format!("{}_Texture", sampler.name)
                    && resource.resource_type == 2
                    && resource.bind_point == sampler.shader_register
            }) || !pixel_reflection.resources.iter().any(|resource| {
                resource.name == format!("{}_Sampler", sampler.name)
                    && resource.resource_type == 3
                    && resource.bind_point == sampler.shader_register
            }) {
                return Err(format!(
                    "XMAP04 compiled pixel program omitted sampler {} at register {}",
                    sampler.name, sampler.shader_register
                )
                .into());
            }
        }
        Ok(())
    }
}

impl PreparedUnderwaterTerrainProgram {
    pub fn validate(&self) -> Result<()> {
        if self.effect_file != TERRAIN_EFFECT_FILE.to_ascii_lowercase()
            || self.effect_name != UNDERWATER_EFFECT_NAME
            || self.ordered_shader_content.is_empty()
            || self.vertex_bytecode.is_empty()
            || self.pixel_bytecode.is_empty()
            || self.vertex_bytecode_sha256 != Sha256::hex(&self.vertex_bytecode)
            || self.pixel_bytecode_sha256 != Sha256::hex(&self.pixel_bytecode)
            || self.samplers.len() != UNDERWATER_RESOURCE_OWNERS.len()
            || self.required_downstream_packets != ["XMAP09", "XMAP10"]
            || validate_terrain_sha256(
                &self.reflection_contract_sha256,
                "underwater reflection contract",
            )
            .is_err()
            || self.deterministic_sha256 != prepared_underwater_program_digest(self)
        {
            return Err("XMAP04 executable underwater terrain program is invalid".into());
        }
        validate_terrain_sha256(
            &self.vertex_translation_unit_sha256,
            "underwater vertex translation unit",
        )?;
        validate_terrain_sha256(
            &self.pixel_translation_unit_sha256,
            "underwater pixel translation unit",
        )?;
        let vertex_reflection = crate::parse_dxbc_reflection(&self.vertex_bytecode)?;
        let pixel_reflection = crate::parse_dxbc_reflection(&self.pixel_bytecode)?;
        if self.reflection_contract_sha256
            != underwater_reflection_contract_digest(&vertex_reflection, &pixel_reflection)
            || vertex_reflection.target != 0xfffe_0500
            || pixel_reflection.target != 0xffff_0500
            || !underwater_vertex_input_is_exact(&vertex_reflection)
        {
            return Err("XMAP04 underwater bytecode has the wrong stage/input contract".into());
        }
        for (sampler, (name, slot, owner, path)) in
            self.samplers.iter().zip(UNDERWATER_RESOURCE_OWNERS)
        {
            if sampler.name != name
                || sampler.shader_register != slot
                || sampler.owner_packet != owner
                || sampler.resource_type.is_empty()
                || sampler.mag_filter.is_empty()
                || sampler.min_filter.is_empty()
                || sampler.mip_filter.is_empty()
                || sampler.address_u.is_empty()
                || sampler.address_v.is_empty()
            {
                return Err("XMAP04 underwater sampler order/descriptor is invalid".into());
            }
            match (owner, path, &sampler.admitted_static_content) {
                ("XMAP04", Some(expected), Some(content))
                    if content.normalized_game_relative_path == expected =>
                {
                    content.validate().map_err(|error| {
                        format!("XMAP04 underwater used-content identity: {error}")
                    })?;
                }
                ("XMAP09" | "XMAP10", None, None) => {}
                _ => {
                    return Err(
                        "XMAP04 underwater sampler owner/content boundary is invalid".into(),
                    )
                }
            }
            if sampler.generated_static_payload.is_some()
                || sampler.generated_static_descriptor.is_some()
                || !pixel_reflection.resources.iter().any(|resource| {
                    resource.name == format!("{}_Texture", name)
                        && resource.resource_type == 2
                        && resource.dimension == 4
                        && resource.bind_point == slot
                        && resource.bind_count == 1
                })
                || !pixel_reflection.resources.iter().any(|resource| {
                    resource.name == format!("{}_Sampler", name)
                        && resource.resource_type == 3
                        && resource.dimension == 0
                        && resource.bind_point == slot
                        && resource.bind_count == 1
                })
            {
                return Err(format!(
                    "XMAP04 compiled underwater program omitted sampler {name} at register {slot}"
                )
                .into());
            }
        }
        validate_underwater_cbuffer(
            &vertex_reflection,
            "vertex",
            0,
            416,
            &underwater_scene_cbuffer_layout(true),
        )?;
        validate_underwater_cbuffer(
            &vertex_reflection,
            "vertex",
            1,
            u32::try_from(crate::TERRAIN_CHUNK_CB1_BYTES)?,
            &underwater_vertex_draw_cbuffer_layout(),
        )?;
        validate_underwater_cbuffer(
            &pixel_reflection,
            "pixel",
            0,
            416,
            &underwater_scene_cbuffer_layout(false),
        )?;
        validate_underwater_cbuffer(
            &pixel_reflection,
            "pixel",
            2,
            16,
            &[("GridStart_InvCellSize", 0, 16, true)],
        )?;
        let vertex_contract_is_exact = vertex_reflection.resources.len() == 2
            && vertex_reflection.cbuffers.len() == 2
            && vertex_reflection.resources.iter().all(|resource| {
                resource.resource_type == 0
                    && resource.dimension == 0
                    && resource.bind_count == 1
                    && matches!(resource.bind_point, 0 | 1)
            });
        let pixel_contract_is_exact = pixel_reflection.resources.len() == 14
            && pixel_reflection.cbuffers.len() == 2
            && pixel_reflection
                .resources
                .iter()
                .all(|resource| match resource.resource_type {
                    0 => {
                        resource.dimension == 0
                            && resource.bind_count == 1
                            && matches!(resource.bind_point, 0 | 2)
                    }
                    2 => UNDERWATER_RESOURCE_OWNERS.iter().any(|(name, slot, _, _)| {
                        resource.bind_point == *slot
                            && resource.name == format!("{name}_Texture")
                            && resource.dimension == 4
                            && resource.bind_count == 1
                    }),
                    3 => UNDERWATER_RESOURCE_OWNERS.iter().any(|(name, slot, _, _)| {
                        resource.bind_point == *slot
                            && resource.name == format!("{name}_Sampler")
                            && resource.dimension == 0
                            && resource.bind_count == 1
                    }),
                    _ => false,
                });
        if !vertex_contract_is_exact || !pixel_contract_is_exact {
            return Err(
                "XMAP04 underwater shader reflection contains an unsupported extra or missing resource"
                    .into(),
            );
        }
        Ok(())
    }
}

fn underwater_vertex_input_is_exact(reflection: &crate::DxbcReflection) -> bool {
    let expected = [
        ("POSITION", 0, 0, 2, 0x0f, 0x0f),
        ("TEXCOORD", 0, 1, 2, 0x03, 0x03),
    ];
    reflection.inputs.len() == expected.len()
        && reflection
            .inputs
            .iter()
            .zip(expected)
            .all(|(actual, expected)| {
                actual.semantic_name.eq_ignore_ascii_case(expected.0)
                    && actual.semantic_index == expected.1
                    && actual.register == expected.2
                    && actual.component_type == expected.3
                    && actual.mask == expected.4
                    && actual.read_write_mask == expected.5
            })
}

fn underwater_scene_cbuffer_layout(vertex_stage: bool) -> Vec<(&'static str, u32, u32, bool)> {
    let pixel_used = |name: &str| {
        matches!(
            name,
            "vVirtualSunPos"
                | "vVirtualMoonPos"
                | "vSecondVirtualSunPos"
                | "vSecondVirtualMoonPos"
                | "vCamPos"
                | "HdrRange"
                | "vFoWOpacity_FoWTime_SnowMudFade_MaxGameSpeed"
                | "DayNight_Hour_SunDir"
                | "ShadowFadeFactor"
                | "NegFogMultiplier"
                | "SunDiffuseIntensity"
                | "MoonDiffuseIntensity"
                | "SunSpecularIntensity"
        )
    };
    [
        ("ViewProjectionMatrix", 0, 64),
        ("InvViewProjMatrix", 64, 64),
        ("vGBCamDistOverride_GBOutlineCutoff", 128, 16),
        ("vVirtualSunPos", 144, 16),
        ("vVirtualMoonPos", 160, 16),
        ("vSecondVirtualSunPos", 176, 16),
        ("vSecondVirtualMoonPos", 192, 16),
        ("vCamPos", 208, 12),
        ("HdrRange", 220, 4),
        ("vCamLookAtDir", 224, 12),
        ("vGlobalTime", 236, 4),
        ("vFoWOpacity_FoWTime_SnowMudFade_MaxGameSpeed", 240, 16),
        ("DayNight_Hour_SunDir", 256, 16),
        ("AmbientPosX", 272, 12),
        ("ShadowFadeFactor", 284, 4),
        ("AmbientNegX", 288, 12),
        ("FOWFadeFactor", 300, 4),
        ("AmbientPosY", 304, 12),
        ("MinMeshAlpha", 316, 4),
        ("AmbientNegY", 320, 12),
        ("NegFogMultiplier", 332, 4),
        ("AmbientPosZ", 336, 12),
        ("AmbientNegZ", 352, 12),
        ("CubemapIntensity", 364, 4),
        ("SunDiffuseIntensity", 368, 16),
        ("MoonDiffuseIntensity", 384, 16),
        ("GB_TextureHeight", 400, 4),
        ("SunSpecularIntensity", 404, 4),
    ]
    .into_iter()
    .map(|(name, offset, size)| {
        let used = if vertex_stage {
            name == "ViewProjectionMatrix"
        } else {
            pixel_used(name)
        };
        (name, offset, size, used)
    })
    .collect()
}

fn underwater_vertex_draw_cbuffer_layout() -> [(&'static str, u32, u32, bool); 4] {
    [
        ("ShadowMapTextureMatrix", 0, 64, true),
        ("QuadOffset_Scale_IsDetail", 64, 16, true),
        ("vSnap", 80, 16, true),
        (
            "vBorderLookup_HeightScale_UseMultisample_SeasonLerp",
            96,
            16,
            false,
        ),
    ]
}

fn underwater_reflection_contract_digest(
    vertex: &crate::DxbcReflection,
    pixel: &crate::DxbcReflection,
) -> String {
    fn append_text(identity: &mut Vec<u8>, value: &str) {
        identity.extend_from_slice(&(value.len() as u64).to_le_bytes());
        identity.extend_from_slice(value.as_bytes());
    }
    fn append_reflection(identity: &mut Vec<u8>, reflection: &crate::DxbcReflection) {
        identity.extend_from_slice(&reflection.target.to_le_bytes());
        append_text(identity, &reflection.creator);
        identity.extend_from_slice(&(reflection.inputs.len() as u64).to_le_bytes());
        for input in &reflection.inputs {
            append_text(identity, &input.semantic_name);
            for value in [
                input.semantic_index,
                input.register,
                input.component_type,
                u32::from(input.mask),
                u32::from(input.read_write_mask),
            ] {
                identity.extend_from_slice(&value.to_le_bytes());
            }
        }
        identity.extend_from_slice(&(reflection.outputs.len() as u64).to_le_bytes());
        for output in &reflection.outputs {
            append_text(identity, &output.semantic_name);
            for value in [
                output.semantic_index,
                output.register,
                output.component_type,
                u32::from(output.mask),
                u32::from(output.read_write_mask),
            ] {
                identity.extend_from_slice(&value.to_le_bytes());
            }
        }
        identity.extend_from_slice(&(reflection.resources.len() as u64).to_le_bytes());
        for resource in &reflection.resources {
            append_text(identity, &resource.name);
            for value in [
                resource.resource_type,
                resource.dimension,
                resource.bind_point,
                resource.bind_count,
            ] {
                identity.extend_from_slice(&value.to_le_bytes());
            }
        }
        identity.extend_from_slice(&(reflection.cbuffers.len() as u64).to_le_bytes());
        for buffer in &reflection.cbuffers {
            append_text(identity, &buffer.name);
            identity.extend_from_slice(&buffer.size.to_le_bytes());
            identity.extend_from_slice(&(buffer.vars.len() as u64).to_le_bytes());
            for var in &buffer.vars {
                append_text(identity, &var.name);
                identity.extend_from_slice(&var.offset.to_le_bytes());
                identity.extend_from_slice(&var.size.to_le_bytes());
                identity.push(u8::from(var.used));
            }
        }
    }

    let mut identity = b"hoi4.xmap04.underwater-reflection-contract.v1\0".to_vec();
    append_reflection(&mut identity, vertex);
    append_reflection(&mut identity, pixel);
    Sha256::hex(&identity)
}

fn validate_underwater_cbuffer(
    reflection: &crate::DxbcReflection,
    stage: &str,
    slot: u32,
    expected_size: u32,
    expected_vars: &[(&str, u32, u32, bool)],
) -> Result<()> {
    let resource = reflection
        .resources
        .iter()
        .find(|resource| resource.resource_type == 0 && resource.bind_point == slot)
        .ok_or_else(|| format!("XMAP04 underwater {stage} reflection omitted cb{slot}"))?;
    if resource.dimension != 0 || resource.bind_count != 1 {
        return Err(format!("XMAP04 underwater {stage} cb{slot} binding is not singular").into());
    }
    let buffer = reflection
        .cbuffers
        .iter()
        .find(|buffer| buffer.name == resource.name)
        .ok_or_else(|| format!("XMAP04 underwater {stage} cb{slot} has no matching RDEF buffer"))?;
    let exact = buffer.size == expected_size
        && buffer.vars.len() == expected_vars.len()
        && buffer
            .vars
            .iter()
            .zip(expected_vars)
            .all(|(actual, expected)| {
                actual.name == expected.0
                    && actual.offset == expected.1
                    && actual.size == expected.2
                    && actual.used == expected.3
            });
    if !exact {
        let mismatches = buffer
            .vars
            .iter()
            .zip(expected_vars)
            .enumerate()
            .filter_map(|(index, (actual, expected))| {
                (actual.name != expected.0
                    || actual.offset != expected.1
                    || actual.size != expected.2
                    || actual.used != expected.3)
                    .then(|| {
                        format!(
                            "#{index} actual {}@{}+{} used={}, expected {}@{}+{} used={}",
                            actual.name,
                            actual.offset,
                            actual.size,
                            actual.used,
                            expected.0,
                            expected.1,
                            expected.2,
                            expected.3
                        )
                    })
            })
            .collect::<Vec<_>>();
        return Err(if mismatches.is_empty() {
            format!(
                "XMAP04 underwater {stage} cb{slot} size/census differs: actual {} bytes/{} vars, expected {expected_size} bytes/{} vars",
                buffer.size,
                buffer.vars.len(),
                expected_vars.len()
            )
        } else {
            format!(
                "XMAP04 underwater {stage} cb{slot} variable layout differs: {}",
                mismatches.join("; ")
            )
        }
        .into());
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedTerrainDraw {
    pub render_view_mask: u8,
    pub map_copy_offset_f32_bits: u32,
    pub min_x: u32,
    pub min_z: u32,
    pub tile_size: u32,
    pub vsnap_f32_bits: [u32; 4],
    pub slot1: Arc<[u8]>,
    pub cb1: Arc<[u8]>,
    pub slot1_sha256: String,
    pub cb1_sha256: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Xmap04MapCopyReceiptRow {
    pub render_view_mask: u8,
    pub map_copy_offset_f32_bits: u32,
}

pub fn xmap04_map_copy_receipt_set_sha256(
    map_width: u32,
    rows: &[Xmap04MapCopyReceiptRow],
) -> Result<String> {
    let mut seen_masks = BTreeSet::new();
    let expected_offset = |mask| match mask {
        1 => Some(0.0f32.to_bits()),
        2 => Some((map_width as f32).to_bits()),
        4 => Some((-(map_width as f32)).to_bits()),
        _ => None,
    };
    let masks = rows
        .iter()
        .map(|row| row.render_view_mask)
        .collect::<Vec<_>>();
    if map_width == 0
        || rows.is_empty()
        || !matches!(masks.as_slice(), [1] | [1, 2] | [1, 4] | [1, 2, 4])
        || rows.iter().any(|row| {
            !matches!(row.render_view_mask, 1 | 2 | 4)
                || !f32::from_bits(row.map_copy_offset_f32_bits).is_finite()
                || !seen_masks.insert(row.render_view_mask)
                || expected_offset(row.render_view_mask) != Some(row.map_copy_offset_f32_bits)
        })
    {
        return Err(
            "XMAP04 map-copy receipt rows violate exact engine subset/order/offset law".into(),
        );
    }
    let mut identity = XMAP04_MAP_COPY_RECEIPT_SET_DOMAIN.to_vec();
    identity.extend_from_slice(&u32::try_from(rows.len())?.to_le_bytes());
    for row in rows {
        identity.push(row.render_view_mask);
        identity.extend_from_slice(&row.map_copy_offset_f32_bits.to_le_bytes());
    }
    Ok(Sha256::hex(&identity))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedTerrainFrame {
    pub schema: String,
    pub replay_frame_identity_sha256: String,
    pub camera_frame_sha256: String,
    pub map_width: u32,
    pub map_height: u32,
    pub draw_hires_terrain: bool,
    pub render_view_side_plane_bias_f32_bits: u32,
    pub cull_state_bytes: Vec<u8>,
    pub ordered_used_content: Vec<UsedContentKey>,
    pub program: PreparedTerrainProgram,
    pub underwater_program: Option<PreparedUnderwaterTerrainProgram>,
    pub shared_slot0: Arc<[u8]>,
    pub shared_index: Arc<[u8]>,
    pub draws: Vec<PreparedTerrainDraw>,
    pub deterministic_sha256: String,
}

impl PreparedTerrainFrame {
    pub fn validate(&self) -> Result<()> {
        if self.schema != XMAP04_TERRAIN_LOD_SCHEMA
            || validate_terrain_sha256(&self.replay_frame_identity_sha256, "XMAP04 replay frame")
                .is_err()
            || validate_terrain_sha256(&self.camera_frame_sha256, "XMAP04 camera frame").is_err()
            || self.map_width == 0
            || self.map_height == 0
            || self.render_view_side_plane_bias_f32_bits != 0.0f32.to_bits()
            || self.cull_state_bytes.is_empty()
            || self.ordered_used_content.is_empty()
            || self.shared_slot0.len() != crate::TERRAIN_CHUNK_SLOT0_BYTES
            || self.shared_index.len() != crate::TERRAIN_CHUNK_INDEX_BYTES
            || self.draws.is_empty()
        {
            return Err("XMAP04 prepared terrain frame has incomplete production payload".into());
        }
        validate_underwater_profile(&self.program.effect_name, self.underwater_program.as_ref())?;
        self.program.validate()?;
        if let Some(program) = &self.underwater_program {
            program.validate()?;
        }
        let mut paths = BTreeSet::new();
        for content in &self.ordered_used_content {
            content
                .validate()
                .map_err(|error| format!("XMAP04 used-content identity: {error}"))?;
            if !paths.insert(content.normalized_game_relative_path.as_str()) {
                return Err("XMAP04 prepared terrain content path is duplicated".into());
            }
        }
        for draw in &self.draws {
            if !matches!(draw.render_view_mask, 1 | 2 | 4)
                || !f32::from_bits(draw.map_copy_offset_f32_bits).is_finite()
                || draw.tile_size < crate::TERRAIN_LOD_BASE_TILE_SIZE
                || !draw.tile_size.is_power_of_two()
                || draw.slot1.len() != crate::TERRAIN_CHUNK_SLOT1_TILE_BYTES
                || draw.cb1.len() != crate::TERRAIN_CHUNK_CB1_BYTES
                || draw.slot1_sha256 != Sha256::hex(&draw.slot1)
                || draw.cb1_sha256 != Sha256::hex(&draw.cb1)
            {
                return Err("XMAP04 prepared terrain draw geometry is invalid".into());
            }
        }
        xmap04_map_copy_receipt_set_sha256(
            self.map_width,
            &derive_ordered_map_copy_receipts(&self.draws, self.map_width)?,
        )?;
        if self.deterministic_sha256 != prepared_terrain_digest(self) {
            return Err("XMAP04 prepared terrain frame digest mismatch".into());
        }
        Ok(())
    }

    pub fn ordered_map_copy_receipts(&self) -> Result<Vec<Xmap04MapCopyReceiptRow>> {
        self.validate()?;
        derive_ordered_map_copy_receipts(&self.draws, self.map_width)
    }

    pub fn map_copy_receipt_set_sha256(&self) -> Result<String> {
        xmap04_map_copy_receipt_set_sha256(self.map_width, &self.ordered_map_copy_receipts()?)
    }

    pub fn admit_visible_draw_dependencies(
        &self,
        dependencies: TerrainVisibleDrawDependencies,
    ) -> Result<ReadyTerrainDrawContract> {
        self.validate()?;
        validate_terrain_sha256(&dependencies.scene_cb0_sha256, "terrain scene CB0")?;
        let expected = TERRAIN_DOWNSTREAM_RESOURCE_OWNERS
            .iter()
            .map(|(symbol, owner)| ((*symbol).to_string(), (*owner).to_string()))
            .collect::<Vec<_>>();
        if dependencies.resources.len() != expected.len() {
            return Err("visible terrain dependencies are incomplete".into());
        }
        for (binding, (symbol, owner)) in dependencies.resources.iter().zip(&expected) {
            if &binding.symbol != symbol || &binding.owner_packet != owner {
                return Err(format!(
                    "visible terrain dependency order/owner mismatch: expected {owner}:{symbol}"
                )
                .into());
            }
            validate_terrain_sha256(&binding.resource_sha256, "terrain dependency resource")?;
            validate_terrain_sha256(&binding.descriptor_sha256, "terrain dependency descriptor")?;
        }
        let mut identity = Vec::new();
        identity.extend_from_slice(self.deterministic_sha256.as_bytes());
        identity.extend_from_slice(self.program.deterministic_sha256.as_bytes());
        identity.extend_from_slice(dependencies.scene_cb0_sha256.as_bytes());
        for binding in &dependencies.resources {
            identity.extend_from_slice(binding.symbol.as_bytes());
            identity.extend_from_slice(binding.owner_packet.as_bytes());
            identity.extend_from_slice(binding.resource_sha256.as_bytes());
            identity.extend_from_slice(binding.descriptor_sha256.as_bytes());
        }
        Ok(ReadyTerrainDrawContract {
            terrain_frame_sha256: self.deterministic_sha256.clone(),
            terrain_program_sha256: self.program.deterministic_sha256.clone(),
            scene_cb0_sha256: dependencies.scene_cb0_sha256,
            ordered_dependency_resources: dependencies.resources,
            deterministic_sha256: Sha256::hex(&identity),
        })
    }
}

fn derive_ordered_map_copy_receipts(
    draws: &[PreparedTerrainDraw],
    map_width: u32,
) -> Result<Vec<Xmap04MapCopyReceiptRow>> {
    if map_width == 0 {
        return Err("XMAP04 map-copy law requires a nonzero map width".into());
    }
    let mut offsets_by_mask = BTreeMap::new();
    let mut rows = Vec::new();
    let mut last_mask = None;
    for draw in draws {
        if last_mask.is_some_and(|mask| mask != draw.render_view_mask)
            && offsets_by_mask.contains_key(&draw.render_view_mask)
        {
            return Err("XMAP04 map-copy draw groups are interleaved".into());
        }
        match offsets_by_mask.get(&draw.render_view_mask) {
            Some(offset) if *offset != draw.map_copy_offset_f32_bits => {
                return Err("XMAP04 one render-view mask carries multiple map-copy offsets".into())
            }
            Some(_) => {}
            None => {
                offsets_by_mask.insert(draw.render_view_mask, draw.map_copy_offset_f32_bits);
                rows.push(Xmap04MapCopyReceiptRow {
                    render_view_mask: draw.render_view_mask,
                    map_copy_offset_f32_bits: draw.map_copy_offset_f32_bits,
                });
            }
        }
        last_mask = Some(draw.render_view_mask);
    }
    xmap04_map_copy_receipt_set_sha256(map_width, &rows)?;
    Ok(rows)
}

fn validate_terrain_sha256(value: &str, label: &str) -> Result<()> {
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

struct Xmap04TerrainContent {
    ordered_used_content: Vec<UsedContentKey>,
    heightmap: IndexImage,
    program: PreparedTerrainProgram,
    underwater_program: Option<PreparedUnderwaterTerrainProgram>,
    shared_slot0: Arc<[u8]>,
    shared_index: Arc<[u8]>,
}

fn validate_underwater_profile(
    terrain_effect_name: &str,
    underwater: Option<&PreparedUnderwaterTerrainProgram>,
) -> Result<()> {
    match (underwater.is_some(), terrain_effect_name) {
        (true, "terrain") | (false, "terrain_low_gfx") => Ok(()),
        _ => Err("XMAP04 underwater program presence does not match the graphics profile".into()),
    }
}

impl Xmap04TerrainContent {
    fn admit(
        resolver: &GameFileResolver,
        admission: &ValidatedContentAdmission,
        options: TerrainLodCullOptions,
    ) -> Result<Self> {
        let mut ordered_used_content = Vec::new();
        let mut seen = BTreeSet::new();
        let mut heightmap = None;
        let translation_unit =
            crate::shader_compile::effect_translation_unit_files(resolver, TERRAIN_EFFECT_FILE)?;
        let mut shader_paths = translation_unit
            .iter()
            .map(|(path, _)| path.clone())
            .collect::<Vec<_>>();
        shader_paths.push(TERRAIN_PLATFORM_HEADER.into());
        shader_paths.push(TERRAIN_GRAPHICS_DEFINES.into());
        for path in shader_paths
            .iter()
            .map(String::as_str)
            .chain(TERRAIN_FILE_CONTENT)
            .chain(
                options
                    .high_gfx_shaders
                    .then_some(UNDERWATER_TERRAIN_DIFFUSE)
                    .into_iter(),
            )
        {
            if !seen.insert(path) {
                continue;
            }
            let content = admission
                .require_used_content(path)
                .map_err(|error| format!("XMAP04 requires admitted {path}: {error}"))?
                .clone();
            let bytes = resolver.read_admitted_used_content(&content)?;
            if path == "map/heightmap.bmp" {
                heightmap = Some(decode_bmp_indexed(&bytes, Path::new(path))?);
            }
            ordered_used_content.push(content);
        }
        let terrain_id_map = crate::synthesize_terrain_id_map(resolver)?;
        let admitted_heightmap = heightmap
            .as_ref()
            .ok_or("XMAP04 content closure omitted map/heightmap.bmp")?;
        if terrain_id_map.width != admitted_heightmap.width
            || terrain_id_map.height != admitted_heightmap.height
        {
            return Err(format!(
                "XMAP04 generated TerrainIDMap {}x{} disagrees with heightmap {}x{}",
                terrain_id_map.width,
                terrain_id_map.height,
                admitted_heightmap.width,
                admitted_heightmap.height
            )
            .into());
        }
        let program = prepare_terrain_program(
            resolver,
            admission,
            &translation_unit,
            options.high_gfx_shaders,
            admitted_heightmap,
            &terrain_id_map,
        )?;
        let underwater_program = options
            .high_gfx_shaders
            .then(|| {
                prepare_underwater_program(
                    resolver,
                    admission,
                    &translation_unit,
                    admitted_heightmap,
                )
            })
            .transpose()?;
        Ok(Self {
            ordered_used_content,
            heightmap: heightmap.ok_or("XMAP04 content closure omitted map/heightmap.bmp")?,
            program,
            underwater_program,
            shared_slot0: generate_terrain_chunk_slot0().into(),
            shared_index: generate_terrain_chunk_index().into(),
        })
    }
}

fn prepare_underwater_program(
    resolver: &GameFileResolver,
    admission: &ValidatedContentAdmission,
    translation_unit: &[(String, crate::EffectFile)],
    heightmap: &IndexImage,
) -> Result<PreparedUnderwaterTerrainProgram> {
    let parsed = translation_unit
        .last()
        .map(|(_, file)| file)
        .ok_or("XMAP04 underwater translation unit is empty")?;
    let effect = parsed.effect(UNDERWATER_EFFECT_NAME).ok_or_else(|| {
        format!("effective {TERRAIN_EFFECT_FILE} has no Effect named {UNDERWATER_EFFECT_NAME}")
    })?;
    if effect.vertex_shader.is_none() || effect.pixel_shader.is_none() {
        return Err("effective underwater Effect omits a shader stage".into());
    }
    let constants = crate::derive_a729_engine_map_shader_constants(
        heightmap.width as u32,
        heightmap.height as u32,
        false,
        0,
    )?;
    let defines = crate::engine_map_shader_defines(constants);
    let vertex_hlsl = crate::shader_compile::build_stage_hlsl_with_defines(
        resolver,
        TERRAIN_EFFECT_FILE,
        UNDERWATER_EFFECT_NAME,
        crate::ShaderStage::Vertex,
        &defines,
    )?;
    let pixel_hlsl = crate::shader_compile::build_stage_hlsl_with_defines(
        resolver,
        TERRAIN_EFFECT_FILE,
        UNDERWATER_EFFECT_NAME,
        crate::ShaderStage::Pixel,
        &defines,
    )?;
    let (vertex_bytecode, pixel_bytecode) =
        compile_terrain_program_stages(&vertex_hlsl, &pixel_hlsl)?;
    let vertex_reflection = crate::parse_dxbc_reflection(&vertex_bytecode)?;
    let pixel_reflection = crate::parse_dxbc_reflection(&pixel_bytecode)?;
    let reflected_slots = pixel_reflection
        .resources
        .iter()
        .filter(|resource| resource.resource_type == 2)
        .map(|resource| (resource.name.clone(), resource.bind_point))
        .collect::<BTreeSet<_>>();
    let pixel_stage = crate::shader_compile::merged_effect_translation_unit_stage(
        translation_unit,
        crate::ShaderStage::Pixel,
    );
    let sampler_by_name = pixel_stage
        .samplers
        .iter()
        .map(|sampler| (sampler.name.as_str(), sampler))
        .collect::<BTreeMap<_, _>>();
    let required = |value: &Option<String>, name: &str, field: &str| -> Result<String> {
        value.clone().ok_or_else(|| {
            format!("effective underwater sampler {name} omits required {field}").into()
        })
    };
    let mut samplers = Vec::with_capacity(UNDERWATER_RESOURCE_OWNERS.len());
    for (name, slot, owner, path) in UNDERWATER_RESOURCE_OWNERS {
        if !reflected_slots.contains(&(format!("{name}_Texture"), slot)) {
            return Err(format!(
                "compiled underwater pixel program does not sample {name} at t{slot}"
            )
            .into());
        }
        let sampler = sampler_by_name.get(name).ok_or_else(|| {
            format!("effective underwater translation unit has no sampler {name}")
        })?;
        if sampler.index != slot {
            return Err(format!(
                "effective underwater sampler {name} moved from exact register {slot} to {}",
                sampler.index
            )
            .into());
        }
        let admitted_static_content = path
            .map(|path| {
                admission
                    .require_used_content(path)
                    .map_err(|error| {
                        format!("XMAP04 underwater sampler {name} requires {path}: {error}")
                    })
                    .cloned()
            })
            .transpose()?;
        samplers.push(TerrainSamplerRequirement {
            name: name.into(),
            shader_register: slot,
            resource_type: sampler
                .sampler_type
                .clone()
                .unwrap_or_else(|| "Texture2D".into()),
            mag_filter: required(&sampler.mag_filter, name, "MagFilter")?,
            min_filter: required(&sampler.min_filter, name, "MinFilter")?,
            mip_filter: required(&sampler.mip_filter, name, "MipFilter")?,
            address_u: required(&sampler.address_u, name, "AddressU")?,
            address_v: required(&sampler.address_v, name, "AddressV")?,
            mip_map_lod_bias: sampler.mip_map_lod_bias.clone(),
            mip_map_min_lod: sampler.mip_map_min_lod.clone(),
            mip_map_max_lod: sampler.mip_map_max_lod.clone(),
            max_anisotropy: sampler.max_anisotropy.clone(),
            owner_packet: owner.into(),
            admitted_static_content,
            generated_static_payload: None,
            generated_static_descriptor: None,
        });
    }
    let mut ordered_shader_content = Vec::new();
    for path in translation_unit
        .iter()
        .map(|(path, _)| path.as_str())
        .chain([TERRAIN_PLATFORM_HEADER])
    {
        ordered_shader_content.push(
            admission
                .require_used_content(path)
                .map_err(|error| {
                    format!("XMAP04 underwater program requires admitted {path}: {error}")
                })?
                .clone(),
        );
    }
    let mut program = PreparedUnderwaterTerrainProgram {
        effect_file: TERRAIN_EFFECT_FILE.to_ascii_lowercase(),
        effect_name: UNDERWATER_EFFECT_NAME.into(),
        ordered_shader_content,
        vertex_translation_unit_sha256: Sha256::hex(vertex_hlsl.as_bytes()),
        pixel_translation_unit_sha256: Sha256::hex(pixel_hlsl.as_bytes()),
        vertex_bytecode_sha256: Sha256::hex(&vertex_bytecode),
        pixel_bytecode_sha256: Sha256::hex(&pixel_bytecode),
        vertex_bytecode: vertex_bytecode.into(),
        pixel_bytecode: pixel_bytecode.into(),
        samplers,
        required_downstream_packets: vec!["XMAP09".into(), "XMAP10".into()],
        reflection_contract_sha256: underwater_reflection_contract_digest(
            &vertex_reflection,
            &pixel_reflection,
        ),
        deterministic_sha256: String::new(),
    };
    program.deterministic_sha256 = prepared_underwater_program_digest(&program);
    program.validate()?;
    Ok(program)
}

fn prepare_terrain_program(
    resolver: &GameFileResolver,
    admission: &ValidatedContentAdmission,
    translation_unit: &[(String, crate::EffectFile)],
    high_gfx_shaders: bool,
    heightmap: &IndexImage,
    terrain_id_map: &crate::SynthesizedTerrainIdMap,
) -> Result<PreparedTerrainProgram> {
    let effect_name = if high_gfx_shaders {
        "terrain"
    } else {
        "terrain_low_gfx"
    };
    let parsed = translation_unit
        .last()
        .map(|(_, file)| file)
        .ok_or("XMAP04 terrain translation unit is empty")?;
    let effect = parsed.effect(effect_name).ok_or_else(|| {
        format!("effective {TERRAIN_EFFECT_FILE} has no Effect named {effect_name}")
    })?;
    if effect.vertex_shader.is_none() || effect.pixel_shader.is_none() {
        return Err(format!("effective terrain Effect {effect_name} omits a shader stage").into());
    }
    let constants = crate::derive_a729_engine_map_shader_constants(
        heightmap.width as u32,
        heightmap.height as u32,
        false,
        0,
    )?;
    let defines = crate::engine_map_shader_defines(constants);
    let vertex_hlsl = crate::shader_compile::build_stage_hlsl_with_defines(
        resolver,
        TERRAIN_EFFECT_FILE,
        effect_name,
        crate::ShaderStage::Vertex,
        &defines,
    )?;
    let pixel_hlsl = crate::shader_compile::build_stage_hlsl_with_defines(
        resolver,
        TERRAIN_EFFECT_FILE,
        effect_name,
        crate::ShaderStage::Pixel,
        &defines,
    )?;
    let (vertex_bytecode, pixel_bytecode) =
        compile_terrain_program_stages(&vertex_hlsl, &pixel_hlsl)?;

    let static_paths = TERRAIN_STATIC_SAMPLER_CONTENT
        .iter()
        .map(|(symbol, path)| ((*symbol).to_string(), (*path).to_string()))
        .collect::<BTreeMap<_, _>>();
    let downstream = TERRAIN_DOWNSTREAM_RESOURCE_OWNERS
        .iter()
        .map(|(symbol, owner)| ((*symbol).to_string(), (*owner).to_string()))
        .collect::<BTreeMap<_, _>>();
    let terrain_id_descriptor = terrain_id_generated_resource_descriptor(terrain_id_map)?;
    let pixel_stage = crate::shader_compile::merged_effect_translation_unit_stage(
        translation_unit,
        crate::ShaderStage::Pixel,
    );
    let mut samplers = Vec::new();
    for sampler in &pixel_stage.samplers {
        let (
            owner_packet,
            admitted_static_content,
            generated_static_payload,
            generated_static_descriptor,
        ) = if let Some(path) = static_paths.get(&sampler.name) {
            (
                "XMAP04".to_string(),
                Some(
                    admission
                        .require_used_content(path)
                        .map_err(|error| {
                            format!(
                                "XMAP04 terrain sampler {} requires admitted {path}: {error}",
                                sampler.name
                            )
                        })?
                        .clone(),
                ),
                None,
                None,
            )
        } else if sampler.name == "TerrainIDMap" {
            (
                "XMAP04".to_string(),
                None,
                Some(Arc::from(terrain_id_map.bytes.clone())),
                Some(terrain_id_descriptor.clone()),
            )
        } else if let Some(owner) = downstream.get(&sampler.name) {
            (owner.clone(), None, None, None)
        } else {
            return Err(format!(
                "effective terrain Effect {effect_name} introduced unowned sampler {}",
                sampler.name
            )
            .into());
        };
        let required = |value: &Option<String>, field: &str| -> Result<String> {
            value.clone().ok_or_else(|| {
                format!(
                    "effective terrain sampler {} omits required {field}",
                    sampler.name
                )
                .into()
            })
        };
        samplers.push(TerrainSamplerRequirement {
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
            mip_map_lod_bias: sampler.mip_map_lod_bias.clone(),
            mip_map_min_lod: sampler.mip_map_min_lod.clone(),
            mip_map_max_lod: sampler.mip_map_max_lod.clone(),
            max_anisotropy: sampler.max_anisotropy.clone(),
            owner_packet,
            admitted_static_content,
            generated_static_payload,
            generated_static_descriptor,
        });
    }
    samplers.sort_by_key(|sampler| sampler.shader_register);
    if samplers
        .iter()
        .enumerate()
        .any(|(index, sampler)| sampler.shader_register != index as u32)
    {
        return Err(
            "effective terrain Effect sampler registers are not the exact contiguous a729 layout"
                .into(),
        );
    }

    let mut ordered_shader_content = Vec::new();
    for path in translation_unit
        .iter()
        .map(|(path, _)| path.as_str())
        .chain([TERRAIN_PLATFORM_HEADER])
    {
        ordered_shader_content.push(
            admission
                .require_used_content(path)
                .map_err(|error| {
                    format!("XMAP04 terrain program requires admitted {path}: {error}")
                })?
                .clone(),
        );
    }
    let mut program = PreparedTerrainProgram {
        effect_file: TERRAIN_EFFECT_FILE.to_ascii_lowercase(),
        effect_name: effect_name.into(),
        ordered_shader_content,
        vertex_translation_unit_sha256: Sha256::hex(vertex_hlsl.as_bytes()),
        pixel_translation_unit_sha256: Sha256::hex(pixel_hlsl.as_bytes()),
        vertex_bytecode_sha256: Sha256::hex(&vertex_bytecode),
        pixel_bytecode_sha256: Sha256::hex(&pixel_bytecode),
        vertex_bytecode: vertex_bytecode.into(),
        pixel_bytecode: pixel_bytecode.into(),
        samplers,
        required_downstream_packets: vec![
            "XMAP05".into(),
            "XMAP07".into(),
            "XMAP09".into(),
            "XMAP10".into(),
        ],
        deterministic_sha256: String::new(),
    };
    program.deterministic_sha256 = prepared_terrain_program_digest(&program);
    program.validate()?;
    Ok(program)
}

#[cfg(windows)]
fn compile_terrain_program_stages(
    vertex_hlsl: &str,
    pixel_hlsl: &str,
) -> Result<(Vec<u8>, Vec<u8>)> {
    Ok((
        crate::d3d_compile_stage(
            vertex_hlsl,
            crate::ShaderStage::Vertex,
            crate::shader_compile::ENGINE_COMPILE_FLAGS,
        )?,
        crate::d3d_compile_stage(
            pixel_hlsl,
            crate::ShaderStage::Pixel,
            crate::shader_compile::ENGINE_COMPILE_FLAGS,
        )?,
    ))
}

#[cfg(not(windows))]
fn compile_terrain_program_stages(
    _vertex_hlsl: &str,
    _pixel_hlsl: &str,
) -> Result<(Vec<u8>, Vec<u8>)> {
    Err("XMAP04 executable terrain-program handoff requires Windows D3DCompile".into())
}

struct Xmap04RuntimeState {
    previous: Option<TerrainLodCullState>,
    last_camera_sha256: Option<String>,
    last_prepared: Option<PreparedTerrainFrame>,
}

pub(crate) struct Xmap04TerrainAdapter {
    content: Arc<Xmap04TerrainContent>,
    options: TerrainLodCullOptions,
    runtime: Mutex<Xmap04RuntimeState>,
}

impl Xmap04TerrainAdapter {
    pub(crate) fn new(
        resolver: &GameFileResolver,
        admission: &ValidatedContentAdmission,
        options: TerrainLodCullOptions,
    ) -> Result<Self> {
        Ok(Self {
            content: Arc::new(Xmap04TerrainContent::admit(resolver, admission, options)?),
            options,
            runtime: Mutex::new(Xmap04RuntimeState {
                previous: None,
                last_camera_sha256: None,
                last_prepared: None,
            }),
        })
    }
}

impl ProductionMapFeatureAdapter for Xmap04TerrainAdapter {
    fn family(&self) -> MapVisualFamily {
        MapVisualFamily::Terrain
    }

    fn prepare(&self, context: &FoundationFeatureContext<'_>) -> Result<FoundationFeatureOutput> {
        let replay_frame_identity_sha256 =
            xmap04_replay_frame_identity_sha256(context.world_frame)?;
        let camera = context
            .viewer
            .camera
            .as_ref()
            .ok_or("XMAP04 terrain requires the XMAP03 viewer camera")?;
        camera.validate()?;
        if self.content.heightmap.width as u32 != camera.map_width
            || self.content.heightmap.height as u32 != camera.map_height
        {
            return Err(format!(
                "XMAP04 admitted heightmap {}x{} disagrees with effective map {}x{}",
                self.content.heightmap.width,
                self.content.heightmap.height,
                camera.map_width,
                camera.map_height
            )
            .into());
        }
        let mut runtime = self
            .runtime
            .lock()
            .map_err(|_| "XMAP04 terrain state lock is poisoned")?;
        let mut prepared = if runtime.last_camera_sha256.as_deref()
            == Some(camera.deterministic_sha256.as_str())
        {
            runtime
                .last_prepared
                .clone()
                .ok_or("XMAP04 terrain cache identity has no prepared payload")?
        } else {
            let (state, enabled_views) =
                prepare_terrain_lod_state(camera, runtime.previous.as_ref(), self.options)?;
            runtime.previous = Some(state.clone());

            let mut draws = Vec::new();
            for (render_view_mask, map_copy_offset) in enabled_views {
                for tile in state.draw_tiles(render_view_mask) {
                    let slot1: Arc<[u8]> = generate_terrain_chunk_slot1(
                        &self.content.heightmap,
                        tile.min_x as i64,
                        tile.min_z as i64,
                        tile.tile_size,
                    )?
                    .into();
                    let cb1: Arc<[u8]> = generate_terrain_chunk_cb1(
                        tile.min_x as f32 + map_copy_offset,
                        tile.min_z as f32,
                        tile.tile_size,
                        tile.vsnap,
                    )?
                    .into();
                    draws.push(PreparedTerrainDraw {
                        render_view_mask,
                        map_copy_offset_f32_bits: map_copy_offset.to_bits(),
                        min_x: tile.min_x,
                        min_z: tile.min_z,
                        tile_size: tile.tile_size,
                        vsnap_f32_bits: tile.vsnap.map(f32::to_bits),
                        slot1_sha256: Sha256::hex(&slot1),
                        cb1_sha256: Sha256::hex(&cb1),
                        slot1,
                        cb1,
                    });
                }
            }

            let mut prepared = PreparedTerrainFrame {
                schema: XMAP04_TERRAIN_LOD_SCHEMA.into(),
                replay_frame_identity_sha256: replay_frame_identity_sha256.clone(),
                camera_frame_sha256: camera.deterministic_sha256.clone(),
                map_width: camera.map_width,
                map_height: camera.map_height,
                draw_hires_terrain: self.options.suppress_near_stride_one,
                render_view_side_plane_bias_f32_bits: 0.0f32.to_bits(),
                cull_state_bytes: state.deterministic_bytes(),
                ordered_used_content: self.content.ordered_used_content.clone(),
                program: self.content.program.clone(),
                underwater_program: self.content.underwater_program.clone(),
                shared_slot0: Arc::clone(&self.content.shared_slot0),
                shared_index: Arc::clone(&self.content.shared_index),
                draws,
                deterministic_sha256: String::new(),
            };
            prepared.deterministic_sha256 = prepared_terrain_digest(&prepared);
            prepared.validate()?;
            runtime.last_camera_sha256 = Some(camera.deterministic_sha256.clone());
            runtime.last_prepared = Some(prepared.clone());
            prepared
        };
        if prepared.replay_frame_identity_sha256 != replay_frame_identity_sha256 {
            prepared.replay_frame_identity_sha256 = replay_frame_identity_sha256;
            prepared.deterministic_sha256 = prepared_terrain_digest(&prepared);
            prepared.validate()?;
            runtime.last_prepared = Some(prepared.clone());
        }

        let registration = Sha256::hex(
            format!("XMAP04\0{XMAP04_TERRAIN_LOD_SCHEMA}\0{XMAP03_A729_EXECUTABLE_SHA256}")
                .as_bytes(),
        );
        let ordered_resources_sha256 = Sha256::hex(
            &prepared
                .ordered_used_content
                .iter()
                .flat_map(|content| content.payload_sha256.bytes())
                .collect::<Vec<_>>(),
        );
        let target_sha256 = Sha256::hex(
            format!(
                "{}\0{}\0{}\0{}",
                camera.map_width, camera.map_height, camera.viewport_width, camera.viewport_height
            )
            .as_bytes(),
        );

        Ok(FoundationFeatureOutput {
            disposition: FamilyDisposition::Present {
                payload_schema: XMAP04_TERRAIN_LOD_SCHEMA.into(),
                producer_registration_sha256: registration.clone(),
                payload_sha256: prepared.deterministic_sha256.clone(),
                ordered_operations: vec![FoundationOperationIdentity {
                    operation_id: "terrain.geometry.prepare".into(),
                    global_order: 1,
                    program_sha256: prepared.program.deterministic_sha256.clone(),
                    ordered_resources_sha256,
                    target_sha256,
                }],
                ordered_used_content: prepared.ordered_used_content.clone(),
            },
            prepared_terrain: Some(prepared),
            prepared_political: None,
            prepared_thin_borders: None,
            prepared_rivers_straits: None,
            prepared_lighting: None,
            prepared_rail_supply: None,
            prepared_weather: None,
            prepared_water: None,
            prepared_effects: None,
            prepared_units: None,
            prepared_combat: None,
            prepared_orders: None,
            prepared_map_modes: None,
            consumed_gap_ids: Vec::new(),
        })
    }
}

fn prepared_terrain_digest(prepared: &PreparedTerrainFrame) -> String {
    let mut identity = Vec::new();
    identity.extend_from_slice(prepared.schema.as_bytes());
    identity.extend_from_slice(prepared.replay_frame_identity_sha256.as_bytes());
    identity.extend_from_slice(prepared.camera_frame_sha256.as_bytes());
    identity.extend_from_slice(&prepared.map_width.to_le_bytes());
    identity.extend_from_slice(&prepared.map_height.to_le_bytes());
    identity.push(prepared.draw_hires_terrain as u8);
    identity.extend_from_slice(&prepared.render_view_side_plane_bias_f32_bits.to_le_bytes());
    identity.extend_from_slice(&Sha256::hex(&prepared.cull_state_bytes).into_bytes());
    identity.extend_from_slice(&Sha256::hex(&prepared.shared_slot0).into_bytes());
    identity.extend_from_slice(&Sha256::hex(&prepared.shared_index).into_bytes());
    identity.extend_from_slice(prepared.program.deterministic_sha256.as_bytes());
    match &prepared.underwater_program {
        Some(program) => {
            identity.push(1);
            identity.extend_from_slice(program.deterministic_sha256.as_bytes());
        }
        None => identity.push(0),
    }
    for content in &prepared.ordered_used_content {
        identity.extend_from_slice(content.normalized_game_relative_path.as_bytes());
        identity.extend_from_slice(content.payload_sha256.as_bytes());
    }
    for draw in &prepared.draws {
        identity.push(draw.render_view_mask);
        identity.extend_from_slice(&draw.map_copy_offset_f32_bits.to_le_bytes());
        identity.extend_from_slice(&draw.min_x.to_le_bytes());
        identity.extend_from_slice(&draw.min_z.to_le_bytes());
        identity.extend_from_slice(&draw.tile_size.to_le_bytes());
        for value in draw.vsnap_f32_bits {
            identity.extend_from_slice(&value.to_le_bytes());
        }
        identity.extend_from_slice(draw.slot1_sha256.as_bytes());
        identity.extend_from_slice(draw.cb1_sha256.as_bytes());
    }
    Sha256::hex(&identity)
}

fn prepared_terrain_program_digest(program: &PreparedTerrainProgram) -> String {
    let mut identity = Vec::new();
    identity.extend_from_slice(program.effect_file.as_bytes());
    identity.extend_from_slice(program.effect_name.as_bytes());
    identity.extend_from_slice(program.vertex_translation_unit_sha256.as_bytes());
    identity.extend_from_slice(program.pixel_translation_unit_sha256.as_bytes());
    identity.extend_from_slice(program.vertex_bytecode_sha256.as_bytes());
    identity.extend_from_slice(program.pixel_bytecode_sha256.as_bytes());
    for content in &program.ordered_shader_content {
        identity.extend_from_slice(content.normalized_game_relative_path.as_bytes());
        identity.extend_from_slice(content.payload_sha256.as_bytes());
    }
    for sampler in &program.samplers {
        identity.extend_from_slice(sampler.name.as_bytes());
        identity.extend_from_slice(&sampler.shader_register.to_le_bytes());
        identity.extend_from_slice(sampler.resource_type.as_bytes());
        identity.extend_from_slice(sampler.mag_filter.as_bytes());
        identity.extend_from_slice(sampler.min_filter.as_bytes());
        identity.extend_from_slice(sampler.mip_filter.as_bytes());
        identity.extend_from_slice(sampler.address_u.as_bytes());
        identity.extend_from_slice(sampler.address_v.as_bytes());
        append_optional_sampler_control(&mut identity, &sampler.mip_map_lod_bias);
        append_optional_sampler_control(&mut identity, &sampler.mip_map_min_lod);
        append_optional_sampler_control(&mut identity, &sampler.mip_map_max_lod);
        append_optional_sampler_control(&mut identity, &sampler.max_anisotropy);
        identity.extend_from_slice(sampler.owner_packet.as_bytes());
        if let Some(content) = &sampler.admitted_static_content {
            identity.extend_from_slice(content.normalized_game_relative_path.as_bytes());
            identity.extend_from_slice(content.payload_sha256.as_bytes());
        }
        if let Some(descriptor) = &sampler.generated_static_descriptor {
            identity.extend_from_slice(descriptor.payload_sha256.as_bytes());
            identity.extend_from_slice(descriptor.descriptor_sha256.as_bytes());
        }
    }
    for packet in &program.required_downstream_packets {
        identity.extend_from_slice(packet.as_bytes());
    }
    Sha256::hex(&identity)
}

fn prepared_underwater_program_digest(program: &PreparedUnderwaterTerrainProgram) -> String {
    let terrain_shape = PreparedTerrainProgram {
        effect_file: program.effect_file.clone(),
        effect_name: program.effect_name.clone(),
        ordered_shader_content: program.ordered_shader_content.clone(),
        vertex_translation_unit_sha256: program.vertex_translation_unit_sha256.clone(),
        pixel_translation_unit_sha256: program.pixel_translation_unit_sha256.clone(),
        vertex_bytecode: Arc::clone(&program.vertex_bytecode),
        pixel_bytecode: Arc::clone(&program.pixel_bytecode),
        vertex_bytecode_sha256: program.vertex_bytecode_sha256.clone(),
        pixel_bytecode_sha256: program.pixel_bytecode_sha256.clone(),
        samplers: program.samplers.clone(),
        required_downstream_packets: program.required_downstream_packets.clone(),
        deterministic_sha256: String::new(),
    };
    let mut identity = b"hoi4.xmap04.underwater-program.v2\0".to_vec();
    identity.extend_from_slice(prepared_terrain_program_digest(&terrain_shape).as_bytes());
    identity.extend_from_slice(program.reflection_contract_sha256.as_bytes());
    Sha256::hex(&identity)
}

fn append_optional_sampler_control(identity: &mut Vec<u8>, value: &Option<String>) {
    match value {
        Some(value) => {
            identity.push(1);
            identity.extend_from_slice(&(value.len() as u64).to_le_bytes());
            identity.extend_from_slice(value.as_bytes());
        }
        None => identity.push(0),
    }
}

fn terrain_id_generated_resource_descriptor(
    terrain_id_map: &crate::SynthesizedTerrainIdMap,
) -> Result<TerrainGeneratedResourceDescriptor> {
    let mut descriptor = TerrainGeneratedResourceDescriptor {
        schema: "hoi4.xmap04.terrain-id-map.generated.v1".into(),
        width: u32::try_from(terrain_id_map.width)?,
        height: u32::try_from(terrain_id_map.height)?,
        dxgi_format: terrain_id_map.descriptor.dxgi_format,
        row_pitch: u32::try_from(terrain_id_map.descriptor.row_pitch)?,
        mip_levels: terrain_id_map.descriptor.mip_count,
        payload_sha256: Sha256::hex(&terrain_id_map.bytes),
        descriptor_sha256: String::new(),
    };
    descriptor.descriptor_sha256 = generated_resource_descriptor_digest(&descriptor);
    Ok(descriptor)
}

fn generated_resource_descriptor_digest(descriptor: &TerrainGeneratedResourceDescriptor) -> String {
    let mut identity = Vec::new();
    identity.extend_from_slice(descriptor.schema.as_bytes());
    identity.extend_from_slice(&descriptor.width.to_le_bytes());
    identity.extend_from_slice(&descriptor.height.to_le_bytes());
    identity.extend_from_slice(&descriptor.dxgi_format.to_le_bytes());
    identity.extend_from_slice(&descriptor.row_pitch.to_le_bytes());
    identity.extend_from_slice(&descriptor.mip_levels.to_le_bytes());
    identity.extend_from_slice(descriptor.payload_sha256.as_bytes());
    Sha256::hex(&identity)
}

fn prepare_terrain_lod_state(
    camera: &ViewerMapCameraState,
    previous: Option<&TerrainLodCullState>,
    options: TerrainLodCullOptions,
) -> Result<(TerrainLodCullState, Vec<(u8, f32)>)> {
    let product_camera = camera.production_camera()?;
    let width = camera.viewport_width as f32;
    let height = camera.viewport_height as f32;
    let mut footprint_x = Vec::with_capacity(4);
    for (screen_x, screen_y) in [(0.0, 0.0), (width, 0.0), (0.0, height), (width, height)] {
        let (world_x, _) = product_camera
            .screen_to_ground_y0(screen_x, screen_y, width, height)?
            .ok_or("XMAP04 terrain camera corner misses the map plane")?;
        footprint_x.push(world_x);
    }
    let min_x = footprint_x.iter().copied().fold(f32::INFINITY, f32::min);
    let max_x = footprint_x
        .iter()
        .copied()
        .fold(f32::NEG_INFINITY, f32::max);
    let map_width = camera.map_width as f32;
    let wrap_offsets = map_wrap_offsets(min_x, max_x, map_width);
    if wrap_offsets.is_empty() {
        return Err("XMAP04 terrain camera produced no admitted map copy".into());
    }

    let main = TerrainRenderViewFrustum::from_view_projection_matrix(
        product_camera.view_projection_matrix,
    )?;
    let projection = ProductionMapCameraProjection::from_engine_constants(width / height)?;
    let shifted_frustum = |camera_x_shift: f32| -> Result<TerrainRenderViewFrustum> {
        let mut eye = camera.current_eye();
        let mut look_at = camera.current_look_at();
        eye[0] += camera_x_shift;
        look_at[0] += camera_x_shift;
        let shifted = projection.camera_from_eye_look_at(eye, look_at)?;
        TerrainRenderViewFrustum::from_view_projection_matrix(shifted.view_projection_matrix)
    };

    let west_wrap = if wrap_offsets.contains(&map_width) {
        Some(shifted_frustum(-map_width)?)
    } else {
        None
    };
    let east_wrap = if wrap_offsets.contains(&-map_width) {
        Some(shifted_frustum(map_width)?)
    } else {
        None
    };
    let state = terrain_lod_cull_update(
        previous,
        camera.current_eye(),
        camera.map_width,
        camera.map_height,
        &TerrainRenderViews {
            main,
            west_wrap,
            east_wrap,
        },
        options,
    )?;
    let mut enabled_views = vec![(1, 0.0)];
    if west_wrap.is_some() {
        enabled_views.push((2, map_width));
    }
    if east_wrap.is_some() {
        enabled_views.push((4, -map_width));
    }
    Ok((state, enabled_views))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_program() -> PreparedTerrainProgram {
        PreparedTerrainProgram {
            effect_file: String::new(),
            effect_name: String::new(),
            ordered_shader_content: Vec::new(),
            vertex_translation_unit_sha256: String::new(),
            pixel_translation_unit_sha256: String::new(),
            vertex_bytecode: Arc::from([]),
            pixel_bytecode: Arc::from([]),
            vertex_bytecode_sha256: String::new(),
            pixel_bytecode_sha256: String::new(),
            samplers: Vec::new(),
            required_downstream_packets: Vec::new(),
            deterministic_sha256: String::new(),
        }
    }

    #[test]
    fn prepared_frame_validation_rejects_metadata_only_payload() {
        let frame = PreparedTerrainFrame {
            schema: XMAP04_TERRAIN_LOD_SCHEMA.into(),
            replay_frame_identity_sha256: "2".repeat(64),
            camera_frame_sha256: "1".repeat(64),
            map_width: 5632,
            map_height: 2048,
            draw_hires_terrain: false,
            render_view_side_plane_bias_f32_bits: 0,
            cull_state_bytes: vec![1, 1],
            ordered_used_content: Vec::new(),
            program: empty_program(),
            underwater_program: None,
            shared_slot0: Arc::from([]),
            shared_index: Arc::from([]),
            draws: Vec::new(),
            deterministic_sha256: String::new(),
        };
        assert!(frame.validate().is_err());
    }

    #[test]
    fn terrain_sha256_requires_canonical_nonzero_identity() {
        let canonical = "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";
        assert!(validate_terrain_sha256(canonical, "test identity").is_ok());
        assert!(validate_terrain_sha256(&canonical.to_ascii_uppercase(), "test identity").is_err());
        assert!(validate_terrain_sha256(&"0".repeat(64), "test identity").is_err());
    }

    #[test]
    fn map_copy_receipt_set_is_ordered_distinct_and_domain_bound() {
        let rows = [
            Xmap04MapCopyReceiptRow {
                render_view_mask: 1,
                map_copy_offset_f32_bits: 0.0f32.to_bits(),
            },
            Xmap04MapCopyReceiptRow {
                render_view_mask: 2,
                map_copy_offset_f32_bits: 5632.0f32.to_bits(),
            },
        ];
        let hash = xmap04_map_copy_receipt_set_sha256(5632, &rows).unwrap();
        assert_eq!(hash.len(), 64);
        assert!(xmap04_map_copy_receipt_set_sha256(5632, &[rows[1], rows[0]]).is_err());
        assert!(xmap04_map_copy_receipt_set_sha256(5632, &[rows[0], rows[0]]).is_err());
        assert!(xmap04_map_copy_receipt_set_sha256(
            5632,
            &[Xmap04MapCopyReceiptRow {
                render_view_mask: 4,
                map_copy_offset_f32_bits: f32::NAN.to_bits(),
            }]
        )
        .is_err());
    }

    #[test]
    fn prepared_map_copy_rows_enforce_exact_engine_signs_and_subset_order() {
        let draw = |render_view_mask: u8, offset: f32| PreparedTerrainDraw {
            render_view_mask,
            map_copy_offset_f32_bits: offset.to_bits(),
            min_x: 0,
            min_z: 0,
            tile_size: crate::TERRAIN_LOD_BASE_TILE_SIZE,
            vsnap_f32_bits: [0; 4],
            slot1: Arc::from([]),
            cb1: Arc::from([]),
            slot1_sha256: String::new(),
            cb1_sha256: String::new(),
        };
        let width = 5632;
        let exact = [
            draw(1, 0.0),
            draw(2, width as f32),
            draw(4, -(width as f32)),
        ];
        assert_eq!(
            derive_ordered_map_copy_receipts(&exact, width)
                .unwrap()
                .iter()
                .map(|row| row.render_view_mask)
                .collect::<Vec<_>>(),
            [1, 2, 4]
        );
        assert!(
            derive_ordered_map_copy_receipts(&[draw(1, 0.0), draw(2, -(width as f32))], width)
                .is_err()
        );
        assert!(derive_ordered_map_copy_receipts(
            &[
                draw(1, 0.0),
                draw(4, -(width as f32)),
                draw(2, width as f32)
            ],
            width
        )
        .is_err());
        assert!(derive_ordered_map_copy_receipts(
            &[draw(1, 0.0), draw(2, width as f32), draw(1, 0.0)],
            width
        )
        .is_err());
        assert!(derive_ordered_map_copy_receipts(&[draw(2, width as f32)], width).is_err());
    }

    #[test]
    fn terrain_program_digest_covers_optional_sampler_controls() {
        let mut program = empty_program();
        program.samplers.push(TerrainSamplerRequirement {
            name: "TerrainDiffuse".into(),
            shader_register: 0,
            resource_type: "Texture2D".into(),
            mag_filter: "Linear".into(),
            min_filter: "Linear".into(),
            mip_filter: "Linear".into(),
            address_u: "Wrap".into(),
            address_v: "Wrap".into(),
            mip_map_lod_bias: None,
            mip_map_min_lod: None,
            mip_map_max_lod: None,
            max_anisotropy: None,
            owner_packet: "XMAP04".into(),
            admitted_static_content: None,
            generated_static_payload: None,
            generated_static_descriptor: None,
        });
        let baseline = prepared_terrain_program_digest(&program);
        program.samplers[0].max_anisotropy = Some("16".into());
        assert_ne!(baseline, prepared_terrain_program_digest(&program));
    }

    #[test]
    fn high_gfx_frame_requires_underwater_program_and_low_gfx_forbids_it() {
        let underwater = PreparedUnderwaterTerrainProgram {
            effect_file: String::new(),
            effect_name: "underwater".into(),
            ordered_shader_content: Vec::new(),
            vertex_translation_unit_sha256: String::new(),
            pixel_translation_unit_sha256: String::new(),
            vertex_bytecode: Arc::from([]),
            pixel_bytecode: Arc::from([]),
            vertex_bytecode_sha256: String::new(),
            pixel_bytecode_sha256: String::new(),
            samplers: Vec::new(),
            required_downstream_packets: Vec::new(),
            reflection_contract_sha256: String::new(),
            deterministic_sha256: String::new(),
        };
        assert!(validate_underwater_profile("terrain", Some(&underwater)).is_ok());
        assert!(validate_underwater_profile("terrain", None).is_err());
        assert!(validate_underwater_profile("terrain_low_gfx", None).is_ok());
        assert!(validate_underwater_profile("terrain_low_gfx", Some(&underwater)).is_err());
    }
}
