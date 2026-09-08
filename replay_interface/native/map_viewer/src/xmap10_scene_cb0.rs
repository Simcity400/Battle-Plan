
use crate::{xmap04_map_copy_receipt_set_sha256, Result, Xmap04MapCopyReceiptRow};
use portable_render_stream::{ConstantBufferBinding, PreparedRenderTarget, RenderTargetDesc};
use serde::{Deserialize, Serialize};
use snapshot::sha256::Sha256;

pub const XMAP10_SCENE_CB0_SCHEMA: &str = "hoi4.xmap10.scene-cb0.a729.v1";
pub const XMAP10_SCENE_CB0_LAYOUT_SOURCE: &str = "gfx/FX/standardfuncsgfx.fxh:ConstantBuffer(0,0)";
pub const XMAP10_SCENE_CB0_BYTE_LEN: usize = 416;
pub const XMAP10_SCENE_CB0_BIND_POINT: u32 = 0;
pub const XMAP10_SCENE_CB0_GLOBAL_TIME_OFFSET: usize = 236;
pub const XMAP10_PRE_PARTICLE_SCENE_SCHEMA: &str = "hoi4.xmap10.pre-particle-scene.v1";
pub const XMAP10_PRE_PARTICLE_OPERATION_ID: &str = "scene.pre_particles_complete";

