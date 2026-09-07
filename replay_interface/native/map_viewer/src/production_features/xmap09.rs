use crate::xmap09_light_census::{
    admit_light_census, Xmap09LightCensusCounts,
    Xmap09LightCensusRefusal, Xmap09LightCensusRows,
};

use crate::{
    build_shadow_frame, plan_s1_shadow_screen_chain, project_l03_light_depth,
    shadow_blur_constant_buffers_from_factor, FoundationFeatureContext, FoundationFeatureOutput,
    L03LightDepthProjection, L03LightDepthTarget, L03ProjectionRequest, L03SelectedLight,
    L03SelectedLightFrame, L03SemanticInputRow, L03TreeBatchInput, ProductionMapFeatureAdapter,
    Result, ShadowFrameInput, ShadowLightKind, ShadowScreenChainPlan, ShadowScreenPassKind,
    ShadowScreenPassPlan, ShadowScreenTarget, L03_LIGHT_DEPTH_DXGI_FORMAT, L03_LIGHT_DEPTH_HEIGHT,
    L03_LIGHT_DEPTH_MIP_LEVELS, L03_LIGHT_DEPTH_WIDTH, SHADOW_SCREEN_BIND_FLAGS,
    SHADOW_SCREEN_DXGI_FORMAT, SHADOW_SCREEN_MIP_LEVELS,
};
use portable_content_manifest::UsedContentKey;
use portable_render_stream::{FamilyDisposition, MapVisualFamily};
use serde::{Deserialize, Serialize};
use snapshot::sha256::Sha256;
use std::collections::BTreeSet;
use std::sync::Arc;

pub const XMAP09_INPUT_SCHEMA: &str = "hoi4.xmap09.semantic-lighting-input.a729.v4";
pub const XMAP09_PREPARED_SCHEMA: &str = "hoi4.xmap09.prepared-lighting-shadow.a729.v3";
pub const XMAP09_A729_EXECUTABLE_SHA256: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";
pub const XMAP09_SCENE_LIGHT_WRITER_RVA: u32 = 0x0126_7270;
pub const XMAP09_SHADOW_FIT_RVA: u32 = 0x022c_4f30;
pub const XMAP09_SHADOW_CHAIN_RVA: u32 = 0x022c_5d30;
pub const XMAP09_SHADOW_BLUR_FACTOR_RVA: u32 = 0x022c_6bc0;
pub const XMAP09_SCENE_CB0_PATCH_SCHEMA: &str = "hoi4.xmap09.scene-cb0-field-patch.a729.v1";
pub const XMAP09_SCENE_CB0_APPLY_SCHEMA: &str = "hoi4.xmap09.scene-cb0-field-apply.a729.v1";
pub const XMAP09_SCENE_CB0_BYTE_LEN: u32 = 416;
pub const XMAP09_POINT_LIGHT_CB2_SYMBOL: &str = "PointLightsCB2";
pub const XMAP09_POINT_LIGHT_CB2_SHADER_REGISTER: u32 = 2;
pub const XMAP09_LIGHT_INDEX_MAP_SYMBOL: &str = "LightIndexMap";
pub const XMAP09_LIGHT_DATA_MAP_SYMBOL: &str = "LightDataMap";
pub const XMAP09_SHADOW_MAP_SYMBOL: &str = "ShadowMap";
pub const XMAP09_PREPARE_STAGE_NAME: &str = "xmap09.lighting_shadow.prepare";
pub const XMAP09_PREPARE_OPERATION_NAME: &str = "lighting_shadow.prepare";
pub const XMAP09_EXECUTE_STAGE_NAME: &str = "xmap09.shadow_map.execute";
pub const XMAP09_EXECUTE_OPERATION_NAME: &str = "shadow_map.depth.draw";

pub fn xmap09_assigned_stage_topology() -> Result<crate::AssignedStageTopology> {
    let prepare =
        crate::AssignedStageIdentity::new(XMAP09_PREPARE_STAGE_NAME, XMAP09_PREPARE_OPERATION_NAME);
    let execute =
        crate::AssignedStageIdentity::new(XMAP09_EXECUTE_STAGE_NAME, XMAP09_EXECUTE_OPERATION_NAME);
    crate::AssignedStageTopology::seal(
        vec![prepare.clone(), execute.clone()],
        vec![crate::AssignedStageEdge::new(prepare, execute)],
    )
    .map_err(Into::into)
}

