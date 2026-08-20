
use crate::xmap10_scene_cb0::{
    xmap10_scene_cb0_layout_sha256, xmap10_scene_cb0_reflected_layout_sha256,
};
use crate::{
    FoundationFeatureContext, FoundationFeatureOutput, GameFileResolver,
    ProductionMapFeatureAdapter, Result, SnowMudAddressing, TerrainLodCullOptions,
};
use portable_content_manifest::UsedContentKey;
use portable_render_stream::{FamilyDisposition, MapVisualFamily, Xmap07PassiveSnowMudCarrier};
use replay_core::ValidatedContentAdmission;
use serde::Serialize;
use snapshot::sha256::Sha256;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

pub const XMAP07_WATER_SCHEMA: &str = "hoi4.xmap07.water-ice.a729.v1";
pub const XMAP07_ASSIGNED_STAGE_NAME: &str = "water_surface";
pub const XMAP07_ASSIGNED_OPERATION_NAME: &str = "water.surface.draw";
pub const XMAP07_SURFACE_TARGET_REQUIREMENT_SCHEMA: &str =
    "hoi4.xmap07.surface-target-requirement.v1";
pub const XMAP07_WATER_EFFECT_FILE: &str = "gfx/FX/pdxwater.shader";
const XMAP07_PLATFORM_HEADER: &str = "gfx/FX/defines_hlsl_dx11.fxh";
const XMAP07_GRAPHICS_DEFINES: &str = "common/defines/00_graphics.lua";
pub const XMAP07_WATER_VERTEX_FORMAT: u32 = 38;
pub const XMAP07_WATER_VERTEX_STRIDE: u32 = 4;
pub const XMAP07_WATER_VERTEX_COUNT: u32 = 4;
pub const XMAP07_WATER_TOPOLOGY: u32 = 5;
pub const XMAP07_REFRACTION_FORMAT: u32 = 87;
pub const XMAP07_SNOW_MUD_FORMAT: u32 = 87;

const WATER_FILE_CONTENT: [&str; 10] = [
    "map/heightmap.bmp",
    "map/terrain/colormap_water_0.dds",
    "map/terrain/lean1.dds",
    "map/terrain/lean2.dds",
    "map/terrain/fow_rgb_waterspec_a.dds",
    "map/terrain/ice_diffuse.dds",
    "map/terrain/ice_noise_0.dds",
    "map/terrain/reflection.dds",
    "map/terrain/underwater_terrain_0.dds",
    XMAP07_GRAPHICS_DEFINES,
];

const WATER_STATIC_SAMPLERS: [(&str, &str, &str); 8] = [
    ("HeightTexture", "map/heightmap.bmp", "Texture2D"),
    ("LeanTexture1", "map/terrain/lean1.dds", "Texture2D"),
    ("LeanTexture2", "map/terrain/lean2.dds", "Texture2D"),
    (
        "SpecularMap",
        "map/terrain/fow_rgb_waterspec_a.dds",
        "Texture2D",
    ),
    ("IceDiffuse", "map/terrain/ice_diffuse.dds", "Texture2D"),
    ("IceNoise", "map/terrain/ice_noise_0.dds", "Texture2D"),
    (
        "ReflectionCubeMap",
        "map/terrain/reflection.dds",
        "TextureCube",
    ),
    (
        "WaterColor",
        "map/terrain/colormap_water_0.dds",
        "Texture2D",
    ),
];

const WATER_DEPENDENT_SAMPLERS: [(&str, &str, &str); 9] = [
    ("ProvinceSecondaryColorMap", "XMAP05", "Texture2D"),
    ("WaterRefraction", "XMAP07", "Texture2D"),
    ("SnowMudTexture", "XMAP07", "Texture2D"),
    ("LightIndexMap", "XMAP09", "Texture2D"),
    ("LightDataMap", "XMAP09", "Texture2D"),
    ("GradientBorderChannel1", "XMAP05", "Texture2D"),
    ("GradientBorderChannel2", "XMAP05", "Texture2D"),
    ("GradientBorderChannel3", "XMAP05", "Texture2D"),
    ("ShadowMap", "XMAP10", "Texture2D"),
];