const XMAP10_SCENE_CB0_REFLECTED_ROWS: &[(&str, u32, u32)] = &[
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
    ("vGlobalTime", XMAP10_SCENE_CB0_GLOBAL_TIME_OFFSET as u32, 4),
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
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Xmap10SceneFieldOwner {
    Xmap03Camera,
    Xmap05PoliticalFow,
    Xmap07SnowMud,
    Xmap09Lighting,
    Xmap10Presentation,
    Padding,
}

impl Xmap10SceneFieldOwner {
    fn label(self) -> &'static str {
        match self {
            Self::Xmap03Camera => "XMAP03_CAMERA",
            Self::Xmap05PoliticalFow => "XMAP05_POLITICAL_FOW",
            Self::Xmap07SnowMud => "XMAP07_SNOW_MUD",
            Self::Xmap09Lighting => "XMAP09_LIGHTING",
            Self::Xmap10Presentation => "XMAP10_PRESENTATION",
            Self::Padding => "HLSL_PADDING",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Xmap10SceneFieldCoverage {
    pub field: &'static str,
    pub offset: usize,
    pub byte_len: usize,
    pub owner: Xmap10SceneFieldOwner,
}

pub const XMAP10_SCENE_FIELD_COVERAGE: &[Xmap10SceneFieldCoverage] = &[
    coverage(
        "ViewProjectionMatrix",
        0,
        64,
        Xmap10SceneFieldOwner::Xmap03Camera,
    ),
    coverage(
        "InvViewProjMatrix",
        64,
        64,
        Xmap10SceneFieldOwner::Xmap03Camera,
    ),
    coverage(
        "vGBCamDistOverride_GBOutlineCutoff",
        128,
        16,
        Xmap10SceneFieldOwner::Xmap05PoliticalFow,
    ),
    coverage(
        "vVirtualSunPos",
        144,
        16,
        Xmap10SceneFieldOwner::Xmap09Lighting,
    ),
    coverage(
        "vVirtualMoonPos",
        160,
        16,
        Xmap10SceneFieldOwner::Xmap09Lighting,
    ),
    coverage(
        "vSecondVirtualSunPos",
        176,
        16,
        Xmap10SceneFieldOwner::Xmap09Lighting,
    ),
    coverage(
        "vSecondVirtualMoonPos",
        192,
        16,
        Xmap10SceneFieldOwner::Xmap09Lighting,
    ),
    coverage("vCamPos", 208, 12, Xmap10SceneFieldOwner::Xmap03Camera),
    coverage(
        "HdrRange",
        220,
        4,
        Xmap10SceneFieldOwner::Xmap10Presentation,
    ),
    coverage(
        "vCamLookAtDir",
        224,
        12,
        Xmap10SceneFieldOwner::Xmap03Camera,
    ),
    coverage(
        "vGlobalTime",
        XMAP10_SCENE_CB0_GLOBAL_TIME_OFFSET,
        4,
        Xmap10SceneFieldOwner::Xmap10Presentation,
    ),
    coverage(
        "FoWOpacity",
        240,
        4,
        Xmap10SceneFieldOwner::Xmap05PoliticalFow,
    ),
    coverage("FoWTime", 244, 4, Xmap10SceneFieldOwner::Xmap05PoliticalFow),
    coverage("SnowMudFade", 248, 4, Xmap10SceneFieldOwner::Xmap07SnowMud),
    coverage(
        "MaxGameSpeed",
        252,
        4,
        Xmap10SceneFieldOwner::Xmap10Presentation,
    ),
    coverage(
        "DayNight_Hour_SunDir",
        256,
        16,
        Xmap10SceneFieldOwner::Xmap09Lighting,
    ),
    coverage(
        "AmbientPosX",
        272,
        12,
        Xmap10SceneFieldOwner::Xmap09Lighting,
    ),
    coverage(
        "ShadowFadeFactor",
        284,
        4,
        Xmap10SceneFieldOwner::Xmap09Lighting,
    ),
    coverage(
        "AmbientNegX",
        288,
        12,
        Xmap10SceneFieldOwner::Xmap09Lighting,
    ),
    coverage(
        "FOWFadeFactor",
        300,
        4,
        Xmap10SceneFieldOwner::Xmap05PoliticalFow,
    ),
    coverage(
        "AmbientPosY",
        304,
        12,
        Xmap10SceneFieldOwner::Xmap09Lighting,
    ),
    coverage(
        "MinMeshAlpha",
        316,
        4,
        Xmap10SceneFieldOwner::Xmap05PoliticalFow,
    ),
    coverage(
        "AmbientNegY",
        320,
        12,
        Xmap10SceneFieldOwner::Xmap09Lighting,
    ),
    coverage(
        "NegFogMultiplier",
        332,
        4,
        Xmap10SceneFieldOwner::Xmap10Presentation,
    ),
    coverage(
        "AmbientPosZ",
        336,
        12,
        Xmap10SceneFieldOwner::Xmap09Lighting,
    ),
    coverage(
        "register_21_padding",
        348,
        4,
        Xmap10SceneFieldOwner::Padding,
    ),
    coverage(
        "AmbientNegZ",
        352,
        12,
        Xmap10SceneFieldOwner::Xmap09Lighting,
    ),
    coverage(
        "CubemapIntensity",
        364,
        4,
        Xmap10SceneFieldOwner::Xmap09Lighting,
    ),
    coverage(
        "SunDiffuseIntensity",
        368,
        16,
        Xmap10SceneFieldOwner::Xmap09Lighting,
    ),
    coverage(
        "MoonDiffuseIntensity",
        384,
        16,
        Xmap10SceneFieldOwner::Xmap09Lighting,
    ),
    coverage(
        "GB_TextureHeight",
        400,
        4,
        Xmap10SceneFieldOwner::Xmap05PoliticalFow,
    ),
    coverage(
        "SunSpecularIntensity",
        404,
        4,
        Xmap10SceneFieldOwner::Xmap09Lighting,
    ),
    coverage(
        "register_25_padding",
        408,
        8,
        Xmap10SceneFieldOwner::Padding,
    ),
];

const fn coverage(
    field: &'static str,
    offset: usize,
    byte_len: usize,
    owner: Xmap10SceneFieldOwner,
) -> Xmap10SceneFieldCoverage {
    Xmap10SceneFieldCoverage {
        field,
        offset,
        byte_len,
        owner,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Xmap10SceneFrameIdentity {
    pub content_manifest_sha256: String,
    pub product_modset_sha256: String,
    pub effective_vfs_manifest_sha256: String,
    pub date: String,
    pub hour_index: i64,
    pub map_width: u32,
    pub map_height: u32,
    pub viewport_width: u32,
    pub viewport_height: u32,
    pub replay_frame_identity_sha256: String,
    pub camera_view_sha256: String,
    pub map_copy_receipt_set_sha256: String,
    pub render_view_mask: u8,
    pub map_copy_offset_f32_bits: u32,
    pub snow_mud_input_sha256: String,
    pub identity_sha256: String,
}

impl Xmap10SceneFrameIdentity {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        content_manifest_sha256: String,
        product_modset_sha256: String,
        effective_vfs_manifest_sha256: String,
        date: String,
        hour_index: i64,
        map_width: u32,
        map_height: u32,
        viewport_width: u32,
        viewport_height: u32,
        replay_frame_identity_sha256: String,
        camera_view_sha256: String,
        ordered_map_copies: &[Xmap04MapCopyReceiptRow],
        render_view_mask: u8,
        snow_mud_input_sha256: String,
    ) -> Result<Self> {
        let map_copy_receipt_set_sha256 =
            xmap04_map_copy_receipt_set_sha256(map_width, ordered_map_copies)?;
        let map_copy_offset_f32_bits = ordered_map_copies
            .iter()
            .find(|row| row.render_view_mask == render_view_mask)
            .map(|row| row.map_copy_offset_f32_bits)
            .ok_or("XMAP10 SceneCB0 render view is absent from the XMAP04 map-copy set")?;
        let mut identity = Self {
            content_manifest_sha256,
            product_modset_sha256,
            effective_vfs_manifest_sha256,
            date,
            hour_index,
            map_width,
            map_height,
            viewport_width,
            viewport_height,
            replay_frame_identity_sha256,
            camera_view_sha256,
            map_copy_receipt_set_sha256,
            render_view_mask,
            map_copy_offset_f32_bits,
            snow_mud_input_sha256,
            identity_sha256: String::new(),
        };
        identity.validate_fields()?;
        identity.identity_sha256 = scene_frame_identity_digest(&identity);
        Ok(identity)
    }

    pub fn validate(&self) -> Result<()> {
        self.validate_fields()?;
        if self.identity_sha256 != scene_frame_identity_digest(self) {
            return Err("XMAP10 SceneCB0 frame identity digest mismatch".into());
        }
        Ok(())
    }

    fn validate_fields(&self) -> Result<()> {
        for (label, value) in [
            ("content manifest", &self.content_manifest_sha256),
            ("product modset", &self.product_modset_sha256),
            (
                "effective VFS manifest",
                &self.effective_vfs_manifest_sha256,
            ),
            ("replay frame", &self.replay_frame_identity_sha256),
            ("camera view", &self.camera_view_sha256),
            ("map-copy set", &self.map_copy_receipt_set_sha256),
            ("snow/mud input", &self.snow_mud_input_sha256),
        ] {
            if !is_lower_sha256(value) {
                return Err(
                    format!("XMAP10 SceneCB0 {label} identity is not a lowercase SHA-256").into(),
                );
            }
        }
        if self.date.is_empty()
            || self.hour_index < 0
            || self.map_width == 0
            || self.map_height == 0
            || self.viewport_width == 0
            || self.viewport_height == 0
        {
            return Err("XMAP10 SceneCB0 frame dimensions/date are invalid".into());
        }
        let expected_offset = match self.render_view_mask {
            1 => 0.0f32.to_bits(),
            2 => (self.map_width as f32).to_bits(),
            4 => (-(self.map_width as f32)).to_bits(),
            _ => {
                return Err("XMAP10 SceneCB0 render-view mask is not main/west/east".into());
            }
        };
        if self.map_copy_offset_f32_bits != expected_offset {
            return Err("XMAP10 SceneCB0 map-copy offset differs from its render-view mask".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Xmap03CameraScenePatch {
    pub view_projection_matrix: [f32; 16],
    pub inverse_view_projection_matrix: [f32; 16],
    pub camera_position: [f32; 3],
    pub camera_look_at_direction: [f32; 3],
}

impl Xmap03CameraScenePatch {
    pub fn from_xmap03(camera: &crate::ViewerMapCameraState) -> Result<Self> {
        camera.validate()?;
        let production = camera.production_camera()?;
        let inverse = camera.inverse_view_projection_matrix()?;
        Ok(Self {
            view_projection_matrix: production
                .view_projection_matrix
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .try_into()
                .expect("4x4 camera matrix has exactly 16 lanes"),
            inverse_view_projection_matrix: inverse
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .try_into()
                .expect("4x4 inverse camera matrix has exactly 16 lanes"),
            camera_position: production.position,
            camera_look_at_direction: camera.current_look_at_direction()?,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Xmap05PoliticalFowScenePatch {
    pub gb_camera_distance_override_outline_cutoff: [f32; 4],
    pub fow_opacity: f32,
    pub fow_time: f32,
    pub fow_fade_factor: f32,
    pub minimum_mesh_alpha: f32,
    pub gradient_border_texture_height: f32,
}

impl Xmap05PoliticalFowScenePatch {
    pub fn from_xmap05(
        patch: &crate::Xmap05SceneCb0Patch,
        frame: &Xmap10SceneFrameIdentity,
    ) -> Result<Self> {
        patch.validate()?;
        frame.validate()?;
        if patch.hour_index != frame.hour_index
            || patch.camera_state_sha256 != frame.camera_view_sha256
            || patch.effective_vfs_manifest_sha256 != frame.effective_vfs_manifest_sha256
        {
            return Err("XMAP10 received an XMAP05 patch from another frame or VFS".into());
        }
        Ok(Self {
            gb_camera_distance_override_outline_cutoff: patch
                .v_gb_cam_dist_override_gb_outline_cutoff,
            fow_opacity: patch.fow_opacity,
            fow_time: patch.fow_time,
            fow_fade_factor: patch.fow_fade_factor,
            minimum_mesh_alpha: patch.min_mesh_alpha,
            gradient_border_texture_height: patch.gb_texture_height,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Xmap07SnowMudScenePatch {
    pub snow_mud_fade: f32,
}

impl Xmap07SnowMudScenePatch {
    pub fn from_xmap07(
        input: &crate::Xmap07SceneSnowMudInput,
        frame: &Xmap10SceneFrameIdentity,
    ) -> Result<Self> {
        input.validate()?;
        frame.validate()?;
        if input.date != frame.date
            || input.hour_index != frame.hour_index
            || input.replay_frame_identity_sha256 != frame.replay_frame_identity_sha256
            || input.deterministic_sha256 != frame.snow_mud_input_sha256
        {
            return Err(
                "XMAP10 received an XMAP07 SnowMud patch from another frame or input".into(),
            );
        }
        Ok(Self {
            snow_mud_fade: f32::from_bits(input.snow_mud_fade_f32_bits),
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Xmap09LightingScenePatch {
    pub virtual_sun_position: [f32; 4],
    pub virtual_moon_position: [f32; 4],
    pub second_virtual_sun_position: [f32; 4],
    pub second_virtual_moon_position: [f32; 4],
    pub day_night_hour_sun_direction: [f32; 4],
    pub ambient_positive_x: [f32; 3],
    pub shadow_fade_factor: f32,
    pub ambient_negative_x: [f32; 3],
    pub ambient_positive_y: [f32; 3],
    pub ambient_negative_y: [f32; 3],
    pub ambient_positive_z: [f32; 3],
    pub ambient_negative_z: [f32; 3],
    pub cubemap_intensity: f32,
    pub sun_diffuse_intensity: [f32; 4],
    pub moon_diffuse_intensity: [f32; 4],
    pub sun_specular_intensity: f32,
}

impl Xmap09LightingScenePatch {
    pub fn from_xmap09(
        patch: &crate::Xmap09SceneCb0Patch,
        frame: &Xmap10SceneFrameIdentity,
    ) -> Result<Self> {
        patch.validate()?;
        frame.validate()?;
        if patch.replay_frame_identity_sha256 != frame.replay_frame_identity_sha256 {
            return Err(
                "XMAP10 received an XMAP09 SceneCB0 patch from another replay frame".into(),
            );
        }
        let field = |symbol: &str| -> Result<&[u8]> {
            patch
                .ordered_fields
                .iter()
                .find(|field| field.symbol == symbol)
                .map(|field| field.bytes.as_slice())
                .ok_or_else(|| format!("XMAP10 XMAP09 patch omitted {symbol}").into())
        };
        let lanes = |symbol: &str, count: usize| -> Result<Vec<f32>> {
            let bytes = field(symbol)?;
            if bytes.len() != count * 4 {
                return Err(
                    format!("XMAP10 XMAP09 field {symbol} has the wrong byte length").into(),
                );
            }
            bytes
                .chunks_exact(4)
                .map(|word| {
                    Ok(f32::from_bits(u32::from_le_bytes(
                        word.try_into().expect("four-byte chunk"),
                    )))
                })
                .collect()
        };
        let array = |symbol: &str, count: usize| -> Result<Vec<f32>> {
            let values = lanes(symbol, count)?;
            if values.iter().any(|value| !value.is_finite()) {
                return Err(format!("XMAP10 XMAP09 field {symbol} is non-finite").into());
            }
            Ok(values)
        };
        Ok(Self {
            virtual_sun_position: array("vVirtualSunPos", 4)?.try_into().unwrap(),
            virtual_moon_position: array("vVirtualMoonPos", 4)?.try_into().unwrap(),
            second_virtual_sun_position: array("vSecondVirtualSunPos", 4)?.try_into().unwrap(),
            second_virtual_moon_position: array("vSecondVirtualMoonPos", 4)?.try_into().unwrap(),
            day_night_hour_sun_direction: array("DayNight_Hour_SunDir", 4)?.try_into().unwrap(),
            ambient_positive_x: array("AmbientPosX", 3)?.try_into().unwrap(),
            shadow_fade_factor: array("ShadowFadeFactor", 1)?[0],
            ambient_negative_x: array("AmbientNegX", 3)?.try_into().unwrap(),
            ambient_positive_y: array("AmbientPosY", 3)?.try_into().unwrap(),
            ambient_negative_y: array("AmbientNegY", 3)?.try_into().unwrap(),
            ambient_positive_z: array("AmbientPosZ", 3)?.try_into().unwrap(),
            ambient_negative_z: array("AmbientNegZ", 3)?.try_into().unwrap(),
            cubemap_intensity: array("CubemapIntensity", 1)?[0],
            sun_diffuse_intensity: array("SunDiffuseIntensity", 4)?.try_into().unwrap(),
            moon_diffuse_intensity: array("MoonDiffuseIntensity", 4)?.try_into().unwrap(),
            sun_specular_intensity: array("SunSpecularIntensity", 1)?[0],
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Xmap10PresentationScenePatch {
    frame_sequence: u64,
    replay_frame_identity_sha256: String,
    prior_presentation_timing_receipt_sha256: String,
    hdr_enabled: bool,
    effective_light_hdr_range: f32,
    application_elapsed_seconds: f32,
    day_night_gate_factor: f32,
    presentation_timing_receipt_sha256: String,
}

impl Xmap10PresentationScenePatch {
    pub fn frame_sequence(&self) -> u64 {
        self.frame_sequence
    }

    pub fn replay_frame_identity_sha256(&self) -> &str {
        &self.replay_frame_identity_sha256
    }

    pub fn prior_presentation_timing_receipt_sha256(&self) -> &str {
        &self.prior_presentation_timing_receipt_sha256
    }

    pub fn hdr_enabled(&self) -> bool {
        self.hdr_enabled
    }

    pub fn effective_light_hdr_range(&self) -> f32 {
        self.effective_light_hdr_range
    }

    pub fn application_elapsed_seconds(&self) -> f32 {
        self.application_elapsed_seconds
    }

    pub fn day_night_gate_factor(&self) -> f32 {
        self.day_night_gate_factor
    }

    pub fn presentation_timing_receipt_sha256(&self) -> &str {
        &self.presentation_timing_receipt_sha256
    }
}

pub const XMAP10_VIEWER_PRESENTATION_STATE_SCHEMA: &str =
    "hoi4.xmap10.viewer-presentation-state.a729.v1";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Xmap10ViewerPresentationState {
    pub schema: String,
    pub frame_sequence: u64,
    pub application_elapsed_seconds_f32_bits: u32,
    pub day_night_gate_factor_f32_bits: u32,
    pub replay_frame_identity_sha256: String,
    pub recorded_speed_raw: u8,
    pub recorded_paused: bool,
    pub day_night_enabled: bool,
    pub prior_receipt_sha256: String,
    pub deterministic_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap10PresentationRecurrenceStep {
    pub replay_frame_identity_sha256: String,
    pub frame_delta_seconds_f32_bits: u32,
    pub recorded_speed_raw: u8,
    pub recorded_paused: bool,
    pub day_night_enabled: bool,
}

impl Xmap10PresentationRecurrenceStep {
    pub fn from_world_frame(
        world_frame: &replay_core::WorldFrame,
        replay_frame_identity_sha256: &str,
        frame_delta_seconds: f32,
        day_night_enabled: bool,
    ) -> Result<Self> {
        use replay_core::Leaf;
        let events = world_frame
            .session_metadata()
            .speed_events
            .measured()
            .ok_or("XMAP10 presentation update has no authenticated speed/pause history")?;
        let event = events
            .last()
            .ok_or("XMAP10 presentation update has an empty speed/pause history")?;
        let (Leaf::Measured(speed), Leaf::Measured(paused)) = (&event.speed, &event.paused) else {
            return Err("XMAP10 presentation update speed/pause leaves are unavailable".into());
        };
        let step = Self {
            replay_frame_identity_sha256: replay_frame_identity_sha256.into(),
            frame_delta_seconds_f32_bits: frame_delta_seconds.to_bits(),
            recorded_speed_raw: u8::try_from(*speed)
                .map_err(|_| "XMAP10 recorded speed is outside u8")?,
            recorded_paused: *paused,
            day_night_enabled,
        };
        step.validate()?;
        Ok(step)
    }

    pub fn validate(&self) -> Result<()> {
        let delta = f32::from_bits(self.frame_delta_seconds_f32_bits);
        if !is_lower_sha256(&self.replay_frame_identity_sha256)
            || !delta.is_finite()
            || delta < 0.0
            || self.recorded_speed_raw > 4
        {
            return Err("XMAP10 presentation recurrence step is invalid".into());
        }
        Ok(())
    }
}

impl Xmap10ViewerPresentationState {
    pub fn new() -> Result<Self> {
        let reset_receipt = Sha256::hex(b"XMAP10\0viewer-presentation-state\0exact-a729-reset\0v1");
        let mut state = Self {
            schema: XMAP10_VIEWER_PRESENTATION_STATE_SCHEMA.into(),
            frame_sequence: 0,
            application_elapsed_seconds_f32_bits: 0.0f32.to_bits(),
            day_night_gate_factor_f32_bits: 0.0f32.to_bits(),
            replay_frame_identity_sha256: Sha256::hex(b"XMAP10 presentation reset"),
            recorded_speed_raw: 0,
            recorded_paused: false,
            day_night_enabled: false,
            prior_receipt_sha256: reset_receipt,
            deterministic_sha256: String::new(),
        };
        state.deterministic_sha256 = viewer_presentation_state_digest(&state);
        state.validate()?;
        Ok(state)
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema != XMAP10_VIEWER_PRESENTATION_STATE_SCHEMA
            || self.recorded_speed_raw > 4
            || !f32::from_bits(self.application_elapsed_seconds_f32_bits).is_finite()
            || f32::from_bits(self.application_elapsed_seconds_f32_bits) < 0.0
            || !matches!(
                f32::from_bits(self.day_night_gate_factor_f32_bits),
                value if value.is_finite() && (0.0..=1.0).contains(&value)
            )
            || !is_lower_sha256(&self.replay_frame_identity_sha256)
            || !is_lower_sha256(&self.prior_receipt_sha256)
            || !is_lower_sha256(&self.deterministic_sha256)
            || self.deterministic_sha256 != viewer_presentation_state_digest(self)
        {
            return Err("XMAP10 viewer presentation state is invalid".into());
        }
        Ok(())
    }

    pub fn application_elapsed_seconds(&self) -> f32 {
        f32::from_bits(self.application_elapsed_seconds_f32_bits)
    }

    pub fn advance(
        &mut self,
        world_frame: &replay_core::WorldFrame,
        replay_frame_identity_sha256: &str,
        frame_delta_seconds: f32,
        day_night_enabled: bool,
        main_scene: &crate::ProductionMainSceneTargetDesc,
    ) -> Result<Xmap10PresentationScenePatch> {
        main_scene.validate()?;
        if main_scene.color_format != 10 {
            return Err(
                "XMAP10 presentation state requires HDR MainScene R16G16B16A16_FLOAT".into(),
            );
        }
        if !is_lower_sha256(replay_frame_identity_sha256) {
            return Err("XMAP10 presentation update has no replay-frame identity".into());
        }
        let step = Xmap10PresentationRecurrenceStep::from_world_frame(
            world_frame,
            replay_frame_identity_sha256,
            frame_delta_seconds,
            day_night_enabled,
        )?;
        self.advance_step(&step, main_scene)
    }

    pub fn advance_step(
        &mut self,
        step: &Xmap10PresentationRecurrenceStep,
        main_scene: &crate::ProductionMainSceneTargetDesc,
    ) -> Result<Xmap10PresentationScenePatch> {
        main_scene.validate()?;
        if main_scene.color_format != 10 {
            return Err(
                "XMAP10 presentation state requires HDR MainScene R16G16B16A16_FLOAT".into(),
            );
        }
        step.validate()?;
        self.advance_exact(
            &step.replay_frame_identity_sha256,
            f32::from_bits(step.frame_delta_seconds_f32_bits),
            step.recorded_speed_raw,
            step.recorded_paused,
            step.day_night_enabled,
        )
    }

    pub fn reconstruct_from_zero(
        &mut self,
        ordered_steps: &[Xmap10PresentationRecurrenceStep],
        main_scene: &crate::ProductionMainSceneTargetDesc,
    ) -> Result<Xmap10PresentationScenePatch> {
        if ordered_steps.is_empty() {
            return Err("XMAP10 seek reconstruction has no recurrence steps".into());
        }
        let mut rebuilt = Self::new()?;
        let mut patch = None;
        for step in ordered_steps {
            patch = Some(rebuilt.advance_step(step, main_scene)?);
        }
        rebuilt.validate()?;
        *self = rebuilt;
        patch.ok_or_else(|| "XMAP10 seek reconstruction produced no patch".into())
    }

    fn advance_exact(
        &mut self,
        replay_frame_identity_sha256: &str,
        frame_delta_seconds: f32,
        recorded_speed_raw: u8,
        recorded_paused: bool,
        day_night_enabled: bool,
    ) -> Result<Xmap10PresentationScenePatch> {
        self.validate()?;
        if !is_lower_sha256(replay_frame_identity_sha256)
            || !frame_delta_seconds.is_finite()
            || frame_delta_seconds < 0.0
            || recorded_speed_raw > 4
        {
            return Err("XMAP10 viewer presentation update input is invalid".into());
        }
        let prior_receipt_sha256 = self.deterministic_sha256.clone();
        let elapsed =
            f32::from_bits(self.application_elapsed_seconds_f32_bits) + frame_delta_seconds;
        if !elapsed.is_finite() {
            return Err("XMAP10 viewer presentation elapsed time overflowed".into());
        }
        let alpha = (frame_delta_seconds * 12.0).clamp(0.001, 1.0);
        let target = if (recorded_paused || recorded_speed_raw < 4) && day_night_enabled {
            1.0
        } else {
            0.0
        };
        let old = f32::from_bits(self.day_night_gate_factor_f32_bits);
        let gate = (old + (target - old) * alpha).clamp(0.0, 1.0);

        self.frame_sequence = self
            .frame_sequence
            .checked_add(1)
            .ok_or("XMAP10 presentation frame sequence overflowed")?;
        self.application_elapsed_seconds_f32_bits = elapsed.to_bits();
        self.day_night_gate_factor_f32_bits = gate.to_bits();
        self.replay_frame_identity_sha256 = replay_frame_identity_sha256.into();
        self.recorded_speed_raw = recorded_speed_raw;
        self.recorded_paused = recorded_paused;
        self.day_night_enabled = day_night_enabled;
        self.prior_receipt_sha256 = prior_receipt_sha256;
        self.deterministic_sha256 = viewer_presentation_state_digest(self);
        self.validate()?;

        Ok(Xmap10PresentationScenePatch {
            frame_sequence: self.frame_sequence,
            replay_frame_identity_sha256: self.replay_frame_identity_sha256.clone(),
            prior_presentation_timing_receipt_sha256: self.prior_receipt_sha256.clone(),
            hdr_enabled: true,
            effective_light_hdr_range: 1.0,
            application_elapsed_seconds: elapsed,
            day_night_gate_factor: gate,
            presentation_timing_receipt_sha256: self.deterministic_sha256.clone(),
        })
    }
}

fn viewer_presentation_state_digest(state: &Xmap10ViewerPresentationState) -> String {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(state.schema.as_bytes());
    bytes.extend_from_slice(&state.frame_sequence.to_le_bytes());
    bytes.extend_from_slice(&state.application_elapsed_seconds_f32_bits.to_le_bytes());
    bytes.extend_from_slice(&state.day_night_gate_factor_f32_bits.to_le_bytes());
    bytes.extend_from_slice(state.replay_frame_identity_sha256.as_bytes());
    bytes.push(state.recorded_speed_raw);
    bytes.push(u8::from(state.recorded_paused));
    bytes.push(u8::from(state.day_night_enabled));
    bytes.extend_from_slice(state.prior_receipt_sha256.as_bytes());
    Sha256::hex(&bytes)
}

#[derive(Clone, Debug, PartialEq)]
pub struct Xmap10SceneCompositionInput {
    pub frame_identity: Xmap10SceneFrameIdentity,
    pub expected_layout_sha256: String,
    pub camera: Xmap03CameraScenePatch,
    pub political_fow: Xmap05PoliticalFowScenePatch,
    pub snow_mud: Xmap07SnowMudScenePatch,
    pub lighting: Xmap09LightingScenePatch,
    pub presentation: Xmap10PresentationScenePatch,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Xmap10SceneConstantBufferDescriptor {
    pub schema: String,
    pub layout_source: String,
    pub bind_point: u32,
    pub byte_len: usize,
    pub layout_sha256: String,
    pub ownership_coverage_sha256: String,
    pub descriptor_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Xmap10ScenePatchHashes {
    pub xmap03_camera_sha256: String,
    pub xmap05_political_fow_sha256: String,
    pub xmap07_snow_mud_sha256: String,
    pub xmap09_lighting_sha256: String,
    pub xmap10_presentation_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedXmap10SceneConstantBuffer {
    pub frame_identity: Xmap10SceneFrameIdentity,
    pub descriptor: Xmap10SceneConstantBufferDescriptor,
    pub bytes: Vec<u8>,
    pub payload_sha256: String,
    pub patch_hashes: Xmap10ScenePatchHashes,
    pub composition_receipt_sha256: String,
}

impl PreparedXmap10SceneConstantBuffer {
    pub fn validate(&self) -> Result<()> {
        validate_coverage()?;
        self.frame_identity.validate()?;
        self.descriptor.validate()?;
        if self.bytes.len() != XMAP10_SCENE_CB0_BYTE_LEN {
            return Err("XMAP10 SceneCB0 payload is not 416 bytes".into());
        }
        if self.payload_sha256 != Sha256::hex(&self.bytes) {
            return Err("XMAP10 SceneCB0 payload digest mismatch".into());
        }
        for range in XMAP10_SCENE_FIELD_COVERAGE {
            if range.owner == Xmap10SceneFieldOwner::Padding
                && self.bytes[range.offset..range.offset + range.byte_len]
                    .iter()
                    .any(|byte| *byte != 0)
            {
                return Err(format!("XMAP10 SceneCB0 {} is non-zero", range.field).into());
            }
        }
        if !self.patch_hashes.all_valid() {
            return Err("XMAP10 SceneCB0 patch identity is invalid".into());
        }
        if self.composition_receipt_sha256 != composition_receipt_digest(self) {
            return Err("XMAP10 SceneCB0 composition receipt mismatch".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap10PreParticleSceneReceipt {
    pub schema: String,
    pub hour_index: i64,
    pub content_manifest_sha256: String,
    pub product_modset_sha256: String,
    pub effective_vfs_manifest_sha256: String,
    pub xmap09_lighting_admission_sha256: String,
    pub pre_particle_admission_sha256: String,
    pub scene_cb0: ConstantBufferBinding,
    pub main_scene_color: RenderTargetDesc,
    pub main_scene_depth: RenderTargetDesc,
    pub main_scene_target_sha256: String,
    pub deterministic_sha256: String,
}

impl Xmap10PreParticleSceneReceipt {
    #[allow(clippy::too_many_arguments)]
    pub fn seal(
        hour_index: i64,
        content_manifest_sha256: String,
        product_modset_sha256: String,
        effective_vfs_manifest_sha256: String,
        xmap09_lighting_admission_sha256: String,
        scene_cb0: &PreparedXmap10SceneConstantBuffer,
        main_scene_color: RenderTargetDesc,
        main_scene_depth: RenderTargetDesc,
    ) -> Result<Self> {
        scene_cb0.validate()?;
        if scene_cb0.frame_identity.hour_index != hour_index {
            return Err("XMAP10 pre-particle hour differs from SceneCB0".into());
        }
        if scene_cb0.frame_identity.content_manifest_sha256 != content_manifest_sha256
            || scene_cb0.frame_identity.product_modset_sha256 != product_modset_sha256
            || scene_cb0.frame_identity.effective_vfs_manifest_sha256
                != effective_vfs_manifest_sha256
        {
            return Err("XMAP10 pre-particle content identity differs from SceneCB0".into());
        }
        let main_scene_target_sha256 =
            xmap10_main_scene_target_sha256(&main_scene_color, &main_scene_depth)?;
        let pre_particle_admission_sha256 = pre_particle_admission_digest(
            hour_index,
            &content_manifest_sha256,
            &product_modset_sha256,
            &effective_vfs_manifest_sha256,
            &xmap09_lighting_admission_sha256,
            &scene_cb0.payload_sha256,
            &main_scene_target_sha256,
        );
        let scene_cb0_binding = ConstantBufferBinding {
            stage_mask_u8: 0b0000_0011,
            slot: XMAP10_SCENE_CB0_BIND_POINT as u8,
            byte_offset: 0,
            byte_count: XMAP10_SCENE_CB0_BYTE_LEN as u32,
            raw_bytes: scene_cb0.bytes.clone(),
            sha256: scene_cb0.payload_sha256.clone(),
            semantic_layout: XMAP10_SCENE_CB0_LAYOUT_SOURCE.into(),
        };
        let mut receipt = Self {
            schema: XMAP10_PRE_PARTICLE_SCENE_SCHEMA.into(),
            hour_index,
            content_manifest_sha256,
            product_modset_sha256,
            effective_vfs_manifest_sha256,
            xmap09_lighting_admission_sha256,
            pre_particle_admission_sha256,
            scene_cb0: scene_cb0_binding,
            main_scene_color,
            main_scene_depth,
            main_scene_target_sha256,
            deterministic_sha256: String::new(),
        };
        receipt.validate_body()?;
        receipt.deterministic_sha256 = Sha256::hex(&serde_json::to_vec(&receipt)?);
        receipt.validate()?;
        Ok(receipt)
    }

    pub fn validate(&self) -> Result<()> {
        self.validate_body()?;
        if !is_lower_sha256(&self.deterministic_sha256) {
            return Err("XMAP10 pre-particle receipt digest is invalid".into());
        }
        let mut unsigned = self.clone();
        unsigned.deterministic_sha256.clear();
        if self.deterministic_sha256 != Sha256::hex(&serde_json::to_vec(&unsigned)?) {
            return Err("XMAP10 pre-particle receipt digest mismatch".into());
        }
        Ok(())
    }

    pub fn color_target(&self) -> &PreparedRenderTarget {
        &self.main_scene_color.prepared_target
    }

    pub fn depth_target(&self) -> &PreparedRenderTarget {
        &self.main_scene_depth.prepared_target
    }

    pub fn scene_cb0(&self) -> &ConstantBufferBinding {
        &self.scene_cb0
    }

    fn validate_body(&self) -> Result<()> {
        if self.schema != XMAP10_PRE_PARTICLE_SCENE_SCHEMA || self.hour_index < 0 {
            return Err("XMAP10 pre-particle receipt identity is invalid".into());
        }
        for (label, value) in [
            ("content manifest", &self.content_manifest_sha256),
            ("product modset", &self.product_modset_sha256),
            (
                "effective VFS manifest",
                &self.effective_vfs_manifest_sha256,
            ),
            (
                "XMAP09 lighting admission",
                &self.xmap09_lighting_admission_sha256,
            ),
            (
                "pre-particle admission",
                &self.pre_particle_admission_sha256,
            ),
            ("SceneCB0 payload", &self.scene_cb0.sha256),
            ("MainScene target", &self.main_scene_target_sha256),
        ] {
            if !is_lower_sha256(value) {
                return Err(format!("XMAP10 pre-particle {label} is not SHA-256").into());
            }
        }
        if self.scene_cb0.stage_mask_u8 != 0b0000_0011
            || self.scene_cb0.slot != XMAP10_SCENE_CB0_BIND_POINT as u8
            || self.scene_cb0.byte_offset != 0
            || self.scene_cb0.byte_count != XMAP10_SCENE_CB0_BYTE_LEN as u32
            || self.scene_cb0.raw_bytes.len() != XMAP10_SCENE_CB0_BYTE_LEN
            || self.scene_cb0.sha256 != Sha256::hex(&self.scene_cb0.raw_bytes)
            || self.scene_cb0.semantic_layout != XMAP10_SCENE_CB0_LAYOUT_SOURCE
        {
            return Err("XMAP10 pre-particle SceneCB0 binding is not 416-byte VS/PS b0".into());
        }
        let expected_target =
            xmap10_main_scene_target_sha256(&self.main_scene_color, &self.main_scene_depth)?;
        if self.main_scene_target_sha256 != expected_target {
            return Err("XMAP10 pre-particle MainScene target digest mismatch".into());
        }
        if self.pre_particle_admission_sha256
            != pre_particle_admission_digest(
                self.hour_index,
                &self.content_manifest_sha256,
                &self.product_modset_sha256,
                &self.effective_vfs_manifest_sha256,
                &self.xmap09_lighting_admission_sha256,
                &self.scene_cb0.sha256,
                &self.main_scene_target_sha256,
            )
        {
            return Err("XMAP10 pre-particle operation admission mismatch".into());
        }
        Ok(())
    }
}

pub fn xmap10_main_scene_target_sha256(
    color: &RenderTargetDesc,
    depth: &RenderTargetDesc,
) -> Result<String> {
    if color.width == 0
        || color.height == 0
        || color.width != depth.width
        || color.height != depth.height
        || color.sample_count == 0
        || color.sample_count != depth.sample_count
        || color.sample_quality != depth.sample_quality
        || color.dxgi_format_u32 != 10
        || depth.dxgi_format_u32 == 0
        || color.prepared_target == depth.prepared_target
        || matches!(
            color.prepared_target,
            PreparedRenderTarget::SwapchainBackBuffer
        )
        || matches!(
            depth.prepared_target,
            PreparedRenderTarget::SwapchainBackBuffer
        )
    {
        return Err("XMAP10 pre-particle MainScene color/depth descriptors are invalid".into());
    }
    Ok(Sha256::hex(&serde_json::to_vec(&(color, depth))?))
}

fn pre_particle_admission_digest(
    hour_index: i64,
    content_manifest_sha256: &str,
    product_modset_sha256: &str,
    effective_vfs_manifest_sha256: &str,
    xmap09_lighting_admission_sha256: &str,
    scene_cb0_identity_sha256: &str,
    main_scene_target_sha256: &str,
) -> String {
    Sha256::hex(
        &serde_json::to_vec(&(
            XMAP10_PRE_PARTICLE_SCENE_SCHEMA,
            XMAP10_PRE_PARTICLE_OPERATION_ID,
            hour_index,
            content_manifest_sha256,
            product_modset_sha256,
            effective_vfs_manifest_sha256,
            xmap09_lighting_admission_sha256,
            XMAP10_SCENE_CB0_BYTE_LEN as u32,
            XMAP10_SCENE_CB0_BIND_POINT,
            XMAP10_SCENE_CB0_BIND_POINT,
            scene_cb0_identity_sha256,
            main_scene_target_sha256,
        ))
        .expect("pre-particle identity serialization cannot fail"),
    )
}

impl Xmap10SceneConstantBufferDescriptor {
    fn new() -> Result<Self> {
        validate_coverage()?;
        let mut descriptor = Self {
            schema: XMAP10_SCENE_CB0_SCHEMA.into(),
            layout_source: XMAP10_SCENE_CB0_LAYOUT_SOURCE.into(),
            bind_point: XMAP10_SCENE_CB0_BIND_POINT,
            byte_len: XMAP10_SCENE_CB0_BYTE_LEN,
            layout_sha256: xmap10_scene_cb0_layout_sha256(),
            ownership_coverage_sha256: ownership_coverage_digest(),
            descriptor_sha256: String::new(),
        };
        descriptor.descriptor_sha256 = descriptor_digest(&descriptor);
        Ok(descriptor)
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema != XMAP10_SCENE_CB0_SCHEMA
            || self.layout_source != XMAP10_SCENE_CB0_LAYOUT_SOURCE
            || self.bind_point != XMAP10_SCENE_CB0_BIND_POINT
            || self.byte_len != XMAP10_SCENE_CB0_BYTE_LEN
            || self.layout_sha256 != xmap10_scene_cb0_layout_sha256()
            || self.ownership_coverage_sha256 != ownership_coverage_digest()
            || self.descriptor_sha256 != descriptor_digest(self)
        {
            return Err("XMAP10 SceneCB0 descriptor identity mismatch".into());
        }
        Ok(())
    }
}

impl Xmap10ScenePatchHashes {
    fn all_valid(&self) -> bool {
        [
            &self.xmap03_camera_sha256,
            &self.xmap05_political_fow_sha256,
            &self.xmap07_snow_mud_sha256,
            &self.xmap09_lighting_sha256,
            &self.xmap10_presentation_sha256,
        ]
        .into_iter()
        .all(|hash| is_lower_sha256(hash))
    }
}

fn scene_cb0_reflected_layout_digest(
    cbuffer_name: &str,
    byte_len: u32,
    rows: &[(&str, u32, u32)],
) -> String {
    let mut identity = b"XMAP10\0SceneCB0ReflectedLayout\0v1\0".to_vec();
    identity.extend_from_slice(&(cbuffer_name.len() as u32).to_le_bytes());
    identity.extend_from_slice(cbuffer_name.as_bytes());
    identity.extend_from_slice(&byte_len.to_le_bytes());
    identity.extend_from_slice(&(rows.len() as u32).to_le_bytes());
    for (name, offset, size) in rows {
        identity.extend_from_slice(&(name.len() as u32).to_le_bytes());
        identity.extend_from_slice(name.as_bytes());
        identity.extend_from_slice(&offset.to_le_bytes());
        identity.extend_from_slice(&size.to_le_bytes());
    }
    Sha256::hex(&identity)
}

pub fn xmap10_scene_cb0_layout_sha256() -> String {
    scene_cb0_reflected_layout_digest(
        "dx11_cb0",
        XMAP10_SCENE_CB0_BYTE_LEN as u32,
        XMAP10_SCENE_CB0_REFLECTED_ROWS,
    )
}

fn xmap10_scene_cb0_stage_layout(
    bytecode: &[u8],
    stage: &str,
) -> Result<(String, u32, Vec<(String, u32, u32)>)> {
    let reflection = crate::parse_dxbc_reflection(bytecode)?;
    let resources = reflection
        .resources
        .iter()
        .filter(|resource| {
            resource.resource_type == 0
                && resource.bind_point == XMAP10_SCENE_CB0_BIND_POINT
                && resource.bind_count == 1
        })
        .collect::<Vec<_>>();
    if resources.len() != 1 {
        return Err(format!(
            "XMAP10 {stage} stage has {} register-b0 resources, expected exactly one",
            resources.len()
        )
        .into());
    }
    let resource = resources[0];
    let cbuffer = reflection
        .cbuffers
        .iter()
        .find(|cbuffer| cbuffer.name == resource.name)
        .ok_or_else(|| {
            format!(
                "XMAP10 {stage} reflection omitted register-b0 layout {}",
                resource.name
            )
        })?;
    let rows = cbuffer
        .vars
        .iter()
        .map(|var| (var.name.clone(), var.offset, var.size))
        .collect::<Vec<_>>();
    Ok((cbuffer.name.clone(), cbuffer.size, rows))
}

pub fn xmap10_scene_cb0_stage_reflected_layout_sha256(
    bytecode: &[u8],
    stage: &str,
) -> Result<String> {
    let reflected = xmap10_scene_cb0_stage_layout(bytecode, stage)?;
    let expected_rows = XMAP10_SCENE_CB0_REFLECTED_ROWS
        .iter()
        .map(|(name, offset, size)| ((*name).to_string(), *offset, *size))
        .collect::<Vec<_>>();
    if reflected.0 != "dx11_cb0"
        || reflected.1 != XMAP10_SCENE_CB0_BYTE_LEN as u32
        || reflected.2 != expected_rows
    {
        return Err(format!(
            "XMAP10 {stage} register-b0 layout differs from the canonical aggregate SceneCB0"
        )
        .into());
    }
    Ok(scene_cb0_reflected_layout_digest(
        &reflected.0,
        reflected.1,
        XMAP10_SCENE_CB0_REFLECTED_ROWS,
    ))
}

pub fn xmap10_scene_cb0_reflected_layout_sha256(
    vertex_bytecode: &[u8],
    pixel_bytecode: &[u8],
) -> Result<String> {
    let vertex = xmap10_scene_cb0_stage_layout(vertex_bytecode, "vertex")?;
    let pixel = xmap10_scene_cb0_stage_layout(pixel_bytecode, "pixel")?;
    if vertex != pixel {
        return Err("XMAP10 vertex/pixel SceneCB0 reflected layouts disagree".into());
    }
    let expected_rows = XMAP10_SCENE_CB0_REFLECTED_ROWS
        .iter()
        .map(|(name, offset, size)| ((*name).to_string(), *offset, *size))
        .collect::<Vec<_>>();
    if vertex.0 != "dx11_cb0"
        || vertex.1 != XMAP10_SCENE_CB0_BYTE_LEN as u32
        || vertex.2 != expected_rows
    {
        return Err(
            "XMAP10 consumer register-b0 layout differs from the canonical aggregate SceneCB0"
                .into(),
        );
    }
    Ok(scene_cb0_reflected_layout_digest(
        &vertex.0,
        vertex.1,
        XMAP10_SCENE_CB0_REFLECTED_ROWS,
    ))
}

pub fn compose_xmap10_scene_cb0(
    input: Xmap10SceneCompositionInput,
) -> Result<PreparedXmap10SceneConstantBuffer> {
    validate_coverage()?;
    input.frame_identity.validate()?;
    let descriptor = Xmap10SceneConstantBufferDescriptor::new()?;
    if input.expected_layout_sha256 != descriptor.layout_sha256 {
        return Err(
            "XMAP10 SceneCB0 caller layout identity does not match the 416-byte contract".into(),
        );
    }
    validate_input_finite(&input)?;

    let patch_hashes = patch_hashes(&input);
    let mut bytes = vec![0u8; XMAP10_SCENE_CB0_BYTE_LEN];
    let mut written = [false; XMAP10_SCENE_CB0_BYTE_LEN];

    put_f32s(
        &mut bytes,
        &mut written,
        0,
        &input.camera.view_projection_matrix,
    )?;
    put_f32s(
        &mut bytes,
        &mut written,
        64,
        &input.camera.inverse_view_projection_matrix,
    )?;
    put_f32s(
        &mut bytes,
        &mut written,
        128,
        &input
            .political_fow
            .gb_camera_distance_override_outline_cutoff,
    )?;
    put_f32s(
        &mut bytes,
        &mut written,
        144,
        &input.lighting.virtual_sun_position,
    )?;
    put_f32s(
        &mut bytes,
        &mut written,
        160,
        &input.lighting.virtual_moon_position,
    )?;
    put_f32s(
        &mut bytes,
        &mut written,
        176,
        &input.lighting.second_virtual_sun_position,
    )?;
    put_f32s(
        &mut bytes,
        &mut written,
        192,
        &input.lighting.second_virtual_moon_position,
    )?;
    put_f32s(&mut bytes, &mut written, 208, &input.camera.camera_position)?;
    put_f32s(
        &mut bytes,
        &mut written,
        220,
        &[if input.presentation.hdr_enabled {
            1.0
        } else {
            input.presentation.effective_light_hdr_range
        }],
    )?;
    put_f32s(
        &mut bytes,
        &mut written,
        224,
        &input.camera.camera_look_at_direction,
    )?;
    put_f32s(
        &mut bytes,
        &mut written,
        236,
        &[input.presentation.application_elapsed_seconds],
    )?;
    put_f32s(
        &mut bytes,
        &mut written,
        240,
        &[input.political_fow.fow_opacity],
    )?;
    put_f32s(
        &mut bytes,
        &mut written,
        244,
        &[input.political_fow.fow_time],
    )?;
    put_f32s(
        &mut bytes,
        &mut written,
        248,
        &[input.snow_mud.snow_mud_fade],
    )?;
    put_f32s(
        &mut bytes,
        &mut written,
        252,
        &[input.presentation.day_night_gate_factor],
    )?;
    put_f32s(
        &mut bytes,
        &mut written,
        256,
        &input.lighting.day_night_hour_sun_direction,
    )?;
    put_f32s(
        &mut bytes,
        &mut written,
        272,
        &input.lighting.ambient_positive_x,
    )?;
    put_f32s(
        &mut bytes,
        &mut written,
        284,
        &[input.lighting.shadow_fade_factor],
    )?;
    put_f32s(
        &mut bytes,
        &mut written,
        288,
        &input.lighting.ambient_negative_x,
    )?;
    put_f32s(
        &mut bytes,
        &mut written,
        300,
        &[input.political_fow.fow_fade_factor],
    )?;
    put_f32s(
        &mut bytes,
        &mut written,
        304,
        &input.lighting.ambient_positive_y,
    )?;
    put_f32s(
        &mut bytes,
        &mut written,
        316,
        &[input.political_fow.minimum_mesh_alpha],
    )?;
    put_f32s(
        &mut bytes,
        &mut written,
        320,
        &input.lighting.ambient_negative_y,
    )?;
    put_f32s(&mut bytes, &mut written, 332, &[1.0])?;
    put_f32s(
        &mut bytes,
        &mut written,
        336,
        &input.lighting.ambient_positive_z,
    )?;
    put_f32s(
        &mut bytes,
        &mut written,
        352,
        &input.lighting.ambient_negative_z,
    )?;
    put_f32s(
        &mut bytes,
        &mut written,
        364,
        &[input.lighting.cubemap_intensity],
    )?;
    put_f32s(
        &mut bytes,
        &mut written,
        368,
        &input.lighting.sun_diffuse_intensity,
    )?;
    put_f32s(
        &mut bytes,
        &mut written,
        384,
        &input.lighting.moon_diffuse_intensity,
    )?;
    put_f32s(
        &mut bytes,
        &mut written,
        400,
        &[input.political_fow.gradient_border_texture_height],
    )?;
    put_f32s(
        &mut bytes,
        &mut written,
        404,
        &[input.lighting.sun_specular_intensity],
    )?;

    for range in XMAP10_SCENE_FIELD_COVERAGE {
        let actual = &written[range.offset..range.offset + range.byte_len];
        if range.owner == Xmap10SceneFieldOwner::Padding {
            if actual.iter().any(|value| *value) {
                return Err(format!("XMAP10 SceneCB0 padding {} was written", range.field).into());
            }
        } else if actual.iter().any(|value| !*value) {
            return Err(format!(
                "XMAP10 SceneCB0 field {} was not fully supplied",
                range.field
            )
            .into());
        }
    }

    let mut prepared = PreparedXmap10SceneConstantBuffer {
        frame_identity: input.frame_identity,
        descriptor,
        payload_sha256: Sha256::hex(&bytes),
        bytes,
        patch_hashes,
        composition_receipt_sha256: String::new(),
    };
    prepared.composition_receipt_sha256 = composition_receipt_digest(&prepared);
    prepared.validate()?;
    Ok(prepared)
}

fn validate_input_finite(input: &Xmap10SceneCompositionInput) -> Result<()> {
    if input.presentation.frame_sequence == 0
        || input.presentation.replay_frame_identity_sha256
            != input.frame_identity.replay_frame_identity_sha256
        || !is_lower_sha256(&input.presentation.replay_frame_identity_sha256)
        || !is_lower_sha256(&input.presentation.prior_presentation_timing_receipt_sha256)
        || !is_lower_sha256(&input.presentation.presentation_timing_receipt_sha256)
        || input.presentation.presentation_timing_receipt_sha256
            == input.presentation.prior_presentation_timing_receipt_sha256
    {
        return Err(
            "XMAP10 SceneCB0 presentation timing carrier is stale or unauthenticated".into(),
        );
    }
    let mut values = Vec::with_capacity(102);
    values.extend(input.camera.view_projection_matrix);
    values.extend(input.camera.inverse_view_projection_matrix);
    values.extend(input.camera.camera_position);
    values.extend(input.camera.camera_look_at_direction);
    values.extend(
        input
            .political_fow
            .gb_camera_distance_override_outline_cutoff,
    );
    values.extend([
        input.political_fow.fow_opacity,
        input.political_fow.fow_time,
        input.political_fow.fow_fade_factor,
        input.political_fow.minimum_mesh_alpha,
        input.political_fow.gradient_border_texture_height,
        input.snow_mud.snow_mud_fade,
    ]);
    values.extend(input.lighting.virtual_sun_position);
    values.extend(input.lighting.virtual_moon_position);
    values.extend(input.lighting.second_virtual_sun_position);
    values.extend(input.lighting.second_virtual_moon_position);
    values.extend(input.lighting.day_night_hour_sun_direction);
    values.extend(input.lighting.ambient_positive_x);
    values.push(input.lighting.shadow_fade_factor);
    values.extend(input.lighting.ambient_negative_x);
    values.extend(input.lighting.ambient_positive_y);
    values.extend(input.lighting.ambient_negative_y);
    values.extend(input.lighting.ambient_positive_z);
    values.extend(input.lighting.ambient_negative_z);
    values.push(input.lighting.cubemap_intensity);
    values.extend(input.lighting.sun_diffuse_intensity);
    values.extend(input.lighting.moon_diffuse_intensity);
    values.push(input.lighting.sun_specular_intensity);
    values.extend([
        input.presentation.effective_light_hdr_range,
        input.presentation.application_elapsed_seconds,
        input.presentation.day_night_gate_factor,
    ]);
    if values.iter().any(|value| !value.is_finite()) {
        return Err("XMAP10 SceneCB0 input contains a non-finite value".into());
    }
    Ok(())
}

fn put_f32s(
    bytes: &mut [u8],
    written: &mut [bool; XMAP10_SCENE_CB0_BYTE_LEN],
    offset: usize,
    values: &[f32],
) -> Result<()> {
    let byte_len = values
        .len()
        .checked_mul(4)
        .ok_or("XMAP10 SceneCB0 field length overflow")?;
    let end = offset
        .checked_add(byte_len)
        .ok_or("XMAP10 SceneCB0 field offset overflow")?;
    if end > bytes.len() || written[offset..end].iter().any(|value| *value) {
        return Err("XMAP10 SceneCB0 field write overlaps or exceeds the layout".into());
    }
    for (index, value) in values.iter().enumerate() {
        let start = offset + index * 4;
        bytes[start..start + 4].copy_from_slice(&value.to_le_bytes());
        written[start..start + 4].fill(true);
    }
    Ok(())
}

fn validate_coverage() -> Result<()> {
    let mut cursor = 0usize;
    for range in XMAP10_SCENE_FIELD_COVERAGE {
        if range.offset != cursor || range.byte_len == 0 || range.byte_len % 4 != 0 {
            return Err(format!("XMAP10 SceneCB0 coverage breaks at {}", range.field).into());
        }
        cursor = cursor
            .checked_add(range.byte_len)
            .ok_or("XMAP10 SceneCB0 coverage overflow")?;
    }
    if cursor != XMAP10_SCENE_CB0_BYTE_LEN {
        return Err("XMAP10 SceneCB0 coverage does not span 416 bytes".into());
    }
    Ok(())
}

fn patch_hashes(input: &Xmap10SceneCompositionInput) -> Xmap10ScenePatchHashes {
    Xmap10ScenePatchHashes {
        xmap03_camera_sha256: hash_floats(
            "XMAP03",
            input
                .camera
                .view_projection_matrix
                .iter()
                .chain(input.camera.inverse_view_projection_matrix.iter())
                .chain(input.camera.camera_position.iter())
                .chain(input.camera.camera_look_at_direction.iter())
                .copied(),
        ),
        xmap05_political_fow_sha256: hash_floats(
            "XMAP05",
            input
                .political_fow
                .gb_camera_distance_override_outline_cutoff
                .iter()
                .copied()
                .chain([
                    input.political_fow.fow_opacity,
                    input.political_fow.fow_time,
                    input.political_fow.fow_fade_factor,
                    input.political_fow.minimum_mesh_alpha,
                    input.political_fow.gradient_border_texture_height,
                ]),
        ),
        xmap07_snow_mud_sha256: hash_floats(
            "XMAP07",
            std::iter::once(input.snow_mud.snow_mud_fade),
        ),
        xmap09_lighting_sha256: hash_floats(
            "XMAP09",
            input
                .lighting
                .virtual_sun_position
                .iter()
                .chain(input.lighting.virtual_moon_position.iter())
                .chain(input.lighting.second_virtual_sun_position.iter())
                .chain(input.lighting.second_virtual_moon_position.iter())
                .copied()
                .chain(input.lighting.day_night_hour_sun_direction)
                .chain(input.lighting.ambient_positive_x)
                .chain(std::iter::once(input.lighting.shadow_fade_factor))
                .chain(input.lighting.ambient_negative_x)
                .chain(input.lighting.ambient_positive_y)
                .chain(input.lighting.ambient_negative_y)
                .chain(input.lighting.ambient_positive_z)
                .chain(input.lighting.ambient_negative_z)
                .chain(std::iter::once(input.lighting.cubemap_intensity))
                .chain(input.lighting.sun_diffuse_intensity)
                .chain(input.lighting.moon_diffuse_intensity)
                .chain(std::iter::once(input.lighting.sun_specular_intensity)),
        ),
        xmap10_presentation_sha256: presentation_patch_digest(&input.presentation),
    }
}

fn hash_floats(label: &str, values: impl IntoIterator<Item = f32>) -> String {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(XMAP10_SCENE_CB0_SCHEMA.as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(label.as_bytes());
    bytes.push(0);
    for value in values {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    Sha256::hex(&bytes)
}

fn presentation_patch_digest(patch: &Xmap10PresentationScenePatch) -> String {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(XMAP10_SCENE_CB0_SCHEMA.as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(b"XMAP10");
    bytes.push(0);
    bytes.extend_from_slice(&patch.frame_sequence.to_le_bytes());
    bytes.extend_from_slice(patch.replay_frame_identity_sha256.as_bytes());
    bytes.extend_from_slice(patch.prior_presentation_timing_receipt_sha256.as_bytes());
    bytes.extend_from_slice(patch.presentation_timing_receipt_sha256.as_bytes());
    bytes.extend_from_slice(&(if patch.hdr_enabled { 1.0_f32 } else { 0.0_f32 }).to_le_bytes());
    bytes.extend_from_slice(&patch.effective_light_hdr_range.to_le_bytes());
    bytes.extend_from_slice(&patch.application_elapsed_seconds.to_le_bytes());
    bytes.extend_from_slice(&patch.day_night_gate_factor.to_le_bytes());
    bytes.extend_from_slice(&1.0_f32.to_le_bytes());
    Sha256::hex(&bytes)
}

fn scene_frame_identity_digest(identity: &Xmap10SceneFrameIdentity) -> String {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(XMAP10_SCENE_CB0_SCHEMA.as_bytes());
    bytes.extend_from_slice(identity.content_manifest_sha256.as_bytes());
    bytes.extend_from_slice(identity.product_modset_sha256.as_bytes());
    bytes.extend_from_slice(identity.effective_vfs_manifest_sha256.as_bytes());
    bytes.extend_from_slice(identity.date.as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(&identity.hour_index.to_le_bytes());
    bytes.extend_from_slice(&identity.map_width.to_le_bytes());
    bytes.extend_from_slice(&identity.map_height.to_le_bytes());
    bytes.extend_from_slice(&identity.viewport_width.to_le_bytes());
    bytes.extend_from_slice(&identity.viewport_height.to_le_bytes());
    bytes.extend_from_slice(identity.replay_frame_identity_sha256.as_bytes());
    bytes.extend_from_slice(identity.camera_view_sha256.as_bytes());
    bytes.extend_from_slice(identity.map_copy_receipt_set_sha256.as_bytes());
    bytes.push(identity.render_view_mask);
    bytes.extend_from_slice(&identity.map_copy_offset_f32_bits.to_le_bytes());
    bytes.extend_from_slice(identity.snow_mud_input_sha256.as_bytes());
    Sha256::hex(&bytes)
}

#[cfg(windows)]
mod native {
    use super::*;
    use crate::d3d11_device::D3dPtr;
    use crate::{
        NativeProductionMainSceneTarget, NativeXmap04UnderwaterSceneConstantBuffer,
        NativeXmap07SceneConstantBufferView,
    };
    use std::marker::PhantomData;
    use std::ptr;
    use winapi::um::d3d11::{
        ID3D11Buffer, ID3D11Device, ID3D11Texture2D, D3D11_BIND_CONSTANT_BUFFER, D3D11_BUFFER_DESC,
        D3D11_SUBRESOURCE_DATA, D3D11_USAGE_IMMUTABLE,
    };

    pub struct NativeXmap10SceneConstantBuffer {
        buffer: D3dPtr<ID3D11Buffer>,
        prepared: PreparedXmap10SceneConstantBuffer,
        device_identity: usize,
    }

    impl NativeXmap10SceneConstantBuffer {
        pub unsafe fn create_on_device(
            device: *mut ID3D11Device,
            prepared: PreparedXmap10SceneConstantBuffer,
        ) -> Result<Self> {
            if device.is_null() {
                return Err("XMAP10 SceneCB0 received a null D3D11 device".into());
            }
            prepared.validate()?;
            let desc = D3D11_BUFFER_DESC {
                ByteWidth: XMAP10_SCENE_CB0_BYTE_LEN as u32,
                Usage: D3D11_USAGE_IMMUTABLE,
                BindFlags: D3D11_BIND_CONSTANT_BUFFER,
                CPUAccessFlags: 0,
                MiscFlags: 0,
                StructureByteStride: 0,
            };
            let initial = D3D11_SUBRESOURCE_DATA {
                pSysMem: prepared.bytes.as_ptr() as *const _,
                SysMemPitch: 0,
                SysMemSlicePitch: 0,
            };
            let mut raw = ptr::null_mut();
            let hr = (*device).CreateBuffer(&desc, &initial, &mut raw);
            if hr < 0 {
                return Err(format!(
                    "CreateBuffer(XMAP10 SceneCB0) failed with HRESULT 0x{:08x}",
                    hr as u32
                )
                .into());
            }
            Ok(Self {
                buffer: D3dPtr::from_raw(raw, "XMAP10 SceneCB0")?,
                prepared,
                device_identity: device as usize,
            })
        }

        pub fn prepared(&self) -> &PreparedXmap10SceneConstantBuffer {
            &self.prepared
        }

        pub fn raw_buffer(&self) -> *mut ID3D11Buffer {
            self.buffer.as_raw()
        }

        pub fn device_identity(&self) -> usize {
            self.device_identity
        }

        pub fn bind_main_scene<'a>(
            &'a self,
            pre_particle_scene: &'a Xmap10PreParticleSceneReceipt,
            main_scene: &'a NativeProductionMainSceneTarget,
        ) -> Result<NativeXmap10MapCopySceneCb0View<'a>> {
            self.prepared.validate()?;
            pre_particle_scene.validate()?;
            let identity = &self.prepared.frame_identity;
            let target = main_scene.desc();
            if self.device_identity == 0
                || self.device_identity != main_scene.device_identity()
                || main_scene.color_texture().is_null()
                || identity.hour_index != pre_particle_scene.hour_index
                || identity.content_manifest_sha256 != pre_particle_scene.content_manifest_sha256
                || identity.product_modset_sha256 != pre_particle_scene.product_modset_sha256
                || identity.effective_vfs_manifest_sha256
                    != pre_particle_scene.effective_vfs_manifest_sha256
                || identity.viewport_width != target.color_width
                || identity.viewport_height != target.color_height
                || target.color_width != pre_particle_scene.main_scene_color.width
                || target.color_height != pre_particle_scene.main_scene_color.height
                || target.color_format != pre_particle_scene.main_scene_color.dxgi_format_u32
                || target.color_samples != pre_particle_scene.main_scene_color.sample_count
                || target.color_quality != pre_particle_scene.main_scene_color.sample_quality
                || target.depth_width != pre_particle_scene.main_scene_depth.width
                || target.depth_height != pre_particle_scene.main_scene_depth.height
                || target.depth_format != pre_particle_scene.main_scene_depth.dxgi_format_u32
                || target.depth_samples != pre_particle_scene.main_scene_depth.sample_count
                || target.depth_quality != pre_particle_scene.main_scene_depth.sample_quality
                || !is_lower_sha256(main_scene.resource_identity_sha256())
            {
                return Err(
                    "XMAP10 SceneCB0/MainScene view mixes a device, frame, or target descriptor"
                        .into(),
                );
            }
            if identity.render_view_mask == 1
                && (pre_particle_scene.scene_cb0.sha256 != self.prepared.payload_sha256
                    || pre_particle_scene.scene_cb0.raw_bytes != self.prepared.bytes)
            {
                return Err(
                    "XMAP10 pre-particle SceneCB0 is not the canonical main map-copy buffer".into(),
                );
            }
            Ok(NativeXmap10MapCopySceneCb0View {
                scene_cb0: self,
                main_scene,
                _borrow: PhantomData,
            })
        }
    }

    pub struct NativeXmap10MapCopySceneCb0View<'a> {
        scene_cb0: &'a NativeXmap10SceneConstantBuffer,
        main_scene: &'a NativeProductionMainSceneTarget,
        _borrow: PhantomData<&'a ()>,
    }

    impl NativeXmap10MapCopySceneCb0View<'_> {
        fn frame(&self) -> &Xmap10SceneFrameIdentity {
            &self.scene_cb0.prepared.frame_identity
        }

        pub fn owner_packet(&self) -> &'static str {
            "XMAP10"
        }

        pub fn symbol(&self) -> &'static str {
            "SceneCB0"
        }

        pub fn shader_register(&self) -> u32 {
            XMAP10_SCENE_CB0_BIND_POINT
        }

        pub fn byte_len(&self) -> u32 {
            XMAP10_SCENE_CB0_BYTE_LEN as u32
        }

        pub fn payload_sha256(&self) -> &str {
            &self.scene_cb0.prepared.payload_sha256
        }

        pub fn descriptor_sha256(&self) -> &str {
            &self.scene_cb0.prepared.descriptor.descriptor_sha256
        }

        pub fn map_copy_receipt_set_sha256(&self) -> &str {
            &self.frame().map_copy_receipt_set_sha256
        }

        pub fn render_view_mask(&self) -> u8 {
            self.frame().render_view_mask
        }

        pub fn map_copy_offset_f32_bits(&self) -> u32 {
            self.frame().map_copy_offset_f32_bits
        }

        pub fn composition_receipt_sha256(&self) -> &str {
            &self.scene_cb0.prepared.composition_receipt_sha256
        }

        pub fn reflected_layout_sha256(&self) -> &str {
            &self.scene_cb0.prepared.descriptor.layout_sha256
        }

        pub fn application_elapsed_seconds(&self) -> f32 {
            let bytes: [u8; 4] = self.scene_cb0.prepared.bytes
                [XMAP10_SCENE_CB0_GLOBAL_TIME_OFFSET..XMAP10_SCENE_CB0_GLOBAL_TIME_OFFSET + 4]
                .try_into()
                .expect("validated XMAP10 SceneCB0 contains vGlobalTime");
            f32::from_le_bytes(bytes)
        }

        pub fn camera_frame_sha256(&self) -> &str {
            &self.frame().camera_view_sha256
        }

        pub fn replay_frame_identity_sha256(&self) -> &str {
            &self.frame().replay_frame_identity_sha256
        }

        pub fn snow_mud_input_sha256(&self) -> &str {
            &self.frame().snow_mud_input_sha256
        }

        pub fn date(&self) -> &str {
            &self.frame().date
        }

        pub fn hour_index(&self) -> i64 {
            self.frame().hour_index
        }

        pub fn map_width(&self) -> u32 {
            self.frame().map_width
        }

        pub fn map_height(&self) -> u32 {
            self.frame().map_height
        }

        pub fn viewport_width(&self) -> u32 {
            self.frame().viewport_width
        }

        pub fn viewport_height(&self) -> u32 {
            self.frame().viewport_height
        }

        pub fn main_scene_resource_identity_sha256(&self) -> &str {
            self.main_scene.resource_identity_sha256()
        }

        pub fn main_scene_allocation_generation(&self) -> u64 {
            self.main_scene.allocation_generation()
        }

        pub fn device_identity(&self) -> usize {
            self.scene_cb0.device_identity
        }

        pub fn raw_buffer(&self) -> *mut ID3D11Buffer {
            self.scene_cb0.raw_buffer()
        }

        pub fn raw_main_scene_color_texture(&self) -> *mut ID3D11Texture2D {
            self.main_scene.color_texture()
        }
    }

    unsafe impl NativeXmap04UnderwaterSceneConstantBuffer for NativeXmap10MapCopySceneCb0View<'_> {
        fn owner_packet(&self) -> &str {
            self.owner_packet()
        }
        fn symbol(&self) -> &str {
            self.symbol()
        }
        fn shader_register(&self) -> u32 {
            self.shader_register()
        }
        fn byte_len(&self) -> u32 {
            self.byte_len()
        }
        fn viewport_width(&self) -> u32 {
            self.viewport_width()
        }
        fn viewport_height(&self) -> u32 {
            self.viewport_height()
        }
        fn frame_identity_sha256(&self) -> &str {
            self.camera_frame_sha256()
        }
        fn replay_frame_identity_sha256(&self) -> &str {
            self.replay_frame_identity_sha256()
        }
        fn payload_sha256(&self) -> &str {
            self.payload_sha256()
        }
        fn descriptor_sha256(&self) -> &str {
            self.descriptor_sha256()
        }
        fn raw_buffer(&self) -> *mut ID3D11Buffer {
            self.scene_cb0.raw_buffer()
        }
    }

    unsafe impl NativeXmap07SceneConstantBufferView for NativeXmap10MapCopySceneCb0View<'_> {
        fn symbol(&self) -> &str {
            NativeXmap10MapCopySceneCb0View::symbol(self)
        }
        fn owner_packet(&self) -> &str {
            NativeXmap10MapCopySceneCb0View::owner_packet(self)
        }
        fn shader_register(&self) -> u32 {
            NativeXmap10MapCopySceneCb0View::shader_register(self)
        }
        fn byte_len(&self) -> u32 {
            NativeXmap10MapCopySceneCb0View::byte_len(self)
        }
        fn payload_sha256(&self) -> &str {
            NativeXmap10MapCopySceneCb0View::payload_sha256(self)
        }
        fn descriptor_sha256(&self) -> &str {
            NativeXmap10MapCopySceneCb0View::descriptor_sha256(self)
        }
        fn composition_receipt_sha256(&self) -> &str {
            NativeXmap10MapCopySceneCb0View::composition_receipt_sha256(self)
        }
        fn reflected_layout_sha256(&self) -> &str {
            NativeXmap10MapCopySceneCb0View::reflected_layout_sha256(self)
        }
        fn date(&self) -> &str {
            NativeXmap10MapCopySceneCb0View::date(self)
        }
        fn hour_index(&self) -> i64 {
            NativeXmap10MapCopySceneCb0View::hour_index(self)
        }
        fn map_width(&self) -> u32 {
            NativeXmap10MapCopySceneCb0View::map_width(self)
        }
        fn map_height(&self) -> u32 {
            NativeXmap10MapCopySceneCb0View::map_height(self)
        }
        fn viewport_width(&self) -> u32 {
            NativeXmap10MapCopySceneCb0View::viewport_width(self)
        }
        fn viewport_height(&self) -> u32 {
            NativeXmap10MapCopySceneCb0View::viewport_height(self)
        }
        fn camera_view_sha256(&self) -> &str {
            NativeXmap10MapCopySceneCb0View::camera_frame_sha256(self)
        }
        fn replay_frame_identity_sha256(&self) -> &str {
            NativeXmap10MapCopySceneCb0View::replay_frame_identity_sha256(self)
        }
        fn map_copy_offset_f32_bits(&self) -> u32 {
            NativeXmap10MapCopySceneCb0View::map_copy_offset_f32_bits(self)
        }
        fn snow_mud_input_sha256(&self) -> &str {
            NativeXmap10MapCopySceneCb0View::snow_mud_input_sha256(self)
        }
        fn raw_buffer(&self) -> *mut ID3D11Buffer {
            NativeXmap10MapCopySceneCb0View::raw_buffer(self)
        }
    }

    pub use NativeXmap10MapCopySceneCb0View as ExportedNativeXmap10MapCopySceneCb0View;
    pub use NativeXmap10SceneConstantBuffer as ExportedNativeXmap10SceneConstantBuffer;
}

#[cfg(windows)]
pub use native::ExportedNativeXmap10MapCopySceneCb0View as NativeXmap10MapCopySceneCb0View;
#[cfg(windows)]
pub use native::ExportedNativeXmap10SceneConstantBuffer as NativeXmap10SceneConstantBuffer;

fn ownership_coverage_digest() -> String {
    let mut bytes = Vec::new();
    for range in XMAP10_SCENE_FIELD_COVERAGE {
        bytes.extend_from_slice(range.field.as_bytes());
        bytes.push(0);
        bytes.extend_from_slice(&(range.offset as u64).to_le_bytes());
        bytes.extend_from_slice(&(range.byte_len as u64).to_le_bytes());
        bytes.extend_from_slice(range.owner.label().as_bytes());
        bytes.push(0);
    }
    Sha256::hex(&bytes)
}

fn descriptor_digest(descriptor: &Xmap10SceneConstantBufferDescriptor) -> String {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(descriptor.schema.as_bytes());
    bytes.extend_from_slice(descriptor.layout_source.as_bytes());
    bytes.extend_from_slice(&descriptor.bind_point.to_le_bytes());
    bytes.extend_from_slice(&(descriptor.byte_len as u64).to_le_bytes());
    bytes.extend_from_slice(descriptor.layout_sha256.as_bytes());
    bytes.extend_from_slice(descriptor.ownership_coverage_sha256.as_bytes());
    Sha256::hex(&bytes)
}

fn composition_receipt_digest(prepared: &PreparedXmap10SceneConstantBuffer) -> String {
    let hashes = &prepared.patch_hashes;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(XMAP10_SCENE_CB0_SCHEMA.as_bytes());
    bytes.extend_from_slice(prepared.frame_identity.identity_sha256.as_bytes());
    bytes.extend_from_slice(prepared.descriptor.descriptor_sha256.as_bytes());
    bytes.extend_from_slice(prepared.payload_sha256.as_bytes());
    bytes.extend_from_slice(hashes.xmap03_camera_sha256.as_bytes());
    bytes.extend_from_slice(hashes.xmap05_political_fow_sha256.as_bytes());
    bytes.extend_from_slice(hashes.xmap07_snow_mud_sha256.as_bytes());
    bytes.extend_from_slice(hashes.xmap09_lighting_sha256.as_bytes());
    bytes.extend_from_slice(hashes.xmap10_presentation_sha256.as_bytes());
    Sha256::hex(&bytes)
}

fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sha(label: &str) -> String {
        Sha256::hex(label.as_bytes())
    }

    #[test]
    fn viewer_phase_reaches_scene_bytes_through_foundation_water_binding() {
        let graphics_runtime = crate::ViewerGraphicsRuntime::new();
        let application = graphics_runtime.new_application();
        let application_frame = application.enter_frame().unwrap();
        use crate::xmap07_snow_mud_phase::{Xmap07SnowMudPhaseDurations, Xmap07SnowMudPhaseState};
        use crate::production_features::xmap07::{Xmap07SceneSnowMudInput, Xmap07WaterTemporalBinding};
        struct Source;
        impl replay_core::ReplaySource for Source {
            fn game_id(&self) -> &str { "snow-mud-phase-scene" }
            fn hours(&self) -> &[i64] { &[25, 37] }
            fn warnings(&self) -> &[String] { &[] }
            fn frame(&self, hour_index: i64) -> std::result::Result<replay_core::RawFrame, replay_core::SourceError> {
                Ok(replay_core::RawFrame {
                    hour_index, date: Some("1936.1.1".into()), save_version: None,
                    schema_version: None, rest: serde_json::Map::new().into(),
                })
            }
        }
        let timeline = replay_core::WorldTimeline::open(Box::new(Source)).unwrap();
        let first = timeline.frame(25).unwrap();
        let next = timeline.frame(37).unwrap();
        let durations = Xmap07SnowMudPhaseDurations::from_sources(&[("00.lua".into(),
            "NDefines = { NGame = { GAME_SPEED_SECONDS = {2, 1, 0.5, 0.25, 0} } }".into())]).unwrap();
        let mut state = Xmap07SnowMudPhaseState::default();
        let viewer = crate::test_support::bare_viewer();
        let selection = crate::Xmap15ViewerSelection::default();
        let mut previous_provenance = String::new();
        for (world, now, speed, expected) in [
            (&first, 100.0, 0, 0.0f32), (&next, 100.5, 0, 0.125),
            (&next, 101.0, 0, 0.25), (&next, 101.25, 4, 0.5),
        ] {
            let phase = state.sample(world, now, speed, &durations).unwrap();
            let mut context = crate::FoundationFeatureContext::new_for_owned_accounting_tests(application_frame.witness(),
                world, &viewer, &[], &[], &selection,
            );
            context.snow_mud_phase = Some(&phase);
            let temporal = Xmap07WaterTemporalBinding::from_foundation(&context).unwrap();
            assert_eq!(temporal.season_lerp_f32_bits, expected.to_bits());
            assert_eq!(temporal.phase_provenance_sha256, phase.provenance_sha256);
            assert_ne!(temporal.phase_provenance_sha256, previous_provenance);
            previous_provenance = temporal.phase_provenance_sha256.clone();
            let input = Xmap07SceneSnowMudInput::from_temporal(
                world.date().unwrap().into(), world.hour_index(), &temporal,
                sha("grid-bytes"), sha("grid-custody"), sha("water-frame"),
            ).unwrap();
            assert_eq!(input.phase_provenance_sha256, temporal.phase_provenance_sha256);
            let mut composition = sample_input();
            composition.frame_identity = Xmap10SceneFrameIdentity::new(
                sha("content"), sha("modset"), sha("vfs"), input.date.clone(), input.hour_index,
                5632, 2048, 1920, 1080, input.replay_frame_identity_sha256.clone(), sha("camera"),
                &[Xmap04MapCopyReceiptRow { render_view_mask: 1, map_copy_offset_f32_bits: 0 }],
                1, input.deterministic_sha256.clone(),
            ).unwrap();
            composition.presentation.replay_frame_identity_sha256 = input.replay_frame_identity_sha256.clone();
            composition.snow_mud = Xmap07SnowMudScenePatch::from_xmap07(&input, &composition.frame_identity).unwrap();
            let bytes = compose_xmap10_scene_cb0(composition).unwrap();
            assert_eq!(&bytes.bytes[248..252], &expected.to_bits().to_le_bytes());
            assert_eq!(bytes.frame_identity.snow_mud_input_sha256, input.deterministic_sha256);
            if world.hour_index() == 37 {
                context.world_frame = &first;
                assert!(Xmap07WaterTemporalBinding::from_foundation(&context).is_err());
            }
        }
    }

    #[test]
    fn canonical_reflected_layout_identity_is_stable() {
        let canonical_from_rows = scene_cb0_reflected_layout_digest(
            "dx11_cb0",
            XMAP10_SCENE_CB0_BYTE_LEN as u32,
            XMAP10_SCENE_CB0_REFLECTED_ROWS,
        );
        assert_eq!(
            xmap10_scene_cb0_layout_sha256(),
            "b7c41002fa35c2b17b652e662f13708462e2da4ef35b9951e7cbed8cf5eb175a"
        );
        assert_eq!(xmap10_scene_cb0_layout_sha256(), canonical_from_rows);

        let mut tampered_rows = XMAP10_SCENE_CB0_REFLECTED_ROWS.to_vec();
        tampered_rows[0].1 += 4;
        assert_ne!(
            xmap10_scene_cb0_layout_sha256(),
            scene_cb0_reflected_layout_digest(
                "dx11_cb0",
                XMAP10_SCENE_CB0_BYTE_LEN as u32,
                &tampered_rows,
            )
        );
    }

    #[test]
    fn viewer_presentation_gate_uses_exact_pause_speed_toggle_and_smoothing_law() {
        let mut state = Xmap10ViewerPresentationState::new().unwrap();
        let replay = sha("presentation-replay-frame");

        let enabled = state
            .advance_exact(&replay, 1.0 / 60.0, 4, true, true)
            .unwrap();
        let first_alpha = ((1.0f32 / 60.0f32) * 12.0f32).clamp(0.001, 1.0);
        assert_eq!(
            enabled.day_night_gate_factor.to_bits(),
            (0.0f32 + (1.0f32 - 0.0f32) * first_alpha)
                .clamp(0.0, 1.0)
                .to_bits()
        );
        assert_eq!(
            enabled.application_elapsed_seconds.to_bits(),
            (1.0f32 / 60.0).to_bits()
        );
        assert_eq!(enabled.frame_sequence, 1);
        assert_eq!(enabled.replay_frame_identity_sha256, replay);

        let fastest_running = state
            .advance_exact(&replay, 1.0 / 60.0, 4, false, true)
            .unwrap();
        let alpha = ((1.0f32 / 60.0f32) * 12.0f32).clamp(0.001, 1.0);
        assert_eq!(
            fastest_running.day_night_gate_factor.to_bits(),
            (enabled.day_night_gate_factor + (0.0f32 - enabled.day_night_gate_factor) * alpha)
                .clamp(0.0, 1.0)
                .to_bits()
        );

        let toggled_off = state.advance_exact(&replay, 1.0, 0, true, false).unwrap();
        assert_eq!(
            toggled_off.day_night_gate_factor.to_bits(),
            0.0f32.to_bits()
        );
        assert_eq!(
            toggled_off.presentation_timing_receipt_sha256,
            state.deterministic_sha256
        );
    }

    #[test]
    fn viewer_presentation_seek_reconstructs_from_constructor_zero() {
        let main_scene = crate::ProductionMainSceneTargetDesc {
            color_attachment_ordinal: 0,
            color_width: 1920,
            color_height: 1080,
            color_format: 10,
            color_samples: 1,
            color_quality: 0,
            depth_attachment_ordinal: 1,
            depth_width: 1920,
            depth_height: 1080,
            depth_format: 45,
            depth_samples: 1,
            depth_quality: 0,
        };
        let steps = [
            Xmap10PresentationRecurrenceStep {
                replay_frame_identity_sha256: sha("seek-frame-0"),
                frame_delta_seconds_f32_bits: (1.0f32 / 60.0).to_bits(),
                recorded_speed_raw: 2,
                recorded_paused: false,
                day_night_enabled: true,
            },
            Xmap10PresentationRecurrenceStep {
                replay_frame_identity_sha256: sha("seek-frame-1"),
                frame_delta_seconds_f32_bits: (1.0f32 / 30.0).to_bits(),
                recorded_speed_raw: 4,
                recorded_paused: false,
                day_night_enabled: true,
            },
        ];

        let mut expected = Xmap10ViewerPresentationState::new().unwrap();
        let mut expected_patch = None;
        for step in &steps {
            expected_patch = Some(expected.advance_step(step, &main_scene).unwrap());
        }

        let mut stale = Xmap10ViewerPresentationState::new().unwrap();
        stale
            .advance_step(
                &Xmap10PresentationRecurrenceStep {
                    replay_frame_identity_sha256: sha("stale-pre-seek-frame"),
                    frame_delta_seconds_f32_bits: 9.0f32.to_bits(),
                    recorded_speed_raw: 0,
                    recorded_paused: true,
                    day_night_enabled: true,
                },
                &main_scene,
            )
            .unwrap();
        let rebuilt_patch = stale.reconstruct_from_zero(&steps, &main_scene).unwrap();
        assert_eq!(stale, expected);
        assert_eq!(rebuilt_patch, expected_patch.unwrap());

        let mut invalid = steps.to_vec();
        invalid[1].frame_delta_seconds_f32_bits = f32::NAN.to_bits();
        assert!(stale.reconstruct_from_zero(&invalid, &main_scene).is_err());
    }

    fn sample_input() -> Xmap10SceneCompositionInput {
        let sequence = |start: u32, count: usize| -> Vec<f32> {
            (0..count)
                .map(|index| (start + index as u32) as f32)
                .collect()
        };
        Xmap10SceneCompositionInput {
            frame_identity: Xmap10SceneFrameIdentity::new(
                sha("content-manifest"),
                sha("product-modset"),
                sha("effective-vfs"),
                "1936.1.1".into(),
                144,
                5632,
                2048,
                1920,
                1080,
                sha("replay-frame"),
                sha("camera-view"),
                &[Xmap04MapCopyReceiptRow {
                    render_view_mask: 1,
                    map_copy_offset_f32_bits: 0.0_f32.to_bits(),
                }],
                1,
                sha("snow-mud-input"),
            )
            .unwrap(),
            expected_layout_sha256: xmap10_scene_cb0_layout_sha256(),
            camera: Xmap03CameraScenePatch {
                view_projection_matrix: sequence(1, 16).try_into().unwrap(),
                inverse_view_projection_matrix: sequence(17, 16).try_into().unwrap(),
                camera_position: [33.0, 34.0, 35.0],
                camera_look_at_direction: [36.0, 37.0, 38.0],
            },
            political_fow: Xmap05PoliticalFowScenePatch {
                gb_camera_distance_override_outline_cutoff: [39.0, 40.0, 41.0, 42.0],
                fow_opacity: 43.0,
                fow_time: 44.0,
                fow_fade_factor: 45.0,
                minimum_mesh_alpha: 46.0,
                gradient_border_texture_height: 47.0,
            },
            snow_mud: Xmap07SnowMudScenePatch {
                snow_mud_fade: 48.0,
            },
            lighting: Xmap09LightingScenePatch {
                virtual_sun_position: [48.0, 49.0, 50.0, 51.0],
                virtual_moon_position: [52.0, 53.0, 54.0, 55.0],
                second_virtual_sun_position: [56.0, 57.0, 58.0, 59.0],
                second_virtual_moon_position: [60.0, 61.0, 62.0, 63.0],
                day_night_hour_sun_direction: [65.0, 66.0, 67.0, 68.0],
                ambient_positive_x: [69.0, 70.0, 71.0],
                shadow_fade_factor: 72.0,
                ambient_negative_x: [73.0, 74.0, 75.0],
                ambient_positive_y: [76.0, 77.0, 78.0],
                ambient_negative_y: [79.0, 80.0, 81.0],
                ambient_positive_z: [82.0, 83.0, 84.0],
                ambient_negative_z: [85.0, 86.0, 87.0],
                cubemap_intensity: 88.0,
                sun_diffuse_intensity: [89.0, 90.0, 91.0, 92.0],
                moon_diffuse_intensity: [93.0, 94.0, 95.0, 96.0],
                sun_specular_intensity: 97.0,
            },
            presentation: Xmap10PresentationScenePatch {
                frame_sequence: 1,
                replay_frame_identity_sha256: sha("replay-frame"),
                prior_presentation_timing_receipt_sha256: sha("prior-presentation-timing"),
                hdr_enabled: false,
                effective_light_hdr_range: 64.0,
                application_elapsed_seconds: 98.0,
                day_night_gate_factor: 0.75,
                presentation_timing_receipt_sha256: sha("presentation-timing"),
            },
        }
    }

    fn read_f32(bytes: &[u8], offset: usize) -> f32 {
        f32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
    }

    #[test]
    fn ownership_is_contiguous_and_marks_only_real_padding_as_padding() {
        validate_coverage().unwrap();
        let padding: Vec<_> = XMAP10_SCENE_FIELD_COVERAGE
            .iter()
            .filter(|row| row.owner == Xmap10SceneFieldOwner::Padding)
            .map(|row| (row.offset, row.byte_len))
            .collect();
        assert_eq!(padding, vec![(348, 4), (408, 8)]);
    }

    #[test]
    fn composes_all_owned_fields_without_a_caller_byte_baseline() {
        let prepared = compose_xmap10_scene_cb0(sample_input()).unwrap();
        prepared.validate().unwrap();
        assert_eq!(prepared.bytes.len(), 416);
        assert_eq!(read_f32(&prepared.bytes, 0), 1.0);
        assert_eq!(read_f32(&prepared.bytes, 64), 17.0);
        assert_eq!(read_f32(&prepared.bytes, 208), 33.0);
        assert_eq!(read_f32(&prepared.bytes, 236), 98.0);
        assert_eq!(read_f32(&prepared.bytes, 240), 43.0);
        assert_eq!(read_f32(&prepared.bytes, 248), 48.0);
        assert_eq!(read_f32(&prepared.bytes, 252), 0.75);
        assert_eq!(read_f32(&prepared.bytes, 300), 45.0);
        assert_eq!(read_f32(&prepared.bytes, 316), 46.0);
        assert_eq!(read_f32(&prepared.bytes, 332), 1.0);
        assert_eq!(read_f32(&prepared.bytes, 400), 47.0);
        assert_eq!(read_f32(&prepared.bytes, 404), 97.0);
        assert_eq!(&prepared.bytes[348..352], &[0; 4]);
        assert_eq!(&prepared.bytes[408..416], &[0; 8]);
    }

    #[test]
    fn rejects_layout_identity_and_non_finite_inputs() {
        let mut wrong_layout = sample_input();
        wrong_layout.expected_layout_sha256 = sha("wrong-layout");
        assert!(compose_xmap10_scene_cb0(wrong_layout).is_err());

        let mut negative_hour = sample_input();
        negative_hour.frame_identity.hour_index = -1;
        assert!(compose_xmap10_scene_cb0(negative_hour).is_err());

        let mut unauthenticated_timing = sample_input();
        unauthenticated_timing
            .presentation
            .presentation_timing_receipt_sha256 = "not-a-receipt".into();
        assert!(compose_xmap10_scene_cb0(unauthenticated_timing).is_err());

        let mut non_finite = sample_input();
        non_finite.presentation.application_elapsed_seconds = f32::NAN;
        assert!(compose_xmap10_scene_cb0(non_finite).is_err());
    }

    #[test]
    fn frame_identity_consumes_xmap04_map_copy_order_without_rederiving_it() {
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
        let identity = Xmap10SceneFrameIdentity::new(
            sha("content"),
            sha("modset"),
            sha("vfs"),
            "1936.1.1".into(),
            144,
            5632,
            2048,
            1920,
            1080,
            sha("replay"),
            sha("camera"),
            &rows,
            2,
            sha("snow"),
        )
        .unwrap();
        assert_eq!(identity.render_view_mask, 2);
        assert_eq!(
            identity.map_copy_receipt_set_sha256,
            xmap04_map_copy_receipt_set_sha256(5632, &rows).unwrap()
        );
        let mut tampered = identity;
        tampered.map_copy_offset_f32_bits = (-5632.0f32).to_bits();
        assert!(tampered.validate().is_err());
    }

    #[test]
    fn receipt_detects_payload_or_identity_tampering() {
        let mut prepared = compose_xmap10_scene_cb0(sample_input()).unwrap();
        prepared.bytes[0] ^= 1;
        assert!(prepared.validate().is_err());

        let mut prepared = compose_xmap10_scene_cb0(sample_input()).unwrap();
        prepared.frame_identity.hour_index += 1;
        assert!(prepared.validate().is_err());
    }

    #[test]
    fn pre_particle_receipt_binds_scene_and_complete_main_scene_descriptors() {
        let prepared = compose_xmap10_scene_cb0(sample_input()).unwrap();
        let target = |attachment_ordinal, dxgi_format_u32| RenderTargetDesc {
            prepared_target: PreparedRenderTarget::OpaqueRenderTarget { attachment_ordinal },
            width: 1920,
            height: 1080,
            dxgi_format_u32,
            mip_level: 0,
            sample_count: 1,
            sample_quality: 0,
            persistent_content: None,
            exact_resource: None,
        };
        let receipt = Xmap10PreParticleSceneReceipt::seal(
            144,
            sha("content-manifest"),
            sha("product-modset"),
            sha("effective-vfs"),
            sha("xmap09-lighting"),
            &prepared,
            target(0, 10),
            target(1, 45),
        )
        .unwrap();
        receipt.validate().unwrap();
        assert_eq!(receipt.scene_cb0().stage_mask_u8, 0b0000_0011);
        assert_eq!(receipt.scene_cb0().slot, 0);
        assert_eq!(receipt.scene_cb0().raw_bytes, prepared.bytes);
        assert_eq!(
            receipt.color_target(),
            &PreparedRenderTarget::OpaqueRenderTarget {
                attachment_ordinal: 0
            }
        );
        assert_eq!(
            receipt.depth_target(),
            &PreparedRenderTarget::OpaqueRenderTarget {
                attachment_ordinal: 1
            }
        );

        let mut tampered = receipt;
        tampered.main_scene_depth.width = 1919;
        assert!(tampered.validate().is_err());
    }
}