const GAME_HOUR_RAW_EPOCH: i64 = 43_799_999;
pub(crate) const LIGHT_INDEX_WIDTH: usize = 64;
pub(crate) const LIGHT_INDEX_HEIGHT: usize = 64;
pub(crate) const LIGHT_DATA_TEXELS: usize = 128;
const LIGHT_CAPACITY: usize = LIGHT_DATA_TEXELS / 2;
pub(crate) const DXGI_FORMAT_R32G32B32A32_FLOAT: u32 = 2;
pub(crate) const DXGI_FORMAT_R8G8B8A8_UNORM: u32 = 28;
pub(crate) const D3D11_BIND_SHADER_RESOURCE: u32 = 0x8;
const D3D11_BIND_DEPTH_STENCIL: u32 = 0x40;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap09PointLight {
    pub stable_light_id: String,
    pub global_order: u32,
    pub position_radius_f32_bits: [u32; 4],
    pub color_falloff_f32_bits: [u32; 4],
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap09LightingInput {
    pub schema: String,
    pub executable_sha256: String,
    pub scene_light_writer_rva: u32,
    pub product_modset_sha256: String,
    pub effective_vfs_manifest_sha256: String,
    pub game_hour_raw: i32,
    pub virtual_sun_position_f32_bits: [u32; 4],
    pub virtual_moon_position_f32_bits: [u32; 4],
    pub second_virtual_sun_position_f32_bits: [u32; 4],
    pub second_virtual_moon_position_f32_bits: [u32; 4],
    pub day_night_hour_sun_dir_f32_bits: [u32; 4],
    pub sun_diffuse_intensity_f32_bits: [u32; 4],
    pub moon_diffuse_intensity_f32_bits: [u32; 4],
    pub ambient_rig_f32_bits: [[u32; 3]; 6],
    pub sun_specular_intensity_f32_bits: u32,
    pub cubemap_intensity_f32_bits: u32,
    pub draw_shadows_cutoff_f32_bits: u32,
    pub draw_shadows_fade_length_f32_bits: u32,
    pub shadow_pass_active: bool,
    pub point_light_grid_start_inv_cell_size_f32_bits: [u32; 4],
    pub point_lights: Vec<Xmap09PointLight>,
    pub point_light_cell_indices: Vec<[u8; 4]>,
    /// Map objects XMAP12 placed earlier in this same frame. `None` is XMAP12
    /// not vouching this hour; `Some(0)` is a measured-empty placement.
    pub placed_map_object_count: Option<u32>,
    pub point_light_census: std::result::Result<Xmap09LightCensusCounts, Xmap09LightCensusRefusal>,
    pub point_light_census_sha256: String,
    pub source_observation_sha256: String,
    pub payload_sha256: String,
}

impl Xmap09LightingInput {
    pub fn computed_point_light_census_sha256(&self) -> String {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(
            &serde_json::to_vec(&self.point_light_census).expect("census serializes"),
        );
        for light in &self.point_lights {
            bytes.extend_from_slice(light.stable_light_id.as_bytes());
            bytes.extend_from_slice(&light.global_order.to_le_bytes());
            for lane in light
                .position_radius_f32_bits
                .iter()
                .chain(light.color_falloff_f32_bits.iter())
            {
                bytes.extend_from_slice(&lane.to_le_bytes());
            }
        }
        for cell in &self.point_light_cell_indices {
            bytes.extend_from_slice(cell);
        }
        Sha256::hex(&bytes)
    }

    pub fn computed_payload_sha256(&self) -> Result<String> {
        let mut unsigned = self.clone();
        unsigned.payload_sha256.clear();
        Ok(Sha256::hex(&serde_json::to_vec(&unsigned)?))
    }

    pub fn seal(mut self) -> Result<Self> {
        self.point_light_census_sha256 = self.computed_point_light_census_sha256();
        self.payload_sha256 = self.computed_payload_sha256()?;
        self.validate_for_hour(
            i64::from(self.game_hour_raw)
                .checked_sub(GAME_HOUR_RAW_EPOCH)
                .ok_or("XMAP09 game-hour conversion overflowed")?,
        )?;
        Ok(self)
    }

    pub fn validate_for_hour(&self, hour_index: i64) -> Result<()> {
        if self.schema != XMAP09_INPUT_SCHEMA
            || self.executable_sha256 != XMAP09_A729_EXECUTABLE_SHA256
            || self.scene_light_writer_rva != XMAP09_SCENE_LIGHT_WRITER_RVA
        {
            return Err("XMAP09 semantic lighting producer identity mismatch".into());
        }
        validate_sha256(&self.source_observation_sha256, "source observation")?;
        validate_sha256(&self.product_modset_sha256, "product modset")?;
        validate_sha256(
            &self.effective_vfs_manifest_sha256,
            "effective VFS manifest",
        )?;
        validate_sha256(&self.point_light_census_sha256, "point-light census")?;
        validate_sha256(&self.payload_sha256, "lighting payload")?;
        let input_hour = i64::from(self.game_hour_raw)
            .checked_sub(GAME_HOUR_RAW_EPOCH)
            .ok_or("XMAP09 game-hour conversion overflowed")?;
        if input_hour != hour_index {
            return Err(format!(
                "XMAP09 semantic lighting hour {input_hour} does not match replay hour {hour_index}"
            )
            .into());
        }
        for (label, lanes) in [
            ("virtual sun", self.virtual_sun_position_f32_bits.as_slice()),
            (
                "virtual moon",
                self.virtual_moon_position_f32_bits.as_slice(),
            ),
            (
                "second virtual sun",
                self.second_virtual_sun_position_f32_bits.as_slice(),
            ),
            (
                "second virtual moon",
                self.second_virtual_moon_position_f32_bits.as_slice(),
            ),
            (
                "day/night driver",
                self.day_night_hour_sun_dir_f32_bits.as_slice(),
            ),
            (
                "sun diffuse",
                self.sun_diffuse_intensity_f32_bits.as_slice(),
            ),
            (
                "moon diffuse",
                self.moon_diffuse_intensity_f32_bits.as_slice(),
            ),
            (
                "point-light grid transform",
                self.point_light_grid_start_inv_cell_size_f32_bits
                    .as_slice(),
            ),
        ] {
            validate_finite_bits(lanes, label)?;
        }
        for (index, row) in self.ambient_rig_f32_bits.iter().enumerate() {
            validate_finite_bits(row, &format!("ambient rig row {index}"))?;
        }
        validate_finite_bits(
            &[
                self.sun_specular_intensity_f32_bits,
                self.cubemap_intensity_f32_bits,
                self.draw_shadows_cutoff_f32_bits,
                self.draw_shadows_fade_length_f32_bits,
            ],
            "lighting scalar",
        )?;
        if f32::from_bits(self.draw_shadows_cutoff_f32_bits) <= 0.0
            || f32::from_bits(self.draw_shadows_fade_length_f32_bits) <= 0.0
        {
            return Err("XMAP09 effective shadow cutoff/fade length must be positive".into());
        }
        let sun_dir = self.day_night_hour_sun_dir_f32_bits[1..]
            .iter()
            .map(|bits| f32::from_bits(*bits))
            .collect::<Vec<_>>();
        let length_sq = sun_dir.iter().map(|value| value * value).sum::<f32>();
        if !length_sq.is_finite() || (length_sq.sqrt() - 1.0).abs() > 1.0e-6 {
            return Err(
                "XMAP09 date-specific sun direction is not the normalized engine output".into(),
            );
        }
        let hour_fraction = f32::from_bits(self.day_night_hour_sun_dir_f32_bits[0]);
        if !(0.0..1.0).contains(&hour_fraction) {
            return Err("XMAP09 day/night hour fraction is outside [0,1)".into());
        }
        if self.point_light_grid_start_inv_cell_size_f32_bits[2..]
            .iter()
            .any(|bits| f32::from_bits(*bits) <= 0.0)
        {
            return Err("XMAP09 point-light inverse cell sizes must be positive".into());
        }
        match &self.point_light_census {
            Ok(counts)
                if counts.is_consistent() && counts.uploaded_lights == self.point_lights.len() => {}
            Err(_)
                if self.point_lights.is_empty()
                    && self
                        .point_light_cell_indices
                        .iter()
                        .all(|cell| *cell == [u8::MAX; 4]) => {}
            _ => return Err("XMAP09 light census counts or refusal disagree with the grid".into()),
        }
        if self.point_lights.len() > LIGHT_CAPACITY {
            return Err(
                format!("XMAP09 point-light census exceeds {LIGHT_CAPACITY} texture rows").into(),
            );
        }
        if self.point_light_cell_indices.len() != LIGHT_INDEX_WIDTH * LIGHT_INDEX_HEIGHT {
            return Err(format!(
                "XMAP09 point-light census must contain {} cells",
                LIGHT_INDEX_WIDTH * LIGHT_INDEX_HEIGHT
            )
            .into());
        }
        let mut ids = BTreeSet::new();
        for (index, light) in self.point_lights.iter().enumerate() {
            if light.stable_light_id.trim().is_empty()
                || !ids.insert(light.stable_light_id.as_str())
                || light.global_order != index as u32
            {
                return Err(
                    "XMAP09 point lights are blank, duplicated, or outside exact global order"
                        .into(),
                );
            }
            validate_finite_bits(
                &light.position_radius_f32_bits,
                &format!("point light {index} position/radius"),
            )?;
            validate_finite_bits(
                &light.color_falloff_f32_bits,
                &format!("point light {index} color/falloff"),
            )?;
        }
        for (cell_index, cell) in self.point_light_cell_indices.iter().enumerate() {
            let mut saw_sentinel = false;
            let mut previous = None;
            for channel in crate::xmap09_lighting_law::XMAP09_LIGHT_CELL_CHANNEL_ORDER {
                let slot = &cell[channel];
                if *slot == u8::MAX {
                    saw_sentinel = true;
                } else if saw_sentinel
                    || usize::from(*slot) >= self.point_lights.len()
                    || previous.is_some_and(|last| last >= *slot)
                {
                    return Err(format!(
                        "XMAP09 point-light cell {cell_index} has invalid ordered indices"
                    )
                    .into());
                }
                previous = Some(*slot);
            }
        }
        if self.point_light_census_sha256 != self.computed_point_light_census_sha256() {
            return Err("XMAP09 point-light census digest mismatch".into());
        }
        if self.payload_sha256 != self.computed_payload_sha256()? {
            return Err("XMAP09 semantic lighting payload digest mismatch".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Xmap09TextureResource {
    pub stable_resource_id: String,
    pub width: u32,
    pub height: u32,
    pub dxgi_format: u32,
    pub mip_levels: u32,
    pub bind_flags: u32,
    pub row_pitch: u32,
    pub bytes: Arc<[u8]>,
    pub payload_sha256: String,
    pub descriptor_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Xmap09SceneConstantPatch {
    pub game_hour_raw: i32,
    pub day_night_hour_sun_dir_f32_bits: [u32; 4],
    pub virtual_sun_position_f32_bits: [u32; 4],
    pub virtual_moon_position_f32_bits: [u32; 4],
    pub second_virtual_sun_position_f32_bits: [u32; 4],
    pub second_virtual_moon_position_f32_bits: [u32; 4],
    pub sun_diffuse_intensity_f32_bits: [u32; 4],
    pub moon_diffuse_intensity_f32_bits: [u32; 4],
    pub ambient_rig_f32_bits: [[u32; 3]; 6],
    pub sun_specular_intensity_f32_bits: u32,
    pub cubemap_intensity_f32_bits: u32,
    pub shadow_fade_factor_f32_bits: u32,
    pub point_light_grid_start_inv_cell_size_f32_bits: [u32; 4],
    pub downstream_scene_constant_owners: [&'static str; 4],
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap09ReflectedSceneCb0Field {
    pub symbol: String,
    pub byte_offset: u32,
    pub byte_len: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap09ReflectedSceneCb0Layout {
    pub byte_len: u32,
    pub ordered_fields: Vec<Xmap09ReflectedSceneCb0Field>,
    pub deterministic_sha256: String,
}

impl Xmap09ReflectedSceneCb0Layout {
    pub fn seal(byte_len: u32, ordered_fields: Vec<Xmap09ReflectedSceneCb0Field>) -> Result<Self> {
        let mut result = Self {
            byte_len,
            ordered_fields,
            deterministic_sha256: String::new(),
        };
        result.deterministic_sha256 = result.computed_sha256()?;
        result.validate()?;
        Ok(result)
    }

    pub fn validate(&self) -> Result<()> {
        if self.byte_len != XMAP09_SCENE_CB0_BYTE_LEN {
            return Err(format!(
                "XMAP09 reflected SceneCB0 is {} bytes, expected {}",
                self.byte_len, XMAP09_SCENE_CB0_BYTE_LEN
            )
            .into());
        }
        validate_sha256(&self.deterministic_sha256, "reflected SceneCB0 layout")?;
        let mut symbols = BTreeSet::new();
        let mut previous_end = 0u32;
        for field in &self.ordered_fields {
            if field.symbol.trim().is_empty()
                || field.byte_len == 0
                || !symbols.insert(field.symbol.as_str())
            {
                return Err("XMAP09 reflected SceneCB0 has a blank/duplicate field".into());
            }
            let end = field
                .byte_offset
                .checked_add(field.byte_len)
                .ok_or("XMAP09 reflected SceneCB0 field range overflowed")?;
            if field.byte_offset < previous_end || end > self.byte_len {
                return Err(
                    "XMAP09 reflected SceneCB0 fields overlap, are unordered, or exceed CB0".into(),
                );
            }
            previous_end = end;
            if let Some((expected_offset, expected_len)) =
                xmap09_scene_cb0_field_contract(&field.symbol)
            {
                if field.byte_offset != expected_offset || field.byte_len != expected_len {
                    return Err(format!(
                        "XMAP09 reflected SceneCB0 field {} moved from {expected_offset}+{expected_len} to {}+{}",
                        field.symbol, field.byte_offset, field.byte_len
                    )
                    .into());
                }
            }
        }
        if self.computed_sha256()? != self.deterministic_sha256 {
            return Err("XMAP09 reflected SceneCB0 layout digest mismatch".into());
        }
        Ok(())
    }

    fn computed_sha256(&self) -> Result<String> {
        let mut unsigned = self.clone();
        unsigned.deterministic_sha256.clear();
        Ok(Sha256::hex(&serde_json::to_vec(&unsigned)?))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap09SceneCb0FieldPatch {
    pub symbol: String,
    pub byte_offset: u32,
    pub bytes: Vec<u8>,
    pub payload_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap09SceneCb0Patch {
    pub schema: String,
    pub owner_packet: String,
    pub input_payload_sha256: String,
    pub replay_frame_identity_sha256: String,
    pub ordered_fields: Vec<Xmap09SceneCb0FieldPatch>,
    pub deterministic_sha256: String,
}

impl Xmap09SceneCb0Patch {
    pub fn validate(&self) -> Result<()> {
        if self.schema != XMAP09_SCENE_CB0_PATCH_SCHEMA || self.owner_packet != "XMAP09" {
            return Err("XMAP09 SceneCB0 patch identity mismatch".into());
        }
        validate_sha256(&self.input_payload_sha256, "SceneCB0 patch input")?;
        validate_sha256(
            &self.replay_frame_identity_sha256,
            "SceneCB0 patch replay frame",
        )?;
        validate_sha256(&self.deterministic_sha256, "SceneCB0 patch")?;
        let mut previous_end = 0u32;
        let mut symbols = BTreeSet::new();
        for field in &self.ordered_fields {
            let (expected_offset, expected_len) = xmap09_scene_cb0_field_contract(&field.symbol)
                .ok_or_else(|| {
                    format!(
                        "XMAP09 SceneCB0 patch contains unknown field {}",
                        field.symbol
                    )
                })?;
            let byte_len = u32::try_from(field.bytes.len())?;
            if field.byte_offset != expected_offset
                || byte_len != expected_len
                || field.byte_offset < previous_end
                || !symbols.insert(field.symbol.as_str())
                || field.payload_sha256 != Sha256::hex(&field.bytes)
            {
                return Err(format!(
                    "XMAP09 SceneCB0 patch field {} has invalid offset/bytes/order/digest",
                    field.symbol
                )
                .into());
            }
            previous_end = field.byte_offset + byte_len;
        }
        if self.ordered_fields.len() != XMAP09_SCENE_CB0_FIELD_COUNT {
            return Err(
                "XMAP09 SceneCB0 patch does not contain its complete owned field set".into(),
            );
        }
        if self.computed_sha256()? != self.deterministic_sha256 {
            return Err("XMAP09 SceneCB0 patch digest mismatch".into());
        }
        Ok(())
    }

    pub fn apply_to_reflected_buffer(
        &self,
        base: &[u8],
        layout: &Xmap09ReflectedSceneCb0Layout,
    ) -> Result<Xmap09ComposedSceneCb0> {
        self.validate()?;
        layout.validate()?;
        if base.len() != layout.byte_len as usize {
            return Err("XMAP09 SceneCB0 base bytes do not match reflected byte length".into());
        }
        let mut bytes = base.to_vec();
        let mut applied_symbols = Vec::new();
        for reflected in &layout.ordered_fields {
            let Some(patch) = self
                .ordered_fields
                .iter()
                .find(|field| field.symbol == reflected.symbol)
            else {
                continue;
            };
            let start = reflected.byte_offset as usize;
            let end = start + reflected.byte_len as usize;
            bytes[start..end].copy_from_slice(&patch.bytes);
            applied_symbols.push(reflected.symbol.clone());
        }
        if applied_symbols.is_empty() {
            return Err("XMAP09 SceneCB0 reflection consumes no XMAP09-owned fields".into());
        }
        let base_sha256 = Sha256::hex(base);
        let final_payload_sha256 = Sha256::hex(&bytes);
        let descriptor_sha256 = Sha256::hex(
            format!(
                "{}\0{}\0{}\0{}\0{}",
                self.replay_frame_identity_sha256,
                layout.deterministic_sha256,
                layout.byte_len,
                self.deterministic_sha256,
                applied_symbols.join("\0")
            )
            .as_bytes(),
        );
        let deterministic_sha256 = Sha256::hex(
            format!(
                "{XMAP09_SCENE_CB0_APPLY_SCHEMA}\0XMAP09\0{}\0{}\0{}\0{}\0{}\0{}",
                self.replay_frame_identity_sha256,
                layout.deterministic_sha256,
                base_sha256,
                self.deterministic_sha256,
                final_payload_sha256,
                descriptor_sha256
            )
            .as_bytes(),
        );
        Ok(Xmap09ComposedSceneCb0 {
            bytes: bytes.into(),
            receipt: Xmap09SceneCb0ApplyReceipt {
                schema: XMAP09_SCENE_CB0_APPLY_SCHEMA.into(),
                contributor_packet: "XMAP09".into(),
                replay_frame_identity_sha256: self.replay_frame_identity_sha256.clone(),
                reflected_layout_sha256: layout.deterministic_sha256.clone(),
                base_payload_sha256: base_sha256,
                patch_sha256: self.deterministic_sha256.clone(),
                final_payload_sha256,
                descriptor_sha256,
                applied_symbols,
                deterministic_sha256,
            },
        })
    }

    fn computed_sha256(&self) -> Result<String> {
        let mut unsigned = self.clone();
        unsigned.deterministic_sha256.clear();
        Ok(Sha256::hex(&serde_json::to_vec(&unsigned)?))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap09SceneCb0ApplyReceipt {
    pub schema: String,
    pub contributor_packet: String,
    pub replay_frame_identity_sha256: String,
    pub reflected_layout_sha256: String,
    pub base_payload_sha256: String,
    pub patch_sha256: String,
    pub final_payload_sha256: String,
    pub descriptor_sha256: String,
    pub applied_symbols: Vec<String>,
    pub deterministic_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Xmap09ComposedSceneCb0 {
    pub bytes: Arc<[u8]>,
    pub receipt: Xmap09SceneCb0ApplyReceipt,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap09ShadowMatrixBinding {
    pub symbol: String,
    pub owner_packet: String,
    pub input_payload_sha256: String,
    pub shadow_map_texture_matrix_f32_bits: [u32; 16],
    pub payload_sha256: String,
    pub descriptor_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Xmap09ShadowResourcePlan {
    pub wrapped_fit_basis_f32_bits: [[u32; 3]; 4],
    pub map_width_f32_bits: u32,
    pub selected_light_frame: L03SelectedLightFrame,
    pub light_depth_bind_flags: u32,
    pub shadow_pass_active: bool,
    pub blur_camera_factor_f32_bits: u32,
    pub producer_and_final_target: ShadowScreenTarget,
    pub blur_intermediate_target: ShadowScreenTarget,
    pub screen_passes: Vec<ShadowScreenPassPlan>,
    pub exact_caster_owner_packets: [&'static str; 5],
    pub exact_caster_global_order_required: bool,
    pub final_consumer_symbol: &'static str,
}

fn partition_shadow_texture_matrix(plan: &Xmap09ShadowResourcePlan, enabled_mask: u8, partition: u8)
    -> Result<[u32; 16]> {
    if enabled_mask & 1 == 0 || enabled_mask & !7 != 0 || !matches!(partition, 1 | 2 | 4)
        || enabled_mask & partition == 0 {
        return Err("XMAP09 shadow matrix partition is outside the selected render views".into());
    }
    if partition == 1 { return Ok(plan.selected_light_frame.shadow_map_texture_matrix_f32_bits); }
    let [mut eye, mut center, scale, translate] = plan.wrapped_fit_basis_f32_bits.map(|row| row.map(f32::from_bits));
    let width = f32::from_bits(plan.map_width_f32_bits);
    if !width.is_finite() || width <= 0.0 { return Err("XMAP09 wrapped shadow map width is invalid".into()); }
    if partition == 2 {
        eye[0] -= width;
        center[0] -= width;
    } else {
        if enabled_mask & 2 != 0 {
            eye[0] = (eye[0] - width) + width;
            center[0] = (center[0] - width) + width;
        }
        eye[0] += width;
        center[0] += width;
    }
    let view = crate::shadow_law::look_at_row_vector_a729(eye, center)?;
    let projection = [[scale[0], 0.0, 0.0, 0.0], [0.0, scale[1], 0.0, 0.0],
        [0.0, 0.0, scale[2], 0.0], [translate[0], translate[1], translate[2], 1.0]];
    let multiply = |left: [[f32; 4]; 4], right: [[f32; 4]; 4]| -> [[f32; 4]; 4] {
        std::array::from_fn(|row| std::array::from_fn(|col|
            left[row][1] * right[1][col] + left[row][0] * right[0][col]
                + left[row][2] * right[2][col] + left[row][3] * right[3][col]))
    };
    let projected = multiply(view, projection);
    let texture = multiply(projected, [[0.5, 0.0, 0.0, 0.0], [0.0, -0.5, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0], [0.5, 0.5, 0.0, 1.0]]);
    if texture.iter().flatten().any(|value| !value.is_finite()) {
        return Err("XMAP09 wrapped shadow matrix is non-finite".into());
    }
    Ok(std::array::from_fn(|lane| texture[lane / 4][lane % 4].to_bits()))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedXmap09LightingFrame {
    pub schema: String,
    pub input_payload_sha256: String,
    pub replay_frame_identity_sha256: String,
    pub camera_frame_sha256: String,
    pub prepare_stage: crate::AssignedStage,
    pub shadow_execute_stage: crate::AssignedStage,
    pub scene_constants: Xmap09SceneConstantPatch,
    pub point_light_census: std::result::Result<Xmap09LightCensusCounts, Xmap09LightCensusRefusal>,
    pub light_index_map: Xmap09TextureResource,
    pub light_data_map: Xmap09TextureResource,
    pub shadow: Xmap09ShadowResourcePlan,
    pub deterministic_sha256: String,
}

impl PreparedXmap09LightingFrame {
    pub fn validate(&self) -> Result<()> {
        if self.schema != XMAP09_PREPARED_SCHEMA {
            return Err("XMAP09 prepared frame schema mismatch".into());
        }
        validate_sha256(&self.input_payload_sha256, "prepared input payload")?;
        validate_sha256(&self.replay_frame_identity_sha256, "prepared replay frame")?;
        validate_sha256(&self.camera_frame_sha256, "prepared camera frame")?;
        validate_sha256(&self.deterministic_sha256, "prepared frame")?;
        self.prepare_stage
            .validate()
            .map_err(|error| format!("XMAP09 prepare stage: {error}"))?;
        self.shadow_execute_stage
            .validate()
            .map_err(|error| format!("XMAP09 shadow execute stage: {error}"))?;
        if self.prepare_stage.stage_name != XMAP09_PREPARE_STAGE_NAME
            || self.prepare_stage.operation_name != XMAP09_PREPARE_OPERATION_NAME
            || self.shadow_execute_stage.stage_name != XMAP09_EXECUTE_STAGE_NAME
            || self.shadow_execute_stage.operation_name != XMAP09_EXECUTE_OPERATION_NAME
            || self.prepare_stage.topology_sha256 != self.shadow_execute_stage.topology_sha256
            || self.prepare_stage.assigned_ordinal >= self.shadow_execute_stage.assigned_ordinal
        {
            return Err(
                "XMAP09 assigned-stage identities/order are not the promoted contract".into(),
            );
        }
        validate_texture(
            &self.light_index_map,
            "LightIndexMap",
            LIGHT_INDEX_WIDTH as u32,
            LIGHT_INDEX_HEIGHT as u32,
            DXGI_FORMAT_R8G8B8A8_UNORM,
            (LIGHT_INDEX_WIDTH * 4) as u32,
        )?;
        validate_texture(
            &self.light_data_map,
            "LightDataMap",
            LIGHT_DATA_TEXELS as u32,
            1,
            DXGI_FORMAT_R32G32B32A32_FLOAT,
            (LIGHT_DATA_TEXELS * 16) as u32,
        )?;
        match &self.point_light_census {
            Ok(counts)
                if counts.is_consistent()
                    && self.light_index_map.bytes.iter().all(|slot| {
                        *slot == u8::MAX || usize::from(*slot) < counts.uploaded_lights
                    }) => {}
            Err(_)
                if self
                    .light_index_map
                    .bytes
                    .iter()
                    .all(|slot| *slot == u8::MAX)
                    && self.light_data_map.bytes.iter().all(|byte| *byte == 0) => {}
            _ => return Err("XMAP09 prepared light census disagrees with its textures".into()),
        }
        let target = &self.shadow.selected_light_frame.light_depth_target;
        if target.width != L03_LIGHT_DEPTH_WIDTH
            || target.height != L03_LIGHT_DEPTH_HEIGHT
            || target.dxgi_format != L03_LIGHT_DEPTH_DXGI_FORMAT
            || target.mip_levels != L03_LIGHT_DEPTH_MIP_LEVELS
            || self.shadow.light_depth_bind_flags
                != D3D11_BIND_DEPTH_STENCIL | D3D11_BIND_SHADER_RESOURCE
        {
            return Err("XMAP09 light-depth target descriptor mismatch".into());
        }
        if self.shadow.producer_and_final_target.width != self.shadow.blur_intermediate_target.width
            || self.shadow.producer_and_final_target.height
                != self.shadow.blur_intermediate_target.height
            || self.shadow.producer_and_final_target.stable_resource_id
                == self.shadow.blur_intermediate_target.stable_resource_id
        {
            return Err("XMAP09 screen-shadow ping-pong targets are invalid".into());
        }
        let blur = shadow_blur_constant_buffers_from_factor(
            f32::from_bits(self.shadow.blur_camera_factor_f32_bits),
            self.shadow.producer_and_final_target.width,
            self.shadow.producer_and_final_target.height,
        )?;
        let expected_passes = vec![
            ShadowScreenPassPlan {
                kind: ShadowScreenPassKind::TerrainUnlitProducer,
                source_resource_id: target.stable_resource_id.clone(),
                target_resource_id: self
                    .shadow
                    .producer_and_final_target
                    .stable_resource_id
                    .clone(),
                cb0_f32_bits: None,
            },
            ShadowScreenPassPlan {
                kind: ShadowScreenPassKind::HorizontalBlur,
                source_resource_id: self
                    .shadow
                    .producer_and_final_target
                    .stable_resource_id
                    .clone(),
                target_resource_id: self
                    .shadow
                    .blur_intermediate_target
                    .stable_resource_id
                    .clone(),
                cb0_f32_bits: Some(blur.horizontal_f32_bits),
            },
            ShadowScreenPassPlan {
                kind: ShadowScreenPassKind::VerticalBlur,
                source_resource_id: self
                    .shadow
                    .blur_intermediate_target
                    .stable_resource_id
                    .clone(),
                target_resource_id: self
                    .shadow
                    .producer_and_final_target
                    .stable_resource_id
                    .clone(),
                cb0_f32_bits: Some(blur.vertical_f32_bits),
            },
        ];
        if self.shadow.screen_passes != expected_passes {
            return Err("XMAP09 screen-shadow pass order or blur constants mismatch".into());
        }
        if self.deterministic_sha256 != prepared_digest(self)? {
            return Err("XMAP09 prepared frame digest mismatch".into());
        }
        Ok(())
    }

    /// Consumers retain ownership of the base buffer and all non-XMAP09 bytes.
    pub fn scene_cb0_patch(&self) -> Result<Xmap09SceneCb0Patch> {
        self.validate()?;
        let mut ordered_fields = Vec::with_capacity(XMAP09_SCENE_CB0_FIELD_COUNT);
        let mut push = |symbol: &str, byte_offset: u32, bytes: Vec<u8>| {
            let payload_sha256 = Sha256::hex(&bytes);
            ordered_fields.push(Xmap09SceneCb0FieldPatch {
                symbol: symbol.into(),
                byte_offset,
                bytes,
                payload_sha256,
            });
        };
        let lanes = |values: &[u32]| {
            values
                .iter()
                .flat_map(|bits| bits.to_le_bytes())
                .collect::<Vec<_>>()
        };
        push(
            "vVirtualSunPos",
            144,
            lanes(&self.scene_constants.virtual_sun_position_f32_bits),
        );
        push(
            "vVirtualMoonPos",
            160,
            lanes(&self.scene_constants.virtual_moon_position_f32_bits),
        );
        push(
            "vSecondVirtualSunPos",
            176,
            lanes(&self.scene_constants.second_virtual_sun_position_f32_bits),
        );
        push(
            "vSecondVirtualMoonPos",
            192,
            lanes(&self.scene_constants.second_virtual_moon_position_f32_bits),
        );
        push(
            "DayNight_Hour_SunDir",
            256,
            lanes(&self.scene_constants.day_night_hour_sun_dir_f32_bits),
        );
        for (symbol, byte_offset, row) in [
            (
                "AmbientPosX",
                272,
                self.scene_constants.ambient_rig_f32_bits[0],
            ),
            (
                "AmbientNegX",
                288,
                self.scene_constants.ambient_rig_f32_bits[1],
            ),
            (
                "AmbientPosY",
                304,
                self.scene_constants.ambient_rig_f32_bits[2],
            ),
            (
                "AmbientNegY",
                320,
                self.scene_constants.ambient_rig_f32_bits[3],
            ),
            (
                "AmbientPosZ",
                336,
                self.scene_constants.ambient_rig_f32_bits[4],
            ),
            (
                "AmbientNegZ",
                352,
                self.scene_constants.ambient_rig_f32_bits[5],
            ),
        ] {
            push(symbol, byte_offset, lanes(&row));
        }
        push(
            "ShadowFadeFactor",
            284,
            self.scene_constants
                .shadow_fade_factor_f32_bits
                .to_le_bytes()
                .to_vec(),
        );
        push(
            "CubemapIntensity",
            364,
            self.scene_constants
                .cubemap_intensity_f32_bits
                .to_le_bytes()
                .to_vec(),
        );
        push(
            "SunDiffuseIntensity",
            368,
            lanes(&self.scene_constants.sun_diffuse_intensity_f32_bits),
        );
        push(
            "MoonDiffuseIntensity",
            384,
            lanes(&self.scene_constants.moon_diffuse_intensity_f32_bits),
        );
        push(
            "SunSpecularIntensity",
            404,
            self.scene_constants
                .sun_specular_intensity_f32_bits
                .to_le_bytes()
                .to_vec(),
        );
        ordered_fields.sort_by_key(|field| field.byte_offset);
        let mut result = Xmap09SceneCb0Patch {
            schema: XMAP09_SCENE_CB0_PATCH_SCHEMA.into(),
            owner_packet: "XMAP09".into(),
            input_payload_sha256: self.input_payload_sha256.clone(),
            replay_frame_identity_sha256: self.replay_frame_identity_sha256.clone(),
            ordered_fields,
            deterministic_sha256: String::new(),
        };
        result.deterministic_sha256 = result.computed_sha256()?;
        result.validate()?;
        Ok(result)
    }

    pub fn point_lights_cb2_bytes(&self) -> Result<[u8; 16]> {
        self.validate()?;
        let mut bytes = [0u8; 16];
        for (lane, bits) in self
            .scene_constants
            .point_light_grid_start_inv_cell_size_f32_bits
            .iter()
            .enumerate()
        {
            bytes[lane * 4..lane * 4 + 4].copy_from_slice(&bits.to_le_bytes());
        }
        Ok(bytes)
    }

    pub fn shadow_map_texture_matrix(&self) -> Result<Xmap09ShadowMatrixBinding> {
        self.shadow_map_texture_matrix_for_partition(1, 1)
    }

    pub fn shadow_map_texture_matrix_for_partition(&self, enabled_mask: u8, partition: u8)
        -> Result<Xmap09ShadowMatrixBinding> {
        self.validate()?;
        let bits = partition_shadow_texture_matrix(&self.shadow, enabled_mask, partition)?;
        let bytes = bits
            .iter()
            .flat_map(|lane| lane.to_le_bytes())
            .collect::<Vec<_>>();
        Ok(Xmap09ShadowMatrixBinding {
            symbol: "ShadowMapTextureMatrix".into(),
            owner_packet: "XMAP09".into(),
            input_payload_sha256: self.input_payload_sha256.clone(),
            payload_sha256: Sha256::hex(&bytes),
            descriptor_sha256: Sha256::hex(b"XMAP09\0ShadowMapTextureMatrix\0float4x4\064"),
            shadow_map_texture_matrix_f32_bits: bits,
        })
    }

    pub fn project_exact_casters(
        &self,
        enabled_partition_mask: u8,
        semantic_rows: Vec<L03SemanticInputRow>,
        tree_batches: Vec<L03TreeBatchInput>,
    ) -> Result<L03LightDepthProjection> {
        self.validate()?;
        project_l03_light_depth(&L03ProjectionRequest {
            shadow_pass_active: Some(self.shadow.shadow_pass_active),
            selected_light_frame: Some(self.shadow.selected_light_frame.clone()),
            enabled_partition_mask: Some(enabled_partition_mask),
            semantic_rows: Some(semantic_rows),
            tree_batches: Some(tree_batches),
        })
    }

    pub fn plan_shadow_consumer(
        &self,
        projection: &L03LightDepthProjection,
    ) -> Result<Option<ShadowScreenChainPlan>> {
        self.validate()?;
        if self.shadow.shadow_pass_active == projection.inactive_excluded {
            return Err(
                "XMAP09 projection active state does not belong to the current lighting frame"
                    .into(),
            );
        }
        if !projection.inactive_excluded
            && (projection.selected_light != Some(self.shadow.selected_light_frame.selected_light)
                || projection.light_depth_target.as_ref()
                    != Some(&self.shadow.selected_light_frame.light_depth_target))
        {
            return Err(
                "XMAP09 projection selected-light/target ancestry differs from the current lighting frame"
                    .into(),
            );
        }
        plan_s1_shadow_screen_chain(
            projection,
            Some(self.shadow.blur_camera_factor_f32_bits),
            Some(self.shadow.producer_and_final_target.clone()),
            Some(self.shadow.blur_intermediate_target.clone()),
        )
    }
}

const XMAP09_SCENE_CB0_FIELD_COUNT: usize = 16;

fn xmap09_scene_cb0_field_contract(symbol: &str) -> Option<(u32, u32)> {
    match symbol {
        "vVirtualSunPos" => Some((144, 16)),
        "vVirtualMoonPos" => Some((160, 16)),
        "vSecondVirtualSunPos" => Some((176, 16)),
        "vSecondVirtualMoonPos" => Some((192, 16)),
        "DayNight_Hour_SunDir" => Some((256, 16)),
        "AmbientPosX" => Some((272, 12)),
        "ShadowFadeFactor" => Some((284, 4)),
        "AmbientNegX" => Some((288, 12)),
        "AmbientPosY" => Some((304, 12)),
        "AmbientNegY" => Some((320, 12)),
        "AmbientPosZ" => Some((336, 12)),
        "AmbientNegZ" => Some((352, 12)),
        "CubemapIntensity" => Some((364, 4)),
        "SunDiffuseIntensity" => Some((368, 16)),
        "MoonDiffuseIntensity" => Some((384, 16)),
        "SunSpecularIntensity" => Some((404, 4)),
        _ => None,
    }
}

pub(crate) struct Xmap09LightingAdapter {
    resolver: crate::GameFileResolver,
    lifecycle: std::sync::Mutex<crate::xmap09_light_lifecycle::ViewerLightLifecycle>,
    pending_entities: std::sync::Mutex<Option<crate::xmap09_light_lifecycle::PreparedLightEntityFrame>>,
    derived_entities: std::sync::Mutex<Option<std::result::Result<crate::xmap09_light_lifecycle::DerivedLightRows, Xmap09LightCensusRefusal>>>,
    particle_visibility: std::sync::Mutex<Option<(u32, std::collections::BTreeMap<String, [bool; 3]>)>>,
    defines: crate::Xmap09LightingDefines,
    ordered_used_content: Vec<UsedContentKey>,
    defines_source_sha256: String,
}

impl Xmap09LightingAdapter {
    pub(crate) fn new(resolver: &crate::GameFileResolver) -> Result<Self> {
        Self::with_entity_manager(resolver, std::sync::Arc::new(std::sync::Mutex::new(
            crate::particle_runtime::ViewerEntityManager::default(),
        )))
    }

    pub(crate) fn with_entity_manager(
        resolver: &crate::GameFileResolver,
        entity_manager: std::sync::Arc<std::sync::Mutex<crate::particle_runtime::ViewerEntityManager>>,
    ) -> Result<Self> {
        let (sources, ordered_used_content) =
            crate::xmap09_lighting_law::admitted_effective_define_sources(
                resolver,
                "XMAP09 lighting defines",
            )?;
        let mut digest_bytes = Vec::new();
        for content in &ordered_used_content {
            digest_bytes.extend_from_slice(content.normalized_game_relative_path.as_bytes());
            digest_bytes.push(0);
            digest_bytes.extend_from_slice(content.payload_sha256.as_bytes());
            digest_bytes.push(0);
        }
        let defines = crate::Xmap09LightingDefines::from_effective_sources(&sources)?;
        Ok(Self {
            resolver: resolver.clone(),
            lifecycle: std::sync::Mutex::new(crate::xmap09_light_lifecycle::ViewerLightLifecycle::with_entity_manager(entity_manager)),
            pending_entities: std::sync::Mutex::new(None),
            particle_visibility: std::sync::Mutex::new(None),
            derived_entities: std::sync::Mutex::new(None),
            defines,
            ordered_used_content,
            defines_source_sha256: Sha256::hex(&digest_bytes),
        })
    }

    fn derive_lighting_input(
        &self,
        hour_index: i64,
        product_modset_sha256: &str,
        effective_vfs_manifest_sha256: &str,
        camera: &crate::ViewerMapCameraState,
        placed_map_object_count: Option<u32>,
        light_rows: std::result::Result<Xmap09LightCensusRows<'_>, Xmap09LightCensusRefusal>,
    ) -> Result<Xmap09LightingInput> {
        let game_hour_raw = hour_index
            .checked_add(GAME_HOUR_RAW_EPOCH)
            .and_then(|raw| i32::try_from(raw).ok())
            .ok_or("XMAP09 replay hour does not fit the engine game-hour stamp")?;
        let hour_fraction = crate::engine_hour_of_day_fraction(game_hour_raw)?;
        let map_width = camera.map_width as f32;
        let map_height = camera.map_height as f32;
        let rig = crate::virtual_rig(
            hour_fraction,
            crate::XMAP09_REPLAY_DAY_NIGHT_FACTOR,
            map_width,
            map_height,
            &self.defines,
        )?;
        let sun_direction = xmap09_sun_direction(camera, &rig)?;
        let bits4 = |lanes: [f32; 4]| -> [u32; 4] { std::array::from_fn(|i| lanes[i].to_bits()) };
        let camera_height = camera.current_eye()[1];
        let shadow_pass_active = crate::xmap09_shadow_fade_factor(
            camera_height,
            self.defines.draw_shadows_cutoff,
            self.defines.draw_shadows_fade_length,
        )? > 0.0;
        let ambient = self.defines.ambient_rig();
        let grid = crate::xmap09_tiled_point_light_grid(
            camera.current_eye(),
            camera.current_look_at_direction()?,
            map_width,
            map_height,
        )?;
        let (point_light_census, point_lights, point_light_cell_indices) =
            match admit_light_census(light_rows, grid) {
                Ok(census) => (Ok(census.counts), census.lights, census.cells),
                Err(refusal) => (
                    Err(refusal),
                    Vec::new(),
                    vec![[u8::MAX; 4]; LIGHT_INDEX_WIDTH * LIGHT_INDEX_HEIGHT],
                ),
            };
        let input = Xmap09LightingInput {
            schema: XMAP09_INPUT_SCHEMA.into(),
            executable_sha256: XMAP09_A729_EXECUTABLE_SHA256.into(),
            scene_light_writer_rva: XMAP09_SCENE_LIGHT_WRITER_RVA,
            product_modset_sha256: product_modset_sha256.to_string(),
            effective_vfs_manifest_sha256: effective_vfs_manifest_sha256.to_string(),
            game_hour_raw,
            virtual_sun_position_f32_bits: bits4(rig.virtual_sun),
            virtual_moon_position_f32_bits: bits4(rig.virtual_moon),
            second_virtual_sun_position_f32_bits: bits4(rig.second_virtual_sun),
            second_virtual_moon_position_f32_bits: bits4(rig.second_virtual_moon),
            day_night_hour_sun_dir_f32_bits: bits4([
                hour_fraction,
                sun_direction[0],
                sun_direction[1],
                sun_direction[2],
            ]),
            sun_diffuse_intensity_f32_bits: bits4(self.defines.sun_diffuse_intensity()),
            moon_diffuse_intensity_f32_bits: bits4(self.defines.moon_diffuse_intensity()),
            ambient_rig_f32_bits: std::array::from_fn(|row| {
                std::array::from_fn(|lane| ambient[row][lane].to_bits())
            }),
            sun_specular_intensity_f32_bits: self.defines.sun_specular_intensity.to_bits(),
            cubemap_intensity_f32_bits: self.defines.cubemap_intensity.to_bits(),
            draw_shadows_cutoff_f32_bits: self.defines.draw_shadows_cutoff.to_bits(),
            draw_shadows_fade_length_f32_bits: self.defines.draw_shadows_fade_length.to_bits(),
            shadow_pass_active,
            point_light_grid_start_inv_cell_size_f32_bits: bits4(grid),
            point_light_census,
            point_lights,
            point_light_cell_indices,
            placed_map_object_count,
            point_light_census_sha256: String::new(),
            source_observation_sha256: Sha256::hex(
                format!(
                    "{XMAP09_INPUT_SCHEMA}\0{}\0{}\0{}\0{}\0{}",
                    self.defines_source_sha256,
                    camera.deterministic_sha256,
                    effective_vfs_manifest_sha256,
                    hour_index,
                    match placed_map_object_count {
                        Some(count) => format!("placed:{count}"),
                        None => "placed:unvouched".to_string(),
                    }
                )
                .as_bytes(),
            ),
            payload_sha256: String::new(),
        };
        input.seal()
    }
}

struct PreparedLightingOwner<'a> {
    adapter: &'a Xmap09LightingAdapter,
    lifecycle: std::sync::MutexGuard<'a, crate::xmap09_light_lifecycle::ViewerLightLifecycle>,
    prepared: Option<crate::xmap09_light_lifecycle::PreparedLightEntityFrame>,
    frame: crate::particle_runtime::EntityManagerFrame,
}

impl crate::production_foundation::PreparedEntityManagerOwner for PreparedLightingOwner<'_> {
    fn manager(&self) -> std::sync::Arc<std::sync::Mutex<crate::particle_runtime::ViewerEntityManager>> {
        self.lifecycle.entity_manager()
    }
    fn owner(&self) -> crate::particle_runtime::EntityManagerOwner { self.lifecycle.manager_owner() }
    fn frame(&self) -> crate::particle_runtime::EntityManagerFrame { self.frame }
    fn advance_root(&mut self, root: &str, _scene: u32,
        random: &mut crate::particle_runtime::ViewerParticleSeed) -> Result<()> {
        self.lifecycle.advance_prepared_root(&self.adapter.resolver,
            self.prepared.as_ref().ok_or("entity_manager.light_owner.already_finished")?, root, random)
    }
    fn advance_effect(&mut self, id: &str, phase: crate::particle_runtime::EntityEffectPhase) -> Result<()> {
        self.lifecycle.advance_prepared_effect(&self.adapter.resolver,
            self.prepared.as_ref().ok_or("entity_manager.light_owner.already_finished")?, id, phase)
    }
    fn finish(&mut self) -> Result<()> {
        let prepared = self.prepared.take().ok_or("entity_manager.light_owner.already_finished")?;
        let result = self.lifecycle.finish_entity_frame(&self.adapter.resolver, prepared);
        let outcome = result.as_ref().map(|_| ()).map_err(|error| Box::new(error.clone()) as Box<dyn std::error::Error>);
        *self.adapter.derived_entities.lock().map_err(|_| "entity_manager.light_result.lock_poisoned")? = Some(result);
        outcome
    }
    fn refuse(&mut self, reason: &str) {
        self.lifecycle.refuse_entity_publication(reason);
        if let Ok(mut result) = self.adapter.derived_entities.lock() {
            *result = Some(Err(Xmap09LightCensusRefusal::MissingDomain { domain: reason.into() }));
        }
    }
}

impl ProductionMapFeatureAdapter for Xmap09LightingAdapter {
    fn prepare_entity_manager_population(&self, context: &FoundationFeatureContext<'_>) -> Result<()> {
        let mut visibility = std::collections::BTreeMap::<String, [bool; 3]>::new();
        if let Some(frame) = context.prepared_map_objects {
            for draw in &frame.ordered_draws {
                let row = visibility.entry(draw.stable_typed_id.clone()).or_insert([false; 3]);
                for (index, bit) in [crate::L03_PARTITION_PRIMARY, crate::L03_PARTITION_WRAP_WEST,
                    crate::L03_PARTITION_WRAP_EAST].into_iter().enumerate() {
                    row[index] |= draw.partition_mask & bit != 0;
                }
            }
        }
        *self.particle_visibility.lock().map_err(|_| "PARTICLE_DRAW visibility lock poisoned")? =
            Some((context.viewer.animation_time_f32_bits, visibility));
        *self.derived_entities.lock().map_err(|_| "entity_manager.light_result.lock_poisoned")? = None;
        let mut pending = self.pending_entities.lock().map_err(|_| "entity_manager.light_pending.lock_poisoned")?;
        if pending.is_some() { return Err("entity_manager.light_owner.not_dispatched".into()); }
        let population = context.prepared_map_objects
            .ok_or_else(|| Xmap09LightCensusRefusal::MissingDomain { domain: "XMAP12.placed_entities".into() })
            .and_then(|frame| frame.placed_entities.as_ref().map_err(Clone::clone));
        let mut lifecycle = self.lifecycle.lock().map_err(|_| "XMAP09 viewer lifecycle lock is poisoned")?;
        match population {
            Ok(rows) => match lifecycle.prepare_entity_frame(&self.resolver, rows, context.viewer, context.prepared_weather) {
                Ok(prepared) => {
                    let mut visibility = self.particle_visibility.lock()
                        .map_err(|_| "PARTICLE_DRAW visibility lock poisoned")?;
                    let (_, roots) = visibility.as_mut().ok_or("PARTICLE_DRAW visibility unavailable")?;
                    match lifecycle.extend_particle_only_visibility(&prepared, context.viewer.camera.as_ref(), roots) {
                        Ok(()) => *pending = Some(prepared),
                        Err(error) => {
                            lifecycle.refuse_entity_publication(&error.to_string());
                            *self.derived_entities.lock().map_err(|_| "entity_manager.light_result.lock_poisoned")? =
                                Some(Err(Xmap09LightCensusRefusal::MissingDomain { domain: error.to_string() }));
                        }
                    }
                },
                Err(refusal) => *self.derived_entities.lock().map_err(|_| "entity_manager.light_result.lock_poisoned")? = Some(Err(refusal)),
            },
            Err(refusal) => {
                lifecycle.reset_population();
                *self.derived_entities.lock().map_err(|_| "entity_manager.light_result.lock_poisoned")? = Some(Err(refusal));
            }
        }
        Ok(())
    }

    fn take_prepared_entity_manager_owner(&self) -> Result<Option<Box<dyn crate::production_foundation::PreparedEntityManagerOwner + '_>>> {
        let prepared = self.pending_entities.lock().map_err(|_| "entity_manager.light_pending.lock_poisoned")?.take();
        let Some(prepared) = prepared else { return Ok(None); };
        let frame = prepared.frame;
        Ok(Some(Box::new(PreparedLightingOwner { adapter: self,
            lifecycle: self.lifecycle.lock().map_err(|_| "XMAP09 viewer lifecycle lock is poisoned")?,
            prepared: Some(prepared), frame })))
    }

    fn weather_particles_bound(&self, time: f32, identity: &str) -> Result<bool> {
        Ok(self.lifecycle.lock().map_err(|_| "XMAP09 viewer lifecycle lock is poisoned")?
            .weather_particles_bound(time, identity))
    }

    fn take_retained_particle_draw_owner(
        &self, time: f32, cameras: [crate::particle_runtime::ParticleSortCamera; 3],
    ) -> Result<Option<Box<dyn crate::production_foundation::RetainedParticleDrawOwner + '_>>> {
        let visibility = self.particle_visibility.lock().map_err(|_| "PARTICLE_DRAW visibility lock poisoned")?;
        let (published_time, roots) = visibility.as_ref().ok_or("PARTICLE_DRAW visibility unavailable")?;
        if *published_time != time.to_bits() { return Err("PARTICLE_DRAW visibility belongs to another frame".into()); }
        let roots = roots.clone();
        drop(visibility);
        Ok(Some(Box::new(crate::xmap09_light_lifecycle::RetainedLifecycleParticleOwner::new(
            self.lifecycle.lock().map_err(|_| "PARTICLE_DRAW lifecycle lock poisoned")?, time, cameras, roots,
        )?)))
    }

    fn family(&self) -> MapVisualFamily {
        MapVisualFamily::LightingShadows
    }

    fn prepare(&self, context: &FoundationFeatureContext<'_>) -> Result<FoundationFeatureOutput> {
        let consumed_gap_ids = xmap09_gap_ids(context);
        let camera = context
            .viewer
            .camera
            .as_ref()
            .ok_or("XMAP09 lighting/shadows require the XMAP03 viewer camera")?;
        camera.validate()?;
        let population = context.prepared_map_objects
            .ok_or_else(|| Xmap09LightCensusRefusal::MissingDomain { domain: "XMAP12.placed_entities".into() })
            .and_then(|frame| frame.placed_entities.as_ref().map_err(Clone::clone));
        let placed_count = population.as_ref().ok().map(|rows| u32::try_from(rows.len()))
            .transpose().map_err(|_| "XMAP09 placed population counter overflow")?;
        let derived = self.derived_entities.lock().map_err(|_| "entity_manager.light_result.lock_poisoned")?
            .take().ok_or("entity_manager.light_owner.not_prepared")?;
        let input = match derived {
            Ok(rows) => rows.with_rows(|rows| self.derive_lighting_input(
                context.world_frame.hour_index(), context.product_modset_sha256,
                context.effective_vfs_manifest_sha256, camera, placed_count, Ok(rows),
            ))?,
            Err(refusal) => self.derive_lighting_input(
                context.world_frame.hour_index(), context.product_modset_sha256,
                context.effective_vfs_manifest_sha256, camera, placed_count, Err(refusal),
            )?,
        };
        let prepare_stage = context
            .require_assigned_stage(XMAP09_PREPARE_STAGE_NAME, XMAP09_PREPARE_OPERATION_NAME)?;
        let shadow_execute_stage = context
            .require_assigned_stage(XMAP09_EXECUTE_STAGE_NAME, XMAP09_EXECUTE_OPERATION_NAME)?;
        let prepared = prepare_lighting(
            &input,
            camera,
            crate::xmap04_replay_frame_identity_sha256(context.world_frame)?,
            prepare_stage.clone(),
            shadow_execute_stage.clone(),
        )?;
        prepared.validate()?;

        let registration = Sha256::hex(
            format!(
                "XMAP09\0{XMAP09_PREPARED_SCHEMA}\0{XMAP09_A729_EXECUTABLE_SHA256}\0{XMAP09_SCENE_LIGHT_WRITER_RVA:08x}\0{XMAP09_SHADOW_FIT_RVA:08x}\0{XMAP09_SHADOW_CHAIN_RVA:08x}\0{XMAP09_SHADOW_BLUR_FACTOR_RVA:08x}"
            )
            .as_bytes(),
        );
        let resource_order = Sha256::hex(
            &[
                prepared.light_index_map.descriptor_sha256.as_bytes(),
                prepared.light_data_map.descriptor_sha256.as_bytes(),
                prepared
                    .shadow
                    .selected_light_frame
                    .light_depth_target
                    .stable_resource_id
                    .as_bytes(),
                prepared
                    .shadow
                    .producer_and_final_target
                    .stable_resource_id
                    .as_bytes(),
                prepared
                    .shadow
                    .blur_intermediate_target
                    .stable_resource_id
                    .as_bytes(),
            ]
            .concat(),
        );
        let target_sha256 =
            Sha256::hex(format!("{}x{}", camera.viewport_width, camera.viewport_height).as_bytes());
        let semantic_suboperations_sha256 = Sha256::hex(&serde_json::to_vec(&[
            "lighting.scene_constants.prepare",
            "lighting.point_light_resources.materialize",
            "shadow.camera_matrices.prepare",
            "shadow.light_depth_contract.prepare",
            "shadow.screen_blur_contract.prepare",
        ])?);
        let operations = vec![prepare_stage
            .foundation_operation_identity(
                registration.clone(),
                Sha256::hex(
                    format!("{semantic_suboperations_sha256}\0{resource_order}").as_bytes(),
                ),
                target_sha256,
            )
            .map_err(|error| format!("XMAP09 prepare assignment: {error}"))?];
        Ok(FoundationFeatureOutput {
            disposition: FamilyDisposition::Present {
                payload_schema: XMAP09_PREPARED_SCHEMA.into(),
                producer_registration_sha256: registration,
                payload_sha256: prepared.deterministic_sha256.clone(),
                ordered_operations: operations,
                ordered_used_content: self.ordered_used_content.clone(),
            },
            prepared_terrain: None,
            prepared_political: None,
            prepared_thin_borders: None,
            prepared_rivers_straits: None,
            prepared_water: None,
            prepared_lighting: Some(prepared),
            prepared_rail_supply: None,
            prepared_weather: None,
            prepared_units: None,
            prepared_combat: None,
            prepared_orders: None,
            prepared_map_modes: None,
            consumed_gap_ids,
            element_diagnostics: Vec::new(),
        })
    }
}

fn xmap09_sun_direction(
    camera: &crate::ViewerMapCameraState,
    rig: &crate::Xmap09VirtualRig,
) -> Result<[f32; 3]> {
    let look = camera.current_look_at();
    let selected = crate::select_shadow_light(&ShadowFrameInput {
        map_focus: [look[0], look[2]],
        camera_x: camera.current_eye()[0],
        map_width: camera.map_width as f32,
        primary_sun: [rig.virtual_sun[0], rig.virtual_sun[1], rig.virtual_sun[2]],
        primary_moon: [
            rig.virtual_moon[0],
            rig.virtual_moon[1],
            rig.virtual_moon[2],
        ],
        fit_center: [look[0], 0.0, look[2]],
        fit_points: [[look[0], 0.0, look[2]]; 4],
    })?;
    Ok(selected.direction)
}

fn xmap09_gap_ids(context: &FoundationFeatureContext<'_>) -> Vec<String> {
    context
        .map_authority_gaps
        .iter()
        .filter(|gap| {
            gap.input_id == "lighting.point_light_lifecycle"
                || gap.input_id == "world.calendar_time"
        })
        .map(|gap| gap.input_id.clone())
        .collect()
}

fn prepare_lighting(
    input: &Xmap09LightingInput,
    camera: &crate::ViewerMapCameraState,
    replay_frame_identity_sha256: String,
    prepare_stage: crate::AssignedStage,
    shadow_execute_stage: crate::AssignedStage,
) -> Result<PreparedXmap09LightingFrame> {
    let product_camera = camera.production_camera()?;
    validate_sha256(
        &replay_frame_identity_sha256,
        "canonical XMAP04 replay frame",
    )?;
    let corners = [
        (0.0, 0.0),
        (camera.viewport_width as f32, 0.0),
        (0.0, camera.viewport_height as f32),
        (camera.viewport_width as f32, camera.viewport_height as f32),
    ];
    let mut fit_points = [[0.0; 3]; 4];
    for (index, (screen_x, screen_y)) in corners.into_iter().enumerate() {
        let (x, z) = product_camera
            .screen_to_ground_y0(
                screen_x,
                screen_y,
                camera.viewport_width as f32,
                camera.viewport_height as f32,
            )?
            .ok_or("XMAP09 shadow camera corner misses the map plane")?;
        fit_points[index] = [x, 0.0, z];
    }
    let look = camera.current_look_at();
    let input_frame = ShadowFrameInput {
        map_focus: [look[0], look[2]],
        camera_x: camera.current_eye()[0],
        map_width: camera.map_width as f32,
        primary_sun: std::array::from_fn(|lane| {
            f32::from_bits(input.virtual_sun_position_f32_bits[lane])
        }),
        primary_moon: std::array::from_fn(|lane| {
            f32::from_bits(input.virtual_moon_position_f32_bits[lane])
        }),
        fit_center: [look[0], 0.0, look[2]],
        fit_points,
    };
    let matrices = build_shadow_frame(&input_frame)?;
    let selected_light = match matrices.selected_light.kind {
        ShadowLightKind::Sun => L03SelectedLight::Sun,
        ShadowLightKind::Moon => L03SelectedLight::Moon,
    };
    let matrix_bits = |matrix: [[f32; 4]; 4]| -> [u32; 16] {
        std::array::from_fn(|lane| matrix[lane / 4][lane % 4].to_bits())
    };
    let light_depth_id = format!(
        "xmap09/{}/{}/light-depth",
        input.payload_sha256, camera.deterministic_sha256
    );
    let selected_light_frame = L03SelectedLightFrame {
        selected_light,
        light_view_projection_f32_bits: matrix_bits(matrices.light_view_projection),
        shadow_map_texture_matrix_f32_bits: matrix_bits(matrices.shadow_map_texture_matrix),
        light_depth_target: L03LightDepthTarget {
            stable_resource_id: light_depth_id,
            width: L03_LIGHT_DEPTH_WIDTH,
            height: L03_LIGHT_DEPTH_HEIGHT,
            dxgi_format: L03_LIGHT_DEPTH_DXGI_FORMAT,
            mip_levels: L03_LIGHT_DEPTH_MIP_LEVELS,
        },
    };
    let screen_target = |suffix: &str| ShadowScreenTarget {
        stable_resource_id: format!(
            "xmap09/{}/{}/{suffix}/{}x{}",
            input.payload_sha256,
            camera.deterministic_sha256,
            camera.viewport_width,
            camera.viewport_height
        ),
        width: camera.viewport_width,
        height: camera.viewport_height,
        dxgi_format: SHADOW_SCREEN_DXGI_FORMAT,
        mip_levels: SHADOW_SCREEN_MIP_LEVELS,
        bind_flags: SHADOW_SCREEN_BIND_FLAGS,
    };
    let light_index_bytes = input
        .point_light_cell_indices
        .iter()
        .flat_map(|cell| {
            crate::xmap09_lighting_law::XMAP09_LIGHT_CELL_CHANNEL_ORDER.map(|channel| cell[channel])
        })
        .collect::<Vec<_>>();
    let mut light_data_bytes = Vec::with_capacity(LIGHT_DATA_TEXELS * 16);
    for light in &input.point_lights {
        for bits in light
            .position_radius_f32_bits
            .iter()
            .chain(light.color_falloff_f32_bits.iter())
        {
            light_data_bytes.extend_from_slice(&bits.to_le_bytes());
        }
    }
    light_data_bytes.resize(LIGHT_DATA_TEXELS * 16, 0);
    let scene_constants = Xmap09SceneConstantPatch {
        game_hour_raw: input.game_hour_raw,
        day_night_hour_sun_dir_f32_bits: input.day_night_hour_sun_dir_f32_bits,
        virtual_sun_position_f32_bits: input.virtual_sun_position_f32_bits,
        virtual_moon_position_f32_bits: input.virtual_moon_position_f32_bits,
        second_virtual_sun_position_f32_bits: input.second_virtual_sun_position_f32_bits,
        second_virtual_moon_position_f32_bits: input.second_virtual_moon_position_f32_bits,
        sun_diffuse_intensity_f32_bits: input.sun_diffuse_intensity_f32_bits,
        moon_diffuse_intensity_f32_bits: input.moon_diffuse_intensity_f32_bits,
        ambient_rig_f32_bits: input.ambient_rig_f32_bits,
        sun_specular_intensity_f32_bits: input.sun_specular_intensity_f32_bits,
        cubemap_intensity_f32_bits: input.cubemap_intensity_f32_bits,
        shadow_fade_factor_f32_bits: ((f32::from_bits(input.draw_shadows_cutoff_f32_bits)
            - camera.current_eye()[1])
            / f32::from_bits(input.draw_shadows_fade_length_f32_bits))
        .clamp(0.0, 1.0)
        .to_bits(),
        point_light_grid_start_inv_cell_size_f32_bits: input
            .point_light_grid_start_inv_cell_size_f32_bits,
        downstream_scene_constant_owners: ["XMAP05", "XMAP07", "XMAP09", "XMAP10"],
    };
    let light_index_map = texture_resource(
        format!("xmap09/{}/LightIndexMap", input.payload_sha256),
        LIGHT_INDEX_WIDTH as u32,
        LIGHT_INDEX_HEIGHT as u32,
        DXGI_FORMAT_R8G8B8A8_UNORM,
        (LIGHT_INDEX_WIDTH * 4) as u32,
        light_index_bytes,
    );
    let light_data_map = texture_resource(
        format!("xmap09/{}/LightDataMap", input.payload_sha256),
        LIGHT_DATA_TEXELS as u32,
        1,
        DXGI_FORMAT_R32G32B32A32_FLOAT,
        (LIGHT_DATA_TEXELS * 16) as u32,
        light_data_bytes,
    );
    let producer_and_final_target = screen_target("shadow-final");
    let blur_intermediate_target = screen_target("shadow-blur-intermediate");
    let blur_camera_factor = (camera.current_eye()[1] / 700.0).min(1.0);
    let blur = shadow_blur_constant_buffers_from_factor(
        blur_camera_factor,
        camera.viewport_width,
        camera.viewport_height,
    )?;
    let screen_passes = vec![
        ShadowScreenPassPlan {
            kind: ShadowScreenPassKind::TerrainUnlitProducer,
            source_resource_id: selected_light_frame
                .light_depth_target
                .stable_resource_id
                .clone(),
            target_resource_id: producer_and_final_target.stable_resource_id.clone(),
            cb0_f32_bits: None,
        },
        ShadowScreenPassPlan {
            kind: ShadowScreenPassKind::HorizontalBlur,
            source_resource_id: producer_and_final_target.stable_resource_id.clone(),
            target_resource_id: blur_intermediate_target.stable_resource_id.clone(),
            cb0_f32_bits: Some(blur.horizontal_f32_bits),
        },
        ShadowScreenPassPlan {
            kind: ShadowScreenPassKind::VerticalBlur,
            source_resource_id: blur_intermediate_target.stable_resource_id.clone(),
            target_resource_id: producer_and_final_target.stable_resource_id.clone(),
            cb0_f32_bits: Some(blur.vertical_f32_bits),
        },
    ];
    let (_, _, wrapped_fit_basis) = crate::shadow_law::fit_shadow_map_basis(
        input_frame.fit_points, input_frame.fit_center, matrices.selected_light.direction)?;
    let shadow = Xmap09ShadowResourcePlan {
        wrapped_fit_basis_f32_bits: wrapped_fit_basis.map(|row| row.map(f32::to_bits)),
        map_width_f32_bits: input_frame.map_width.to_bits(),
        selected_light_frame,
        light_depth_bind_flags: D3D11_BIND_DEPTH_STENCIL | D3D11_BIND_SHADER_RESOURCE,
        shadow_pass_active: input.shadow_pass_active,
        blur_camera_factor_f32_bits: blur_camera_factor.to_bits(),
        producer_and_final_target,
        blur_intermediate_target,
        screen_passes,
        exact_caster_owner_packets: ["XMAP12", "XMAP13", "XMAP11", "XMAP15", "XMAP16"],
        exact_caster_global_order_required: true,
        final_consumer_symbol: "ShadowMap",
    };
    let mut prepared = PreparedXmap09LightingFrame {
        schema: XMAP09_PREPARED_SCHEMA.into(),
        input_payload_sha256: input.payload_sha256.clone(),
        replay_frame_identity_sha256,
        camera_frame_sha256: camera.deterministic_sha256.clone(),
        prepare_stage,
        shadow_execute_stage,
        scene_constants,
        point_light_census: input.point_light_census.clone(),
        light_index_map,
        light_data_map,
        shadow,
        deterministic_sha256: String::new(),
    };
    prepared.deterministic_sha256 = prepared_digest(&prepared)?;
    Ok(prepared)
}

fn texture_resource(
    stable_resource_id: String,
    width: u32,
    height: u32,
    dxgi_format: u32,
    row_pitch: u32,
    bytes: Vec<u8>,
) -> Xmap09TextureResource {
    let payload_sha256 = Sha256::hex(&bytes);
    let descriptor_sha256 = texture_descriptor_digest(
        &stable_resource_id,
        width,
        height,
        dxgi_format,
        row_pitch,
        &payload_sha256,
    );
    Xmap09TextureResource {
        stable_resource_id,
        width,
        height,
        dxgi_format,
        mip_levels: 1,
        bind_flags: D3D11_BIND_SHADER_RESOURCE,
        row_pitch,
        bytes: bytes.into(),
        payload_sha256,
        descriptor_sha256,
    }
}

fn validate_texture(
    resource: &Xmap09TextureResource,
    symbol: &str,
    width: u32,
    height: u32,
    format: u32,
    row_pitch: u32,
) -> Result<()> {
    if resource.stable_resource_id.trim().is_empty()
        || resource.width != width
        || resource.height != height
        || resource.dxgi_format != format
        || resource.mip_levels != 1
        || resource.bind_flags != D3D11_BIND_SHADER_RESOURCE
        || resource.row_pitch != row_pitch
        || resource.bytes.len() != row_pitch as usize * height as usize
    {
        return Err(format!("XMAP09 {symbol} descriptor/payload mismatch").into());
    }
    if resource.payload_sha256 != Sha256::hex(&resource.bytes) {
        return Err(format!("XMAP09 {symbol} payload digest mismatch").into());
    }
    if resource.descriptor_sha256
        != texture_descriptor_digest(
            &resource.stable_resource_id,
            resource.width,
            resource.height,
            resource.dxgi_format,
            resource.row_pitch,
            &resource.payload_sha256,
        )
    {
        return Err(format!("XMAP09 {symbol} descriptor digest mismatch").into());
    }
    Ok(())
}

fn texture_descriptor_digest(
    stable_resource_id: &str,
    width: u32,
    height: u32,
    dxgi_format: u32,
    row_pitch: u32,
    payload_sha256: &str,
) -> String {
    Sha256::hex(
        format!(
            "{stable_resource_id}\0{width}\0{height}\0{dxgi_format}\01\0{D3D11_BIND_SHADER_RESOURCE}\0{row_pitch}\0{payload_sha256}"
        )
        .as_bytes(),
    )
}

fn prepared_digest(prepared: &PreparedXmap09LightingFrame) -> Result<String> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(prepared.schema.as_bytes());
    bytes.extend_from_slice(&serde_json::to_vec(&prepared.point_light_census)?);
    bytes.extend_from_slice(prepared.input_payload_sha256.as_bytes());
    bytes.extend_from_slice(prepared.replay_frame_identity_sha256.as_bytes());
    bytes.extend_from_slice(prepared.camera_frame_sha256.as_bytes());
    bytes.extend_from_slice(&serde_json::to_vec(&prepared.prepare_stage)?);
    bytes.extend_from_slice(&serde_json::to_vec(&prepared.shadow_execute_stage)?);
    bytes.extend_from_slice(&serde_json::to_vec(&prepared.scene_constants)?);
    bytes.extend_from_slice(prepared.light_index_map.descriptor_sha256.as_bytes());
    bytes.extend_from_slice(prepared.light_data_map.descriptor_sha256.as_bytes());
    bytes.extend_from_slice(&serde_json::to_vec(&prepared.shadow)?);
    Ok(Sha256::hex(&bytes))
}

fn validate_finite_bits(bits: &[u32], label: &str) -> Result<()> {
    if bits.iter().any(|bits| !f32::from_bits(*bits).is_finite()) {
        return Err(format!("XMAP09 {label} contains non-finite binary32").into());
    }
    Ok(())
}

fn validate_sha256(value: &str, label: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!("XMAP09 {label} is not canonical lowercase SHA-256").into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        L03DrawArguments, L03MaterialFamily, L03PdxMeshGroupInput, L03PdxMeshObjectInput,
        L03PdxMeshSubmeshInput, L03TypedOwner, L03_PARTITION_PRIMARY,
    };
    use portable_content_manifest::{ContentLayerKind, UsedContentKey};

    fn assigned_stages() -> (crate::AssignedStage, crate::AssignedStage) {
        let topology = "a".repeat(64);
        (
            crate::AssignedStage::seal(
                topology.clone(),
                XMAP09_PREPARE_STAGE_NAME,
                XMAP09_PREPARE_OPERATION_NAME,
                1,
            )
            .unwrap(),
            crate::AssignedStage::seal(
                topology,
                XMAP09_EXECUTE_STAGE_NAME,
                XMAP09_EXECUTE_OPERATION_NAME,
                2,
            )
            .unwrap(),
        )
    }

    fn prepared(
        input: &Xmap09LightingInput,
        camera: &crate::ViewerMapCameraState,
    ) -> PreparedXmap09LightingFrame {
        let (prepare_stage, execute_stage) = assigned_stages();
        prepare_lighting(input, camera, "b".repeat(64), prepare_stage, execute_stage).unwrap()
    }

    fn light() -> Xmap09PointLight {
        Xmap09PointLight {
            stable_light_id: "building-light/1".into(),
            global_order: 0,
            position_radius_f32_bits: [1.0f32, 2.0, 3.0, 100.0].map(f32::to_bits),
            color_falloff_f32_bits: [1.0f32, 0.5, 0.25, 10.0].map(f32::to_bits),
        }
    }

    fn input() -> Xmap09LightingInput {
        let mut cells = vec![[u8::MAX; 4]; LIGHT_INDEX_WIDTH * LIGHT_INDEX_HEIGHT];
        cells[0] = [u8::MAX, u8::MAX, 0, u8::MAX];
        Xmap09LightingInput {
            schema: XMAP09_INPUT_SCHEMA.into(),
            executable_sha256: XMAP09_A729_EXECUTABLE_SHA256.into(),
            scene_light_writer_rva: XMAP09_SCENE_LIGHT_WRITER_RVA,
            product_modset_sha256: "e".repeat(64),
            effective_vfs_manifest_sha256: "f".repeat(64),
            game_hour_raw: (GAME_HOUR_RAW_EPOCH + 17_000_000) as i32,
            virtual_sun_position_f32_bits: [100.0f32, 600.0, 848.0, 5000.0].map(f32::to_bits),
            virtual_moon_position_f32_bits: [2900.0f32, 600.0, 1448.0, 550.0].map(f32::to_bits),
            second_virtual_sun_position_f32_bits: [2900.0f32, 600.0, 100.0, 5000.0]
                .map(f32::to_bits),
            second_virtual_moon_position_f32_bits: [100.0f32, 600.0, 100.0, 550.0]
                .map(f32::to_bits),
            day_night_hour_sun_dir_f32_bits: [0.5f32, 0.0, 0.02, 0.9998].map(f32::to_bits),
            sun_diffuse_intensity_f32_bits: [1.0f32; 4].map(f32::to_bits),
            moon_diffuse_intensity_f32_bits: [0.2f32, 0.3, 1.0, 2.5].map(f32::to_bits),
            ambient_rig_f32_bits: [[0.1f32.to_bits(); 3]; 6],
            sun_specular_intensity_f32_bits: 1.0f32.to_bits(),
            cubemap_intensity_f32_bits: 1.0f32.to_bits(),
            draw_shadows_cutoff_f32_bits: 400.0f32.to_bits(),
            draw_shadows_fade_length_f32_bits: 50.0f32.to_bits(),
            shadow_pass_active: true,
            point_light_grid_start_inv_cell_size_f32_bits: [0.0f32, 0.0, 0.01, 0.01]
                .map(f32::to_bits),
            placed_map_object_count: Some(1),
            point_light_census: Ok(Xmap09LightCensusCounts {
                creator_sites: 4,
                placed_entities: 1,
                authored_light_events: 1,
                fired_instances: 1,
                admitted_instances: 1,
                uploaded_lights: 1,
                paired_fired_instances: 0,
                paired_admitted_instances: 0,
                paired_uploaded_lights: 0,
                fired_by_creator: crate::XMAP09_POINT_LIGHT_CREATORS
                    .iter()
                    .map(|creator| (creator.rva, usize::from(creator.rva == 0x0226_ef30)))
                    .collect(),
            }),
            point_lights: vec![light()],
            point_light_cell_indices: cells,
            point_light_census_sha256: String::new(),
            source_observation_sha256: "a".repeat(64),
            payload_sha256: String::new(),
        }
        .seal()
        .unwrap()
    }

    fn camera() -> crate::ViewerMapCameraState {
        crate::ViewerMapCameraState::new(
            vec![UsedContentKey {
                effective_vfs_manifest_sha256: "b".repeat(64),
                stored_game_relative_path: "common/defines/00_graphics.lua".into(),
                normalized_game_relative_path: "common/defines/00_graphics.lua".into(),
                winning_layer_ordinal: 0,
                layer_kind: ContentLayerKind::Base,
                layer_id: "base".into(),
                source_sha256: "c".repeat(64),
                payload_sha256: "d".repeat(64),
                payload_bytes: 1,
            }],
            crate::Hoi4MapCameraDefines::new(50.0, 3000.0, 200.0, 200.0, 50.0, 0.02, 6.0).unwrap(),
            5632,
            2048,
            1408,
            512,
            50.0,
        )
        .unwrap()
    }

    fn file_derived_adapter() -> Xmap09LightingAdapter {
        adapter_from_defines_corpus(&[])
    }

    /// Build the adapter through the SAME parse the production constructor runs,
    /// from a vanilla base plus any extra `common/defines` files, so a test can
    /// exercise cross-file override resolution rather than poking the struct.
    fn adapter_from_defines_corpus(extra: &[(&str, &str)]) -> Xmap09LightingAdapter {
        let mut sources = vec![(
            "common/defines/00_graphics.lua".to_string(),
            crate::xmap09_lighting_law::XMAP09_VANILLA_TEST_DEFINES.to_string(),
        )];
        for (path, body) in extra {
            sources.push(((*path).to_string(), (*body).to_string()));
        }
        Xmap09LightingAdapter {
            resolver: crate::GameFileResolver::vanilla(".").unwrap(),
            lifecycle: Default::default(),
            pending_entities: Default::default(),
            derived_entities: Default::default(),
            particle_visibility: Default::default(),
            defines: crate::Xmap09LightingDefines::from_effective_sources(&sources).unwrap(),
            ordered_used_content: sources
                .iter()
                .map(|(path, _)| UsedContentKey {
                    effective_vfs_manifest_sha256: "b".repeat(64),
                    stored_game_relative_path: path.clone(),
                    normalized_game_relative_path: path.clone(),
                    winning_layer_ordinal: 0,
                    layer_kind: ContentLayerKind::Base,
                    layer_id: "base".into(),
                    source_sha256: "c".repeat(64),
                    payload_sha256: "d".repeat(64),
                    payload_bytes: 1,
                })
                .collect(),
            defines_source_sha256: "e".repeat(64),
        }
    }

    // kills: the derivation the adapter's prepare() delegates to. Before this
    // lane XMAP09 read a `xmap09_lighting` WorldFrame domain that no extractor
    // has ever written (lane OF's five history sweeps; lane IW's landing
    // a85e96b3), so this family refused on every recording; there was no
    // derivation to call at all. The derived input must satisfy the SAME
    // Xmap09LightingInput::validate_for_hour the wire payload had to satisfy,
    // and be accepted by the real prepare_lighting consumer, so emitting any
    // lane the validator rejects fails here.
    //
    // Scope, honestly: this calls derive_lighting_input directly rather than
    // ProductionMapFeatureAdapter::prepare, because prepare needs a whole
    // FoundationFeatureContext (WorldFrame + ViewerOwnedMapState). It therefore
    // does NOT catch someone rewiring prepare() to stop calling the derivation -
    // prepare() could keep the struct and compile fine. Nothing in this crate
    // catches that today; the gap is named in the lane dossier rather than
    // papered over with a guard that does not exist.
    #[test]
    fn file_derived_input_seals_and_the_real_consumer_prepares_it() {
        let adapter = file_derived_adapter();
        let camera = camera();
        let hour_index = 17_000_000i64;
        let input = adapter
            .derive_lighting_input(
                hour_index,
                &"1".repeat(64),
                &"2".repeat(64),
                &camera,
                Some(3),
                Err(Xmap09LightCensusRefusal::MissingDomain {
                    domain: "XMAP12.placed_entities".into(),
                }),
            )
            .expect("file-derived lighting input");
        input.validate_for_hour(hour_index).expect("seals for hour");
        assert_eq!(input.schema, XMAP09_INPUT_SCHEMA);
        assert_eq!(input.executable_sha256, XMAP09_A729_EXECUTABLE_SHA256);
        assert_eq!(
            input.game_hour_raw,
            (GAME_HOUR_RAW_EPOCH + hour_index) as i32
        );

        // The rig lanes are the engine law, not a stored payload: hour-of-day
        // comes from the engine epoch, the sun longitude from the loaded map
        // width, the moon latitude from map_height - SUN_HEIGHT.
        let hour_fraction = crate::engine_hour_of_day_fraction(input.game_hour_raw).unwrap();
        let expected = crate::virtual_rig(
            hour_fraction,
            crate::XMAP09_REPLAY_DAY_NIGHT_FACTOR,
            camera.map_width as f32,
            camera.map_height as f32,
            &adapter.defines,
        )
        .unwrap();
        let lanes = |bits: [u32; 4]| bits.map(f32::from_bits);
        assert_eq!(
            lanes(input.virtual_sun_position_f32_bits),
            expected.virtual_sun
        );
        assert_eq!(
            lanes(input.virtual_moon_position_f32_bits),
            expected.virtual_moon
        );
        assert_eq!(input.virtual_moon_position_f32_bits[2], 1448.0f32.to_bits());
        assert_eq!(
            f32::from_bits(input.day_night_hour_sun_dir_f32_bits[0]),
            hour_fraction
        );

        // The point-light census is excluded and named, never fabricated.
        assert!(input.point_lights.is_empty());
        assert_eq!(
            input.point_light_cell_indices.len(),
            LIGHT_INDEX_WIDTH * LIGHT_INDEX_HEIGHT
        );
        assert!(input
            .point_light_cell_indices
            .iter()
            .all(|cell| *cell == [u8::MAX; 4]));

        let (prepare_stage, execute_stage) = assigned_stages();
        let prepared = prepare_lighting(
            &input,
            &camera,
            "f".repeat(64),
            prepare_stage,
            execute_stage,
        )
        .expect("real consumer prepares the derived input");
        prepared.validate().expect("prepared frame validates");
        assert_eq!(
            prepared.scene_constants.virtual_sun_position_f32_bits,
            input.virtual_sun_position_f32_bits
        );
    }

    // kills: the layered define resolution the production constructor feeds, and
    // its effect on the drawn rig. A later-loading common/defines file that moves
    // SUN_LATITUDE must move the rig; a producer that fell back to built-in
    // vanilla constants, or whose parser ignored every source but the first,
    // emits identical bits for both adapters and fails here.
    //
    // Scope, honestly: this hands both sources to the same from_effective_sources
    // that Xmap09LightingAdapter::new calls, but it does not run new() itself,
    // which needs a real GameFileResolver plus a ValidatedContentAdmission. So it
    // does NOT prove new() ENUMERATES sibling files. What covers that is
    // require_complete_admitted_dir, which fails closed when the admission's
    // entries differ from its effective-VFS census for common/defines - a
    // resolver-owned contract with its own tests, not something this test
    // re-proves.
    #[test]
    fn modded_defines_change_the_derived_rig() {
        let vanilla = file_derived_adapter();
        let modded = adapter_from_defines_corpus(&[(
            "common/defines/zz_mod_overrides.lua",
            "NDefines.NGraphics.SUN_LATITUDE = 600 * 2
",
        )]);
        assert_eq!(vanilla.defines.sun_latitude, 848.0);
        assert_eq!(modded.defines.sun_latitude, 1200.0);
        let camera = camera();
        let base = vanilla
            .derive_lighting_input(
                17_000_000,
                &"1".repeat(64),
                &"2".repeat(64),
                &camera,
                Some(3),
                Err(Xmap09LightCensusRefusal::MissingDomain {
                    domain: "XMAP12.placed_entities".into(),
                }),
            )
            .unwrap();
        let other = modded
            .derive_lighting_input(
                17_000_000,
                &"1".repeat(64),
                &"2".repeat(64),
                &camera,
                Some(3),
                Err(Xmap09LightCensusRefusal::MissingDomain {
                    domain: "XMAP12.placed_entities".into(),
                }),
            )
            .unwrap();
        assert_ne!(
            base.virtual_sun_position_f32_bits[2],
            other.virtual_sun_position_f32_bits[2]
        );
        assert_ne!(base.payload_sha256, other.payload_sha256);
    }

    fn fixture_caster() -> L03SemanticInputRow {
        let mut matrix = [0.0f32.to_bits(); 16];
        for lane in [0usize, 5, 10, 15] {
            matrix[lane] = 1.0f32.to_bits();
        }
        L03SemanticInputRow::PdxmeshObject(L03PdxMeshObjectInput {
            stable_typed_upstream_id: "xmap12/object/fixture".into(),
            owner: L03TypedOwner::O05StaticFixed,
            world_matrix_f32_bits: matrix,
            selected_mesh: "gfx/models/fixture.mesh".into(),
            groups: vec![L03PdxMeshGroupInput {
                submeshes: vec![L03PdxMeshSubmeshInput {
                    selected_submesh_material: "fixture-material".into(),
                    material_family: L03MaterialFamily::Standard,
                    draw: L03DrawArguments::DrawIndexed {
                        index_count: 6,
                        start_index: 0,
                        base_vertex: 0,
                    },
                }],
            }],
            skinned: false,
            partition_mask: L03_PARTITION_PRIMARY,
        })
    }

    #[test]
    fn semantic_input_round_trips_and_binds_exact_hour() {
        let input = input();
        let bytes = serde_json::to_vec(&input).unwrap();
        let decoded: Xmap09LightingInput = serde_json::from_slice(&bytes).unwrap();
        decoded.validate_for_hour(17_000_000).unwrap();
        assert_eq!(decoded, input);
        assert!(decoded.validate_for_hour(17_000_001).is_err());
    }

    #[test]
    fn admitted_census_materializes_into_the_production_lighting_textures() {
        let adapter = file_derived_adapter();
        let camera = camera();
        let grid = crate::xmap09_tiled_point_light_grid(
            camera.current_eye(),
            camera.current_look_at_direction().unwrap(),
            camera.map_width as f32,
            camera.map_height as f32,
        )
        .unwrap();
        crate::xmap09_light_census::tests::with_rows(
            &[[grid[0] + 4.0, 2.0, grid[1] + 4.0]],
            |rows| {
                let input = adapter
                    .derive_lighting_input(
                        17_000_000,
                        &"1".repeat(64),
                        &"2".repeat(64),
                        &camera,
                        Some(1),
                        Ok(rows),
                    )
                    .unwrap();
                input.validate_for_hour(17_000_000).unwrap();
                assert_eq!(
                    input.point_light_census.as_ref().unwrap().uploaded_lights,
                    1
                );
                assert_eq!(input.point_light_cell_indices[0], [255, 255, 0, 255]);
                let frame = prepared(&input, &camera);
                frame.validate().unwrap();
                assert_eq!(frame.point_light_census, input.point_light_census);
                assert_eq!(&frame.light_index_map.bytes[..4], &[0, 255, 255, 255]);
                let expected = input.point_lights[0]
                    .position_radius_f32_bits
                    .into_iter()
                    .chain(input.point_lights[0].color_falloff_f32_bits)
                    .flat_map(u32::to_le_bytes)
                    .collect::<Vec<_>>();
                assert_eq!(&frame.light_data_map.bytes[..32], expected);
                assert!(frame.light_data_map.bytes[32..]
                    .iter()
                    .all(|byte| *byte == 0));
            },
        );
    }

    #[test]
    fn census_refusal_survives_preparation_and_cannot_carry_light_rows() {
        let mut input = input();
        input.point_light_census = Err(Xmap09LightCensusRefusal::MissingDomain {
            domain: "XMAP12.placed_entities".into(),
        });
        input.point_light_census_sha256 = input.computed_point_light_census_sha256();
        input.payload_sha256 = input.computed_payload_sha256().unwrap();
        assert!(input.validate_for_hour(17_000_000).is_err());
        input.point_lights.clear();
        input.point_light_cell_indices.fill([255; 4]);
        let input = input.seal().unwrap();
        let frame = prepared(&input, &camera());
        frame.validate().unwrap();
        assert_eq!(frame.point_light_census, input.point_light_census);
        assert!(frame.light_index_map.bytes.iter().all(|byte| *byte == 255));
        assert!(frame.light_data_map.bytes.iter().all(|byte| *byte == 0));
    }

    #[test]
    fn admitted_census_rgba_upload_contributes_every_occupied_channel_to_terrain_lighting() {
        use crate::{TerrainLightingProperties, Vec3};

        let adapter = file_derived_adapter();
        let camera = camera();
        let grid = crate::xmap09_tiled_point_light_grid(
            camera.current_eye(),
            camera.current_look_at_direction().unwrap(),
            camera.map_width as f32,
            camera.map_height as f32,
        )
        .unwrap();
        for count in 1..=4 {
            crate::xmap09_light_census::tests::with_rows(
                &vec![[grid[0] + 4.0, 2.0, grid[1] + 4.0]; count],
                |rows| {
                    let input = adapter
                        .derive_lighting_input(
                            17_000_000,
                            &"1".repeat(64),
                            &"2".repeat(64),
                            &camera,
                            Some(count as u32),
                            Ok(rows),
                        )
                        .unwrap();
                    let frame = prepared(&input, &camera);
                    frame.validate().unwrap();
                    let scene = crate::test_support::test_scene_cb0_with_camera_and_fade(10.0, 1.0);
                    let mut resources = crate::test_support::test_post_diffuse_resources(scene);
                    resources.light_index_map.pixels = frame.light_index_map.bytes.to_vec();
                    resources.light_data_map.pixels = frame
                        .light_data_map
                        .bytes
                        .chunks_exact(16)
                        .map(|texel| {
                            std::array::from_fn(|channel| {
                                f32::from_le_bytes(
                                    texel[channel * 4..channel * 4 + 4].try_into().unwrap(),
                                )
                            })
                        })
                        .collect();
                    resources.point_light_cb2.grid_start_inv_cell_size = frame
                        .scene_constants
                        .point_light_grid_start_inv_cell_size_f32_bits
                        .map(f32::from_bits);
                    let zero = Vec3 {
                        x: 0.0,
                        y: 0.0,
                        z: 0.0,
                    };
                    let up = Vec3 {
                        x: 0.0,
                        y: 1.0,
                        z: 0.0,
                    };
                    let properties = TerrainLightingProperties {
                        world_space_pos: Vec3 {
                            x: grid[0] + 4.0,
                            y: 1.5,
                            z: grid[1] + 4.0,
                        },
                        to_camera_dir: up,
                        normal: up,
                        diffuse: Vec3 {
                            x: 1.0,
                            y: 1.0,
                            z: 1.0,
                        },
                        specular_color: zero,
                        glossiness: 0.0,
                        non_linear_glossiness: 1.0,
                    };
                    let mut diffuse = zero;
                    let mut specular = zero;
                    crate::cpu_terrain::calculate_point_lights(
                        &resources,
                        properties,
                        &mut diffuse,
                        &mut specular,
                    );
                    for (actual, expected) in [diffuse.x, diffuse.y, diffuse.z]
                        .into_iter()
                        .zip([count as f32 / 3.0, count as f32 / 6.0, count as f32 / 12.0])
                    {
                        assert!(
                            (actual - expected).abs() < 0.000001,
                            "{count} lights: {actual} != {expected}"
                        );
                    }
                    assert_eq!(specular, zero);
                    resources.light_index_map.pixels.fill(255);
                    let mut unlit = zero;
                    crate::cpu_terrain::calculate_point_lights(
                        &resources,
                        properties,
                        &mut unlit,
                        &mut specular,
                    );
                    assert_eq!(unlit, zero);
                },
            );
        }
    }

    #[test]
    fn point_light_cells_fail_closed_on_holes_or_reordering() {
        let mut input = input();
        input.point_light_cell_indices[0] = [u8::MAX, 0, u8::MAX, u8::MAX];
        input.point_light_census_sha256 = input.computed_point_light_census_sha256();
        input.payload_sha256 = input.computed_payload_sha256().unwrap();
        assert!(input.validate_for_hour(17_000_000).is_err());
        let mut second = input.point_lights[0].clone();
        second.stable_light_id = "second".into();
        second.global_order = 1;
        input.point_lights.push(second);
        let counts = input.point_light_census.as_mut().unwrap();
        counts.uploaded_lights = 2;
        counts.admitted_instances = 2;
        counts.fired_instances = 2;
        counts.fired_by_creator.insert(0x0226_ef30, 2);
        input.point_light_cell_indices[0] = [255, 1, 0, 255];
        let mut input = input.seal().unwrap();
        input.point_light_cell_indices[0] = [255, 0, 1, 255];
        assert!(input.seal().is_err());
    }

    #[test]
    fn census_input_preserves_zero_values_and_refuses_forged_counts() {
        let mut input = input();
        input.point_lights[0].position_radius_f32_bits[3] = 0;
        input.point_lights[0].color_falloff_f32_bits[3] = 0;
        let mut input = input.seal().unwrap();
        input
            .point_light_census
            .as_mut()
            .unwrap()
            .fired_by_creator
            .insert(0x0163_f450, 1);
        assert!(input.seal().is_err());
    }

    #[test]
    fn point_light_materialization_has_exact_engine_texture_layout() {
        let input = input();
        let resource = prepared(&input, &camera()).light_index_map;
        assert_eq!(&resource.bytes[..4], &[0, 255, 255, 255]);
        assert_eq!(resource.bytes.len(), 64 * 64 * 4);
        validate_texture(
            &resource,
            "LightIndexMap",
            64,
            64,
            DXGI_FORMAT_R8G8B8A8_UNORM,
            256,
        )
        .unwrap();
    }

    #[test]
    fn scene_cb0_patch_preserves_other_owners_and_binds_reflection() {
        let prepared = prepared(&input(), &camera());
        let patch = prepared.scene_cb0_patch().unwrap();
        let reflected_fields = patch
            .ordered_fields
            .iter()
            .map(|field| Xmap09ReflectedSceneCb0Field {
                symbol: field.symbol.clone(),
                byte_offset: field.byte_offset,
                byte_len: field.bytes.len() as u32,
            })
            .collect();
        let layout =
            Xmap09ReflectedSceneCb0Layout::seal(XMAP09_SCENE_CB0_BYTE_LEN, reflected_fields)
                .unwrap();
        let base = vec![0xa5; XMAP09_SCENE_CB0_BYTE_LEN as usize];
        let composed = patch.apply_to_reflected_buffer(&base, &layout).unwrap();
        assert_eq!(&composed.bytes[..144], &base[..144]);
        assert_eq!(
            &composed.bytes[144..160],
            &patch
                .ordered_fields
                .iter()
                .find(|field| field.symbol == "vVirtualSunPos")
                .unwrap()
                .bytes
        );
        assert_eq!(&composed.bytes[408..], &base[408..]);
        assert_eq!(composed.receipt.contributor_packet, "XMAP09");
        assert_eq!(
            composed.receipt.replay_frame_identity_sha256,
            prepared.replay_frame_identity_sha256
        );
        assert_eq!(
            composed.receipt.reflected_layout_sha256,
            layout.deterministic_sha256
        );
        assert_eq!(composed.receipt.base_payload_sha256, Sha256::hex(&base));
        assert_eq!(
            composed.receipt.final_payload_sha256,
            Sha256::hex(&composed.bytes)
        );
        assert_eq!(
            composed.receipt.applied_symbols.len(),
            XMAP09_SCENE_CB0_FIELD_COUNT
        );
    }

    #[test]
    fn scene_cb0_reflection_rejects_moved_owned_field() {
        assert!(Xmap09ReflectedSceneCb0Layout::seal(
            XMAP09_SCENE_CB0_BYTE_LEN,
            vec![Xmap09ReflectedSceneCb0Field {
                symbol: "SunDiffuseIntensity".into(),
                byte_offset: 364,
                byte_len: 16,
            }],
        )
        .is_err());
    }

    #[test]
    fn wrapped_shadow_partitions_bind_distinct_matrices_to_terrain_and_tree_cb1() {
        let frame = prepared(&input(), &camera());
        let matrices = [1, 2, 4].map(|partition| frame.shadow_map_texture_matrix_for_partition(7, partition).unwrap());
        assert_eq!(matrices[0].shadow_map_texture_matrix_f32_bits, frame.shadow.selected_light_frame.shadow_map_texture_matrix_f32_bits);
        assert_ne!(matrices[0].payload_sha256, matrices[1].payload_sha256);
        assert_ne!(matrices[0].payload_sha256, matrices[2].payload_sha256);
        assert_ne!(matrices[1].payload_sha256, matrices[2].payload_sha256);
        for matrix in &matrices {
            let mut terrain = vec![0; crate::TERRAIN_CHUNK_CB1_BYTES];
            terrain[64..].fill(0xa5);
            let terrain = crate::xmap10_d3d_handoff::xmap10_terrain_unlit_cb1_bytes(&terrain, matrix).unwrap();
            let tree = crate::xmap11_d3d_handoff::tree_unlit_cb1_bytes(matrix, 0.375f32.to_bits()).unwrap();
            let expected = matrix.shadow_map_texture_matrix_f32_bits.iter().flat_map(|bits| bits.to_le_bytes()).collect::<Vec<_>>();
            assert_eq!(&terrain[..64], expected);
            assert_eq!(&tree[..64], expected);
            assert!(terrain[64..].iter().all(|byte| *byte == 0xa5));
            assert_eq!(&tree[64..72], &[0; 8]);
            assert_eq!(&tree[72..76], &0.375f32.to_le_bytes());
            assert_eq!(&tree[76..], &[0; 4]);
        }
        assert!(frame.shadow_map_texture_matrix_for_partition(5, 2).is_err());
        assert!(frame.shadow_map_texture_matrix_for_partition(6, 4).is_err());
    }

    #[test]
    fn prepared_handoff_regenerates_for_viewer_camera_and_light_state() {
        let input = input();
        let camera = camera();
        let first = prepared(&input, &camera);
        first.validate().unwrap();
        assert_eq!(first.camera_frame_sha256, camera.deterministic_sha256);
        assert_eq!(first.replay_frame_identity_sha256, "b".repeat(64));
        assert_eq!(first.light_index_map.bytes.len(), 64 * 64 * 4);
        assert_eq!(first.light_data_map.bytes.len(), 128 * 16);
        assert_eq!(first.shadow.screen_passes.len(), 3);
        let projection = first
            .project_exact_casters(L03_PARTITION_PRIMARY, vec![fixture_caster()], Vec::new())
            .unwrap();
        assert_eq!(projection.submissions.len(), 1);
        let chain = first.plan_shadow_consumer(&projection).unwrap().unwrap();
        assert_eq!(chain.passes, first.shadow.screen_passes);

        let mut moved = camera.clone();
        moved.pan_screen(120.0, -80.0).unwrap();
        let moved_frame = prepared(&input, &moved);
        let moved_projection = moved_frame
            .project_exact_casters(L03_PARTITION_PRIMARY, vec![fixture_caster()], Vec::new())
            .unwrap();
        assert_eq!(
            first.shadow.producer_and_final_target.width,
            moved_frame.shadow.producer_and_final_target.width
        );
        assert_ne!(first.camera_frame_sha256, moved_frame.camera_frame_sha256);
        assert_ne!(
            first.shadow.producer_and_final_target.stable_resource_id,
            moved_frame
                .shadow
                .producer_and_final_target
                .stable_resource_id
        );
        assert!(first.plan_shadow_consumer(&moved_projection).is_err());
        let mut substituted_camera = first.clone();
        substituted_camera.camera_frame_sha256 = moved_frame.camera_frame_sha256.clone();
        assert!(substituted_camera.validate().is_err());
        let mut substituted_replay_frame = first.clone();
        substituted_replay_frame.replay_frame_identity_sha256 = "c".repeat(64);
        assert!(substituted_replay_frame.validate().is_err());

        let mut resized = camera.clone();
        resized.resize_viewport(1920, 1080).unwrap();
        let second = prepared(&input, &resized);
        second.validate().unwrap();
        assert_eq!(second.camera_frame_sha256, resized.deterministic_sha256);
        assert_ne!(first.camera_frame_sha256, second.camera_frame_sha256);
        assert_ne!(
            first.shadow.producer_and_final_target.stable_resource_id,
            second.shadow.producer_and_final_target.stable_resource_id
        );
        assert_ne!(
            first
                .shadow
                .selected_light_frame
                .light_view_projection_f32_bits,
            second
                .shadow
                .selected_light_frame
                .light_view_projection_f32_bits
        );

        let mut changed_light = input.clone();
        changed_light.virtual_moon_position_f32_bits[0] = 2500.0f32.to_bits();
        changed_light = changed_light.seal().unwrap();
        let third = prepared(&changed_light, &camera);
        third.validate().unwrap();
        assert_ne!(
            first
                .shadow
                .selected_light_frame
                .light_view_projection_f32_bits,
            third
                .shadow
                .selected_light_frame
                .light_view_projection_f32_bits
        );
    }

    #[test]
    fn shadow_target_bind_contract_is_depth_plus_srv() {
        assert_eq!(D3D11_BIND_DEPTH_STENCIL | D3D11_BIND_SHADER_RESOURCE, 0x48);
    }
}