const XMAP07_AUTHORITY_GAPS: [&str; 2] = [
    "terrain.snow_mud_semantic",
    "weather.province_snow_accumulator",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedWaterSampler {
    pub symbol: String,
    pub shader_register: u32,
    pub resource_type: String,
    pub owner_packet: String,
    pub admitted_static_content: Option<UsedContentKey>,
    pub mag_filter: String,
    pub min_filter: String,
    pub mip_filter: String,
    pub address_u: String,
    pub address_v: String,
    pub mip_map_lod_bias: Option<String>,
    pub mip_map_min_lod: Option<String>,
    pub mip_map_max_lod: Option<String>,
    pub max_anisotropy: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedWaterProgram {
    pub effect_file: String,
    pub effect_name: String,
    pub ordered_shader_content: Vec<UsedContentKey>,
    pub vertex_translation_unit_sha256: String,
    pub pixel_translation_unit_sha256: String,
    pub vertex_bytecode: Arc<[u8]>,
    pub pixel_bytecode: Arc<[u8]>,
    pub vertex_bytecode_sha256: String,
    pub pixel_bytecode_sha256: String,
    pub scene_cb0_reflected_layout_sha256: String,
    pub samplers: Vec<PreparedWaterSampler>,
    pub deterministic_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedWaterGeneratedTexture {
    pub symbol: String,
    pub width: u32,
    pub height: u32,
    pub dxgi_format: u32,
    pub row_pitch: u32,
    pub payload: Arc<[u8]>,
    pub payload_sha256: String,
    pub descriptor_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Xmap07WaterTemporalBinding {
    pub replay_frame_identity_sha256: String,
    pub animation_time_f32_bits: u32,
    pub season_lerp_f32_bits: u32,
    pub deterministic_sha256: String,
}

impl Xmap07WaterTemporalBinding {
    pub fn validate(&self) -> Result<()> {
        let animation = f32::from_bits(self.animation_time_f32_bits);
        let season = f32::from_bits(self.season_lerp_f32_bits);
        if validate_sha256(
            &self.replay_frame_identity_sha256,
            "water temporal replay frame",
        )
        .is_err()
            || !animation.is_finite()
            || animation < 0.0
            || !season.is_finite()
            || !(0.0..=1.0).contains(&season)
            || self.deterministic_sha256 != temporal_digest(self)
        {
            return Err("XMAP07 viewer-time binding is invalid".into());
        }
        Ok(())
    }

    pub fn map_object_time_season_bytes(&self) -> [u8; 8] {
        let mut bytes = [0u8; 8];
        bytes[0..4].copy_from_slice(&self.animation_time_f32_bits.to_le_bytes());
        bytes[4..8].copy_from_slice(&self.season_lerp_f32_bits.to_le_bytes());
        bytes
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct WaterDependentResourceBinding {
    pub symbol: String,
    pub owner_packet: String,
    pub replay_frame_identity_sha256: String,
    pub camera_view_sha256: Option<String>,
    pub resource_sha256: String,
    pub descriptor_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WaterVisibleDrawDependencies {
    pub replay_frame_identity_sha256: String,
    pub scene_cb0_sha256: String,
    pub scene_cb0_descriptor_sha256: String,
    pub scene_cb0_composition_receipt_sha256: String,
    pub scene_cb0_reflected_layout_sha256: String,
    pub point_lights_cb2_sha256: Option<String>,
    pub point_lights_cb2_descriptor_sha256: Option<String>,
    pub point_lights_cb2_replay_frame_identity_sha256: Option<String>,
    pub point_lights_cb2_camera_view_sha256: Option<String>,
    pub underwater_draw_receipt_sha256: Option<String>,
    pub resources: Vec<WaterDependentResourceBinding>,
}

/// XMAP07 does not claim ownership of any other CB0 lane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Xmap07SceneSnowMudInput {
    pub date: String,
    pub hour_index: i64,
    pub replay_frame_identity_sha256: String,
    pub snow_mud_fade_f32_bits: u32,
    pub snow_mud_payload_sha256: String,
    pub snow_mud_authority_sha256: String,
    pub water_frame_sha256: String,
    pub deterministic_sha256: String,
}

impl Xmap07SceneSnowMudInput {
    pub fn validate(&self) -> Result<()> {
        let fade = f32::from_bits(self.snow_mud_fade_f32_bits);
        if self.date.trim().is_empty()
            || validate_sha256(
                &self.replay_frame_identity_sha256,
                "scene SnowMud replay frame",
            )
            .is_err()
            || !fade.is_finite()
            || !(0.0..=1.0).contains(&fade)
            || validate_sha256(&self.snow_mud_payload_sha256, "scene SnowMud payload").is_err()
            || validate_sha256(&self.snow_mud_authority_sha256, "scene SnowMud authority").is_err()
            || validate_sha256(&self.water_frame_sha256, "scene SnowMud water frame").is_err()
            || self.deterministic_sha256 != scene_snow_mud_input_digest(self)
        {
            return Err("XMAP07 scene SnowMud input is invalid".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ReadyWaterDrawContract {
    pub replay_frame_identity_sha256: String,
    pub camera_view_sha256: String,
    pub water_frame_sha256: String,
    pub water_program_sha256: String,
    pub scene_cb0_sha256: String,
    pub scene_cb0_descriptor_sha256: String,
    pub scene_cb0_composition_receipt_sha256: String,
    pub scene_cb0_reflected_layout_sha256: String,
    pub point_lights_cb2_sha256: Option<String>,
    pub point_lights_cb2_descriptor_sha256: Option<String>,
    pub point_lights_cb2_camera_view_sha256: Option<String>,
    pub underwater_draw_receipt_sha256: Option<String>,
    pub ordered_dependency_resources: Vec<WaterDependentResourceBinding>,
    pub deterministic_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Xmap07SurfaceTargetRequirement {
    pub schema: String,
    pub target_role: String,
    pub viewport_width: u32,
    pub viewport_height: u32,
    pub refraction_descriptor_sha256: String,
    pub deterministic_sha256: String,
}

impl Xmap07SurfaceTargetRequirement {
    fn seal(
        viewport_width: u32,
        viewport_height: u32,
        refraction_descriptor_sha256: String,
    ) -> Result<Self> {
        let mut requirement = Self {
            schema: XMAP07_SURFACE_TARGET_REQUIREMENT_SCHEMA.into(),
            target_role: "MainScene".into(),
            viewport_width,
            viewport_height,
            refraction_descriptor_sha256,
            deterministic_sha256: String::new(),
        };
        requirement.deterministic_sha256 = surface_target_requirement_digest(&requirement);
        requirement.validate()?;
        Ok(requirement)
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema != XMAP07_SURFACE_TARGET_REQUIREMENT_SCHEMA
            || self.target_role != "MainScene"
            || self.viewport_width == 0
            || self.viewport_height == 0
            || validate_sha256(
                &self.refraction_descriptor_sha256,
                "surface target refraction descriptor",
            )
            .is_err()
            || self.deterministic_sha256 != surface_target_requirement_digest(self)
        {
            return Err("XMAP07 surface target requirement is invalid".into());
        }
        Ok(())
    }

    pub fn validate_actual(&self, target: &crate::ProductionMainSceneTargetDesc) -> Result<()> {
        self.validate()?;
        target.validate()?;
        if target.color_width != self.viewport_width
            || target.color_height != self.viewport_height
            || target.depth_width != self.viewport_width
            || target.depth_height != self.viewport_height
        {
            return Err("XMAP07 MainScene target does not satisfy the deferred requirement".into());
        }
        Ok(())
    }
}

impl ReadyWaterDrawContract {
    pub fn validate(&self) -> Result<()> {
        for (digest, label) in [
            (
                &self.replay_frame_identity_sha256,
                "ready water replay frame",
            ),
            (&self.camera_view_sha256, "ready water camera frame"),
            (&self.water_frame_sha256, "ready water frame"),
            (&self.water_program_sha256, "ready water program"),
            (&self.scene_cb0_sha256, "ready water SceneCB0"),
            (
                &self.scene_cb0_descriptor_sha256,
                "ready water SceneCB0 descriptor",
            ),
            (
                &self.scene_cb0_composition_receipt_sha256,
                "ready water SceneCB0 composition receipt",
            ),
            (
                &self.scene_cb0_reflected_layout_sha256,
                "ready water SceneCB0 reflected layout",
            ),
        ] {
            validate_sha256(digest, label)?;
        }
        match (
            self.point_lights_cb2_sha256.as_deref(),
            self.point_lights_cb2_descriptor_sha256.as_deref(),
            self.point_lights_cb2_camera_view_sha256.as_deref(),
            self.underwater_draw_receipt_sha256.as_deref(),
        ) {
            (Some(payload), Some(descriptor), Some(camera), Some(underwater)) => {
                validate_sha256(payload, "ready water PointLightsCB2")?;
                validate_sha256(descriptor, "ready water PointLightsCB2 descriptor")?;
                validate_sha256(camera, "ready water PointLightsCB2 camera")?;
                if camera != self.camera_view_sha256 {
                    return Err("ready water PointLightsCB2 belongs to another camera".into());
                }
                validate_sha256(underwater, "ready water underwater draw receipt")?;
            }
            (None, None, None, None) => {}
            _ => {
                return Err("ready water high/low-GFX dependency identities are incomplete".into())
            }
        }
        let mut symbols = BTreeSet::new();
        for binding in &self.ordered_dependency_resources {
            if binding.symbol.is_empty()
                || binding.owner_packet.is_empty()
                || binding.owner_packet == "XMAP07"
                || !symbols.insert(binding.symbol.as_str())
            {
                return Err("ready water dependency resource identity is invalid".into());
            }
            validate_sha256(&binding.resource_sha256, "ready water dependency resource")?;
            validate_sha256(
                &binding.replay_frame_identity_sha256,
                "ready water dependency replay frame",
            )?;
            if binding.replay_frame_identity_sha256 != self.replay_frame_identity_sha256 {
                return Err("ready water dependency belongs to a different replay frame".into());
            }
            match binding.owner_packet.as_str() {
                "XMAP09" | "XMAP10" => validate_sha256(
                    binding
                        .camera_view_sha256
                        .as_deref()
                        .ok_or("ready water camera-owned resource omitted camera identity")?,
                    "ready water dependency camera",
                )
                .and_then(|_| {
                    if binding.camera_view_sha256.as_deref()
                        == Some(self.camera_view_sha256.as_str())
                    {
                        Ok(())
                    } else {
                        Err("ready water camera-owned resource belongs to another camera".into())
                    }
                })?,
                "XMAP05" if binding.camera_view_sha256.is_none() => {}
                _ => {
                    return Err(
                        "ready water dependency has an invalid owner/camera identity shape".into(),
                    )
                }
            }
            validate_sha256(
                &binding.descriptor_sha256,
                "ready water dependency descriptor",
            )?;
        }
        if self.deterministic_sha256 != ready_water_draw_contract_digest(self) {
            return Err("ready water draw contract digest mismatch".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedWaterFrame {
    pub schema: String,
    pub assigned_stage: portable_render_stream::AssignedStage,
    pub surface_target_requirement: Xmap07SurfaceTargetRequirement,
    pub map_width: u32,
    pub map_height: u32,
    pub viewport_width: u32,
    pub viewport_height: u32,
    pub camera_view_sha256: String,
    pub replay_frame_identity_sha256: String,
    pub date: String,
    pub hour_index: i64,
    pub program: PreparedWaterProgram,
    pub ordered_used_content: Vec<UsedContentKey>,
    pub water_quad: Arc<[u8]>,
    pub water_cb3: Arc<[u8]>,
    pub temporal: Xmap07WaterTemporalBinding,
    pub snow_mud: PreparedWaterGeneratedTexture,
    pub snow_mud_fade_f32_bits: u32,
    pub snow_mud_authority_sha256: String,
    pub refraction_width: u32,
    pub refraction_height: u32,
    pub refraction_dxgi_format: u32,
    pub refraction_descriptor_sha256: String,
    pub underwater_required: bool,
    pub deterministic_sha256: String,
}

impl PreparedWaterProgram {
    pub fn validate(&self) -> Result<()> {
        if self.effect_file != XMAP07_WATER_EFFECT_FILE.to_ascii_lowercase()
            || !matches!(self.effect_name.as_str(), "water" | "water_low_gfx")
            || self.ordered_shader_content.is_empty()
            || self.vertex_bytecode.is_empty()
            || self.pixel_bytecode.is_empty()
            || self.vertex_bytecode_sha256 != Sha256::hex(&self.vertex_bytecode)
            || self.pixel_bytecode_sha256 != Sha256::hex(&self.pixel_bytecode)
            || self.scene_cb0_reflected_layout_sha256
                != xmap10_scene_cb0_reflected_layout_sha256(
                    &self.vertex_bytecode,
                    &self.pixel_bytecode,
                )?
            || self.scene_cb0_reflected_layout_sha256 != xmap10_scene_cb0_layout_sha256()
            || self.samplers.is_empty()
            || self.deterministic_sha256 != water_program_digest(self)
        {
            return Err("XMAP07 executable water program is invalid".into());
        }
        let vertex = crate::parse_dxbc_reflection(&self.vertex_bytecode)?;
        let pixel = crate::parse_dxbc_reflection(&self.pixel_bytecode)?;
        if vertex.target >> 16 != 0xfffe
            || pixel.target >> 16 != 0xffff
            || !vertex.inputs.iter().any(|input| {
                input.semantic_name.eq_ignore_ascii_case("POSITION")
                    && input.semantic_index == 0
                    && input.component_type == 2
                    && input.mask & 0x03 == 0x03
            })
        {
            return Err("XMAP07 water bytecode has the wrong stage/input contract".into());
        }
        let has_cbuffer = |reflection: &crate::DxbcReflection, slot: u32| {
            reflection.resources.iter().any(|resource| {
                resource.resource_type == 0
                    && resource.bind_point == slot
                    && resource.bind_count == 1
            })
        };
        if !has_cbuffer(&vertex, 0)
            || !has_cbuffer(&vertex, 3)
            || !has_cbuffer(&pixel, 0)
            || !has_cbuffer(&pixel, 3)
            || (self.effect_name == "water" && !has_cbuffer(&pixel, 2))
            || (self.effect_name == "water_low_gfx" && has_cbuffer(&pixel, 2))
        {
            return Err("XMAP07 compiled constant-buffer contract is incomplete".into());
        }
        let mut symbols = BTreeSet::new();
        let mut registers = BTreeSet::new();
        for sampler in &self.samplers {
            if sampler.symbol.is_empty()
                || sampler.resource_type.is_empty()
                || sampler.owner_packet.is_empty()
                || sampler.mag_filter.is_empty()
                || sampler.min_filter.is_empty()
                || sampler.mip_filter.is_empty()
                || sampler.address_u.is_empty()
                || sampler.address_v.is_empty()
                || !symbols.insert(sampler.symbol.as_str())
                || !registers.insert(sampler.shader_register)
            {
                return Err("XMAP07 water sampler contract is incomplete".into());
            }
            if let Some(content) = &sampler.admitted_static_content {
                content
                    .validate()
                    .map_err(|error| format!("XMAP07 sampler content identity: {error}"))?;
                if sampler.owner_packet != "XMAP07" {
                    return Err("XMAP07 attached content to a downstream-owned sampler".into());
                }
            }
            if !pixel.resources.iter().any(|resource| {
                resource.name == format!("{}_Texture", sampler.symbol)
                    && resource.resource_type == 2
                    && resource.bind_point == sampler.shader_register
                    && resource.bind_count == 1
            }) || !pixel.resources.iter().any(|resource| {
                resource.name == format!("{}_Sampler", sampler.symbol)
                    && resource.resource_type == 3
                    && resource.bind_point == sampler.shader_register
                    && resource.bind_count == 1
            }) {
                return Err(format!(
                    "XMAP07 compiled pixel program omitted sampler {} at register {}",
                    sampler.symbol, sampler.shader_register
                )
                .into());
            }
        }
        for resource in pixel
            .resources
            .iter()
            .filter(|resource| matches!(resource.resource_type, 2 | 3))
        {
            let symbol = resource
                .name
                .strip_suffix("_Texture")
                .or_else(|| resource.name.strip_suffix("_Sampler"))
                .ok_or_else(|| {
                    format!(
                        "XMAP07 compiled pixel program introduced unclassified resource {}",
                        resource.name
                    )
                })?;
            if !self.samplers.iter().any(|sampler| {
                sampler.symbol == symbol && sampler.shader_register == resource.bind_point
            }) {
                return Err(format!(
                    "XMAP07 compiled pixel program introduced unowned resource {}",
                    resource.name
                )
                .into());
            }
        }
        Ok(())
    }
}

impl PreparedWaterFrame {
    pub fn validate(&self) -> Result<()> {
        if self.schema != XMAP07_WATER_SCHEMA
            || self.map_width == 0
            || self.map_height == 0
            || self.map_width > i16::MAX as u32
            || self.map_height > i16::MAX as u32
            || self.map_width % 4 != 0
            || self.map_height % 4 != 0
            || self.viewport_width == 0
            || self.viewport_height == 0
            || validate_sha256(&self.camera_view_sha256, "water camera view").is_err()
            || validate_sha256(&self.replay_frame_identity_sha256, "water replay frame").is_err()
            || self.date.trim().is_empty()
            || self.water_quad.len() != 16
            || self.water_cb3.len() != 16
            || self.refraction_width != self.viewport_width
            || self.refraction_height != self.viewport_height
            || self.refraction_dxgi_format != XMAP07_REFRACTION_FORMAT
            || self.underwater_required != (self.program.effect_name == "water")
            || self.deterministic_sha256 != water_frame_digest(self)
        {
            return Err("XMAP07 prepared water frame is incomplete".into());
        }
        self.assigned_stage
            .validate()
            .map_err(|error| format!("XMAP07 assigned water stage: {error}"))?;
        if self.assigned_stage.stage_name != "water_surface"
            || self.assigned_stage.operation_name != "water.surface.draw"
        {
            return Err("XMAP07 prepared water frame has the wrong assigned stage".into());
        }
        self.surface_target_requirement.validate()?;
        if self.surface_target_requirement.viewport_width != self.viewport_width
            || self.surface_target_requirement.viewport_height != self.viewport_height
            || self.surface_target_requirement.refraction_descriptor_sha256
                != self.refraction_descriptor_sha256
        {
            return Err("XMAP07 prepared water target requirement drifted".into());
        }
        self.program.validate()?;
        self.temporal.validate()?;
        if self.temporal.replay_frame_identity_sha256 != self.replay_frame_identity_sha256 {
            return Err("XMAP07 temporal binding belongs to a different replay frame".into());
        }
        validate_generated_texture(&self.snow_mud)?;
        validate_sha256(&self.snow_mud_authority_sha256, "water SnowMud authority")?;
        let snow_mud_fade = f32::from_bits(self.snow_mud_fade_f32_bits);
        if !snow_mud_fade.is_finite() || !(0.0..=1.0).contains(&snow_mud_fade) {
            return Err("XMAP07 SnowMud fade is invalid".into());
        }
        if self.snow_mud.width != self.map_width / 4 || self.snow_mud.height != self.map_height / 4
        {
            return Err(
                "XMAP07 SnowMud texture is not the active map's exact quarter resolution".into(),
            );
        }
        validate_sha256(
            &self.refraction_descriptor_sha256,
            "water refraction descriptor",
        )?;
        let expected_cb3 = water_cb3_bytes(
            f32::from_bits(self.temporal.animation_time_f32_bits),
            self.viewport_width,
            self.viewport_height,
        )?;
        if self.water_cb3.as_ref() != expected_cb3 {
            return Err("XMAP07 water CB3 disagrees with viewer time/viewport".into());
        }
        if self.water_quad.as_ref() != water_quad_bytes(self.map_width, self.map_height)?.as_slice()
        {
            return Err("XMAP07 water quad disagrees with effective map dimensions".into());
        }
        Ok(())
    }

    pub fn scene_snow_mud_input(&self) -> Result<Xmap07SceneSnowMudInput> {
        self.validate()?;
        let mut input = Xmap07SceneSnowMudInput {
            date: self.date.clone(),
            hour_index: self.hour_index,
            replay_frame_identity_sha256: self.replay_frame_identity_sha256.clone(),
            snow_mud_fade_f32_bits: self.snow_mud_fade_f32_bits,
            snow_mud_payload_sha256: self.snow_mud.payload_sha256.clone(),
            snow_mud_authority_sha256: self.snow_mud_authority_sha256.clone(),
            water_frame_sha256: self.deterministic_sha256.clone(),
            deterministic_sha256: String::new(),
        };
        input.deterministic_sha256 = scene_snow_mud_input_digest(&input);
        input.validate()?;
        Ok(input)
    }

    pub fn static_resource(&self, symbol: &str) -> Option<&UsedContentKey> {
        WATER_STATIC_SAMPLERS
            .iter()
            .find(|(candidate, _, _)| *candidate == symbol)
            .and_then(|(_, path, _)| {
                self.ordered_used_content.iter().find(|content| {
                    content
                        .normalized_game_relative_path
                        .eq_ignore_ascii_case(path)
                })
            })
    }

    pub fn admit_visible_draw_dependencies(
        &self,
        dependencies: WaterVisibleDrawDependencies,
    ) -> Result<ReadyWaterDrawContract> {
        self.validate()?;
        validate_sha256(
            &dependencies.replay_frame_identity_sha256,
            "water visible-draw replay frame",
        )?;
        if dependencies.replay_frame_identity_sha256 != self.replay_frame_identity_sha256 {
            return Err("visible water dependencies belong to a different replay frame".into());
        }
        validate_sha256(&dependencies.scene_cb0_sha256, "water scene CB0")?;
        validate_sha256(
            &dependencies.scene_cb0_descriptor_sha256,
            "water scene CB0 descriptor",
        )?;
        validate_sha256(
            &dependencies.scene_cb0_composition_receipt_sha256,
            "water scene CB0 composition receipt",
        )?;
        validate_sha256(
            &dependencies.scene_cb0_reflected_layout_sha256,
            "water scene CB0 reflected layout",
        )?;
        match (
            self.program.effect_name.as_str(),
            dependencies.point_lights_cb2_sha256.as_deref(),
            dependencies.point_lights_cb2_descriptor_sha256.as_deref(),
            dependencies
                .point_lights_cb2_replay_frame_identity_sha256
                .as_deref(),
            dependencies.point_lights_cb2_camera_view_sha256.as_deref(),
            dependencies.underwater_draw_receipt_sha256.as_deref(),
        ) {
            (
                "water",
                Some(identity),
                Some(descriptor),
                Some(replay_frame),
                Some(camera),
                Some(underwater),
            ) => {
                validate_sha256(identity, "water point-lights CB2")?;
                validate_sha256(descriptor, "water point-lights CB2 descriptor")?;
                validate_sha256(replay_frame, "water point-lights replay frame")?;
                validate_sha256(camera, "water point-lights camera frame")?;
                validate_sha256(underwater, "water underwater draw receipt")?;
                if replay_frame != self.replay_frame_identity_sha256 {
                    return Err("water point-lights belong to a different replay frame".into());
                }
                if camera != self.camera_view_sha256 {
                    return Err("water point-lights belong to a different camera frame".into());
                }
            }
            ("water_low_gfx", None, None, None, None, None) => {}
            ("water", _, _, _, _, _) => return Err(
                "high-gfx water requires camera-bound XMAP09 lights and completed underwater draw"
                    .into(),
            ),
            ("water_low_gfx", _, _, _, _, _) => {
                return Err("low-gfx water must not claim high-gfx dependencies".into())
            }
            _ => return Err("XMAP07 visible-draw effect selection is invalid".into()),
        }
        let expected = self
            .program
            .samplers
            .iter()
            .filter(|sampler| sampler.owner_packet != "XMAP07")
            .map(|sampler| (sampler.symbol.as_str(), sampler.owner_packet.as_str()))
            .collect::<Vec<_>>();
        if dependencies.resources.len() != expected.len() {
            return Err("visible water dependencies are incomplete".into());
        }
        for (binding, (symbol, owner)) in dependencies.resources.iter().zip(expected) {
            if binding.symbol != symbol || binding.owner_packet != owner {
                return Err(format!(
                    "visible water dependency order/owner mismatch: expected {owner}:{symbol}"
                )
                .into());
            }
            validate_sha256(&binding.resource_sha256, "water dependency resource")?;
            validate_sha256(&binding.descriptor_sha256, "water dependency descriptor")?;
            match owner {
                "XMAP09" | "XMAP10"
                    if binding.camera_view_sha256.as_deref()
                        == Some(self.camera_view_sha256.as_str()) => {}
                "XMAP05" if binding.camera_view_sha256.is_none() => {}
                _ => {
                    return Err(format!(
                        "visible water dependency {owner}:{symbol} has the wrong camera identity"
                    )
                    .into())
                }
            }
        }
        let mut contract = ReadyWaterDrawContract {
            replay_frame_identity_sha256: self.replay_frame_identity_sha256.clone(),
            camera_view_sha256: self.camera_view_sha256.clone(),
            water_frame_sha256: self.deterministic_sha256.clone(),
            water_program_sha256: self.program.deterministic_sha256.clone(),
            scene_cb0_sha256: dependencies.scene_cb0_sha256,
            scene_cb0_descriptor_sha256: dependencies.scene_cb0_descriptor_sha256,
            scene_cb0_composition_receipt_sha256: dependencies.scene_cb0_composition_receipt_sha256,
            scene_cb0_reflected_layout_sha256: dependencies.scene_cb0_reflected_layout_sha256,
            point_lights_cb2_sha256: dependencies.point_lights_cb2_sha256,
            point_lights_cb2_descriptor_sha256: dependencies.point_lights_cb2_descriptor_sha256,
            point_lights_cb2_camera_view_sha256: dependencies.point_lights_cb2_camera_view_sha256,
            underwater_draw_receipt_sha256: dependencies.underwater_draw_receipt_sha256,
            ordered_dependency_resources: dependencies.resources,
            deterministic_sha256: String::new(),
        };
        contract.deterministic_sha256 = ready_water_draw_contract_digest(&contract);
        contract.validate()?;
        Ok(contract)
    }
}

struct Xmap07Content {
    program: PreparedWaterProgram,
    ordered_used_content: Vec<UsedContentKey>,
}

pub(crate) struct Xmap07WaterAdapter {
    content: Arc<Xmap07Content>,
    high_gfx: bool,
}

impl Xmap07WaterAdapter {
    pub(crate) fn new(
        resolver: &GameFileResolver,
        admission: &ValidatedContentAdmission,
        options: TerrainLodCullOptions,
    ) -> Result<Self> {
        let translation_unit = crate::shader_compile::effect_translation_unit_files(
            resolver,
            XMAP07_WATER_EFFECT_FILE,
        )?;
        let mut ordered_used_content = Vec::new();
        let mut seen = BTreeSet::new();
        for path in translation_unit
            .iter()
            .map(|(path, _)| path.as_str())
            .chain([XMAP07_PLATFORM_HEADER])
            .chain(WATER_FILE_CONTENT)
        {
            let normalized = path.replace('\\', "/").to_ascii_lowercase();
            if !seen.insert(normalized) {
                continue;
            }
            ordered_used_content.push(
                admission
                    .require_used_content(path)
                    .map_err(|error| format!("XMAP07 requires admitted {path}: {error}"))?
                    .clone(),
            );
        }
        let program = prepare_water_program(
            resolver,
            admission,
            &translation_unit,
            options.high_gfx_shaders,
        )?;
        Ok(Self {
            content: Arc::new(Xmap07Content {
                program,
                ordered_used_content,
            }),
            high_gfx: options.high_gfx_shaders,
        })
    }
}

impl ProductionMapFeatureAdapter for Xmap07WaterAdapter {
    fn family(&self) -> MapVisualFamily {
        MapVisualFamily::WaterIce
    }

    fn prepare(&self, context: &FoundationFeatureContext<'_>) -> Result<FoundationFeatureOutput> {
        let assigned_stage = context
            .require_assigned_stage("water_surface", "water.surface.draw")?
            .clone();
        let camera = context
            .viewer
            .camera
            .as_ref()
            .ok_or("XMAP07 water requires the XMAP03 viewer camera")?;
        let date = context
            .world_frame
            .date()
            .ok_or("XMAP07 water requires the current replay date")?
            .to_string();
        let snow_packet = context
            .world_frame
            .xmap07_snow_mud_runtime()
            .ok_or("XMAP07 water requires the exact passive global SnowMud carrier")?;
        let snow_mud_addressing = exact_snow_mud_addressing(camera.map_width, camera.map_height)?;
        let snow_mud = generated_snow_mud(
            snow_packet,
            snow_mud_addressing.texture_width,
            snow_mud_addressing.texture_height,
        )?;
        let fade_phase = f32::from_bits(snow_packet.fade_f32_bits);
        if !fade_phase.is_finite() || !(0.0..=1.0).contains(&fade_phase) {
            return Err("XMAP07 exact SnowMud carrier has an invalid fade value".into());
        }
        let snow_mud_fade_f32_bits = snow_packet.fade_f32_bits;
        let snow_mud_authority_sha256 = snow_packet.authority_sha256.clone();
        let animation_time = f32::from_bits(context.viewer.animation_time_f32_bits);
        if !animation_time.is_finite() || animation_time < 0.0 {
            return Err("XMAP07 viewer animation time is invalid".into());
        }
        let replay_frame_identity_sha256 =
            crate::xmap04_replay_frame_identity_sha256(context.world_frame)?;
        let mut temporal = Xmap07WaterTemporalBinding {
            replay_frame_identity_sha256: replay_frame_identity_sha256.clone(),
            animation_time_f32_bits: animation_time.to_bits(),
            season_lerp_f32_bits: fade_phase.to_bits(),
            deterministic_sha256: String::new(),
        };
        temporal.deterministic_sha256 = temporal_digest(&temporal);
        let refraction_descriptor_sha256 = crate::xmap04_underwater_color_descriptor_sha256(
            context.viewer.viewport_width,
            context.viewer.viewport_height,
            XMAP07_REFRACTION_FORMAT,
            1,
            0,
        );
        let surface_target_requirement = Xmap07SurfaceTargetRequirement::seal(
            context.viewer.viewport_width,
            context.viewer.viewport_height,
            refraction_descriptor_sha256.clone(),
        )?;
        let mut prepared = PreparedWaterFrame {
            schema: XMAP07_WATER_SCHEMA.into(),
            assigned_stage,
            surface_target_requirement,
            map_width: camera.map_width,
            map_height: camera.map_height,
            viewport_width: context.viewer.viewport_width,
            viewport_height: context.viewer.viewport_height,
            camera_view_sha256: camera.deterministic_sha256.clone(),
            replay_frame_identity_sha256,
            date,
            hour_index: context.world_frame.hour_index(),
            program: self.content.program.clone(),
            ordered_used_content: self.content.ordered_used_content.clone(),
            water_quad: water_quad_bytes(camera.map_width, camera.map_height)?.into(),
            water_cb3: water_cb3_bytes(
                animation_time,
                context.viewer.viewport_width,
                context.viewer.viewport_height,
            )?
            .into(),
            temporal,
            snow_mud,
            snow_mud_fade_f32_bits,
            snow_mud_authority_sha256,
            refraction_width: context.viewer.viewport_width,
            refraction_height: context.viewer.viewport_height,
            refraction_dxgi_format: XMAP07_REFRACTION_FORMAT,
            refraction_descriptor_sha256,
            underwater_required: self.high_gfx,
            deterministic_sha256: String::new(),
        };
        prepared.deterministic_sha256 = water_frame_digest(&prepared);
        prepared.validate()?;
        let registration =
            Sha256::hex(format!("XMAP07\0{XMAP07_WATER_SCHEMA}\0FUN_141261100").as_bytes());
        let resources_sha256 = ordered_resources_digest(&prepared);
        let target_sha256 = surface_target_digest(&prepared);
        let surface_operation = prepared
            .assigned_stage
            .foundation_operation_identity(
                prepared.program.deterministic_sha256.clone(),
                resources_sha256,
                target_sha256,
            )
            .map_err(|error| format!("XMAP07 assigned water operation: {error}"))?;
        Ok(FoundationFeatureOutput {
            disposition: FamilyDisposition::Present {
                payload_schema: XMAP07_WATER_SCHEMA.into(),
                producer_registration_sha256: registration,
                payload_sha256: prepared.deterministic_sha256.clone(),
                ordered_operations: vec![surface_operation],
                ordered_used_content: prepared.ordered_used_content.clone(),
            },
            prepared_terrain: None,
            prepared_political: None,
            prepared_water: Some(prepared),
            prepared_thin_borders: None,
            prepared_rivers_straits: None,
            prepared_lighting: None,
            prepared_weather: None,
            prepared_effects: None,
            prepared_units: None,
            prepared_combat: None,
            prepared_orders: None,
            prepared_rail_supply: None,
            prepared_map_modes: None,
            prepared_map_furniture: None,
            element_diagnostics: Vec::new(),
            consumed_gap_ids: XMAP07_AUTHORITY_GAPS
                .iter()
                .map(|gap| (*gap).to_string())
                .collect(),
        })
    }
}

fn exact_snow_mud_addressing(map_width: u32, map_height: u32) -> Result<SnowMudAddressing> {
    if map_width == 0 || map_height == 0 || map_width % 4 != 0 || map_height % 4 != 0 {
        return Err(format!(
            "XMAP07 active map {map_width}x{map_height} cannot produce the engine quarter-resolution SnowMud texture exactly"
        )
        .into());
    }
    Ok(SnowMudAddressing {
        map_size_x: map_width as f32,
        map_size_y: map_height as f32,
        fow_pow2_x: 1.0,
        fow_pow2_y: 1.0,
        texture_width: usize::try_from(map_width / 4)?,
        texture_height: usize::try_from(map_height / 4)?,
    })
}

fn prepare_water_program(
    resolver: &GameFileResolver,
    admission: &ValidatedContentAdmission,
    translation_unit: &[(String, crate::EffectFile)],
    high_gfx: bool,
) -> Result<PreparedWaterProgram> {
    let effect_name = if high_gfx { "water" } else { "water_low_gfx" };
    let parsed = translation_unit
        .last()
        .map(|(_, file)| file)
        .ok_or("XMAP07 water translation unit is empty")?;
    let effect = parsed.effect(effect_name).ok_or_else(|| {
        format!("effective {XMAP07_WATER_EFFECT_FILE} has no Effect named {effect_name}")
    })?;
    if effect.vertex_shader.is_none() || effect.pixel_shader.is_none() {
        return Err(format!("effective water Effect {effect_name} omits a shader stage").into());
    }
    let (map_width, map_height) = admitted_bmp_dimensions(
        resolver,
        admission
            .require_used_content("map/provinces.bmp")
            .map_err(|error| format!("XMAP07 map dimensions: {error}"))?,
    )?;
    let constants =
        crate::derive_a729_engine_map_shader_constants(map_width, map_height, false, 0)?;
    let defines = crate::engine_map_shader_defines(constants);
    let vertex_hlsl = crate::shader_compile::build_stage_hlsl_with_defines(
        resolver,
        XMAP07_WATER_EFFECT_FILE,
        effect_name,
        crate::ShaderStage::Vertex,
        &defines,
    )?;
    let pixel_hlsl = crate::shader_compile::build_stage_hlsl_with_defines(
        resolver,
        XMAP07_WATER_EFFECT_FILE,
        effect_name,
        crate::ShaderStage::Pixel,
        &defines,
    )?;
    let (vertex_bytecode, pixel_bytecode) = compile_water_stages(&vertex_hlsl, &pixel_hlsl)?;
    let static_map = WATER_STATIC_SAMPLERS
        .iter()
        .map(|(symbol, path, ty)| ((*symbol).to_string(), (*path, *ty)))
        .collect::<BTreeMap<_, _>>();
    let dependent_map = WATER_DEPENDENT_SAMPLERS
        .iter()
        .map(|(symbol, owner, ty)| ((*symbol).to_string(), (*owner, *ty)))
        .collect::<BTreeMap<_, _>>();
    let mut samplers = Vec::new();
    let pixel_stage = crate::shader_compile::merged_effect_translation_unit_stage(
        translation_unit,
        crate::ShaderStage::Pixel,
    );
    for sampler in &pixel_stage.samplers {
        let (owner_packet, resource_type, admitted_static_content) =
            if let Some((path, ty)) = static_map.get(&sampler.name) {
                (
                    "XMAP07".to_string(),
                    (*ty).to_string(),
                    Some(
                        admission
                            .require_used_content(path)
                            .map_err(|error| {
                                format!(
                                    "XMAP07 water sampler {} requires admitted {path}: {error}",
                                    sampler.name
                                )
                            })?
                            .clone(),
                    ),
                )
            } else if let Some((owner, ty)) = dependent_map.get(&sampler.name) {
                ((*owner).to_string(), (*ty).to_string(), None)
            } else {
                return Err(format!(
                    "effective water Effect {effect_name} introduced unowned sampler {}",
                    sampler.name
                )
                .into());
            };
        samplers.push(PreparedWaterSampler {
            symbol: sampler.name.clone(),
            shader_register: sampler.index,
            resource_type,
            owner_packet,
            admitted_static_content,
            mag_filter: sampler
                .mag_filter
                .clone()
                .ok_or_else(|| format!("XMAP07 sampler {} omits MagFilter", sampler.name))?,
            min_filter: sampler
                .min_filter
                .clone()
                .ok_or_else(|| format!("XMAP07 sampler {} omits MinFilter", sampler.name))?,
            mip_filter: sampler
                .mip_filter
                .clone()
                .ok_or_else(|| format!("XMAP07 sampler {} omits MipFilter", sampler.name))?,
            address_u: sampler
                .address_u
                .clone()
                .ok_or_else(|| format!("XMAP07 sampler {} omits AddressU", sampler.name))?,
            address_v: sampler
                .address_v
                .clone()
                .ok_or_else(|| format!("XMAP07 sampler {} omits AddressV", sampler.name))?,
            mip_map_lod_bias: sampler.mip_map_lod_bias.clone(),
            mip_map_min_lod: sampler.mip_map_min_lod.clone(),
            mip_map_max_lod: sampler.mip_map_max_lod.clone(),
            max_anisotropy: sampler.max_anisotropy.clone(),
        });
    }
    samplers.sort_by_key(|sampler| sampler.shader_register);
    let ordered_shader_content = translation_unit
        .iter()
        .map(|(path, _)| path.as_str())
        .chain([XMAP07_PLATFORM_HEADER])
        .map(|path| {
            admission
                .require_used_content(path)
                .map_err(|error| format!("XMAP07 program requires admitted {path}: {error}").into())
                .cloned()
        })
        .collect::<Result<Vec<_>>>()?;
    let mut program = PreparedWaterProgram {
        effect_file: XMAP07_WATER_EFFECT_FILE.to_ascii_lowercase(),
        effect_name: effect_name.into(),
        ordered_shader_content,
        vertex_translation_unit_sha256: Sha256::hex(vertex_hlsl.as_bytes()),
        pixel_translation_unit_sha256: Sha256::hex(pixel_hlsl.as_bytes()),
        vertex_bytecode_sha256: Sha256::hex(&vertex_bytecode),
        pixel_bytecode_sha256: Sha256::hex(&pixel_bytecode),
        scene_cb0_reflected_layout_sha256: xmap10_scene_cb0_reflected_layout_sha256(
            &vertex_bytecode,
            &pixel_bytecode,
        )?,
        vertex_bytecode: vertex_bytecode.into(),
        pixel_bytecode: pixel_bytecode.into(),
        samplers,
        deterministic_sha256: String::new(),
    };
    program.deterministic_sha256 = water_program_digest(&program);
    program.validate()?;
    Ok(program)
}

#[cfg(windows)]
fn compile_water_stages(vertex_hlsl: &str, pixel_hlsl: &str) -> Result<(Vec<u8>, Vec<u8>)> {
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
fn compile_water_stages(_vertex_hlsl: &str, _pixel_hlsl: &str) -> Result<(Vec<u8>, Vec<u8>)> {
    Err("XMAP07 executable water-program handoff requires Windows D3DCompile".into())
}

std::thread_local! {
    /// Last prepared SnowMud texture, keyed by the payload digest and the target size.
    ///
    /// `prepare` runs EVERY FRAME while the hour does not change, and building this texture is
    /// whole-grid work: reassembling 2,816 base64 tiles into 2.88 MB, and then a SHA-256 over all
    /// of it inside `generated_snow_mud_parts`. THE SECOND HALF OF THAT WAS ALREADY PER-FRAME
    /// before the grid was tiled -- `packet_payload_sha256 != Sha256::hex(packet_bytes)` re-hashed
    /// the same 2.88 MB on every frame of every hour -- so this memo removes more per-frame work
    /// than tiling added.
    ///
    /// KEYING ON THE DIGEST IS WHAT MAKES IT SOUND. `payload_sha256` is verified against the
    /// bytes when the entry is built, and again at ingress when the carrier is decoded, so a hit
    /// means the very same grid. Re-deriving it every frame from unchanged memory re-checks RAM,
    /// not authenticity, and the hour boundary is exactly where the key changes.
    ///
    /// THE KEY CARRIES THE CARRIER'S OWN SHAPE, NOT JUST THE DIGEST AND THE TARGET SIZE. The
    /// digest is over the LINEAR bytes, so two carriers that are each internally consistent can
    /// share it while declaring different equal-area dimensions -- 8x4 and 4x8 over the same
    /// bytes, reachable with no hash collision at all on a uniform grid. `generated_snow_mud_parts`
    /// rejects a carrier whose dimensions do not match the active map, and a key without those
    /// fields would let the second carrier hit the first one's entry and skip that rejection. So
    /// every field that function tests is part of the key.
    static PREPARED_SNOW_MUD: std::cell::RefCell<
        Option<(SnowMudMemoKey, PreparedWaterGeneratedTexture)>,
    > = const { std::cell::RefCell::new(None) };
}

/// Everything `generated_snow_mud_parts` checks or reports, so a memo hit is only ever the exact
/// case that would have been rebuilt.
#[derive(Clone, PartialEq, Eq)]
struct SnowMudMemoKey {
    payload_sha256: String,
    packet_width: u32,
    packet_height: u32,
    packet_row_pitch: u32,
    packet_bytes_per_texel: u32,
    packet_format: String,
    expected_width: usize,
    expected_height: usize,
}

fn generated_snow_mud(
    packet: &Xmap07PassiveSnowMudCarrier,
    expected_width: usize,
    expected_height: usize,
) -> Result<PreparedWaterGeneratedTexture> {
    let key = SnowMudMemoKey {
        payload_sha256: packet.payload_sha256.clone(),
        packet_width: packet.width,
        packet_height: packet.height,
        packet_row_pitch: packet.row_pitch,
        packet_bytes_per_texel: packet.bytes_per_texel,
        packet_format: packet.format.clone(),
        expected_width,
        expected_height,
    };
    let hit = PREPARED_SNOW_MUD.with(|memo| {
        memo.borrow()
            .as_ref()
            .filter(|(cached, _)| cached == &key)
            .map(|(_, prepared)| prepared.clone())
    });
    if let Some(prepared) = hit {
        return Ok(prepared);
    }
    let grid = packet
        .grid_bytes()
        .map_err(|error| format!("XMAP07 exact SnowMud tiled grid refused: {error}"))?;
    let prepared = generated_snow_mud_parts(
        packet.width,
        packet.height,
        packet.row_pitch,
        packet.bytes_per_texel,
        &packet.format,
        &grid,
        &packet.payload_sha256,
        expected_width,
        expected_height,
    )?;
    PREPARED_SNOW_MUD.with(|memo| {
        *memo.borrow_mut() = Some((key, prepared.clone()));
    });
    Ok(prepared)
}

#[allow(clippy::too_many_arguments)]
fn generated_snow_mud_parts(
    packet_width: u32,
    packet_height: u32,
    packet_row_pitch: u32,
    packet_bytes_per_texel: u32,
    packet_format: &str,
    packet_bytes: &[u8],
    packet_payload_sha256: &str,
    expected_width: usize,
    expected_height: usize,
) -> Result<PreparedWaterGeneratedTexture> {
    let width = u32::try_from(expected_width)?;
    let height = u32::try_from(expected_height)?;
    if packet_width != width
        || packet_height != height
        || packet_row_pitch
            != width
                .checked_mul(4)
                .ok_or("XMAP07 SnowMud row pitch overflow")?
        || packet_bytes_per_texel != 4
        || packet_format != "DXGI_FORMAT_B8G8R8A8_UNORM"
        || packet_bytes.len() != packet_row_pitch as usize * packet_height as usize
        || packet_payload_sha256 != Sha256::hex(packet_bytes)
    {
        return Err(
            "XMAP07 exact SnowMud carrier does not match the active map texture contract".into(),
        );
    }
    let row_pitch = packet_row_pitch;
    let payload_sha256 = packet_payload_sha256.to_string();
    let descriptor_sha256 = Sha256::hex(
        format!(
            "SnowMudTexture\0{width}\0{height}\0{}\0{row_pitch}\0{payload_sha256}",
            XMAP07_SNOW_MUD_FORMAT
        )
        .as_bytes(),
    );
    let result = PreparedWaterGeneratedTexture {
        symbol: "SnowMudTexture".into(),
        width,
        height,
        dxgi_format: XMAP07_SNOW_MUD_FORMAT,
        row_pitch,
        payload: packet_bytes.to_vec().into(),
        payload_sha256,
        descriptor_sha256,
    };
    validate_generated_texture(&result)?;
    Ok(result)
}

fn validate_generated_texture(texture: &PreparedWaterGeneratedTexture) -> Result<()> {
    if texture.symbol != "SnowMudTexture"
        || texture.width == 0
        || texture.height == 0
        || texture.dxgi_format != XMAP07_SNOW_MUD_FORMAT
        || texture.row_pitch != texture.width.saturating_mul(4)
        || texture.payload.len() != texture.row_pitch as usize * texture.height as usize
        || texture.payload_sha256 != Sha256::hex(&texture.payload)
        || texture.descriptor_sha256
            != Sha256::hex(
                format!(
                    "SnowMudTexture\0{}\0{}\0{}\0{}\0{}",
                    texture.width,
                    texture.height,
                    texture.dxgi_format,
                    texture.row_pitch,
                    texture.payload_sha256
                )
                .as_bytes(),
            )
    {
        return Err("XMAP07 generated SnowMud texture is invalid".into());
    }
    Ok(())
}

fn admitted_bmp_dimensions(
    resolver: &GameFileResolver,
    content: &UsedContentKey,
) -> Result<(u32, u32)> {
    let bytes = resolver.read_admitted_used_content(content)?;
    if bytes.get(..2) != Some(b"BM") || bytes.len() < 26 {
        return Err("XMAP07 admitted map/provinces.bmp has no complete BMP header".into());
    }
    let width = i32::from_le_bytes(bytes[18..22].try_into()?);
    let height = i32::from_le_bytes(bytes[22..26].try_into()?);
    if width <= 0 || height == 0 || height == i32::MIN {
        return Err("XMAP07 admitted map/provinces.bmp dimensions are invalid".into());
    }
    Ok((width as u32, height.unsigned_abs()))
}

fn water_quad_bytes(map_width: u32, map_height: u32) -> Result<Vec<u8>> {
    let width = i16::try_from(map_width)
        .map_err(|_| "XMAP07 effective map width exceeds signed-short water vertex range")?;
    let height = i16::try_from(map_height)
        .map_err(|_| "XMAP07 effective map height exceeds signed-short water vertex range")?;
    let vertices = [(0i16, 0i16), (width, 0), (0, height), (width, height)];
    let mut bytes = Vec::with_capacity(16);
    for (x, z) in vertices {
        bytes.extend_from_slice(&x.to_le_bytes());
        bytes.extend_from_slice(&z.to_le_bytes());
    }
    Ok(bytes)
}

fn water_cb3_bytes(
    animation_time: f32,
    viewport_width: u32,
    viewport_height: u32,
) -> Result<[u8; 16]> {
    if !animation_time.is_finite()
        || animation_time < 0.0
        || viewport_width == 0
        || viewport_height == 0
    {
        return Err("XMAP07 cannot build water CB3 from invalid viewer state".into());
    }
    let values = [
        animation_time,
        0.5 / viewport_width as f32,
        0.5 / viewport_height as f32,
        0.0,
    ];
    let mut bytes = [0u8; 16];
    for (index, value) in values.iter().enumerate() {
        bytes[index * 4..index * 4 + 4].copy_from_slice(&value.to_le_bytes());
    }
    Ok(bytes)
}

fn temporal_digest(temporal: &Xmap07WaterTemporalBinding) -> String {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(temporal.replay_frame_identity_sha256.as_bytes());
    bytes.extend_from_slice(&temporal.animation_time_f32_bits.to_le_bytes());
    bytes.extend_from_slice(&temporal.season_lerp_f32_bits.to_le_bytes());
    Sha256::hex(&bytes)
}

fn water_program_digest(program: &PreparedWaterProgram) -> String {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(program.effect_file.as_bytes());
    bytes.extend_from_slice(program.effect_name.as_bytes());
    bytes.extend_from_slice(program.vertex_translation_unit_sha256.as_bytes());
    bytes.extend_from_slice(program.pixel_translation_unit_sha256.as_bytes());
    bytes.extend_from_slice(program.vertex_bytecode_sha256.as_bytes());
    bytes.extend_from_slice(program.pixel_bytecode_sha256.as_bytes());
    bytes.extend_from_slice(program.scene_cb0_reflected_layout_sha256.as_bytes());
    for content in &program.ordered_shader_content {
        bytes.extend_from_slice(content.normalized_game_relative_path.as_bytes());
        bytes.extend_from_slice(content.payload_sha256.as_bytes());
    }
    for sampler in &program.samplers {
        bytes.extend_from_slice(sampler.symbol.as_bytes());
        bytes.extend_from_slice(&sampler.shader_register.to_le_bytes());
        bytes.extend_from_slice(sampler.resource_type.as_bytes());
        bytes.extend_from_slice(sampler.owner_packet.as_bytes());
        bytes.extend_from_slice(sampler.mag_filter.as_bytes());
        bytes.extend_from_slice(sampler.min_filter.as_bytes());
        bytes.extend_from_slice(sampler.mip_filter.as_bytes());
        bytes.extend_from_slice(sampler.address_u.as_bytes());
        bytes.extend_from_slice(sampler.address_v.as_bytes());
        for value in [
            &sampler.mip_map_lod_bias,
            &sampler.mip_map_min_lod,
            &sampler.mip_map_max_lod,
            &sampler.max_anisotropy,
        ] {
            match value {
                Some(value) => {
                    bytes.push(1);
                    bytes.extend_from_slice(value.as_bytes());
                }
                None => bytes.push(0),
            }
        }
        if let Some(content) = &sampler.admitted_static_content {
            bytes.extend_from_slice(content.payload_sha256.as_bytes());
        }
    }
    Sha256::hex(&bytes)
}

fn water_frame_digest(frame: &PreparedWaterFrame) -> String {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(frame.schema.as_bytes());
    bytes.extend_from_slice(frame.assigned_stage.assignment_sha256.as_bytes());
    bytes.extend_from_slice(
        frame
            .surface_target_requirement
            .deterministic_sha256
            .as_bytes(),
    );
    bytes.extend_from_slice(&frame.map_width.to_le_bytes());
    bytes.extend_from_slice(&frame.map_height.to_le_bytes());
    bytes.extend_from_slice(&frame.viewport_width.to_le_bytes());
    bytes.extend_from_slice(&frame.viewport_height.to_le_bytes());
    bytes.extend_from_slice(frame.camera_view_sha256.as_bytes());
    bytes.extend_from_slice(frame.replay_frame_identity_sha256.as_bytes());
    bytes.extend_from_slice(frame.date.as_bytes());
    bytes.extend_from_slice(&frame.hour_index.to_le_bytes());
    bytes.extend_from_slice(frame.program.deterministic_sha256.as_bytes());
    bytes.extend_from_slice(Sha256::hex(&frame.water_quad).as_bytes());
    bytes.extend_from_slice(Sha256::hex(&frame.water_cb3).as_bytes());
    bytes.extend_from_slice(frame.temporal.deterministic_sha256.as_bytes());
    bytes.extend_from_slice(frame.snow_mud.payload_sha256.as_bytes());
    bytes.extend_from_slice(frame.snow_mud.descriptor_sha256.as_bytes());
    bytes.extend_from_slice(&frame.snow_mud_fade_f32_bits.to_le_bytes());
    bytes.extend_from_slice(frame.snow_mud_authority_sha256.as_bytes());
    bytes.extend_from_slice(frame.refraction_descriptor_sha256.as_bytes());
    bytes.push(frame.underwater_required as u8);
    for content in &frame.ordered_used_content {
        bytes.extend_from_slice(content.normalized_game_relative_path.as_bytes());
        bytes.extend_from_slice(content.payload_sha256.as_bytes());
    }
    Sha256::hex(&bytes)
}

fn scene_snow_mud_input_digest(input: &Xmap07SceneSnowMudInput) -> String {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"XMAP07\0SceneSnowMudInput\0v1\0");
    bytes.extend_from_slice(input.date.as_bytes());
    bytes.extend_from_slice(&input.hour_index.to_le_bytes());
    bytes.extend_from_slice(input.replay_frame_identity_sha256.as_bytes());
    bytes.extend_from_slice(&input.snow_mud_fade_f32_bits.to_le_bytes());
    bytes.extend_from_slice(input.snow_mud_payload_sha256.as_bytes());
    bytes.extend_from_slice(input.snow_mud_authority_sha256.as_bytes());
    bytes.extend_from_slice(input.water_frame_sha256.as_bytes());
    Sha256::hex(&bytes)
}

pub(crate) fn ready_water_draw_contract_digest(contract: &ReadyWaterDrawContract) -> String {
    let mut identity = Vec::new();
    identity.extend_from_slice(contract.replay_frame_identity_sha256.as_bytes());
    identity.extend_from_slice(contract.camera_view_sha256.as_bytes());
    identity.extend_from_slice(contract.water_frame_sha256.as_bytes());
    identity.extend_from_slice(contract.water_program_sha256.as_bytes());
    identity.extend_from_slice(contract.scene_cb0_sha256.as_bytes());
    identity.extend_from_slice(contract.scene_cb0_descriptor_sha256.as_bytes());
    identity.extend_from_slice(contract.scene_cb0_composition_receipt_sha256.as_bytes());
    identity.extend_from_slice(contract.scene_cb0_reflected_layout_sha256.as_bytes());
    if let Some(point_lights) = &contract.point_lights_cb2_sha256 {
        identity.extend_from_slice(point_lights.as_bytes());
    }
    if let Some(descriptor) = &contract.point_lights_cb2_descriptor_sha256 {
        identity.extend_from_slice(descriptor.as_bytes());
    }
    if let Some(camera) = &contract.point_lights_cb2_camera_view_sha256 {
        identity.extend_from_slice(camera.as_bytes());
    }
    if let Some(underwater) = &contract.underwater_draw_receipt_sha256 {
        identity.extend_from_slice(underwater.as_bytes());
    }
    for binding in &contract.ordered_dependency_resources {
        identity.extend_from_slice(binding.symbol.as_bytes());
        identity.extend_from_slice(binding.owner_packet.as_bytes());
        identity.extend_from_slice(binding.replay_frame_identity_sha256.as_bytes());
        if let Some(camera) = &binding.camera_view_sha256 {
            identity.extend_from_slice(camera.as_bytes());
        }
        identity.extend_from_slice(binding.resource_sha256.as_bytes());
        identity.extend_from_slice(binding.descriptor_sha256.as_bytes());
    }
    Sha256::hex(&identity)
}

pub(crate) fn ordered_resources_digest(frame: &PreparedWaterFrame) -> String {
    let mut bytes = Vec::new();
    for sampler in &frame.program.samplers {
        bytes.extend_from_slice(sampler.symbol.as_bytes());
        bytes.extend_from_slice(&sampler.shader_register.to_le_bytes());
        bytes.extend_from_slice(sampler.owner_packet.as_bytes());
        if let Some(content) = &sampler.admitted_static_content {
            bytes.extend_from_slice(content.payload_sha256.as_bytes());
        } else if sampler.symbol == "SnowMudTexture" {
            bytes.extend_from_slice(frame.snow_mud.payload_sha256.as_bytes());
        } else if sampler.symbol == "WaterRefraction" {
            bytes.extend_from_slice(frame.refraction_descriptor_sha256.as_bytes());
        }
    }
    Sha256::hex(&bytes)
}

pub(crate) fn surface_target_digest(frame: &PreparedWaterFrame) -> String {
    frame
        .surface_target_requirement
        .deterministic_sha256
        .clone()
}

fn surface_target_requirement_digest(requirement: &Xmap07SurfaceTargetRequirement) -> String {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(requirement.schema.as_bytes());
    bytes.extend_from_slice(requirement.target_role.as_bytes());
    bytes.extend_from_slice(&requirement.viewport_width.to_le_bytes());
    bytes.extend_from_slice(&requirement.viewport_height.to_le_bytes());
    bytes.extend_from_slice(requirement.refraction_descriptor_sha256.as_bytes());
    Sha256::hex(&bytes)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn water_quad_is_exact_signed_short_triangle_strip() {
        let bytes = water_quad_bytes(5632, 2048).unwrap();
        let words = bytes
            .chunks_exact(2)
            .map(|word| i16::from_le_bytes([word[0], word[1]]))
            .collect::<Vec<_>>();
        assert_eq!(words, [0, 0, 5632, 0, 0, 2048, 5632, 2048]);
    }

    #[test]
    fn water_cb3_uses_viewer_time_and_exact_half_pixel_offsets() {
        let bytes = water_cb3_bytes(3.809123, 1920, 1080).unwrap();
        let values = bytes
            .chunks_exact(4)
            .map(|value| f32::from_le_bytes(value.try_into().unwrap()))
            .collect::<Vec<_>>();
        assert_eq!(values[0], 3.809123);
        assert_eq!(values[1], 0.5 / 1920.0);
        assert_eq!(values[2], 0.5 / 1080.0);
        assert_eq!(values[3], 0.0);
    }

    #[test]
    fn temporal_binding_exports_time_and_season_without_inventing_river_direction() {
        let mut temporal = Xmap07WaterTemporalBinding {
            replay_frame_identity_sha256: Sha256::hex(b"replay-frame"),
            animation_time_f32_bits: 15.16874f32.to_bits(),
            season_lerp_f32_bits: 0.25f32.to_bits(),
            deterministic_sha256: String::new(),
        };
        temporal.deterministic_sha256 = temporal_digest(&temporal);
        temporal.validate().unwrap();
        let bytes = temporal.map_object_time_season_bytes();
        assert_eq!(
            f32::from_le_bytes(bytes[0..4].try_into().unwrap()),
            15.16874
        );
        assert_eq!(f32::from_le_bytes(bytes[4..8].try_into().unwrap()), 0.25);
    }

    #[test]
    fn scene_snow_mud_input_is_frame_and_payload_bound() {
        let mut input = Xmap07SceneSnowMudInput {
            date: "1936.1.2".into(),
            hour_index: 23,
            replay_frame_identity_sha256: Sha256::hex(b"replay-frame"),
            snow_mud_fade_f32_bits: 0.375f32.to_bits(),
            snow_mud_payload_sha256: Sha256::hex(b"snow-mud-payload"),
            snow_mud_authority_sha256: Sha256::hex(b"snow-mud-authority"),
            water_frame_sha256: Sha256::hex(b"water-frame"),
            deterministic_sha256: String::new(),
        };
        input.deterministic_sha256 = scene_snow_mud_input_digest(&input);
        input.validate().unwrap();

        let frame = |date: &str, hour_index, replay_frame: String, snow_mud_input: String| {
            crate::Xmap10SceneFrameIdentity::new(
                Sha256::hex(b"content"),
                Sha256::hex(b"modset"),
                Sha256::hex(b"vfs"),
                date.into(),
                hour_index,
                5632,
                2048,
                1920,
                1080,
                replay_frame,
                Sha256::hex(b"camera"),
                &[crate::Xmap04MapCopyReceiptRow {
                    render_view_mask: 1,
                    map_copy_offset_f32_bits: 0.0f32.to_bits(),
                }],
                1,
                snow_mud_input,
            )
            .unwrap()
        };
        let exact_frame = frame(
            &input.date,
            input.hour_index,
            input.replay_frame_identity_sha256.clone(),
            input.deterministic_sha256.clone(),
        );
        let patch = crate::Xmap07SnowMudScenePatch::from_xmap07(&input, &exact_frame).unwrap();
        assert_eq!(patch.snow_mud_fade.to_bits(), input.snow_mud_fade_f32_bits);

        let wrong_date = frame(
            "1936.1.3",
            input.hour_index,
            input.replay_frame_identity_sha256.clone(),
            input.deterministic_sha256.clone(),
        );
        assert!(crate::Xmap07SnowMudScenePatch::from_xmap07(&input, &wrong_date).is_err());
        let wrong_hour = frame(
            &input.date,
            input.hour_index + 1,
            input.replay_frame_identity_sha256.clone(),
            input.deterministic_sha256.clone(),
        );
        assert!(crate::Xmap07SnowMudScenePatch::from_xmap07(&input, &wrong_hour).is_err());
        let wrong_replay = frame(
            &input.date,
            input.hour_index,
            Sha256::hex(b"other-replay"),
            input.deterministic_sha256.clone(),
        );
        assert!(crate::Xmap07SnowMudScenePatch::from_xmap07(&input, &wrong_replay).is_err());
        let wrong_input = frame(
            &input.date,
            input.hour_index,
            input.replay_frame_identity_sha256.clone(),
            Sha256::hex(b"other-snow-input"),
        );
        assert!(crate::Xmap07SnowMudScenePatch::from_xmap07(&input, &wrong_input).is_err());

        let mut tampered = input.clone();
        tampered.hour_index += 1;
        assert!(tampered.validate().is_err());

        let mut invalid_fade = input;
        invalid_fade.snow_mud_fade_f32_bits = f32::NAN.to_bits();
        assert!(invalid_fade.validate().is_err());
    }

    #[test]
    fn snow_mud_addressing_is_active_map_quarter_resolution_or_fails() {
        let vanilla = exact_snow_mud_addressing(5632, 2048).unwrap();
        assert_eq!(vanilla.texture_width, 1408);
        assert_eq!(vanilla.texture_height, 512);
        let modded = exact_snow_mud_addressing(4096, 3072).unwrap();
        assert_eq!(modded.texture_width, 1024);
        assert_eq!(modded.texture_height, 768);
        assert!(exact_snow_mud_addressing(4097, 3072).is_err());
        assert!(exact_snow_mud_addressing(0, 3072).is_err());
    }

    #[test]
    fn xmap07_exact_snow_mud_packet_is_required_to_match_every_descriptor_field() {
        let bytes = vec![0x5a; 8 * 4 * 4];
        let hash = Sha256::hex(&bytes);
        let exact = || {
            generated_snow_mud_parts(
                8,
                4,
                32,
                4,
                "DXGI_FORMAT_B8G8R8A8_UNORM",
                &bytes,
                &hash,
                8,
                4,
            )
        };
        assert!(exact().is_ok());
        assert!(generated_snow_mud_parts(
            7,
            4,
            32,
            4,
            "DXGI_FORMAT_B8G8R8A8_UNORM",
            &bytes,
            &hash,
            8,
            4,
        )
        .is_err());
        assert!(generated_snow_mud_parts(
            8,
            4,
            31,
            4,
            "DXGI_FORMAT_B8G8R8A8_UNORM",
            &bytes,
            &hash,
            8,
            4,
        )
        .is_err());
        assert!(generated_snow_mud_parts(
            8,
            4,
            32,
            4,
            "DXGI_FORMAT_R8G8B8A8_UNORM",
            &bytes,
            &hash,
            8,
            4,
        )
        .is_err());
        assert!(generated_snow_mud_parts(
            8,
            4,
            32,
            4,
            "DXGI_FORMAT_B8G8R8A8_UNORM",
            &bytes[..bytes.len() - 1],
            &hash,
            8,
            4,
        )
        .is_err());
        assert!(generated_snow_mud_parts(
            8,
            4,
            32,
            4,
            "DXGI_FORMAT_B8G8R8A8_UNORM",
            &bytes,
            &Sha256::hex(b"wrong"),
            8,
            4,
        )
        .is_err());
    }

    #[test]
    fn ready_water_draw_contract_rejects_nested_dependency_tampering() {
        let mut contract = ReadyWaterDrawContract {
            replay_frame_identity_sha256: Sha256::hex(b"replay-frame"),
            camera_view_sha256: Sha256::hex(b"camera"),
            water_frame_sha256: Sha256::hex(b"water-frame"),
            water_program_sha256: Sha256::hex(b"water-program"),
            scene_cb0_sha256: Sha256::hex(b"scene-cb0"),
            scene_cb0_descriptor_sha256: Sha256::hex(b"scene-cb0-descriptor"),
            scene_cb0_composition_receipt_sha256: Sha256::hex(b"scene-composition"),
            scene_cb0_reflected_layout_sha256: Sha256::hex(b"scene-layout"),
            point_lights_cb2_sha256: Some(Sha256::hex(b"point-lights")),
            point_lights_cb2_descriptor_sha256: Some(Sha256::hex(b"point-lights-desc")),
            point_lights_cb2_camera_view_sha256: Some(Sha256::hex(b"camera")),
            underwater_draw_receipt_sha256: Some(Sha256::hex(b"underwater")),
            ordered_dependency_resources: vec![WaterDependentResourceBinding {
                symbol: "ShadowMap".into(),
                owner_packet: "XMAP10".into(),
                replay_frame_identity_sha256: Sha256::hex(b"replay-frame"),
                camera_view_sha256: Some(Sha256::hex(b"camera")),
                resource_sha256: Sha256::hex(b"shadow"),
                descriptor_sha256: Sha256::hex(b"shadow-desc"),
            }],
            deterministic_sha256: String::new(),
        };
        contract.deterministic_sha256 = ready_water_draw_contract_digest(&contract);
        contract.validate().unwrap();

        contract.ordered_dependency_resources[0].resource_sha256 = Sha256::hex(b"tampered");
        assert!(contract.validate().is_err());

        contract.ordered_dependency_resources[0].resource_sha256 = Sha256::hex(b"shadow");
        contract.ordered_dependency_resources[0].replay_frame_identity_sha256 =
            Sha256::hex(b"other-replay-frame");
        contract.deterministic_sha256 = ready_water_draw_contract_digest(&contract);
        assert!(contract.validate().is_err());
    }
}
