
use crate::fog_derivation_law;
use crate::fow_law::FOW_REQUIRED_SHADER_SOURCES;
use crate::political_color_synthesis::{
    xmap05_half_res_source_index, xmap05_raster_inputs_with_keying, Xmap05RasterInputs,
};
use crate::{
    exact_a729_intel_map, exact_a729_land_unit_disclosure, exact_a729_taskforce_disclosure,
    read_latin1, synthesize_political_colors_with_keying, ExactUnitDisclosure,
    FoundationFeatureContext, FoundationFeatureOutput, GameFileResolver, KeyingTables,
    PoliticalColorSynthesis, ProductionMapFeatureAdapter, Result, ViewerMapCameraState,
    ViewerOwnedMapState, Xmap10ViewerPresentationState, PROVINCE_COLOR_MAP_DXGI_FORMAT,
};
use portable_content_manifest::UsedContentKey;
use portable_render_stream::{
    ElementDiagnostic, ElementOutcome, FamilyDisposition, FoundationOperationIdentity,
    MapVisualFamily,
};
use replay_core::{ExactSpottedUnit, Leaf, ValidatedContentAdmission, ViewStatus, WorldFrame};
use snapshot::sha256::Sha256;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

pub const XMAP05_POLITICAL_SCHEMA: &str = "hoi4.xmap05.political-gradient.a729.v1";
pub const XMAP05_VISIBILITY_SCHEMA: &str = "hoi4.xmap05.visibility-intel.a729.v1";
pub const XMAP05_MAP_CLOCK_SCHEMA: &str = "hoi4.xmap05.authenticated-map-clock.a729.v2";
pub const XMAP05_SCENE_CB0_PATCH_SCHEMA: &str = "hoi4.xmap05.scene-cb0-patch.a729.v1";
pub const GRADIENT_BORDER_CHANNEL_DXGI_FORMAT: u32 = 87;
pub const GRADIENT_BORDER_CHANNEL3_DXGI_FORMAT: u32 = 91;
pub const INTEL_MAP_DXGI_FORMAT: u32 = 65;

const GRAPHICS_DEFINES: &str = "common/defines/00_graphics.lua";
const NAVAL_DOMINANCE_DDS: &str = "map/terrain/naval_dominance_fx.dds";
const STATIC_REQUIRED_FILES: [&str; 5] = [
    "map/definition.csv",
    "map/provinces.bmp",
    "common/countries/colors.txt",
    GRAPHICS_DEFINES,
    NAVAL_DOMINANCE_DDS,
];
const REQUIRED_PREFIXES: [&str; 3] = [
    "history/states/",
    "common/country_tags/",
    "common/countries/",
];
const POLITICAL_AUTHORITY_GAPS: [&str; 5] = [
    "control.controller_by_province",
    "control.controller_by_state",
    "control.owner_by_state",
    "country.d18_complete_roster",
    "country.presentation_identity",
];
const FOW_AUTHORITY_GAPS: [&str; 4] = [
    "fog.province_visibility_by_pov",
    "fog.spotted_land_by_pov",
    "fog.spotted_taskforces_by_pov",
    "fog.spotted_wings_by_pov",
];
const MAPC1_EXECUTABLE_SHA256: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";
const MAPC1_GHIDRA_MANIFEST_SHA256: &str =
    "a69a7b300422c486ee2b76901a5f39e8e8d2cfe8c2baca38e3b353e4892b1990";
const DRAW_FOW_DEFINE_LOADERS_GHIDRA_MANIFEST_SHA256: &str =
    "2dcf828c69996f7b43521caef41c6bd7d310ec192b755ff617555bcf9f4ee74e";
#[cfg(test)]
const VANILLA_CHANNEL3_BGRA8_SRGB_ORACLE: [u8; 8] =
    [0x53, 0x36, 0xff, 0xff, 0xff, 0xc3, 0x0d, 0xff];

fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct GradientBorderCountryDefines {
    field_low: f32,
    field_high: f32,
    thickness_low: f32,
    thickness_high: f32,
    center_thickness: f32,
    refresh_hours: u32,
    outline_cutoff_strategic: f32,
    outline_cutoff_country: f32,
    camera_override_strategic: f32,
    camera_override_country: f32,
    draw_fow_cutoff: f32,
    draw_fow_fade_length: f32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Xmap05TextureDescriptor {
    pub symbol: String,
    pub width: u32,
    pub height: u32,
    pub dxgi_format: u32,
    pub row_pitch: u32,
    pub mip_levels: u32,
    pub payload_sha256: String,
    pub descriptor_sha256: String,
}

impl Xmap05TextureDescriptor {
    fn new(
        symbol: &str,
        width: usize,
        height: usize,
        dxgi_format: u32,
        bytes_per_texel: usize,
        payload: &[u8],
    ) -> Result<Self> {
        if width == 0
            || height == 0
            || payload.len()
                != width
                    .checked_mul(height)
                    .and_then(|count| count.checked_mul(bytes_per_texel))
                    .ok_or("XMAP05 texture extent overflow")?
        {
            return Err(format!("{symbol} payload extent is invalid").into());
        }
        let mut descriptor = Self {
            symbol: symbol.into(),
            width: u32::try_from(width)?,
            height: u32::try_from(height)?,
            dxgi_format,
            row_pitch: u32::try_from(
                width
                    .checked_mul(bytes_per_texel)
                    .ok_or("XMAP05 row pitch overflow")?,
            )?,
            mip_levels: 1,
            payload_sha256: Sha256::hex(payload),
            descriptor_sha256: String::new(),
        };
        descriptor.descriptor_sha256 = texture_descriptor_digest(&descriptor);
        descriptor.validate()?;
        Ok(descriptor)
    }

    fn validate(&self) -> Result<()> {
        if self.symbol.is_empty()
            || self.width == 0
            || self.height == 0
            || self.row_pitch == 0
            || self.mip_levels != 1
            || self.payload_sha256.len() != 64
            || self.descriptor_sha256 != texture_descriptor_digest(self)
        {
            return Err("XMAP05 texture descriptor is invalid".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Xmap05GeneratedTexture {
    pub descriptor: Xmap05TextureDescriptor,
    pub bytes: Arc<[u8]>,
}

impl Xmap05GeneratedTexture {
    fn validate(&self) -> Result<()> {
        self.descriptor.validate()?;
        if self.descriptor.payload_sha256 != Sha256::hex(&self.bytes)
            || self.bytes.len()
                != usize::try_from(self.descriptor.row_pitch)?
                    .checked_mul(usize::try_from(self.descriptor.height)?)
                    .ok_or("XMAP05 texture byte length overflow")?
        {
            return Err(format!(
                "XMAP05 texture {} payload identity mismatch",
                self.descriptor.symbol
            )
            .into());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Xmap05VisibilityReason {
    OmniscientViewer,
    D95MapVisibilityThresholdExcluded,
    D95DetectedTier,
    ExactD95MapVisible,
    ExactD95ExactDetails,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Xmap05VisibleEntity {
    pub type_id: u32,
    pub stable_id: u32,
    pub disclosure: ExactUnitDisclosure,
    pub reason: Xmap05VisibilityReason,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedXmap05Visibility {
    pub schema: String,
    pub viewer_country: Option<String>,
    pub fow_enabled: bool,
    pub capture_sequence: Option<u64>,
    pub pov_country_id: Option<u32>,
    pub province_visibility_sha256: String,
    pub land_units: Vec<Xmap05VisibleEntity>,
    pub taskforce_units: Vec<Xmap05VisibleEntity>,
    pub air_wing_spotting_unavailable: bool,
    pub land_frame_sequence: Option<u64>,
    pub taskforce_frame_sequence: Option<u64>,
    pub deterministic_sha256: String,
}

impl PreparedXmap05Visibility {
    pub fn land_disclosure(
        &self,
        type_id: u32,
        stable_id: u32,
    ) -> (ExactUnitDisclosure, Xmap05VisibilityReason) {
        if !self.fow_enabled {
            return (
                ExactUnitDisclosure::ExactDetails,
                Xmap05VisibilityReason::OmniscientViewer,
            );
        }
        self.land_units
            .iter()
            .find(|row| row.type_id == type_id && row.stable_id == stable_id)
            .map(|row| (row.disclosure, row.reason))
            .unwrap_or((
                ExactUnitDisclosure::Hidden,
                Xmap05VisibilityReason::D95MapVisibilityThresholdExcluded,
            ))
    }

    pub fn taskforce_disclosure(
        &self,
        type_id: u32,
        stable_id: u32,
    ) -> (ExactUnitDisclosure, Xmap05VisibilityReason) {
        if !self.fow_enabled {
            return (
                ExactUnitDisclosure::ExactDetails,
                Xmap05VisibilityReason::OmniscientViewer,
            );
        }
        self.taskforce_units
            .iter()
            .find(|row| row.type_id == type_id && row.stable_id == stable_id)
            .map(|row| (row.disclosure, row.reason))
            .unwrap_or((
                ExactUnitDisclosure::Hidden,
                Xmap05VisibilityReason::D95MapVisibilityThresholdExcluded,
            ))
    }

    fn validate(&self) -> Result<()> {
        if self.schema != XMAP05_VISIBILITY_SCHEMA
            || !self.air_wing_spotting_unavailable
            || self.province_visibility_sha256.len() != 64
            || self.deterministic_sha256 != visibility_digest(self)
        {
            return Err("XMAP05 visibility payload is invalid".into());
        }
        match (&self.viewer_country, self.fow_enabled) {
            (None, false)
                if self.capture_sequence.is_none()
                    && self.pov_country_id.is_none()
                    && self.land_frame_sequence.is_none()
                    && self.taskforce_frame_sequence.is_none()
                    && self.land_units.is_empty()
                    && self.taskforce_units.is_empty() => {}
            (Some(country), true)
                if !country.trim().is_empty()
                    && self.capture_sequence.is_some_and(|value| value != 0)
                    && self.pov_country_id.is_some_and(|value| value != 0)
                    && self.land_frame_sequence.is_some_and(|value| value != 0)
                    && self
                        .taskforce_frame_sequence
                        .is_some_and(|value| value != 0) =>
            {
                validate_entity_rows(&self.land_units)?;
                validate_entity_rows(&self.taskforce_units)?;
            }
            _ => return Err("XMAP05 omniscient/POV visibility contract is inconsistent".into()),
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Xmap05AuthenticatedMapClock {
    schema: String,
    hour_index: i64,
    seconds_f32_bits: u32,
    fow_opacity_f32_bits: u32,
    viewer_presentation_receipt_sha256: String,
    viewer_state_sha256: String,
    producer_receipt_sha256: String,
}

impl Xmap05AuthenticatedMapClock {
    pub fn from_viewer_presentation(
        hour_index: i64,
        viewer: &ViewerOwnedMapState,
        presentation: &Xmap10ViewerPresentationState,
    ) -> Result<Self> {
        viewer.validate()?;
        presentation.validate()?;
        if hour_index < 0 {
            return Err("XMAP05 viewer map-clock hour is negative".into());
        }
        if viewer.animation_time_f32_bits != presentation.application_elapsed_seconds().to_bits() {
            return Err(
                "XMAP05 viewer animation clock differs from XMAP10 presentation time".into(),
            );
        }
        let viewer_state_sha256 = Sha256::hex(&serde_json::to_vec(viewer)?);
        let mut clock = Self {
            schema: XMAP05_MAP_CLOCK_SCHEMA.into(),
            hour_index,
            seconds_f32_bits: viewer.animation_time_f32_bits,
            fow_opacity_f32_bits: 0.0f32.to_bits(),
            viewer_presentation_receipt_sha256: presentation.deterministic_sha256.clone(),
            viewer_state_sha256,
            producer_receipt_sha256: String::new(),
        };
        clock.producer_receipt_sha256 = clock.computed_producer_receipt_sha256();
        clock.validate()?;
        Ok(clock)
    }

    pub fn seconds(&self) -> f32 {
        f32::from_bits(self.seconds_f32_bits)
    }

    pub fn fow_opacity(&self) -> f32 {
        f32::from_bits(self.fow_opacity_f32_bits)
    }

    pub fn hour_index(&self) -> i64 {
        self.hour_index
    }

    pub fn producer_receipt_sha256(&self) -> &str {
        &self.producer_receipt_sha256
    }

    fn computed_producer_receipt_sha256(&self) -> String {
        let mut bytes = b"XMAP05_AUTHENTICATED_MAP_CLOCK_PRODUCER_RECEIPT_V1\0".to_vec();
        bytes.extend_from_slice(self.schema.as_bytes());
        bytes.extend_from_slice(MAPC1_EXECUTABLE_SHA256.as_bytes());
        bytes.extend_from_slice(&self.hour_index.to_le_bytes());
        bytes.extend_from_slice(&self.seconds_f32_bits.to_le_bytes());
        bytes.extend_from_slice(&self.fow_opacity_f32_bits.to_le_bytes());
        bytes.extend_from_slice(self.viewer_presentation_receipt_sha256.as_bytes());
        bytes.extend_from_slice(self.viewer_state_sha256.as_bytes());
        Sha256::hex(&bytes)
    }

    pub fn validate(&self) -> Result<()> {
        let seconds = self.seconds();
        let fow_opacity = self.fow_opacity();
        if self.schema != XMAP05_MAP_CLOCK_SCHEMA
            || self.hour_index < 0
            || !seconds.is_finite()
            || seconds < 0.0
            || !fow_opacity.is_finite()
            || !(0.0..=1.0).contains(&fow_opacity)
            || !is_lower_sha256(&self.viewer_presentation_receipt_sha256)
            || !is_lower_sha256(&self.viewer_state_sha256)
            || self.fow_opacity_f32_bits != 0.0f32.to_bits()
            || !is_lower_sha256(&self.producer_receipt_sha256)
            || self.producer_receipt_sha256 != self.computed_producer_receipt_sha256()
        {
            return Err("XMAP05 authenticated map clock is invalid".into());
        }
        Ok(())
    }

    #[cfg(test)]
    fn sealed_test_receipt(hour_index: i64, seconds: f32) -> Self {
        let mut receipt = Self {
            schema: XMAP05_MAP_CLOCK_SCHEMA.into(),
            hour_index,
            seconds_f32_bits: seconds.to_bits(),
            fow_opacity_f32_bits: 0.0f32.to_bits(),
            viewer_presentation_receipt_sha256: "a".repeat(64),
            viewer_state_sha256: "b".repeat(64),
            producer_receipt_sha256: String::new(),
        };
        receipt.producer_receipt_sha256 = receipt.computed_producer_receipt_sha256();
        receipt
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Xmap05SceneCb0Patch {
    pub schema: String,
    pub hour_index: i64,
    pub camera_height: f32,
    pub map_clock_seconds: f32,
    pub v_gb_cam_dist_override_gb_outline_cutoff: [f32; 4],
    pub fow_opacity: f32,
    pub fow_time: f32,
    pub fow_fade_factor: f32,
    pub min_mesh_alpha: f32,
    pub gb_texture_height: f32,
    pub payload_bytes: [u8; 36],
    pub effective_vfs_manifest_sha256: String,
    pub used_content_sha256: String,
    pub political_frame_sha256: String,
    pub map_clock_source_sha256: String,
    pub static_law_receipt_sha256: String,
    pub camera_state_sha256: String,
    pub camera_binding_sha256: String,
    pub payload_sha256: String,
    pub deterministic_sha256: String,
}

impl Xmap05SceneCb0Patch {
    pub fn validate(&self) -> Result<()> {
        if self.schema != XMAP05_SCENE_CB0_PATCH_SCHEMA
            || !self.camera_height.is_finite()
            || self.camera_height < 0.0
            || !self.map_clock_seconds.is_finite()
            || self.map_clock_seconds < 0.0
            || !self
                .v_gb_cam_dist_override_gb_outline_cutoff
                .iter()
                .all(|value| value.is_finite())
            || ![
                self.fow_opacity,
                self.fow_time,
                self.fow_fade_factor,
                self.min_mesh_alpha,
                self.gb_texture_height,
            ]
            .iter()
            .all(|value| value.is_finite())
            || !(0.0..=1.0).contains(&self.fow_opacity)
            || !(0.0..=1.0).contains(&self.fow_fade_factor)
            || self.fow_time.to_bits() != self.map_clock_seconds.to_bits()
            || self.min_mesh_alpha.to_bits() != 0.0f32.to_bits()
            || self.gb_texture_height <= 0.0
            || [
                &self.effective_vfs_manifest_sha256,
                &self.used_content_sha256,
                &self.political_frame_sha256,
                &self.map_clock_source_sha256,
                &self.static_law_receipt_sha256,
                &self.camera_state_sha256,
                &self.camera_binding_sha256,
                &self.payload_sha256,
                &self.deterministic_sha256,
            ]
            .into_iter()
            .any(|hash| hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()))
            || self.payload_bytes != scene_cb0_payload_bytes(self)
            || self.payload_sha256 != Sha256::hex(&self.payload_bytes)
            || self.static_law_receipt_sha256 != scene_cb0_static_law_receipt_digest(self)
            || self.camera_binding_sha256 != scene_cb0_camera_binding_digest(self)
            || self.deterministic_sha256 != scene_cb0_patch_digest(self)
        {
            return Err("XMAP05 SceneCB0 patch is invalid".into());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Xmap05SceneCb0Law {
    outline_cutoff_strategic_bits: u32,
    outline_cutoff_country_bits: u32,
    camera_override_strategic_bits: u32,
    camera_override_country_bits: u32,
    draw_fow_cutoff_bits: u32,
    draw_fow_fade_length_bits: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedPoliticalFrame {
    pub schema: String,
    pub hour_index: i64,
    pub ordered_used_content: Vec<UsedContentKey>,
    pub province_primary: Xmap05GeneratedTexture,
    pub province_secondary: Xmap05GeneratedTexture,
    pub gradient_border_channel1: Xmap05GeneratedTexture,
    pub gradient_border_channel2: Xmap05GeneratedTexture,
    pub gradient_border_channel3: Xmap05GeneratedTexture,
    pub intel_map: Xmap05GeneratedTexture,
    pub visibility: PreparedXmap05Visibility,
    pub political_family_sha256: String,
    pub fow_family_sha256: String,
    pub deterministic_sha256: String,
    /// One row per roster country whose cosmetic identity this hour did not
    /// carry. Empty in the ordinary case, which is what keeps the receipt free.
    ///
    /// Cached alongside the textures because it is derived from the same frame:
    /// `xmap05_semantic_cache_digest` already folds the whole `countries` object
    /// into the cache key, so a frame whose cosmetic identities differ cannot
    /// reuse another frame's rows.
    pub excluded_presentation_identities: Vec<ElementDiagnostic>,
    scene_cb0_law: Xmap05SceneCb0Law,
}

impl PreparedPoliticalFrame {
    pub fn validate(&self) -> Result<()> {
        if self.schema != XMAP05_POLITICAL_SCHEMA || self.ordered_used_content.is_empty() {
            return Err("XMAP05 prepared political frame is incomplete".into());
        }
        let mut paths = BTreeSet::new();
        for content in &self.ordered_used_content {
            content
                .validate()
                .map_err(|error| format!("XMAP05 used-content identity: {error}"))?;
            if !paths.insert(content.normalized_game_relative_path.as_str()) {
                return Err("XMAP05 used-content path is duplicated".into());
            }
        }
        for texture in [
            &self.province_primary,
            &self.province_secondary,
            &self.gradient_border_channel1,
            &self.gradient_border_channel2,
            &self.gradient_border_channel3,
            &self.intel_map,
        ] {
            texture.validate()?;
        }
        if self.province_primary.descriptor.width != self.province_secondary.descriptor.width
            || self.province_primary.descriptor.height != self.province_secondary.descriptor.height
            || self.province_primary.descriptor.dxgi_format != PROVINCE_COLOR_MAP_DXGI_FORMAT
            || self.province_secondary.descriptor.dxgi_format != PROVINCE_COLOR_MAP_DXGI_FORMAT
            || self.gradient_border_channel1.descriptor.width
                != self.province_primary.descriptor.width
            || self.gradient_border_channel2.descriptor.width
                != self.province_primary.descriptor.width
            || self.gradient_border_channel1.descriptor.height
                != self
                    .province_primary
                    .descriptor
                    .height
                    .checked_mul(2)
                    .and_then(|value| value.checked_add(2))
                    .ok_or("XMAP05 GradientBorder height overflow")?
            || self.gradient_border_channel2.descriptor.height
                != self.gradient_border_channel1.descriptor.height
            || self.gradient_border_channel1.descriptor.dxgi_format
                != GRADIENT_BORDER_CHANNEL_DXGI_FORMAT
            || self.gradient_border_channel2.descriptor.dxgi_format
                != GRADIENT_BORDER_CHANNEL_DXGI_FORMAT
            || self.gradient_border_channel3.descriptor.symbol != "GradientBorderChannel3"
            || self.gradient_border_channel3.descriptor.width != 2
            || self.gradient_border_channel3.descriptor.height != 1
            || self.gradient_border_channel3.descriptor.dxgi_format
                != GRADIENT_BORDER_CHANNEL3_DXGI_FORMAT
            || self.intel_map.descriptor.dxgi_format != INTEL_MAP_DXGI_FORMAT
        {
            return Err("XMAP05 texture family extent/format contract is invalid".into());
        }
        self.visibility.validate()?;
        if self.political_family_sha256 != political_family_digest(self)
            || self.fow_family_sha256 != fow_family_digest(self)
            || self.deterministic_sha256 != prepared_political_digest(self)
        {
            return Err("XMAP05 prepared political frame digest mismatch".into());
        }
        Ok(())
    }

    pub fn scene_cb0_patch(
        &self,
        camera: &ViewerMapCameraState,
        map_clock: &Xmap05AuthenticatedMapClock,
    ) -> Result<Xmap05SceneCb0Patch> {
        self.validate()?;
        camera.validate()?;
        map_clock.validate()?;
        if map_clock.hour_index() != self.hour_index {
            return Err(format!(
                "XMAP05 map clock hour {} does not match political frame hour {}",
                map_clock.hour_index(),
                self.hour_index
            )
            .into());
        }
        if camera.map_width != self.province_primary.descriptor.width
            || camera.map_height != self.province_primary.descriptor.height
        {
            return Err("XMAP05 SceneCB0 camera and political raster extents differ".into());
        }
        let graphics_defines = self
            .ordered_used_content
            .iter()
            .filter(|content| content.normalized_game_relative_path == GRAPHICS_DEFINES)
            .collect::<Vec<_>>();
        if graphics_defines.len() != 1 || graphics_defines[0] != &camera.graphics_defines_content {
            return Err(
                "XMAP05 SceneCB0 camera and political frame use different graphics defines".into(),
            );
        }
        let camera_height = camera.current_eye()[1];
        let f32_from = |bits| f32::from_bits(bits);
        let cutoff = f32_from(self.scene_cb0_law.draw_fow_cutoff_bits);
        let fade_length = f32_from(self.scene_cb0_law.draw_fow_fade_length_bits);
        let linear_fow_fade = ((cutoff - camera_height) / fade_length).clamp(0.0, 1.0);
        let effective_vfs_manifest_sha256 = effective_vfs_manifest_identity(self)?;
        let used_content_sha256 = used_content_digest(&self.ordered_used_content);
        let mut patch = Xmap05SceneCb0Patch {
            schema: XMAP05_SCENE_CB0_PATCH_SCHEMA.into(),
            hour_index: self.hour_index,
            camera_height,
            map_clock_seconds: map_clock.seconds(),
            v_gb_cam_dist_override_gb_outline_cutoff: [
                f32_from(self.scene_cb0_law.camera_override_strategic_bits),
                f32_from(self.scene_cb0_law.camera_override_country_bits),
                f32_from(self.scene_cb0_law.outline_cutoff_strategic_bits),
                f32_from(self.scene_cb0_law.outline_cutoff_country_bits),
            ],
            fow_opacity: map_clock.fow_opacity(),
            fow_time: map_clock.seconds(),
            fow_fade_factor: linear_fow_fade * linear_fow_fade,
            min_mesh_alpha: 0.0,
            // generated GB textures themselves are (2*h)+2 rows.
            gb_texture_height: self.province_primary.descriptor.height as f32,
            payload_bytes: [0; 36],
            effective_vfs_manifest_sha256,
            used_content_sha256,
            political_frame_sha256: self.deterministic_sha256.clone(),
            map_clock_source_sha256: map_clock.producer_receipt_sha256().into(),
            static_law_receipt_sha256: String::new(),
            camera_state_sha256: camera.deterministic_sha256.clone(),
            camera_binding_sha256: String::new(),
            payload_sha256: String::new(),
            deterministic_sha256: String::new(),
        };
        patch.payload_bytes = scene_cb0_payload_bytes(&patch);
        patch.payload_sha256 = Sha256::hex(&patch.payload_bytes);
        patch.static_law_receipt_sha256 = scene_cb0_static_law_receipt_digest(&patch);
        patch.camera_binding_sha256 = scene_cb0_camera_binding_digest(&patch);
        patch.deterministic_sha256 = scene_cb0_patch_digest(&patch);
        patch.validate()?;
        Ok(patch)
    }

    pub fn terrain_dependency_bindings(&self) -> Vec<crate::TerrainDependentResourceBinding> {
        [
            &self.province_secondary,
            &self.gradient_border_channel1,
            &self.gradient_border_channel2,
        ]
        .into_iter()
        .map(|texture| crate::TerrainDependentResourceBinding {
            symbol: texture.descriptor.symbol.clone(),
            owner_packet: "XMAP05".into(),
            resource_sha256: texture.descriptor.payload_sha256.clone(),
            descriptor_sha256: texture.descriptor.descriptor_sha256.clone(),
        })
        .collect()
    }
}

#[derive(Clone)]
struct Xmap05Content {
    resolver: GameFileResolver,
    ordered_used_content: Vec<UsedContentKey>,
    defines: GradientBorderCountryDefines,
    channel3_bgra8_srgb: [u8; 8],
}

impl Xmap05Content {
    fn admit(resolver: &GameFileResolver, admission: &ValidatedContentAdmission) -> Result<Self> {
        let translation_unit =
            crate::shader_compile::effect_translation_unit_files(resolver, "gfx/FX/pdxmap.shader")?;
        let water_translation_unit = crate::shader_compile::effect_translation_unit_files(
            resolver,
            "gfx/FX/pdxwater.shader",
        )?;
        let mut required = BTreeSet::new();
        required.extend(STATIC_REQUIRED_FILES.map(str::to_string));
        required.extend(FOW_REQUIRED_SHADER_SOURCES.map(str::to_string));
        required.extend(translation_unit.iter().map(|(path, _)| path.clone()));
        required.extend(water_translation_unit.iter().map(|(path, _)| path.clone()));
        for path in admission.required_paths() {
            let folded = path.replace('\\', "/").to_ascii_lowercase();
            if REQUIRED_PREFIXES
                .iter()
                .any(|prefix| folded.starts_with(prefix))
            {
                required.insert(path.to_string());
            }
        }
        for (directory, prefix) in [
            ("history/states", "history/states/"),
            ("common/country_tags", "common/country_tags/"),
            ("common/countries", "common/countries/"),
        ] {
            let effective = resolver.audit_effective_dir_files(directory, Some("txt"))?;
            if effective.is_empty() {
                return Err(format!("XMAP05 effective VFS has no {directory} text files").into());
            }
            for path in effective {
                let folded = path.replace('\\', "/").to_ascii_lowercase();
                if !folded.starts_with(prefix) || admission.used_content(&path).is_none() {
                    return Err(format!(
                        "XMAP05 content manifest omits effective political input {path}"
                    )
                    .into());
                }
                required.insert(path);
            }
        }
        let mut ordered_used_content = Vec::new();
        for path in required {
            let content = admission
                .require_used_content(&path)
                .map_err(|error| format!("XMAP05 requires admitted {path}: {error}"))?
                .clone();
            resolver.read_admitted_used_content(&content)?;
            ordered_used_content.push(content);
        }
        let graphics = admission
            .require_used_content(GRAPHICS_DEFINES)
            .map_err(|error| format!("XMAP05 requires admitted graphics defines: {error}"))?;
        let graphics_bytes = resolver.read_admitted_used_content(graphics)?;
        let graphics_source = String::from_utf8(graphics_bytes)
            .map_err(|_| "XMAP05 graphics defines are not UTF-8/ASCII compatible")?;
        let defines = parse_gradient_border_country_defines(&graphics_source)?;
        let channel3 = admission
            .require_used_content(NAVAL_DOMINANCE_DDS)
            .map_err(|error| format!("XMAP05 requires admitted Channel3 DDS: {error}"))?;
        let channel3_bytes = resolver.read_admitted_used_content(channel3)?;
        let channel3_bgra8_srgb = decode_gradient_border_channel3_dds(&channel3_bytes)?;
        Ok(Self {
            resolver: resolver.clone(),
            ordered_used_content,
            defines,
            channel3_bgra8_srgb,
        })
    }
}

struct Xmap05Runtime {
    cache_key: Option<String>,
    prepared: Option<PreparedPoliticalFrame>,
}

struct Xmap05Shared {
    content: Arc<Xmap05Content>,
    runtime: Mutex<Xmap05Runtime>,
}

impl Xmap05Shared {
    fn prepare(&self, context: &FoundationFeatureContext<'_>) -> Result<PreparedPoliticalFrame> {
        let (keying, province_controllers, presentation) =
            admitted_epoch_keying(context.world_frame)?;
        // The RESOLVED roster, and that is the safe set to hand the raster:
        // `validate_xmap05_state_authority` requires every state owner, state
        // controller and province controller TAG to be in it, so a control row
        // naming a country this hour could not name refuses the frame instead of
        // colouring an unnamed country's land.
        let country_roster = xmap05_roster(context.world_frame)?
            .resolved
            .into_iter()
            .map(|entry| entry.tag)
            .collect::<BTreeSet<_>>();
        let cache_key = xmap05_semantic_cache_key(context, &keying, &province_controllers)?;
        let mut runtime = self
            .runtime
            .lock()
            .map_err(|_| "XMAP05 runtime cache lock is poisoned")?;
        if runtime.cache_key.as_deref() == Some(cache_key.as_str()) {
            return runtime
                .prepared
                .clone()
                .ok_or_else(|| "XMAP05 cache identity has no payload".into());
        }

        let political = synthesize_political_colors_with_keying(&self.content.resolver, &keying)?;
        let raster = xmap05_raster_inputs_with_keying(
            &self.content.resolver,
            &keying,
            &province_controllers,
            &country_roster,
        )?;
        let (visibility, province_visibility) =
            prepare_visibility(context, raster.province_is_eligible_land.len())?;
        let (channel1, channel2) =
            synthesize_gradient_border_channels(&raster, self.content.defines)?;
        let (intel_width, intel_height, intel_bytes) = exact_a729_intel_map(
            &raster.province_ids,
            raster.map_width,
            raster.map_height,
            &raster.province_is_eligible_land,
            &province_visibility,
            visibility.fow_enabled,
        )?;
        let mut prepared = build_prepared_frame(
            context.world_frame.hour_index(),
            self.content.ordered_used_content.clone(),
            self.content.defines,
            self.content.channel3_bgra8_srgb,
            political,
            channel1,
            channel2,
            intel_width,
            intel_height,
            intel_bytes,
            visibility,
            excluded_presentation_identities(&presentation),
        )?;
        prepared.political_family_sha256 = political_family_digest(&prepared);
        prepared.fow_family_sha256 = fow_family_digest(&prepared);
        prepared.deterministic_sha256 = prepared_political_digest(&prepared);
        prepared.validate()?;
        runtime.cache_key = Some(cache_key);
        runtime.prepared = Some(prepared.clone());
        Ok(prepared)
    }
}

fn xmap05_semantic_cache_key(
    context: &FoundationFeatureContext<'_>,
    keying: &KeyingTables,
    province_controllers: &BTreeMap<u32, String>,
) -> Result<String> {
    let map = context
        .world_frame
        .domain("map")
        .ok_or("XMAP05 cache identity requires the map domain")?;
    let countries = context
        .world_frame
        .domain("countries")
        .ok_or("XMAP05 cache identity requires the countries domain")?;
    xmap05_semantic_cache_digest(
        context.world_frame.hour_index(),
        context.viewer.observer_country_id.as_deref(),
        keying,
        province_controllers,
        map,
        countries,
        context.world_frame.domain("visibility"),
    )
}

/// The identity must cover what the render READ, so the mode decides whether the
/// visibility domain is an input at all. That scoping lives HERE rather than at
/// the call site: an observer digest is blind to the recorded vector as a
/// property of the digest, not of who called it.
fn xmap05_semantic_cache_digest(
    hour_index: i64,
    observer_country_id: Option<&str>,
    keying: &KeyingTables,
    province_controllers: &BTreeMap<u32, String>,
    map: &serde_json::Value,
    countries: &serde_json::Value,
    visibility: Option<&serde_json::Value>,
) -> Result<String> {
    let visibility = match observer_country_id {
        None => None,
        Some(_) => Some(
            visibility.ok_or("XMAP05 cache identity requires the visibility domain in POV mode")?,
        ),
    };
    let mut bytes = b"XMAP05_SEMANTIC_CACHE_V3\0".to_vec();
    bytes.extend_from_slice(&hour_index.to_le_bytes());
    bytes.extend_from_slice(observer_country_id.unwrap_or("<observer>").as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(format!("{keying:?}\0{province_controllers:?}\0").as_bytes());
    for (label, value) in [
        ("map", Some(map)),
        ("countries", Some(countries)),
        ("visibility", visibility),
    ] {
        bytes.extend_from_slice(label.as_bytes());
        bytes.push(0);
        match value {
            Some(value) => bytes.extend_from_slice(&serde_json::to_vec(value)?),
            None => bytes.extend_from_slice(b"<unread>"),
        }
        bytes.push(0);
    }
    Ok(Sha256::hex(&bytes))
}

pub(crate) struct Xmap05PoliticalAdapter {
    shared: Arc<Xmap05Shared>,
}

pub(crate) struct Xmap05FowAdapter {
    shared: Arc<Xmap05Shared>,
}

pub(crate) fn adapters(
    resolver: &GameFileResolver,
    admission: &ValidatedContentAdmission,
) -> Result<(Xmap05PoliticalAdapter, Xmap05FowAdapter)> {
    let shared = Arc::new(Xmap05Shared {
        content: Arc::new(Xmap05Content::admit(resolver, admission)?),
        runtime: Mutex::new(Xmap05Runtime {
            cache_key: None,
            prepared: None,
        }),
    });
    Ok((
        Xmap05PoliticalAdapter {
            shared: Arc::clone(&shared),
        },
        Xmap05FowAdapter { shared },
    ))
}

impl ProductionMapFeatureAdapter for Xmap05PoliticalAdapter {
    fn family(&self) -> MapVisualFamily {
        MapVisualFamily::PoliticalFillGradients
    }

    fn prepare(&self, context: &FoundationFeatureContext<'_>) -> Result<FoundationFeatureOutput> {
        // Owned-input accounting first, and identically in both states: an
        // owned row that has been PROVED is admitted rather than gapped, and a
        // gap-only form aborts the whole foundation graph the hour it is proved.
        let consumed_gap_ids = crate::account_owned_map_authority_inputs(
            "XMAP05",
            &POLITICAL_AUTHORITY_GAPS,
            context.map_authority_gaps,
            context.map_authority_admitted,
        )?;
        let prepared = self.shared.prepare(context)?;
        let element_diagnostics = prepared.excluded_presentation_identities.clone();
        Ok(FoundationFeatureOutput {
            disposition: FamilyDisposition::Present {
                payload_schema: XMAP05_POLITICAL_SCHEMA.into(),
                producer_registration_sha256: registration_digest(XMAP05_POLITICAL_SCHEMA),
                payload_sha256: prepared.political_family_sha256.clone(),
                ordered_operations: vec![
                    operation(
                        "political.colors.prepare",
                        2,
                        &[&prepared.province_primary, &prepared.province_secondary],
                    ),
                    operation(
                        "political.gradient-border.prepare",
                        3,
                        &[
                            &prepared.gradient_border_channel1,
                            &prepared.gradient_border_channel2,
                            &prepared.gradient_border_channel3,
                        ],
                    ),
                ],
                ordered_used_content: prepared.ordered_used_content.clone(),
            },
            prepared_terrain: None,
            prepared_political: Some(prepared),
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
            prepared_map_furniture: None,
            element_diagnostics,
            consumed_gap_ids,
        })
    }
}

impl ProductionMapFeatureAdapter for Xmap05FowAdapter {
    fn family(&self) -> MapVisualFamily {
        MapVisualFamily::FogOfWar
    }

    fn prepare(&self, context: &FoundationFeatureContext<'_>) -> Result<FoundationFeatureOutput> {
        let consumed_gap_ids = crate::account_owned_map_authority_inputs(
            "XMAP05",
            &FOW_AUTHORITY_GAPS,
            context.map_authority_gaps,
            context.map_authority_admitted,
        )?;
        let prepared = self.shared.prepare(context)?;
        Ok(FoundationFeatureOutput {
            disposition: FamilyDisposition::Present {
                payload_schema: XMAP05_VISIBILITY_SCHEMA.into(),
                producer_registration_sha256: registration_digest(XMAP05_VISIBILITY_SCHEMA),
                payload_sha256: prepared.fow_family_sha256.clone(),
                ordered_operations: vec![operation(
                    "visibility.intel-map.prepare",
                    4,
                    &[&prepared.intel_map],
                )],
                ordered_used_content: FOW_REQUIRED_SHADER_SOURCES
                    .iter()
                    .map(|path| {
                        prepared
                            .ordered_used_content
                            .iter()
                            .find(|content| {
                                content
                                    .normalized_game_relative_path
                                    .eq_ignore_ascii_case(path)
                            })
                            .cloned()
                            .ok_or_else(|| {
                                format!("XMAP05 FoW closure omitted admitted shader {path}").into()
                            })
                    })
                    .collect::<Result<Vec<_>>>()?,
            },
            prepared_terrain: None,
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
            prepared_map_furniture: None,
            element_diagnostics: Vec::new(),
            consumed_gap_ids,
        })
    }
}

/// What XMAP05's frame-pure admission found this hour.
///
/// # Why this is public API and not a probe
///
/// All eight XMAP05 rows in the XMAP00 ledger
/// (`non_ui_visual_input_ledger_20260730.tsv`) carry the disposition
/// `unavailable_until_owner_proves`, so promoting them to owner-proven is the
/// owner's decision, not a lane's. What a lane owes that decision is the
/// evidence: on a real recording, how many hours would actually admit, and for
/// the ones that would not, what is missing.
///
/// This is that predicate. It is exactly the half of [`admitted_epoch_keying`]
/// that reads only the `WorldFrame` - which is the half a
/// `MapAuthorityOwnerProvenAdmission::status` function is allowed to use, since
/// `RequirementProjector` holds nothing else. If the rows are promoted, the
/// status functions call this; until then it is what measures whether they
/// could be.
///
/// It does NOT cover the fog-of-war half of the family: reading one POV's
/// vector needs the province count from `map/provinces.bmp`, so it is not
/// derivable from a frame alone. The POV-census half that IS frame-pure is
/// reported separately.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Xmap05AdmissionVerdict {
    /// Roster countries whose presentation identity this hour certified.
    pub certified_countries: usize,
    /// Roster countries excluded at country grain, named. Empty is the ordinary
    /// case; a short list is the case MAPC1 v2 exists for.
    pub excluded_countries: Vec<String>,
    pub states: usize,
    pub in_state_provinces: usize,
    /// Whether the visibility POV census equals the complete D18 roster, which
    /// is the frame-pure precondition of the four `fog.*` rows. `None` when the
    /// visibility root is not Present at all.
    pub visibility_povs_match_roster: Option<bool>,
    /// The emitted `countries` keys this hour's D18 read could not name. Empty
    /// on an ordinary hour; on `captures/oz_tip` it is `["367"]` for 469 hours.
    pub roster_unresolved_countries: Vec<String>,
}

/// Run XMAP05's frame-pure admission and report what it found, or why it refused.
pub fn xmap05_admission_verdict(
    frame: &WorldFrame,
) -> std::result::Result<Xmap05AdmissionVerdict, String> {
    let (_, province_controllers, presentation) =
        admitted_epoch_keying(frame).map_err(|error| error.to_string())?;
    let split = xmap05_roster(frame).map_err(|error| error.to_string())?;
    let visibility_povs_match_roster = if frame.visibility_status() == ViewStatus::Present {
        // Against the RESOLVED roster AND with no unresolved key outstanding: a
        // POV set that equals the countries this hour could name is still not
        // all-country visibility while a country the frame carries has no name,
        // so reporting `true` there would overstate what the recording holds.
        let roster = split
            .resolved
            .iter()
            .map(|entry| entry.tag.clone())
            .collect::<BTreeSet<_>>();
        let povs = frame
            .visibility_povs()
            .map(str::to_string)
            .collect::<BTreeSet<_>>();
        Some(povs == roster && split.excluded.is_empty())
    } else {
        None
    };
    Ok(Xmap05AdmissionVerdict {
        certified_countries: presentation.certified.len(),
        excluded_countries: presentation.excluded.clone(),
        roster_unresolved_countries: split.excluded.clone(),
        states: frame.owner_by_state().len(),
        in_state_provinces: province_controllers.len(),
        visibility_povs_match_roster,
    })
}

pub(crate) fn admitted_epoch_keying(
    frame: &WorldFrame,
) -> Result<(KeyingTables, BTreeMap<u32, String>, PresentationCensus)> {
    validate_mapc1_authority(frame)?;
    let tables = xmap05_certified_tables(frame)?;
    validate_mapc1_payload(frame, &tables)?;
    Ok((
        KeyingTables::from_world_frame(frame),
        tables.province_controllers,
        tables.presentation,
    ))
}

/// The frame's own control/roster/presentation census, derived before the MAPC1
/// envelope is consulted at all.
pub(crate) struct Xmap05CertifiedTables {
    owners: BTreeMap<u32, String>,
    controllers: BTreeMap<u32, String>,
    province_controllers: BTreeMap<u32, String>,
    roster: Vec<replay_core::D18CountryRosterEntry>,
    roster_excluded: Vec<String>,
    presentation: PresentationCensus,
}

fn xmap05_certified_tables(frame: &WorldFrame) -> Result<Xmap05CertifiedTables> {
    for (label, status) in [
        ("owner_by_state", frame.owner_by_state_status()),
        ("control_by_state", frame.control_by_state_status()),
        ("control_by_province", frame.control_by_province_status()),
        (
            "country presentation identity",
            frame.country_identities_status(),
        ),
    ] {
        if status != ViewStatus::Present {
            return Err(format!("XMAP05 {label} is {status:?}, not Present").into());
        }
    }

    let Xmap05RosterSplit {
        resolved: roster_entries,
        excluded: roster_excluded,
    } = xmap05_roster(frame)?;
    // One country's unmeasured cosmetic identity used to refuse the WHOLE political fill.
    //
    // The old comment here argued that was the only honest reading, because the cosmetic
    // identity was one term of a single hash over the SORTED WHOLE ROSTER: leaving a country
    // out changed the derived digest, so a partial either failed anyway or, if the comparison
    // were relaxed to match, stopped checking the thing it existed to check. That argument was
    // correct ABOUT THE V1 FORMAT, and it named its own fix - "if the recorder emitted a
    // per-country presentation digest instead, one country's missing cosmetic tag could be
    // excluded at country grain and the rest of the map would fill".
    //
    // MAPC1 v2 is that format, and this is that fix. The recorder now certifies the countries
    // whose cosmetic identity it measured and NAMES the ones it could not, with the excluded
    // census inside the digest preimage - so shrinking the certified set is not a way to make
    // a mismatch pass, and the equality still covers every country. The viewer derives its own
    // split from its own frame (`mapc1_presentation_census`) and the two are compared, so the
    // producer does not get to choose which countries are checked.
    //
    // What draws: every certified country in its measured colour, and each excluded country in
    // its BASE colour - which is exactly what the engine draws for a country with no cosmetic
    // override, and what `KeyingTables::cosmetic_by_tag` already produced for it, since that
    // table only ever carried `Leaf::Measured(Some(..))`. One `ElementDiagnostic` row per
    // excluded country accounts for the identity that did not draw. A malformed measurement -
    // a blank override, an unobtained leaf - is still a whole-frame refusal; only a missing
    // measurement is excludable.
    //
    // MAPC1 v3 does the same thing one level up, for the same reason. A country
    // the recorder could not NAME — no `_r2_tag_db_id`, so the fold kept its
    // numeric key — used to take the whole D18 roster to `DomainUnavailable`,
    // and with it this entire family. On `captures/oz_tip` that was 469 of 535
    // hours over one country that owns 0 of 1,081 states and controls 0 of
    // 10,272 provinces. It is now excluded at country grain, named in
    // `roster_identity_excluded`, and inside the roster digest preimage. What
    // makes that safe rather than a widened blind spot is
    // `validate_xmap05_state_authority`: every state owner, state controller
    // and province controller TAG must be in the RESOLVED roster, so a control
    // row naming an excluded country still refuses the frame by name.
    let presentation = mapc1_presentation_census(frame, &roster_entries)?;

    fn measured_tags(
        label: &str,
        rows: &BTreeMap<u32, Leaf<String>>,
    ) -> Result<BTreeMap<u32, String>> {
        let mut out = BTreeMap::new();
        for (&id, leaf) in rows {
            match leaf {
                Leaf::Measured(tag) if !tag.trim().is_empty() => {
                    out.insert(id, tag.clone());
                }
                Leaf::Measured(_) | Leaf::Absent | Leaf::Unobtained => {
                    return Err(
                        format!("XMAP05 {label} row {id} is empty, absent, or unobtained").into(),
                    );
                }
            }
        }
        if out.is_empty() {
            return Err(format!("XMAP05 {label} census is empty").into());
        }
        Ok(out)
    }

    let owners = measured_tags("owner_by_state", frame.owner_by_state())?;
    let controllers = measured_tags("control_by_state", frame.control_by_state())?;
    if owners.keys().ne(controllers.keys()) {
        return Err(
            "XMAP05 owner_by_state and control_by_state do not cover the same state census".into(),
        );
    }
    let province_controllers = measured_tags("control_by_province", frame.control_by_province())?;
    // Every control TAG must be a RESOLVED roster country. This is what makes
    // roster exclusion safe at country grain: a country the recording could not
    // name owns and controls nothing, so a control row naming it refuses the
    // frame instead of drawing an unnameable country.
    //
    // It lives HERE, in the frame-pure certifier, and not only in
    // `political_color_synthesis::validate_xmap05_state_authority`: that one runs
    // in the render path, long after the four XMAP00 ledger rows have reported
    // Present, so without this a frame could be certified for a TAG the roster
    // does not carry (Codex round 1).
    let resolved = roster_entries
        .iter()
        .map(|entry| entry.tag.as_str())
        .collect::<BTreeSet<_>>();
    for (label, rows) in [
        ("owner_by_state", &owners),
        ("control_by_state", &controllers),
        ("control_by_province", &province_controllers),
    ] {
        for (&id, tag) in rows {
            if !resolved.contains(tag.as_str()) {
                return Err(format!(
                    "XMAP05 {label} row {id} names {tag}, which is not a resolved D18 roster \
                     country this hour"
                )
                .into());
            }
        }
    }
    Ok(Xmap05CertifiedTables {
        owners,
        controllers,
        province_controllers,
        roster: roster_entries,
        roster_excluded,
        presentation,
    })
}

/// Mint the MAPC1 v3 control-authority envelope this frame's own census implies.
///
/// The recorder is the production producer
/// (`recorder_host/.../xmap05_control_authority.rs`); this is the same derivation
/// over a `WorldFrame`, so a test frame can carry a digest-correct envelope
/// without any digest being typed by hand.
/// `mapc1_minted_envelope_is_the_producer_contract_fixtures_own_authority` pins it
/// to the producer contract fixture.
pub fn mapc1_control_authority_envelope(
    frame: &WorldFrame,
    capture_sequence: u64,
) -> std::result::Result<serde_json::Value, String> {
    let tables = xmap05_certified_tables(frame).map_err(|error| error.to_string())?;
    Ok(serde_json::Value::Object(mapc1_envelope_from_tables(
        frame.hour_index(),
        capture_sequence,
        &tables,
    )))
}

fn mapc1_envelope_from_tables(
    frame_hour: i64,
    capture_sequence: u64,
    tables: &Xmap05CertifiedTables,
) -> serde_json::Map<String, serde_json::Value> {
    use serde_json::json;
    let mut envelope = serde_json::Map::new();
    for (key, value) in [
        ("schema", json!("MAPC1")),
        ("schema_version", json!(3)),
        (
            "producer",
            json!("presentation-independent owner/controller epoch"),
        ),
        ("executable_sha256", json!(MAPC1_EXECUTABLE_SHA256)),
        ("ghidra_manifest_sha256", json!(MAPC1_GHIDRA_MANIFEST_SHA256)),
        ("province_controller_field_offset", json!("0x188")),
        ("country_remap_rva", json!("0x00ba6240")),
        ("state_identity_order", json!("state_id_ascending")),
        ("province_identity_order", json!("province_id_ascending")),
        ("presentation_independent", json!(true)),
        ("state_owner_complete", json!(true)),
        ("state_controller_complete", json!(true)),
        ("province_controller_complete", json!(true)),
        ("frame_hour", json!(frame_hour)),
        ("capture_sequence", json!(capture_sequence)),
        ("state_count", json!(tables.owners.len())),
        ("province_count", json!(tables.province_controllers.len())),
        (
            "country_count",
            json!(tables.roster.len() + tables.roster_excluded.len()),
        ),
        (
            "state_owner_sha256",
            json!(mapc1_tag_table_digest("state_owner", &tables.owners)),
        ),
        (
            "state_controller_sha256",
            json!(mapc1_tag_table_digest(
                "state_controller",
                &tables.controllers
            )),
        ),
        (
            "province_controller_sha256",
            json!(mapc1_tag_table_digest(
                "province_controller",
                &tables.province_controllers
            )),
        ),
        (
            "country_roster_sha256",
            json!(mapc1_roster_digest(
                &tables.roster,
                &tables.roster_excluded
            )),
        ),
        (
            "presentation_identity_sha256",
            json!(mapc1_presentation_digest(&tables.presentation)),
        ),
        (
            "presentation_identity_excluded",
            json!(tables.presentation.excluded),
        ),
        ("roster_identity_excluded", json!(tables.roster_excluded)),
    ] {
        envelope.insert(key.to_string(), value);
    }
    envelope
}

/// One accounted row per country whose presentation identity this hour did not
/// carry.
///
/// # Why the outcome is `DrawnFromDocumentedSubstitute` and not an exclusion
///
/// The first draft of this used `ExcludedUnknownInput` and called the excluded
/// element "the country's cosmetic identity", on the reasoning that the identity
/// genuinely did not reach the frame while the fill did. The adversarial review
/// refuted it, and the refutation is recorded here rather than edited away:
/// `ExcludedUnknownInput` means the element was NOT DRAWN, and this country's
/// political fill IS drawn — `KeyingTables::cosmetic_by_tag` simply has no entry
/// for it, so `apply_cosmetic_color_overrides` leaves the base colour in place
/// and `render_political_texels` paints it.
///
/// That base colour is not a measurement. It is what the engine draws for a
/// country with NO cosmetic override, and this country may have had one — D18
/// could not read the field, so the recording does not say. Those pixels may
/// therefore differ from the game's, and a row claiming the element was excluded
/// would be false about pixels that exist while a row claiming it was vouched
/// would be false about where they came from.
///
/// So the outcome says the true thing: the element drew, from a documented
/// substitute, because this hour's recording lacked the input. It counts against
/// completeness (it is not structural) and it vanishes from an hour that carried
/// the field.
///
/// The alternative — excluding the country's territory from the fill — was
/// considered and rejected: it would delete a country's land from the map over a
/// naming field, which is further from the engine's behaviour, not closer.
fn excluded_presentation_identities(census: &PresentationCensus) -> Vec<ElementDiagnostic> {
    census
        .excluded
        .iter()
        .map(|tag| ElementDiagnostic {
            family: MapVisualFamily::PoliticalFillGradients,
            owner_packet: MapVisualFamily::PoliticalFillGradients
                .owner_packet()
                .into(),
            element_id: format!("XMAP05:presentation-identity:{tag}"),
            source_ordinal: None,
            outcome: ElementOutcome::DrawnFromDocumentedSubstitute,
            // Deliberately empty. `part` is reserved for the structural
            // family-exception outcome, and it is also the collapse bound's
            // blind spot: `collapse_family` refuses to aggregate any row that
            // names a part. With hundreds of excluded countries these rows MUST
            // be collapsible, so what was substituted is named in the element id
            // and spelled out in the detail instead.
            part: String::new(),
            element_count: 1,
            detail: format!(
                "country {tag} has no measured cosmetic identity this hour (D18 could not read \
                 the field, so the recording carries no cosmetic_tag for it). Its political \
                 fill IS drawn, in its base country colour — the documented substitute, and \
                 what the engine itself draws for a country with no cosmetic override. If this \
                 country did hold an override this hour, its fill differs from the game's; the \
                 recording cannot say which, so these pixels are substituted rather than \
                 measured"
            ),
        })
        .collect()
}

/// The D18 roster split this family works from.
///
/// Owned rather than borrowed because `d18_country_roster()` builds a fresh view
/// per call and the split has to outlive it.
pub(crate) struct Xmap05RosterSplit {
    pub(crate) resolved: Vec<replay_core::D18CountryRosterEntry>,
    /// The emitted `countries` keys with no D18 identity this hour, ascending.
    /// Carried, never dropped: they ride `roster_identity_excluded` on the
    /// envelope and sit inside the roster digest preimage.
    pub(crate) excluded: Vec<String>,
}

/// PARTIAL, not complete, and this is the whole point of MAPC1 v3.
///
/// XMAP05 is honest per country: the political fill, the three control tables
/// and the presentation identity are all per-country facts, and
/// `validate_xmap05_state_authority` independently requires every control TAG to
/// be a RESOLVED roster country. So a country the recorder could not name costs
/// exactly itself — it draws nothing, colours nothing, and is named on the
/// wire — instead of costing the map.
///
/// `Err` only when the read produced no usable roster at all.
fn xmap05_roster(frame: &WorldFrame) -> Result<Xmap05RosterSplit> {
    let roster = frame.d18_country_roster();
    match roster.partial() {
        Ok(partial) => Ok(Xmap05RosterSplit {
            resolved: partial.resolved.to_vec(),
            excluded: partial.excluded.to_vec(),
        }),
        Err(refusal) => {
            Err(format!("XMAP05 D18 country roster is unusable: {refusal}").into())
        }
    }
}

fn validate_mapc1_authority(frame: &WorldFrame) -> Result<()> {
    let authority = mapc1_authority(frame)?;
    validate_mapc1_authority_object(authority, frame.hour_index())
}

fn mapc1_authority(frame: &WorldFrame) -> Result<&serde_json::Map<String, serde_json::Value>> {
    frame
        .domain("map")
        .and_then(serde_json::Value::as_object)
        .and_then(|map| map.get("_r2_xmap05_control_authority"))
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| "XMAP05 MAPC1 control authority is absent".into())
}

fn validate_mapc1_authority_object(
    authority: &serde_json::Map<String, serde_json::Value>,
    frame_hour: i64,
) -> Result<()> {
    let exact = |key: &str, expected: &str| {
        authority.get(key).and_then(serde_json::Value::as_str) == Some(expected)
    };
    if !exact("schema", "MAPC1")
        // v3, and no earlier version is accepted: each spells its digests over a
        // different preimage, so an older envelope read as v3 would compare
        // digests taken over different sets and refuse every frame with a digest
        // mismatch instead of naming the version. Pinning the integer makes the
        // refusal say what is actually wrong. v3 adds `roster_identity_excluded`
        // and puts it inside `country_roster_sha256`'s preimage.
        || authority
            .get("schema_version")
            .and_then(serde_json::Value::as_u64)
            != Some(3)
        || !exact(
            "producer",
            "presentation-independent owner/controller epoch",
        )
        || !exact("executable_sha256", MAPC1_EXECUTABLE_SHA256)
        || !exact("province_controller_field_offset", "0x188")
        || !exact("country_remap_rva", "0x00ba6240")
        || !exact("ghidra_manifest_sha256", MAPC1_GHIDRA_MANIFEST_SHA256)
        || !exact("state_identity_order", "state_id_ascending")
        || !exact("province_identity_order", "province_id_ascending")
        || authority
            .get("presentation_independent")
            .and_then(serde_json::Value::as_bool)
            != Some(true)
        || authority
            .get("state_owner_complete")
            .and_then(serde_json::Value::as_bool)
            != Some(true)
        || authority
            .get("state_controller_complete")
            .and_then(serde_json::Value::as_bool)
            != Some(true)
        || authority
            .get("province_controller_complete")
            .and_then(serde_json::Value::as_bool)
            != Some(true)
        || authority
            .get("frame_hour")
            .and_then(serde_json::Value::as_i64)
            != Some(frame_hour)
        || !authority
            .get("capture_sequence")
            .and_then(serde_json::Value::as_u64)
            .is_some_and(|value| value != 0)
    {
        return Err("XMAP05 MAPC1 control authority envelope mismatch".into());
    }
    Ok(())
}

fn validate_mapc1_payload(frame: &WorldFrame, tables: &Xmap05CertifiedTables) -> Result<()> {
    let authority = mapc1_authority(frame)?;
    validate_mapc1_payload_tables(
        authority,
        &tables.owners,
        &tables.controllers,
        &tables.province_controllers,
        &tables.roster,
        &tables.roster_excluded,
        &tables.presentation,
    )
}

fn validate_mapc1_payload_tables(
    authority: &serde_json::Map<String, serde_json::Value>,
    owners: &BTreeMap<u32, String>,
    controllers: &BTreeMap<u32, String>,
    province_controllers: &BTreeMap<u32, String>,
    roster: &[replay_core::D18CountryRosterEntry],
    roster_excluded: &[String],
    census: &PresentationCensus,
) -> Result<()> {
    // The producer's declared exclusion list against the viewer's own. This is
    // checked BEFORE the digests and reported separately, because the two say
    // different things: a digest mismatch is "the values disagree", while this is
    // "the two sides disagree about which countries were measured at all", and
    // only the second can name the countries usefully. The digest binds the same
    // list, so this check is not what makes the split tamper-evident - it is what
    // makes the failure legible.
    //
    // REQUIRED and strictly typed. The first draft read this with
    // `filter_map(as_str).unwrap_or_default()`, which meant a missing key, a key
    // of the wrong type, and `["ICE", null, "URG"]` all produced a list that
    // could still match — so on an ordinary frame (no exclusions) the field
    // could be deleted outright and nothing noticed. A field the certificate
    // depends on has to be absent-intolerant, or it is not part of the
    // certificate.
    let Some(declared_rows) = authority
        .get("presentation_identity_excluded")
        .and_then(serde_json::Value::as_array)
    else {
        return Err(
            "XMAP05 MAPC1 envelope has no presentation_identity_excluded array".into(),
        );
    };
    let mut declared_excluded: Vec<String> = Vec::with_capacity(declared_rows.len());
    for row in declared_rows {
        let Some(tag) = row.as_str() else {
            return Err(
                "XMAP05 MAPC1 presentation_identity_excluded carries a non-string member".into(),
            );
        };
        declared_excluded.push(tag.to_string());
    }
    if declared_excluded != census.excluded {
        return Err(format!(
            "XMAP05 MAPC1 presentation-identity exclusions disagree: the recorder declared {:?} \
             and this frame carries {:?}",
            declared_excluded, census.excluded
        )
        .into());
    }
    // The producer's declared ROSTER exclusions against the viewer's own, in the
    // same shape and for the same reason as the presentation split above: this
    // is "the two sides disagree about which countries could be named at all",
    // which the digest also binds but cannot say legibly. REQUIRED and strictly
    // typed - an absent-tolerant read of it would let the field be deleted on an
    // ordinary frame and nothing would notice, which is the finding the v2
    // review made against `presentation_identity_excluded`.
    let Some(declared_roster_rows) = authority
        .get("roster_identity_excluded")
        .and_then(serde_json::Value::as_array)
    else {
        return Err("XMAP05 MAPC1 envelope has no roster_identity_excluded array".into());
    };
    let mut declared_roster_excluded: Vec<String> = Vec::with_capacity(declared_roster_rows.len());
    for row in declared_roster_rows {
        let Some(key) = row.as_str() else {
            return Err(
                "XMAP05 MAPC1 roster_identity_excluded carries a non-string member".into(),
            );
        };
        declared_roster_excluded.push(key.to_string());
    }
    if declared_roster_excluded != roster_excluded {
        return Err(format!(
            "XMAP05 MAPC1 roster-identity exclusions disagree: the recorder declared {:?} and \
             this frame carries {:?}",
            declared_roster_excluded, roster_excluded
        )
        .into());
    }
    let presentation_identity_sha256 = mapc1_presentation_digest(census);
    let exact_count = |key: &str, expected: usize| {
        authority
            .get(key)
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            == Some(expected)
    };
    let state_owner_sha256 = mapc1_tag_table_digest("state_owner", owners);
    let state_controller_sha256 = mapc1_tag_table_digest("state_controller", controllers);
    let province_controller_sha256 =
        mapc1_tag_table_digest("province_controller", province_controllers);
    let country_roster_sha256 = mapc1_roster_digest(roster, roster_excluded);
    if !exact_count("state_count", owners.len())
        || !exact_count("province_count", province_controllers.len())
        // The WHOLE roster, resolved and unresolved alike: `country_count` has
        // always meant "how many countries this frame carries", and dropping the
        // unnameable ones from it would hide them from the one field that counts
        // them.
        || !exact_count("country_count", roster.len() + roster_excluded.len())
        || authority
            .get("state_owner_sha256")
            .and_then(serde_json::Value::as_str)
            != Some(state_owner_sha256.as_str())
        || authority
            .get("state_controller_sha256")
            .and_then(serde_json::Value::as_str)
            != Some(state_controller_sha256.as_str())
        || authority
            .get("province_controller_sha256")
            .and_then(serde_json::Value::as_str)
            != Some(province_controller_sha256.as_str())
        || authority
            .get("country_roster_sha256")
            .and_then(serde_json::Value::as_str)
            != Some(country_roster_sha256.as_str())
        || authority
            .get("presentation_identity_sha256")
            .and_then(serde_json::Value::as_str)
            != Some(presentation_identity_sha256.as_str())
    {
        return Err("XMAP05 MAPC1 counts or payload digests mismatch".into());
    }
    Ok(())
}

fn mapc1_tag_table_digest(label: &str, rows: &BTreeMap<u32, String>) -> String {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(label.as_bytes());
    bytes.push(0);
    for (&id, tag) in rows {
        bytes.extend_from_slice(&id.to_le_bytes());
        bytes.extend_from_slice(tag.as_bytes());
        bytes.push(0);
    }
    Sha256::hex(&bytes)
}

/// The roster digest, over the resolved rows AND the keys that could not be
/// resolved to a country identity.
///
/// The excluded keys are in the preimage for exactly the reason the
/// presentation exclusions are: without them a producer could move a country
/// out of the resolved set to make a mismatch disappear and the digest would
/// not notice. They are LENGTH-PREFIXED rather than delimited - the
/// adversarial review of MAPC1 v2 built a real collision against a
/// NUL-delimited preimage, and a roster key is under no more constraint here
/// than a cosmetic tag was there.
///
/// The u32 length prefixes are sound because both sides bound the keys before
/// they reach here: the viewer refuses the WHOLE roster on a key outside
/// [A-Z0-9_] or longer than 32 bytes (`is_contract_valid_country_tag`), and the
/// recorder applies the identical rule in `roster()` before excluding one. A
/// key that could overflow a u32 length is a key neither side admits.
///
/// The resolved rows keep their v2 spelling so an hour with no exclusions
/// differs from v2 only by the trailing section, which is what makes the
/// ordinary case reviewable against the version it replaces.
fn mapc1_roster_digest(
    rows: &[replay_core::D18CountryRosterEntry],
    excluded: &[String],
) -> String {
    let mut ordered = rows.iter().collect::<Vec<_>>();
    ordered.sort_by(|left, right| left.tag.cmp(&right.tag));
    let mut bytes = b"country_roster\0".to_vec();
    for row in ordered {
        bytes.extend_from_slice(row.tag.as_bytes());
        bytes.push(0);
        bytes.extend_from_slice(&row.tag_db_id.to_le_bytes());
    }
    let mut ordered_excluded = excluded.to_vec();
    ordered_excluded.sort();
    bytes.extend_from_slice(b"country_roster_unresolved\0");
    bytes.extend_from_slice(&(ordered_excluded.len() as u32).to_le_bytes());
    for key in &ordered_excluded {
        push_sized(&mut bytes, key);
    }
    Sha256::hex(&bytes)
}

/// The viewer's own certified/excluded split of the roster's presentation
/// identity, TAG-ascending.
///
/// Derived from the frame alone. The MAPC1 envelope's
/// `presentation_identity_excluded` is then compared against
/// [`PresentationCensus::excluded`] rather than trusted as the source: if the
/// producer's split and the viewer's disagree, the two sides are not looking at
/// the same measurement and the frame refuses, naming the countries.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct PresentationCensus {
    /// TAG -> the certified cosmetic override, or `None` for a measured
    /// "no override".
    certified: BTreeMap<String, Option<String>>,
    /// TAGs whose cosmetic identity this recording did not carry.
    excluded: Vec<String>,
}

fn mapc1_presentation_census(
    frame: &WorldFrame,
    roster: &[replay_core::D18CountryRosterEntry],
) -> Result<PresentationCensus> {
    mapc1_presentation_census_from(roster, |tag| {
        frame
            .country_identity(tag)
            .map(|identity| &identity.cosmetic_tag)
    })
}

/// What this frame carries for one country's presentation identity.
///
/// Three states, because the recorder's MAPC1 v2 envelope certifies at country
/// grain and the viewer has to derive the SAME split from the SAME frame fact.
/// The split is never taken from the envelope — that would let the producer
/// choose which countries get checked — it is derived here and then compared.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CosmeticAdmission<'a> {
    /// A measured cosmetic override; the country draws in its cosmetic colour.
    Override(&'a str),
    /// Measured, and the engine holds no override; the country draws in its base
    /// colour and that is exact.
    NoOverride,
    /// The recording never carried this country's cosmetic identity — D18 could
    /// not read the field, so the snapshot emits no `cosmetic_tag` key.
    ///
    /// EXCLUDABLE AT COUNTRY GRAIN. It is not an error: the country's identity
    /// and its control rows are separate measurements this hole says nothing
    /// about, so the fill still draws it in its base colour and one
    /// `ElementDiagnostic` row accounts for the cosmetic identity that did not.
    Unmeasured,
}

/// Classify one country's cosmetic identity, or refuse the frame.
///
/// The two `Err` arms are MALFORMED measurements, not missing ones, and stay
/// whole-frame refusals: a blank override means the read produced something that
/// cannot be a cosmetic tag (marker 1 already spells "no override" as null), and
/// an unobtained leaf means the decode itself failed. Excluding either at
/// country grain would dress a broken read as absent data.
pub(crate) fn admitted_cosmetic_identity<'a>(
    leaf: Option<&'a Leaf<Option<String>>>,
) -> std::result::Result<CosmeticAdmission<'a>, String> {
    match leaf {
        Some(Leaf::Measured(Some(tag))) if !tag.trim().is_empty() => {
            Ok(CosmeticAdmission::Override(tag.as_str()))
        }
        Some(Leaf::Measured(Some(_))) => Err("measured cosmetic override is blank".into()),
        Some(Leaf::Measured(None)) => Ok(CosmeticAdmission::NoOverride),
        Some(Leaf::Absent) | None => Ok(CosmeticAdmission::Unmeasured),
        Some(Leaf::Unobtained) => Err("cosmetic identity is unobtained".into()),
    }
}

fn mapc1_presentation_census_from<'a>(
    roster: &'a [replay_core::D18CountryRosterEntry],
    cosmetic_tag: impl Fn(&str) -> Option<&'a Leaf<Option<String>>>,
) -> Result<PresentationCensus> {
    let mut ordered = roster.iter().collect::<Vec<_>>();
    ordered.sort_by(|left, right| left.tag.cmp(&right.tag));
    let mut census = PresentationCensus::default();
    for row in ordered {
        match admitted_cosmetic_identity(cosmetic_tag(&row.tag)) {
            Ok(CosmeticAdmission::Override(tag)) => {
                census
                    .certified
                    .insert(row.tag.clone(), Some(tag.to_string()));
            }
            Ok(CosmeticAdmission::NoOverride) => {
                census.certified.insert(row.tag.clone(), None);
            }
            Ok(CosmeticAdmission::Unmeasured) => census.excluded.push(row.tag.clone()),
            Err(detail) => {
                return Err(format!(
                    "XMAP05 presentation identity for {}: {detail}",
                    row.tag
                )
                .into());
            }
        }
    }
    if census.certified.is_empty() {
        return Err(format!(
            "XMAP05 presentation identity: no roster country has a measured cosmetic identity \
             (all {} excluded)",
            census.excluded.len()
        )
        .into());
    }
    Ok(census)
}

/// Transcription of the producer's `presentation_digest`
/// (`recorder_host/.../xmap05_control_authority.rs`), v2 layout.
///
/// The excluded census is inside the preimage, so the equality is over BOTH
/// halves of the split: a producer cannot make a mismatch disappear by moving a
/// country out of the certified set, because that move changes the digest and
/// the viewer derived its own split independently.
/// Append one length-prefixed field. See [`mapc1_presentation_digest`].
///
/// The `as u32` cannot truncate: TAGs are charset- and length-validated by the
/// roster, and every D18 string reaching a frame was u16-length-prefixed on the
/// wire, so 65,535 bytes is the structural ceiling for a cosmetic tag. A
/// truncating cast would silently reintroduce the collision class this framing
/// exists to remove.
fn push_sized(bytes: &mut Vec<u8>, field: &str) {
    bytes.extend_from_slice(&(field.len() as u32).to_le_bytes());
    bytes.extend_from_slice(field.as_bytes());
}

fn mapc1_presentation_digest(census: &PresentationCensus) -> String {
    // Length-prefixed, no delimiters. A cosmetic tag is NOT a TAG: nothing
    // constrains its bytes, so a NUL-delimited preimage let one arrangement of
    // cosmetic values impersonate another's framing, and the adversarial review
    // exhibited two different certified censuses with identical digests. Fixing
    // the extent of every string before its bytes are read removes the class.
    let mut bytes = b"presentation_identity_v2".to_vec();
    bytes.extend_from_slice(&(census.certified.len() as u32).to_le_bytes());
    // `certified` is a BTreeMap, so this is TAG-ascending, matching the
    // producer's sorted rows.
    for (tag, cosmetic) in &census.certified {
        push_sized(&mut bytes, tag);
        match cosmetic {
            Some(cosmetic) => {
                bytes.push(1);
                push_sized(&mut bytes, cosmetic);
            }
            None => bytes.push(0),
        }
    }
    bytes.extend_from_slice(&(census.excluded.len() as u32).to_le_bytes());
    for tag in &census.excluded {
        push_sized(&mut bytes, tag);
    }
    Sha256::hex(&bytes)
}

fn operation(
    operation_id: &str,
    global_order: u32,
    textures: &[&Xmap05GeneratedTexture],
) -> FoundationOperationIdentity {
    let mut resources = Vec::new();
    let mut targets = Vec::new();
    for texture in textures {
        resources.extend_from_slice(texture.descriptor.payload_sha256.as_bytes());
        targets.extend_from_slice(texture.descriptor.descriptor_sha256.as_bytes());
    }
    FoundationOperationIdentity {
        operation_id: operation_id.into(),
        global_order,
        program_sha256: registration_digest(operation_id),
        ordered_resources_sha256: Sha256::hex(&resources),
        target_sha256: Sha256::hex(&targets),
    }
}

fn registration_digest(schema: &str) -> String {
    Sha256::hex(format!("XMAP05\0{schema}\0{}", crate::XMAP03_A729_EXECUTABLE_SHA256).as_bytes())
}

fn build_prepared_frame(
    hour_index: i64,
    ordered_used_content: Vec<UsedContentKey>,
    defines: GradientBorderCountryDefines,
    channel3_bgra8_srgb: [u8; 8],
    political: PoliticalColorSynthesis,
    channel1_bytes: Vec<u8>,
    channel2_bytes: Vec<u8>,
    intel_width: usize,
    intel_height: usize,
    intel_bytes: Vec<u8>,
    visibility: PreparedXmap05Visibility,
    excluded_presentation_identities: Vec<ElementDiagnostic>,
) -> Result<PreparedPoliticalFrame> {
    let width = political.width;
    let height = political.height;
    let gb_height = height
        .checked_mul(2)
        .and_then(|value| value.checked_add(2))
        .ok_or("XMAP05 GradientBorder height overflow")?;
    Ok(PreparedPoliticalFrame {
        schema: XMAP05_POLITICAL_SCHEMA.into(),
        hour_index,
        ordered_used_content,
        province_primary: generated_texture(
            "ProvincePrimaryColorMap",
            width,
            height,
            PROVINCE_COLOR_MAP_DXGI_FORMAT,
            4,
            political.owner_color_bytes,
        )?,
        province_secondary: generated_texture(
            crate::PROVINCE_SECONDARY_COLOR_MAP_SYMBOL,
            width,
            height,
            PROVINCE_COLOR_MAP_DXGI_FORMAT,
            4,
            political.secondary_color_bytes,
        )?,
        gradient_border_channel1: generated_texture(
            "GradientBorderChannel1",
            width,
            gb_height,
            GRADIENT_BORDER_CHANNEL_DXGI_FORMAT,
            4,
            channel1_bytes,
        )?,
        gradient_border_channel2: generated_texture(
            "GradientBorderChannel2",
            width,
            gb_height,
            GRADIENT_BORDER_CHANNEL_DXGI_FORMAT,
            4,
            channel2_bytes,
        )?,
        gradient_border_channel3: generated_texture(
            "GradientBorderChannel3",
            2,
            1,
            GRADIENT_BORDER_CHANNEL3_DXGI_FORMAT,
            4,
            channel3_bgra8_srgb.to_vec(),
        )?,
        intel_map: generated_texture(
            "IntelMap",
            intel_width,
            intel_height,
            INTEL_MAP_DXGI_FORMAT,
            1,
            intel_bytes,
        )?,
        visibility,
        political_family_sha256: String::new(),
        fow_family_sha256: String::new(),
        deterministic_sha256: String::new(),
        excluded_presentation_identities,
        scene_cb0_law: Xmap05SceneCb0Law {
            outline_cutoff_strategic_bits: defines.outline_cutoff_strategic.to_bits(),
            outline_cutoff_country_bits: defines.outline_cutoff_country.to_bits(),
            camera_override_strategic_bits: defines.camera_override_strategic.to_bits(),
            camera_override_country_bits: defines.camera_override_country.to_bits(),
            draw_fow_cutoff_bits: defines.draw_fow_cutoff.to_bits(),
            draw_fow_fade_length_bits: defines.draw_fow_fade_length.to_bits(),
        },
    })
}

fn generated_texture(
    symbol: &str,
    width: usize,
    height: usize,
    dxgi_format: u32,
    bytes_per_texel: usize,
    bytes: Vec<u8>,
) -> Result<Xmap05GeneratedTexture> {
    let descriptor =
        Xmap05TextureDescriptor::new(symbol, width, height, dxgi_format, bytes_per_texel, &bytes)?;
    Ok(Xmap05GeneratedTexture {
        descriptor,
        bytes: bytes.into(),
    })
}

/// Which per-country visibility vectors an XMAP05 render consumes.
///
/// The two arms are the two fog states the real game runs in, and the
/// map_viewer's own CAP4 round corpus captures both from it:
/// `fow_on_pov_front` and `fow_off_observer_front`
/// (`fow_law::analyze_wave1_cap4_fow_pair`, gated by `test_gates.rs`'s
/// `wave1_cap4_rounds_dir`). Observer is not the absence of a POV — it is the
/// recording's world truth drawn with no fog layer, which is why it needs no
/// country's visibility vector to be complete.
///
/// Neither arm reads more than one vector, so a census over the whole D18
/// roster is not a precondition of either. Of what that census used to assert,
/// only the KEY-SET half survives, in
/// [`Xmap05AdmissionVerdict::visibility_povs_match_roster`], which reports it
/// rather than refusing on it. Its per-vector-length half is asserted nowhere
/// now, and no code carries it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Xmap05VisibilityMode {
    /// No fog layer: every province at full visibility, no POV, no recorded
    /// vector read.
    Observer,
    /// One country's fog map, which the recording must carry for that country.
    Pov(String),
}

pub fn xmap05_visibility_mode(viewer: &ViewerOwnedMapState) -> Xmap05VisibilityMode {
    match viewer.observer_country_id.as_deref() {
        None => Xmap05VisibilityMode::Observer,
        Some(country) => Xmap05VisibilityMode::Pov(country.to_string()),
    }
}

fn prepare_visibility(
    context: &FoundationFeatureContext<'_>,
    province_slots: usize,
) -> Result<(PreparedXmap05Visibility, Vec<u8>)> {
    let country = match xmap05_visibility_mode(context.viewer) {
        Xmap05VisibilityMode::Observer => return prepare_observer_visibility(province_slots),
        Xmap05VisibilityMode::Pov(country) => country,
    };
    // PARTIAL: one country's PRESENCE, not a census. Whether some other country
    // could be named this hour says nothing about whether this one was.
    let split = xmap05_roster(context.world_frame)?;
    let expected_country_id = split
        .resolved
        .iter()
        .find(|entry| entry.tag == country)
        .map(|entry| entry.tag_db_id)
        .ok_or_else(|| {
            let unnamed = if split.excluded.is_empty() {
                String::new()
            } else {
                format!(
                    " ({} emitted country key(s) carry no D18 identity this hour: {})",
                    split.excluded.len(),
                    split.excluded.join(",")
                )
            };
            format!("XMAP05 viewer POV {country} is absent from the D18 roster{unnamed}")
        })?;
    prepare_country_visibility(
        context.world_frame,
        &country,
        expected_country_id,
        province_slots,
    )
}

/// [`Xmap05VisibilityMode::Observer`]: the constant fully-visible vector.
///
/// `fow_enabled: false` is what `fow_law::exact_a729_intel_map_texel` reads to
/// take `u8::MAX` for every sample without consulting the vector at all, so
/// this vector is the payload's declared identity rather than an input to it.
fn prepare_observer_visibility(
    province_slots: usize,
) -> Result<(PreparedXmap05Visibility, Vec<u8>)> {
    let province_visibility = vec![u8::MAX; province_slots];
    let mut visibility = PreparedXmap05Visibility {
        schema: XMAP05_VISIBILITY_SCHEMA.into(),
        viewer_country: None,
        fow_enabled: false,
        capture_sequence: None,
        pov_country_id: None,
        province_visibility_sha256: Sha256::hex(&province_visibility),
        land_units: Vec::new(),
        taskforce_units: Vec::new(),
        air_wing_spotting_unavailable: true,
        land_frame_sequence: None,
        taskforce_frame_sequence: None,
        deterministic_sha256: String::new(),
    };
    visibility.deterministic_sha256 = visibility_digest(&visibility);
    visibility.validate()?;
    Ok((visibility, province_visibility))
}

fn prepare_country_visibility(
    frame: &WorldFrame,
    country: &str,
    expected_country_id: u32,
    province_slots: usize,
) -> Result<(PreparedXmap05Visibility, Vec<u8>)> {
    let view = frame.visibility_for(country);
    if view.status != ViewStatus::Present {
        // The recording does not carry this POV. The engine-exact derivation
        // law stands ready, but the binding gate is evaluated for THIS hour and
        // the refusal names what it found: the standing residuals, and the
        // recorded operands this frame does not carry.
        let report = fog_derivation_law::fog_binding_refusals(frame, Some(country));
        let absent = report.missing_recorded();
        let absent = if absent.is_empty() {
            String::new()
        } else {
            format!(
                "; and these recorded operands are absent this hour: {}",
                absent.join(", ")
            )
        };
        return Err(format!(
            "XMAP05 viewer POV {country} visibility is {:?}; the recorded capture carries no \
             vector for this country and per-POV fog-of-war derivation refuses by name until \
             these input bindings seal: {}{absent}",
            view.status,
            report.blocked().join(", ")
        )
        .into());
    }
    let Leaf::Measured(province_visibility) = &view.raw_province_levels else {
        return Err("XMAP05 exact d94 province vector is not measured".into());
    };
    if province_visibility.len() != province_slots
        || view.provinces.len() != province_visibility.len()
    {
        return Err(format!(
            "XMAP05 exact d94 province census has {} raw / {} typed slots, expected {province_slots}",
            province_visibility.len(),
            view.provinces.len()
        )
        .into());
    }
    let capture_sequence = measured_nonzero(&view.capture_sequence, "d94 capture sequence")?;
    let pov_country_id = measured_nonzero(&view.pov_country_id, "d94 POV country id")?;
    if pov_country_id != expected_country_id {
        return Err(format!(
            "XMAP05 POV {country} carries country id {pov_country_id}, expected D18 id {expected_country_id}"
        )
        .into());
    }
    let spotted = &view.spotted_units;
    if spotted.status != ViewStatus::Present {
        return Err("XMAP05 exact d95 spotted-unit state is unavailable".into());
    }
    if !matches!(&spotted.wings, Leaf::Unobtained) {
        return Err(
            "XMAP05 SPU1 must authenticate air-wing spotting as unavailable; wing rows are forbidden"
                .into(),
        );
    }
    let (land_rows, land_sequence) = exact_spotted_rows(
        "land",
        &spotted.land_units,
        &spotted.divisions,
        &spotted.land_frame_sequence,
        exact_a729_land_unit_disclosure,
    )?;
    let (taskforce_rows, taskforce_sequence) = exact_spotted_rows(
        "taskforce",
        &spotted.taskforce_units,
        &spotted.fleets,
        &spotted.taskforce_frame_sequence,
        exact_a729_taskforce_disclosure,
    )?;
    let mut visibility = PreparedXmap05Visibility {
        schema: XMAP05_VISIBILITY_SCHEMA.into(),
        viewer_country: Some(country.into()),
        fow_enabled: true,
        capture_sequence: Some(capture_sequence),
        pov_country_id: Some(pov_country_id),
        province_visibility_sha256: Sha256::hex(province_visibility),
        land_units: land_rows,
        taskforce_units: taskforce_rows,
        air_wing_spotting_unavailable: true,
        land_frame_sequence: Some(land_sequence),
        taskforce_frame_sequence: Some(taskforce_sequence),
        deterministic_sha256: String::new(),
    };
    visibility.deterministic_sha256 = visibility_digest(&visibility);
    visibility.validate()?;
    Ok((visibility, province_visibility.clone()))
}

fn measured_nonzero<T>(leaf: &Leaf<T>, label: &str) -> Result<T>
where
    T: Copy + PartialEq + From<u8>,
{
    match leaf {
        Leaf::Measured(value) if *value != T::from(0) => Ok(*value),
        _ => Err(format!("XMAP05 exact {label} is absent, unobtained, or zero").into()),
    }
}

fn exact_spotted_rows(
    label: &str,
    rows: &Leaf<Vec<ExactSpottedUnit>>,
    handles: &Leaf<Vec<String>>,
    sequence: &Leaf<u64>,
    classify: fn(u8) -> ExactUnitDisclosure,
) -> Result<(Vec<Xmap05VisibleEntity>, u64)> {
    let (Leaf::Measured(rows), Leaf::Measured(handles), Leaf::Measured(sequence)) =
        (rows, handles, sequence)
    else {
        return Err(format!("XMAP05 exact d95 {label} surface is incomplete").into());
    };
    if *sequence == 0 || rows.len() != handles.len() {
        return Err(format!("XMAP05 exact d95 {label} sequence/count is invalid").into());
    }
    let mut seen = BTreeSet::new();
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        if row.type_id == 0 || row.stable_id == 0 || !seen.insert((row.type_id, row.stable_id)) {
            return Err(format!("XMAP05 exact d95 {label} identity is zero or duplicated").into());
        }
        let disclosure = classify(row.raw_intel_u8);
        if row.detected != !matches!(disclosure, ExactUnitDisclosure::Hidden)
            || row.map_visible
                != matches!(
                    disclosure,
                    ExactUnitDisclosure::MapVisible | ExactUnitDisclosure::ExactDetails
                )
            || row.exact_details != matches!(disclosure, ExactUnitDisclosure::ExactDetails)
        {
            return Err(
                format!("XMAP05 exact d95 {label} booleans disagree with raw intel byte").into(),
            );
        }
        out.push(Xmap05VisibleEntity {
            type_id: row.type_id,
            stable_id: row.stable_id,
            disclosure,
            reason: disclosure_reason(disclosure),
        });
    }
    out.sort_by_key(|row| (row.type_id, row.stable_id));
    Ok((out, *sequence))
}

fn disclosure_reason(disclosure: ExactUnitDisclosure) -> Xmap05VisibilityReason {
    match disclosure {
        ExactUnitDisclosure::Hidden => Xmap05VisibilityReason::D95MapVisibilityThresholdExcluded,
        ExactUnitDisclosure::Detected => Xmap05VisibilityReason::D95DetectedTier,
        ExactUnitDisclosure::MapVisible => Xmap05VisibilityReason::ExactD95MapVisible,
        ExactUnitDisclosure::ExactDetails => Xmap05VisibilityReason::ExactD95ExactDetails,
    }
}

fn validate_entity_rows(rows: &[Xmap05VisibleEntity]) -> Result<()> {
    let mut previous = None;
    for row in rows {
        let identity = (row.type_id, row.stable_id);
        if row.type_id == 0 || row.stable_id == 0 || previous.is_some_and(|value| value >= identity)
        {
            return Err("XMAP05 visibility rows are zero, duplicated, or unsorted".into());
        }
        if row.reason != disclosure_reason(row.disclosure) {
            return Err("XMAP05 visibility disclosure reason mismatch".into());
        }
        previous = Some(identity);
    }
    Ok(())
}

fn synthesize_gradient_border_channels(
    raster: &Xmap05RasterInputs,
    defines: GradientBorderCountryDefines,
) -> Result<(Vec<u8>, Vec<u8>)> {
    let input = &raster.gradient;
    if input.width == 0
        || input.height == 0
        || raster.map_width != input.width * 2
        || raster.map_height != input.height * 2
        || raster.province_ids.len() != raster.map_width * raster.map_height
        || input.regions.len() != input.width * input.height
        || input.land_mask.len() != input.regions.len()
        || input.region_colors.len() != input.region_areas.len()
        || raster
            .province_ids
            .iter()
            .any(|province| usize::from(*province) >= input.province_regions.len())
    {
        return Err("XMAP05 GradientBorder topology is invalid".into());
    }
    let boundary = country_boundary_points(
        &raster.province_ids,
        &input.province_regions,
        raster.map_width,
        raster.map_height,
    )?;
    let squared_distance =
        squared_euclidean_distance(&boundary, raster.map_width, raster.map_height)?;
    let layer_pixels = input.width * input.height;
    let packed_height = input
        .height
        .checked_mul(2)
        .and_then(|value| value.checked_add(2))
        .ok_or("XMAP05 GradientBorder packed height overflow")?;
    let mut channel1 = vec![0u8; input.width * packed_height * 4];
    let mut channel2 = vec![0u8; input.width * packed_height * 4];
    let bottom_row = input.height + 1;
    for index in 0..layer_pixels {
        let y = index / input.width;
        let x = index % input.width;
        let source = xmap05_half_res_source_index(raster.map_width, raster.map_height, x, y);
        let region = input.province_regions[usize::from(raster.province_ids[source])] as usize;
        if region != input.regions[index] as usize {
            return Err(
                "XMAP05 half-resolution political sample disagrees with full raster".into(),
            );
        }
        if region == 0 || region >= input.region_colors.len() || !input.land_mask[index] {
            continue;
        }
        let area = input.region_areas[region];
        if area == 0 {
            return Err("XMAP05 GradientBorder controller region has zero area".into());
        }
        let thickness = country_gradient_thickness(area, defines)?;
        let distance = (squared_distance[source] as f32).sqrt();
        let alpha = exact_a729_gradient_byte(distance, defines.center_thickness, thickness);
        let packed = ((bottom_row + y) * input.width + x) * 4;
        let color = input.region_colors[region];
        channel1[packed..packed + 4].copy_from_slice(&[color[0], color[1], color[2], alpha]);
        channel2[packed + 1] = u8::MAX;
    }
    Ok((channel1, channel2))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Xmap05MapModeBand {
    Top,
    Bottom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Xmap05MapModeMask {
    Flat,
    CountryDistance,
    StateDistance,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Xmap05EvaluatedGradient {
    pub center: f32,
    pub thickness: f32,
}

pub(crate) fn xmap05_map_mode_derivative(
    resolver: &GameFileResolver,
    context: &FoundationFeatureContext<'_>,
    base: &PreparedPoliticalFrame,
    band: Xmap05MapModeBand,
    mask: Xmap05MapModeMask,
    province_fills: &BTreeMap<u32, Option<(String, [u8; 4])>>,
) -> Result<PreparedPoliticalFrame> {
    xmap05_map_mode_derivative_impl(
        resolver,
        context,
        base,
        band,
        mask,
        province_fills,
        true,
        Xmap05DerivativeEncoding::MapModeColor,
        None,
    )
}

pub(crate) fn xmap05_evaluated_map_mode_derivative(
    resolver: &GameFileResolver,
    context: &FoundationFeatureContext<'_>,
    base: &PreparedPoliticalFrame,
    band: Xmap05MapModeBand,
    mask: Xmap05MapModeMask,
    province_fills: &BTreeMap<u32, Option<(String, [u8; 4])>>,
) -> Result<PreparedPoliticalFrame> {
    xmap05_map_mode_derivative_impl(
        resolver,
        context,
        base,
        band,
        mask,
        province_fills,
        false,
        Xmap05DerivativeEncoding::MapModeColor,
        None,
    )
}

pub(crate) fn xmap05_naval_dominance_derivative(
    resolver: &GameFileResolver,
    context: &FoundationFeatureContext<'_>,
    base: &PreparedPoliticalFrame,
    province_fills: &BTreeMap<u32, Option<(String, [u8; 4])>>,
) -> Result<PreparedPoliticalFrame> {
    xmap05_map_mode_derivative_impl(
        resolver,
        context,
        base,
        Xmap05MapModeBand::Top,
        Xmap05MapModeMask::Flat,
        province_fills,
        false,
        Xmap05DerivativeEncoding::NavalDominance,
        None,
    )
}

pub(crate) fn xmap05_engine_evaluated_scripted_layer_derivative(
    resolver: &GameFileResolver,
    context: &FoundationFeatureContext<'_>,
    base: &PreparedPoliticalFrame,
    band: Xmap05MapModeBand,
    province_fills: &BTreeMap<u32, Option<(String, [u8; 4])>>,
    gradients: &BTreeMap<String, Option<Xmap05EvaluatedGradient>>,
) -> Result<PreparedPoliticalFrame> {
    xmap05_map_mode_derivative_impl(
        resolver,
        context,
        base,
        band,
        Xmap05MapModeMask::Flat,
        province_fills,
        false,
        Xmap05DerivativeEncoding::MapModeColor,
        Some(gradients),
    )
}

pub(crate) fn xmap05_copy_engine_evaluated_band(
    base: &PreparedPoliticalFrame,
    delegate: &PreparedPoliticalFrame,
    band: Xmap05MapModeBand,
) -> Result<PreparedPoliticalFrame> {
    base.validate()?;
    delegate.validate()?;
    if base.hour_index != delegate.hour_index
        || !xmap05_texture_layout_compatible(
            &base.gradient_border_channel1.descriptor,
            &delegate.gradient_border_channel1.descriptor,
        )
        || !xmap05_texture_layout_compatible(
            &base.gradient_border_channel2.descriptor,
            &delegate.gradient_border_channel2.descriptor,
        )
    {
        return Err(
            "XMAP18 hardcoded scripted delegate differs from XMAP05 resource authority".into(),
        );
    }
    let width = usize::try_from(base.gradient_border_channel1.descriptor.width)?;
    let packed_height = usize::try_from(base.gradient_border_channel1.descriptor.height)?;
    let height = packed_height
        .checked_sub(2)
        .ok_or("XMAP18 GradientBorder packed height is invalid")?
        / 2;
    let target_first_row = match band {
        Xmap05MapModeBand::Top => 0,
        Xmap05MapModeBand::Bottom => height + 1,
    };
    let target_start = target_first_row
        .checked_mul(width)
        .and_then(|v| v.checked_mul(4))
        .ok_or("XMAP18 band offset overflow")?;
    let len = height
        .checked_mul(width)
        .and_then(|v| v.checked_mul(4))
        .ok_or("XMAP18 band extent overflow")?;
    let target_end = target_start
        .checked_add(len)
        .ok_or("XMAP18 band end overflow")?;
    let source_start = 0usize;
    let source_end = source_start
        .checked_add(len)
        .ok_or("XMAP18 delegate band end overflow")?;
    let mut derived = base.clone();
    Arc::make_mut(&mut derived.gradient_border_channel1.bytes)[target_start..target_end]
        .copy_from_slice(&delegate.gradient_border_channel1.bytes[source_start..source_end]);
    Arc::make_mut(&mut derived.gradient_border_channel2.bytes)[target_start..target_end]
        .copy_from_slice(&delegate.gradient_border_channel2.bytes[source_start..source_end]);
    copy_engine_evaluated_scene_law_band(&mut derived.scene_cb0_law, &delegate.scene_cb0_law, band);
    derived.gradient_border_channel1.descriptor.payload_sha256 =
        Sha256::hex(&derived.gradient_border_channel1.bytes);
    derived.gradient_border_channel2.descriptor.payload_sha256 =
        Sha256::hex(&derived.gradient_border_channel2.bytes);
    derived
        .gradient_border_channel1
        .descriptor
        .descriptor_sha256 =
        texture_descriptor_digest(&derived.gradient_border_channel1.descriptor);
    derived
        .gradient_border_channel2
        .descriptor
        .descriptor_sha256 =
        texture_descriptor_digest(&derived.gradient_border_channel2.descriptor);
    derived.political_family_sha256 = political_family_digest(&derived);
    derived.fow_family_sha256 = fow_family_digest(&derived);
    derived.deterministic_sha256 = prepared_political_digest(&derived);
    derived.validate()?;
    Ok(derived)
}

fn copy_engine_evaluated_scene_law_band(
    base: &mut Xmap05SceneCb0Law,
    delegate: &Xmap05SceneCb0Law,
    band: Xmap05MapModeBand,
) {
    match band {
        Xmap05MapModeBand::Top => {
            base.outline_cutoff_strategic_bits = delegate.outline_cutoff_strategic_bits;
            base.camera_override_strategic_bits = delegate.camera_override_strategic_bits;
        }
        Xmap05MapModeBand::Bottom => {
            base.outline_cutoff_country_bits = delegate.outline_cutoff_strategic_bits;
            base.camera_override_country_bits = delegate.camera_override_strategic_bits;
        }
    }
}

fn xmap05_texture_layout_compatible(
    left: &Xmap05TextureDescriptor,
    right: &Xmap05TextureDescriptor,
) -> bool {
    left.symbol == right.symbol
        && left.width == right.width
        && left.height == right.height
        && left.dxgi_format == right.dxgi_format
        && left.row_pitch == right.row_pitch
        && left.mip_levels == right.mip_levels
}

pub(crate) fn xmap05_diplomacy_configuration_derivative(
    resolver: &GameFileResolver,
    base: &PreparedPoliticalFrame,
) -> Result<PreparedPoliticalFrame> {
    base.validate()?;
    let source = effective_graphics_defines_source(
        resolver,
        &[
            "GRADIENT_BORDERS_OUTLINE_CUTOFF_DIPLOMACY",
            "GRADIENT_BORDERS_CAMERA_DISTANCE_OVERRIDE_DIPLOMACY",
        ],
        "XMAP18 diplomacy defines",
    )?;
    let outline = unique_define(&source, "GRADIENT_BORDERS_OUTLINE_CUTOFF_DIPLOMACY")?;
    let camera = unique_define(
        &source,
        "GRADIENT_BORDERS_CAMERA_DISTANCE_OVERRIDE_DIPLOMACY",
    )?;
    if !outline.is_finite()
        || !(0.0..=1.0).contains(&outline)
        || !camera.is_finite()
        || !(0.0..=1.0).contains(&camera)
    {
        return Err("XMAP18 diplomacy GradientBorder defines are invalid".into());
    }
    let mut derived = base.clone();
    derived.scene_cb0_law.outline_cutoff_strategic_bits = outline.to_bits();
    derived.scene_cb0_law.camera_override_strategic_bits = camera.to_bits();
    derived.deterministic_sha256 = prepared_political_digest(&derived);
    derived.validate()?;
    Ok(derived)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Xmap05DerivativeEncoding {
    MapModeColor,
    NavalDominance,
}

fn xmap05_map_mode_derivative_impl(
    resolver: &GameFileResolver,
    context: &FoundationFeatureContext<'_>,
    base: &PreparedPoliticalFrame,
    band: Xmap05MapModeBand,
    mask: Xmap05MapModeMask,
    province_fills: &BTreeMap<u32, Option<(String, [u8; 4])>>,
    require_complete_land: bool,
    encoding: Xmap05DerivativeEncoding,
    explicit_gradients: Option<&BTreeMap<String, Option<Xmap05EvaluatedGradient>>>,
) -> Result<PreparedPoliticalFrame> {
    base.validate()?;
    if base.hour_index != context.world_frame.hour_index() {
        return Err("XMAP18 derivative and XMAP05 base have different hours".into());
    }
    let (keying, province_controllers, _presentation) =
        admitted_epoch_keying(context.world_frame)?;
    // RESOLVED, for the same reason as `Xmap05Shared::prepare`: this is the set
    // the state-authority validator checks every control TAG against.
    let country_roster = xmap05_roster(context.world_frame)?
        .resolved
        .into_iter()
        .map(|entry| entry.tag)
        .collect::<BTreeSet<_>>();
    let raster = xmap05_raster_inputs_with_keying(
        resolver,
        &keying,
        &province_controllers,
        &country_roster,
    )?;

    let supplied = province_fills.keys().copied().collect::<BTreeSet<_>>();
    let eligible = if require_complete_land {
        province_controllers
            .keys()
            .copied()
            .collect::<BTreeSet<_>>()
    } else {
        if supplied.is_empty() || supplied.contains(&0) {
            return Err("XMAP18 evaluated layer has an empty/zero province census".into());
        }
        let raster_provinces = raster
            .province_ids
            .iter()
            .copied()
            .filter(|province| *province != 0)
            .map(u32::from)
            .collect::<BTreeSet<_>>();
        if !supplied.is_subset(&raster_provinces) {
            return Err("XMAP18 evaluated layer names a province absent from provinces.bmp".into());
        }
        supplied.clone()
    };
    if require_complete_land && supplied != eligible {
        return Err(format!(
            "XMAP18 derivative province census differs from XMAP05 eligible land (eligible={}, supplied={})",
            eligible.len(),
            supplied.len()
        )
        .into());
    }

    let mut region_colors = BTreeMap::<String, [u8; 4]>::new();
    let mut region_ids = BTreeMap::<String, u32>::new();
    let mut province_regions = vec![0u32; raster.gradient.province_regions.len()];
    for (&province_id, fill) in province_fills {
        let Some((region_key, rgba)) = fill else {
            continue;
        };
        if region_key.trim().is_empty() {
            return Err(format!("XMAP18 province {province_id} has a blank region key").into());
        }
        if let Some(existing) = region_colors.insert(region_key.clone(), *rgba) {
            if existing != *rgba {
                return Err(
                    format!("XMAP18 region {region_key} has multiple semantic colours").into(),
                );
            }
        }
        let next_id = u32::try_from(region_ids.len() + 1)?;
        let region_id = *region_ids.entry(region_key.clone()).or_insert(next_id);
        let slot = usize::try_from(province_id)?;
        if slot >= province_regions.len() {
            return Err(
                format!("XMAP18 province {province_id} exceeds the XMAP05 raster census").into(),
            );
        }
        province_regions[slot] = region_id;
    }
    if let Some(gradients) = explicit_gradients {
        if gradients.len() != region_ids.len()
            || region_ids.keys().any(|key| !gradients.contains_key(key))
            || gradients.values().flatten().any(|gradient| {
                !gradient.center.is_finite()
                    || gradient.center < 0.0
                    || !gradient.thickness.is_finite()
                    || gradient.thickness <= 0.0
            })
        {
            return Err("XMAP18 scripted gradient dictionary is incomplete or invalid".into());
        }
    }
    let distance_field = if mask == Xmap05MapModeMask::Flat
        && explicit_gradients.is_none_or(|styles| styles.values().all(Option::is_none))
    {
        None
    } else {
        let boundary = country_boundary_points(
            &raster.province_ids,
            &province_regions,
            raster.map_width,
            raster.map_height,
        )?;
        Some(squared_euclidean_distance(
            &boundary,
            raster.map_width,
            raster.map_height,
        )?)
    };
    let mut region_areas = vec![0u32; region_ids.len() + 1];
    for province in &raster.province_ids {
        let region = province_regions[usize::from(*province)] as usize;
        if region != 0 {
            region_areas[region] = region_areas[region].saturating_add(1);
        }
    }
    let graphics_source = effective_graphics_defines_source(
        resolver,
        &[
            "GRADIENT_BORDERS_FIELD_COUNTRY_LOW",
            "GRADIENT_BORDERS_FIELD_COUNTRY_HIGH",
            "GRADIENT_BORDERS_THICKNESS_COUNTRY_LOW",
            "GRADIENT_BORDERS_THICKNESS_COUNTRY_HIGH",
            "GRADIENT_BORDERS_COUNTRY_CENTER_THICKNESS",
            "GRADIENT_BORDERS_FIELD_COUNTRY_REFRESH",
            "GRADIENT_BORDERS_OUTLINE_CUTOFF_STRATEGIC_REGIONS",
            "GRADIENT_BORDERS_OUTLINE_CUTOFF_COUNTRY",
            "GRADIENT_BORDERS_CAMERA_DISTANCE_OVERRIDE_STRATEGIC_REGIONS",
            "GRADIENT_BORDERS_CAMERA_DISTANCE_OVERRIDE_COUNTRY",
            "DRAW_FOW_CUTOFF",
            "DRAW_FOW_FADE_LENGTH",
            "GRADIENT_BORDERS_THICKNESS_STATE",
        ],
        "XMAP18 GradientBorder defines",
    )?;
    let distance_defines = parse_gradient_border_country_defines(&graphics_source)?;
    let state_thickness = unique_define(&graphics_source, "GRADIENT_BORDERS_THICKNESS_STATE")?;
    if !state_thickness.is_finite() || state_thickness <= 0.0 {
        return Err("XMAP18 state GradientBorder thickness is invalid".into());
    }
    let width = raster.gradient.width;
    let height = raster.gradient.height;
    let mut channel1 = base.gradient_border_channel1.bytes.to_vec();
    let mut channel2 = base.gradient_border_channel2.bytes.to_vec();
    let band_row = match band {
        Xmap05MapModeBand::Top => 0,
        Xmap05MapModeBand::Bottom => height + 1,
    };
    clear_derivative_band(
        &mut channel1,
        &mut channel2,
        width,
        height,
        band_row,
        encoding,
    );
    for y in 0..height {
        for x in 0..width {
            let source = xmap05_half_res_source_index(raster.map_width, raster.map_height, x, y);
            let province_id = u32::from(raster.province_ids[source]);
            if !eligible.contains(&province_id) {
                continue;
            }
            let Some((_, rgba)) = &province_fills[&province_id] else {
                continue;
            };
            let packed = ((band_row + y) * width + x) * 4;
            let mut packed_rgba = [rgba[0], rgba[1], rgba[2], 0x7f];
            if let Some(distance_field) = &distance_field {
                let region = province_regions[usize::try_from(province_id)?] as usize;
                let region_key = province_fills[&province_id]
                    .as_ref()
                    .map(|(key, _)| key)
                    .ok_or("XMAP18 scripted region key disappeared")?;
                let parameters = explicit_gradients
                    .and_then(|styles| styles.get(region_key))
                    .copied()
                    .flatten();
                if explicit_gradients.is_none() || parameters.is_some() {
                    let (center, thickness) = if let Some(parameters) = parameters {
                        (parameters.center, parameters.thickness)
                    } else {
                        let thickness = match mask {
                            Xmap05MapModeMask::CountryDistance => {
                                country_gradient_thickness(region_areas[region], distance_defines)?
                            }
                            Xmap05MapModeMask::StateDistance => state_thickness,
                            Xmap05MapModeMask::Flat => unreachable!(),
                        };
                        (distance_defines.center_thickness, thickness)
                    };
                    packed_rgba[3] = exact_a729_gradient_byte(
                        (distance_field[source] as f32).sqrt(),
                        center,
                        thickness,
                    );
                }
            }
            write_derivative_pixel(
                &mut channel1,
                &mut channel2,
                packed,
                packed_rgba,
                *rgba,
                encoding,
            );
        }
    }
    let gb_height = height
        .checked_mul(2)
        .and_then(|value| value.checked_add(2))
        .ok_or("XMAP18 derivative GradientBorder height overflow")?;

    let mut derived = base.clone();
    derived.gradient_border_channel1 = generated_texture(
        "GradientBorderChannel1",
        width,
        gb_height,
        GRADIENT_BORDER_CHANNEL_DXGI_FORMAT,
        4,
        channel1,
    )?;
    derived.gradient_border_channel2 = generated_texture(
        "GradientBorderChannel2",
        width,
        gb_height,
        GRADIENT_BORDER_CHANNEL_DXGI_FORMAT,
        4,
        channel2,
    )?;
    derived.political_family_sha256 = political_family_digest(&derived);
    derived.fow_family_sha256 = fow_family_digest(&derived);
    derived.deterministic_sha256 = prepared_political_digest(&derived);
    derived.validate()?;
    Ok(derived)
}

fn clear_derivative_band(
    channel1: &mut [u8],
    channel2: &mut [u8],
    width: usize,
    height: usize,
    band_row: usize,
    encoding: Xmap05DerivativeEncoding,
) {
    for y in 0..height {
        let start = ((band_row + y) * width) * 4;
        let end = start + width * 4;
        match encoding {
            Xmap05DerivativeEncoding::MapModeColor => {
                channel1[start..end].fill(0);
                channel2[start..end].fill(0);
            }
            Xmap05DerivativeEncoding::NavalDominance => {
                for packed in (start..end).step_by(4) {
                    channel2[packed] = 0;
                    channel2[packed + 3] = 0;
                }
            }
        }
    }
}

fn write_derivative_pixel(
    channel1: &mut [u8],
    channel2: &mut [u8],
    packed: usize,
    packed_rgba: [u8; 4],
    rgba: [u8; 4],
    encoding: Xmap05DerivativeEncoding,
) {
    match encoding {
        Xmap05DerivativeEncoding::MapModeColor => {
            channel1[packed..packed + 4].copy_from_slice(&packed_rgba);
            channel2[packed + 1] = rgba[3];
        }
        Xmap05DerivativeEncoding::NavalDominance => {
            channel2[packed] = rgba[0];
            channel2[packed + 3] = rgba[3];
        }
    }
}

pub(crate) fn xmap05_terrain_map_mode_derivative(
    resolver: &GameFileResolver,
    context: &FoundationFeatureContext<'_>,
    base: &PreparedPoliticalFrame,
    province_rgba: &BTreeMap<u32, [u8; 4]>,
) -> Result<PreparedPoliticalFrame> {
    base.validate()?;
    if base.hour_index != context.world_frame.hour_index() {
        return Err("XMAP18 terrain derivative and XMAP05 base have different hours".into());
    }
    let (keying, province_controllers, _presentation) =
        admitted_epoch_keying(context.world_frame)?;
    // RESOLVED, for the same reason as `Xmap05Shared::prepare`: this is the set
    // the state-authority validator checks every control TAG against.
    let country_roster = xmap05_roster(context.world_frame)?
        .resolved
        .into_iter()
        .map(|entry| entry.tag)
        .collect::<BTreeSet<_>>();
    let raster = xmap05_raster_inputs_with_keying(
        resolver,
        &keying,
        &province_controllers,
        &country_roster,
    )?;
    if raster.map_width != raster.gradient.width * 2
        || raster.map_height != raster.gradient.height * 2
    {
        return Err("XMAP18 terrain carrier is not an exact 2x2 map reduction".into());
    }
    for province_id in raster.province_ids.iter().copied().filter(|id| *id != 0) {
        if !province_rgba.contains_key(&u32::from(province_id)) {
            return Err(format!(
                "XMAP18 terrain carrier has no effective colour for province {province_id}"
            )
            .into());
        }
    }

    let width = raster.gradient.width;
    let height = raster.gradient.height;
    let mut channel1 = base.gradient_border_channel1.bytes.to_vec();
    let mut channel2 = base.gradient_border_channel2.bytes.to_vec();
    channel1[..width * height * 4].fill(0);
    channel2[..width * height * 4].fill(0);
    for y in 0..height {
        let source_y = raster.map_height - y * 2 - 2;
        for x in 0..width {
            let source_x = x * 2;
            let indices = [
                source_y * raster.map_width + source_x,
                source_y * raster.map_width + source_x + 1,
                (source_y + 1) * raster.map_width + source_x,
                (source_y + 1) * raster.map_width + source_x + 1,
            ];
            let mut samples = [None; 4];
            for (sample, index) in samples.iter_mut().zip(indices) {
                let province_id = u32::from(raster.province_ids[index]);
                if province_id == 0 {
                    continue;
                }
                *sample = Some(province_rgba[&province_id]);
            }
            let Some(average) = average_terrain_rgba_block(samples) else {
                continue;
            };
            let packed = (y * width + x) * 4;
            channel1[packed] = average[0];
            channel1[packed + 1] = average[1];
            channel1[packed + 2] = average[2];
            channel1[packed + 3] = 0x7f;
            channel2[packed + 1] = average[3];
        }
    }
    let gb_height = height
        .checked_mul(2)
        .and_then(|value| value.checked_add(2))
        .ok_or("XMAP18 terrain GradientBorder height overflow")?;
    let mut derived = base.clone();
    derived.gradient_border_channel1 = generated_texture(
        "GradientBorderChannel1",
        width,
        gb_height,
        GRADIENT_BORDER_CHANNEL_DXGI_FORMAT,
        4,
        channel1,
    )?;
    derived.gradient_border_channel2 = generated_texture(
        "GradientBorderChannel2",
        width,
        gb_height,
        GRADIENT_BORDER_CHANNEL_DXGI_FORMAT,
        4,
        channel2,
    )?;
    derived.political_family_sha256 = political_family_digest(&derived);
    derived.fow_family_sha256 = fow_family_digest(&derived);
    derived.deterministic_sha256 = prepared_political_digest(&derived);
    derived.validate()?;
    Ok(derived)
}

fn average_terrain_rgba_block(samples: [Option<[u8; 4]>; 4]) -> Option<[u8; 4]> {
    let mut sums = [0u32; 4];
    let mut populated = 0u32;
    for rgba in samples.into_iter().flatten() {
        for component in 0..4 {
            sums[component] += u32::from(rgba[component]);
        }
        populated += 1;
    }
    (populated != 0).then(|| std::array::from_fn(|component| (sums[component] / populated) as u8))
}

fn country_boundary_points(
    province_ids: &[u16],
    province_regions: &[u32],
    width: usize,
    height: usize,
) -> Result<Vec<bool>> {
    if width == 0
        || height == 0
        || province_ids.len() != width * height
        || province_ids
            .iter()
            .any(|province| usize::from(*province) >= province_regions.len())
    {
        return Err("XMAP05 full-resolution province-edge raster is invalid".into());
    }
    let mut boundary = vec![false; province_ids.len()];
    for y in 0..height {
        for x in 0..width {
            let index = y * width + x;
            let province = province_ids[index];
            let region = province_regions[usize::from(province)];
            if region == 0 {
                continue;
            }
            let differs = |other_index: usize| {
                let other = province_ids[other_index];
                other != 0 && province_regions[usize::from(other)] != region
            };
            boundary[index] = (x > 0 && differs(index - 1))
                || (x + 1 < width && differs(index + 1))
                || (y > 0 && differs(index - width))
                || (y + 1 < height && differs(index + width));
        }
    }
    Ok(boundary)
}

fn squared_euclidean_distance(boundary: &[bool], width: usize, height: usize) -> Result<Vec<u64>> {
    if width == 0 || height == 0 || boundary.len() != width * height {
        return Err("XMAP05 GradientBorder distance raster is invalid".into());
    }
    if !boundary.iter().any(|value| *value) {
        return Err("XMAP05 GradientBorder has no country edge candidates".into());
    }
    const INF: u64 = u64::MAX / 16;
    let mut horizontal = vec![INF; width * height];
    let mut source = vec![INF; width.max(height)];
    let mut transformed = vec![INF; width.max(height)];
    for y in 0..height {
        for x in 0..width {
            source[x] = if boundary[y * width + x] { 0 } else { INF };
        }
        distance_transform_1d(&source[..width], &mut transformed[..width]);
        horizontal[y * width..(y + 1) * width].copy_from_slice(&transformed[..width]);
    }
    let mut out = vec![INF; width * height];
    for x in 0..width {
        for y in 0..height {
            source[y] = horizontal[y * width + x];
        }
        distance_transform_1d(&source[..height], &mut transformed[..height]);
        for y in 0..height {
            out[y * width + x] = transformed[y];
        }
    }
    Ok(out)
}

fn distance_transform_1d(source: &[u64], out: &mut [u64]) {
    let n = source.len();
    let mut sites = vec![0usize; n];
    let mut intersections = vec![f64::NEG_INFINITY; n + 1];
    let Some(first) = source.iter().position(|value| *value < u64::MAX / 32) else {
        out.fill(u64::MAX / 16);
        return;
    };
    let mut k = 0usize;
    sites[0] = first;
    intersections[1] = f64::INFINITY;
    for q in first + 1..n {
        if source[q] >= u64::MAX / 32 {
            continue;
        }
        let mut intersection;
        loop {
            let p = sites[k];
            intersection = ((source[q] as f64 + (q * q) as f64)
                - (source[p] as f64 + (p * p) as f64))
                / (2.0 * (q as f64 - p as f64));
            if intersection > intersections[k] || k == 0 {
                break;
            }
            k -= 1;
        }
        k += 1;
        sites[k] = q;
        intersections[k] = intersection;
        intersections[k + 1] = f64::INFINITY;
    }
    k = 0;
    for (q, target) in out.iter_mut().enumerate() {
        while intersections[k + 1] < q as f64 {
            k += 1;
        }
        let delta = q.abs_diff(sites[k]) as u64;
        *target = source[sites[k]].saturating_add(delta.saturating_mul(delta));
    }
}

fn country_gradient_thickness(area: u32, defines: GradientBorderCountryDefines) -> Result<f32> {
    let field_low = defines.field_low.trunc() as i64;
    let field_high = defines.field_high.trunc() as i64;
    if field_high <= field_low {
        return Err("XMAP05 GradientBorder field range is empty".into());
    }
    let clamped = i64::from(area).clamp(field_low, field_high);
    let t = (clamped as f32 - defines.field_low) / (defines.field_high - defines.field_low);
    Ok((defines.thickness_high - defines.thickness_low) * t + defines.thickness_low)
}

fn exact_a729_gradient_byte(distance: f32, center: f32, thickness: f32) -> u8 {
    let mut value = if distance > center {
        255 - ((distance - center) * (1.0 / thickness) * 255.0).trunc() as i32
            - center.trunc() as i32
    } else {
        255 - distance.trunc() as i32
    };
    value -= 8;
    value.clamp(0, 255) as u8
}

fn parse_gradient_border_country_defines(source: &str) -> Result<GradientBorderCountryDefines> {
    let parsed = GradientBorderCountryDefines {
        field_low: unique_define(source, "GRADIENT_BORDERS_FIELD_COUNTRY_LOW")?,
        field_high: unique_define(source, "GRADIENT_BORDERS_FIELD_COUNTRY_HIGH")?,
        thickness_low: unique_define(source, "GRADIENT_BORDERS_THICKNESS_COUNTRY_LOW")?,
        thickness_high: unique_define(source, "GRADIENT_BORDERS_THICKNESS_COUNTRY_HIGH")?,
        center_thickness: unique_define(source, "GRADIENT_BORDERS_COUNTRY_CENTER_THICKNESS")?,
        refresh_hours: unique_define(source, "GRADIENT_BORDERS_FIELD_COUNTRY_REFRESH")?.trunc()
            as u32,
        outline_cutoff_strategic: unique_define(
            source,
            "GRADIENT_BORDERS_OUTLINE_CUTOFF_STRATEGIC_REGIONS",
        )?,
        outline_cutoff_country: unique_define(source, "GRADIENT_BORDERS_OUTLINE_CUTOFF_COUNTRY")?,
        camera_override_strategic: unique_define(
            source,
            "GRADIENT_BORDERS_CAMERA_DISTANCE_OVERRIDE_STRATEGIC_REGIONS",
        )?,
        camera_override_country: unique_define(
            source,
            "GRADIENT_BORDERS_CAMERA_DISTANCE_OVERRIDE_COUNTRY",
        )?,
        draw_fow_cutoff: unique_define(source, "DRAW_FOW_CUTOFF")?,
        draw_fow_fade_length: unique_define(source, "DRAW_FOW_FADE_LENGTH")?,
    };
    if ![
        parsed.field_low,
        parsed.field_high,
        parsed.thickness_low,
        parsed.thickness_high,
        parsed.center_thickness,
        parsed.outline_cutoff_strategic,
        parsed.outline_cutoff_country,
        parsed.draw_fow_cutoff,
        parsed.draw_fow_fade_length,
    ]
    .iter()
    .all(|value| value.is_finite() && *value > 0.0)
        || parsed.field_high <= parsed.field_low
        || parsed.thickness_high < parsed.thickness_low
        || parsed.outline_cutoff_strategic > 1.0
        || parsed.outline_cutoff_country > 1.0
        || ![
            parsed.camera_override_strategic,
            parsed.camera_override_country,
        ]
        .iter()
        .all(|value| value.is_finite() && (0.0..=1.0).contains(value))
        || parsed.refresh_hours == 0
    {
        return Err("XMAP05 GradientBorder country defines are invalid".into());
    }
    Ok(parsed)
}

fn decode_gradient_border_channel3_dds(bytes: &[u8]) -> Result<[u8; 8]> {
    let read_u32 = |offset: usize| -> Result<u32> {
        let raw = bytes
            .get(offset..offset + 4)
            .ok_or("XMAP05 Channel3 DDS header is truncated")?;
        Ok(u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]))
    };
    let header_flags = read_u32(8)?;
    let declared_pitch = read_u32(20)?;
    let effective_mip_levels = read_u32(28)?.max(1);
    let caps = read_u32(108)?;
    let caps2 = read_u32(112)?;
    if bytes.get(..4) != Some(b"DDS ")
        || read_u32(4)? != 124
        || header_flags & 0x1007 != 0x1007
        || read_u32(12)? != 1
        || read_u32(16)? != 2
        || header_flags & 0x800000 != 0
        || read_u32(24)? != 0
        || effective_mip_levels != 1
        || (header_flags & 0x8 != 0 && declared_pitch < 8)
        || read_u32(76)? != 32
        || caps & 0x1000 == 0
        || caps2 & 0x20fe00 != 0
    {
        return Err(
            "XMAP05 Channel3 DDS must decode as one 2x1 BGRA8 mip for the exact-a729 sRGB SRV"
                .into(),
        );
    }
    let payload_offset: usize = if bytes.get(84..88) == Some(b"DX10") {
        if read_u32(80)? & 0x4 == 0
            || read_u32(128)? != GRADIENT_BORDER_CHANNEL3_DXGI_FORMAT
            || read_u32(132)? != 3
            || read_u32(136)? & 0x4 != 0
            || read_u32(140)? != 1
            || read_u32(144)? > 4
        {
            return Err(
                "XMAP05 Channel3 DX10 DDS must be a non-array, non-cube 2D format-91 texture"
                    .into(),
            );
        }
        148
    } else {
        if read_u32(80)? & 0x40 == 0
            || read_u32(80)? & 0x4 != 0
            || read_u32(84)? != 0
            || read_u32(88)? != 32
            || read_u32(92)? != 0x00ff_0000
            || read_u32(96)? != 0x0000_ff00
            || read_u32(100)? != 0x0000_00ff
            || read_u32(104)? != 0xff00_0000
        {
            return Err("XMAP05 Channel3 legacy DDS must use the exact BGRA8 channel masks".into());
        }
        128
    };
    let required_row_bytes = if header_flags & 0x8 != 0 {
        usize::try_from(declared_pitch)
            .map_err(|_| "XMAP05 Channel3 DDS declared pitch does not fit this platform")?
    } else {
        8
    };
    let required_payload_end = payload_offset
        .checked_add(required_row_bytes)
        .ok_or("XMAP05 Channel3 DDS declared pitch overflows the payload range")?;
    bytes
        .get(payload_offset..required_payload_end)
        .ok_or("XMAP05 Channel3 DDS row payload is truncated")?;
    bytes
        .get(payload_offset..payload_offset + 8)
        .ok_or("XMAP05 Channel3 DDS payload is truncated")?
        .try_into()
        .map_err(|_| "XMAP05 Channel3 DDS payload is truncated".into())
}

fn unique_define(source: &str, key: &str) -> Result<f32> {
    optional_unique_define(source, key)?
        .ok_or_else(|| format!("XMAP05 graphics define {key} is missing").into())
}

fn optional_unique_define(source: &str, key: &str) -> Result<Option<f32>> {
    let mut value = None;
    for raw in source.lines() {
        let line = raw.split("--").next().unwrap_or_default();
        let Some((left, right)) = line.split_once('=') else {
            continue;
        };
        if left.trim().rsplit('.').next() == Some(key) {
            let token = right
                .split([',', ' ', '\t', '}'])
                .find(|value| !value.is_empty())
                .ok_or_else(|| format!("XMAP05 graphics define {key} has no value"))?;
            value = Some(
                token
                    .parse::<f32>()
                    .map_err(|_| format!("XMAP05 graphics define {key} is not numeric"))?,
            );
        }
    }
    Ok(value)
}

fn effective_graphics_defines_source(
    resolver: &GameFileResolver,
    keys: &[&str],
    label: &str,
) -> Result<String> {
    let files = resolver.resolve_dir_files_recursive("common/defines", Some("lua"))?;
    if files.is_empty() {
        return Err(format!("{label}: effective common/defines is empty").into());
    }
    let mut sources = Vec::with_capacity(files.len());
    for (relative, path) in files {
        sources.push((relative, read_latin1(&path)?));
    }
    let mut effective = String::new();
    let mut seen = BTreeSet::new();
    for key in keys {
        if !seen.insert(*key) {
            return Err(format!("{label}: requested define {key} more than once").into());
        }
        let (relative, value) = effective_define_from_sources(&sources, key)?
            .ok_or_else(|| format!("{label}: effective graphics define {key} is missing"))?;
        effective.push_str("-- effective from ");
        effective.push_str(relative);
        effective.push('\n');
        effective.push_str(key);
        effective.push_str(" = ");
        effective.push_str(&value.to_string());
        effective.push('\n');
    }
    Ok(effective)
}

fn effective_define_from_sources<'a>(
    sources: &'a [(String, String)],
    key: &str,
) -> Result<Option<(&'a str, f32)>> {
    for (relative, source) in sources.iter().rev() {
        if let Some(value) = optional_unique_define(source, key)? {
            return Ok(Some((relative.as_str(), value)));
        }
    }
    Ok(None)
}

fn texture_descriptor_digest(descriptor: &Xmap05TextureDescriptor) -> String {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(descriptor.symbol.as_bytes());
    bytes.extend_from_slice(&descriptor.width.to_le_bytes());
    bytes.extend_from_slice(&descriptor.height.to_le_bytes());
    bytes.extend_from_slice(&descriptor.dxgi_format.to_le_bytes());
    bytes.extend_from_slice(&descriptor.row_pitch.to_le_bytes());
    bytes.extend_from_slice(&descriptor.mip_levels.to_le_bytes());
    bytes.extend_from_slice(descriptor.payload_sha256.as_bytes());
    Sha256::hex(&bytes)
}

fn visibility_digest(visibility: &PreparedXmap05Visibility) -> String {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(visibility.schema.as_bytes());
    bytes.extend_from_slice(
        visibility
            .viewer_country
            .as_deref()
            .unwrap_or("<omniscient>")
            .as_bytes(),
    );
    bytes.push(visibility.fow_enabled as u8);
    bytes.extend_from_slice(&visibility.capture_sequence.unwrap_or(0).to_le_bytes());
    bytes.extend_from_slice(&visibility.pov_country_id.unwrap_or(0).to_le_bytes());
    bytes.extend_from_slice(visibility.province_visibility_sha256.as_bytes());
    bytes.push(visibility.air_wing_spotting_unavailable as u8);
    bytes.extend_from_slice(&visibility.land_frame_sequence.unwrap_or(0).to_le_bytes());
    bytes.extend_from_slice(
        &visibility
            .taskforce_frame_sequence
            .unwrap_or(0)
            .to_le_bytes(),
    );
    bytes.extend_from_slice(b"land_units\0");
    bytes.extend_from_slice(&(visibility.land_units.len() as u64).to_le_bytes());
    for row in &visibility.land_units {
        bytes.extend_from_slice(&row.type_id.to_le_bytes());
        bytes.extend_from_slice(&row.stable_id.to_le_bytes());
        bytes.push(row.disclosure as u8);
        bytes.push(row.reason as u8);
    }
    bytes.extend_from_slice(b"taskforce_units\0");
    bytes.extend_from_slice(&(visibility.taskforce_units.len() as u64).to_le_bytes());
    for row in &visibility.taskforce_units {
        bytes.extend_from_slice(&row.type_id.to_le_bytes());
        bytes.extend_from_slice(&row.stable_id.to_le_bytes());
        bytes.push(row.disclosure as u8);
        bytes.push(row.reason as u8);
    }
    Sha256::hex(&bytes)
}

fn effective_vfs_manifest_identity(prepared: &PreparedPoliticalFrame) -> Result<String> {
    let mut manifests = prepared
        .ordered_used_content
        .iter()
        .map(|content| content.effective_vfs_manifest_sha256.as_str())
        .collect::<BTreeSet<_>>();
    if manifests.len() != 1 {
        return Err("XMAP05 used content spans multiple effective-VFS manifests".into());
    }
    Ok(manifests
        .pop_first()
        .ok_or("XMAP05 used content has no effective-VFS identity")?
        .to_string())
}

fn used_content_digest(content: &[UsedContentKey]) -> String {
    let mut bytes = b"XMAP05_EFFECTIVE_USED_CONTENT_V1\0".to_vec();
    for item in content {
        bytes.extend_from_slice(item.effective_vfs_manifest_sha256.as_bytes());
        bytes.push(0);
        bytes.extend_from_slice(item.normalized_game_relative_path.as_bytes());
        bytes.push(0);
        bytes.extend_from_slice(item.payload_sha256.as_bytes());
        bytes.extend_from_slice(&item.payload_bytes.to_le_bytes());
    }
    Sha256::hex(&bytes)
}

fn scene_cb0_payload_bytes(patch: &Xmap05SceneCb0Patch) -> [u8; 36] {
    let mut bytes = [0u8; 36];
    let values = [
        patch.v_gb_cam_dist_override_gb_outline_cutoff[0],
        patch.v_gb_cam_dist_override_gb_outline_cutoff[1],
        patch.v_gb_cam_dist_override_gb_outline_cutoff[2],
        patch.v_gb_cam_dist_override_gb_outline_cutoff[3],
        patch.fow_opacity,
        patch.fow_time,
        patch.fow_fade_factor,
        patch.min_mesh_alpha,
        patch.gb_texture_height,
    ];
    for (index, value) in values.into_iter().enumerate() {
        bytes[index * 4..index * 4 + 4].copy_from_slice(&value.to_le_bytes());
    }
    bytes
}

fn scene_cb0_static_law_receipt_digest(patch: &Xmap05SceneCb0Patch) -> String {
    let mut bytes = b"XMAP05_SCENE_CB0_STATIC_LAW_RECEIPT_V1\0".to_vec();
    bytes.extend_from_slice(MAPC1_EXECUTABLE_SHA256.as_bytes());
    bytes.extend_from_slice(DRAW_FOW_DEFINE_LOADERS_GHIDRA_MANIFEST_SHA256.as_bytes());
    bytes.extend_from_slice(patch.effective_vfs_manifest_sha256.as_bytes());
    bytes.extend_from_slice(patch.used_content_sha256.as_bytes());
    bytes.extend_from_slice(patch.political_frame_sha256.as_bytes());
    Sha256::hex(&bytes)
}

fn scene_cb0_camera_binding_digest(patch: &Xmap05SceneCb0Patch) -> String {
    let mut bytes = b"XMAP05_SCENE_CB0_CAMERA_BINDING_V1\0".to_vec();
    bytes.extend_from_slice(&patch.hour_index.to_le_bytes());
    bytes.extend_from_slice(&patch.camera_height.to_bits().to_le_bytes());
    bytes.extend_from_slice(patch.camera_state_sha256.as_bytes());
    Sha256::hex(&bytes)
}

fn scene_cb0_patch_digest(patch: &Xmap05SceneCb0Patch) -> String {
    let mut bytes = b"XMAP05_SCENE_CB0_PATCH_V1\0".to_vec();
    bytes.extend_from_slice(patch.schema.as_bytes());
    bytes.extend_from_slice(&patch.hour_index.to_le_bytes());
    bytes.extend_from_slice(&patch.map_clock_seconds.to_bits().to_le_bytes());
    for hash in [
        &patch.effective_vfs_manifest_sha256,
        &patch.used_content_sha256,
        &patch.political_frame_sha256,
        &patch.map_clock_source_sha256,
        &patch.static_law_receipt_sha256,
        &patch.camera_state_sha256,
        &patch.camera_binding_sha256,
        &patch.payload_sha256,
    ] {
        bytes.extend_from_slice(hash.as_bytes());
    }
    Sha256::hex(&bytes)
}

fn political_family_digest(prepared: &PreparedPoliticalFrame) -> String {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(prepared.schema.as_bytes());
    bytes.extend_from_slice(&prepared.hour_index.to_le_bytes());
    for texture in [
        &prepared.province_primary,
        &prepared.province_secondary,
        &prepared.gradient_border_channel1,
        &prepared.gradient_border_channel2,
        &prepared.gradient_border_channel3,
    ] {
        bytes.extend_from_slice(texture.descriptor.payload_sha256.as_bytes());
        bytes.extend_from_slice(texture.descriptor.descriptor_sha256.as_bytes());
    }
    Sha256::hex(&bytes)
}

fn fow_family_digest(prepared: &PreparedPoliticalFrame) -> String {
    Sha256::hex(
        format!(
            "{}\0{}\0{}",
            prepared.visibility.deterministic_sha256,
            prepared.intel_map.descriptor.payload_sha256,
            prepared.intel_map.descriptor.descriptor_sha256
        )
        .as_bytes(),
    )
}

fn prepared_political_digest(prepared: &PreparedPoliticalFrame) -> String {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(prepared.political_family_sha256.as_bytes());
    bytes.extend_from_slice(prepared.fow_family_sha256.as_bytes());
    for content in &prepared.ordered_used_content {
        bytes.extend_from_slice(content.normalized_game_relative_path.as_bytes());
        bytes.extend_from_slice(content.payload_sha256.as_bytes());
    }
    for bits in [
        prepared.scene_cb0_law.outline_cutoff_strategic_bits,
        prepared.scene_cb0_law.outline_cutoff_country_bits,
        prepared.scene_cb0_law.camera_override_strategic_bits,
        prepared.scene_cb0_law.camera_override_country_bits,
        prepared.scene_cb0_law.draw_fow_cutoff_bits,
        prepared.scene_cb0_law.draw_fow_fade_length_bits,
    ] {
        bytes.extend_from_slice(&bits.to_le_bytes());
    }
    Sha256::hex(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terrain_reduction_uses_exact_truncating_rgba_average() {
        assert_eq!(
            average_terrain_rgba_block([
                Some([10, 20, 30, 127]),
                Some([20, 30, 40, 127]),
                Some([30, 40, 50, 204]),
                Some([40, 50, 60, 204]),
            ]),
            Some([25, 35, 45, 165])
        );
        assert_eq!(
            average_terrain_rgba_block([Some([9, 8, 7, 127]), None, None, None]),
            Some([9, 8, 7, 127])
        );
        assert_eq!(average_terrain_rgba_block([None; 4]), None);
    }

    fn scene_patch_frame() -> PreparedPoliticalFrame {
        let manifest = "a".repeat(64);
        let content = UsedContentKey {
            effective_vfs_manifest_sha256: manifest,
            normalized_game_relative_path: GRAPHICS_DEFINES.into(),
            winning_layer_ordinal: 0,
            layer_kind: portable_content_manifest::ContentLayerKind::Base,
            layer_id: "base".into(),
            source_sha256: "b".repeat(64),
            payload_sha256: "c".repeat(64),
            payload_bytes: 1,
        };
        let mut visibility = PreparedXmap05Visibility {
            schema: XMAP05_VISIBILITY_SCHEMA.into(),
            viewer_country: None,
            fow_enabled: false,
            capture_sequence: None,
            pov_country_id: None,
            province_visibility_sha256: "d".repeat(64),
            land_units: Vec::new(),
            taskforce_units: Vec::new(),
            air_wing_spotting_unavailable: true,
            land_frame_sequence: None,
            taskforce_frame_sequence: None,
            deterministic_sha256: String::new(),
        };
        visibility.deterministic_sha256 = visibility_digest(&visibility);
        let mut frame = PreparedPoliticalFrame {
            schema: XMAP05_POLITICAL_SCHEMA.into(),
            hour_index: 42,
            ordered_used_content: vec![content],
            province_primary: generated_texture(
                "ProvincePrimaryColorMap",
                1,
                1,
                PROVINCE_COLOR_MAP_DXGI_FORMAT,
                4,
                vec![0; 4],
            )
            .unwrap(),
            province_secondary: generated_texture(
                crate::PROVINCE_SECONDARY_COLOR_MAP_SYMBOL,
                1,
                1,
                PROVINCE_COLOR_MAP_DXGI_FORMAT,
                4,
                vec![0; 4],
            )
            .unwrap(),
            gradient_border_channel1: generated_texture(
                "GradientBorderChannel1",
                1,
                4,
                GRADIENT_BORDER_CHANNEL_DXGI_FORMAT,
                4,
                vec![0; 16],
            )
            .unwrap(),
            gradient_border_channel2: generated_texture(
                "GradientBorderChannel2",
                1,
                4,
                GRADIENT_BORDER_CHANNEL_DXGI_FORMAT,
                4,
                vec![0; 16],
            )
            .unwrap(),
            gradient_border_channel3: generated_texture(
                "GradientBorderChannel3",
                2,
                1,
                GRADIENT_BORDER_CHANNEL3_DXGI_FORMAT,
                4,
                VANILLA_CHANNEL3_BGRA8_SRGB_ORACLE.to_vec(),
            )
            .unwrap(),
            intel_map: generated_texture("IntelMap", 1, 1, INTEL_MAP_DXGI_FORMAT, 1, vec![255])
                .unwrap(),
            visibility,
            political_family_sha256: String::new(),
            fow_family_sha256: String::new(),
            deterministic_sha256: String::new(),
            excluded_presentation_identities: Vec::new(),
            scene_cb0_law: Xmap05SceneCb0Law {
                outline_cutoff_strategic_bits: 0.98f32.to_bits(),
                outline_cutoff_country_bits: 0.973f32.to_bits(),
                camera_override_strategic_bits: 1.0f32.to_bits(),
                camera_override_country_bits: 0.0f32.to_bits(),
                draw_fow_cutoff_bits: 400.0f32.to_bits(),
                draw_fow_fade_length_bits: 350.0f32.to_bits(),
            },
        };
        frame.political_family_sha256 = political_family_digest(&frame);
        frame.fow_family_sha256 = fow_family_digest(&frame);
        frame.deterministic_sha256 = prepared_political_digest(&frame);
        frame
    }

    fn scene_patch_camera(
        frame: &PreparedPoliticalFrame,
        camera_height: f32,
    ) -> ViewerMapCameraState {
        let graphics_defines_content = frame
            .ordered_used_content
            .iter()
            .find(|content| content.normalized_game_relative_path == GRAPHICS_DEFINES)
            .unwrap()
            .clone();
        let defines =
            crate::Hoi4MapCameraDefines::new(1.0, camera_height, 100.0, 100.0, 1.0, 1.0, 1.0)
                .unwrap();
        ViewerMapCameraState::new(
            graphics_defines_content,
            defines,
            frame.province_primary.descriptor.width,
            frame.province_primary.descriptor.height,
            1920,
            1080,
            50.0,
        )
        .unwrap()
    }

    #[test]
    fn scene_cb0_patch_is_hour_camera_and_clock_bound() {
        let frame = scene_patch_frame();
        let camera = scene_patch_camera(&frame, 272.866_03);
        let clock = Xmap05AuthenticatedMapClock::sealed_test_receipt(42, 650.8913);
        let patch = frame.scene_cb0_patch(&camera, &clock).unwrap();
        assert_eq!(
            patch.v_gb_cam_dist_override_gb_outline_cutoff,
            [1.0, 0.0, 0.98, 0.973]
        );
        assert!((patch.fow_fade_factor - 0.131_943_24).abs() < 0.000_001);
        assert_eq!(patch.fow_opacity, 0.0);
        assert_eq!(patch.fow_time, clock.seconds());
        assert_eq!(patch.min_mesh_alpha.to_bits(), 0.0f32.to_bits());
        assert_eq!(patch.gb_texture_height, 1.0);
        assert_eq!(patch.payload_bytes.len(), 36);
        patch.validate().unwrap();
    }

    fn test_viewer(observer_country_id: Option<&str>) -> ViewerOwnedMapState {
        ViewerOwnedMapState {
            viewport_width: 1920,
            viewport_height: 1080,
            camera: None,
            observer_country_id: observer_country_id.map(str::to_string),
            map_mode_id: "viewer.map_mode.unselected".into(),
            map_mode_selection_epoch: 0,
            selected_object_ids: Vec::new(),
            input_revision: 0,
            animation_time_f32_bits: 0.0f32.to_bits(),
            frame_delta_seconds_f32_bits: 0.0f32.to_bits(),
        }
    }

    #[test]
    fn observer_mode_consumes_no_recorded_visibility_vector() {
        assert_eq!(
            xmap05_visibility_mode(&test_viewer(None)),
            Xmap05VisibilityMode::Observer
        );
        assert_eq!(
            xmap05_visibility_mode(&test_viewer(Some("SOV"))),
            Xmap05VisibilityMode::Pov("SOV".into())
        );

        let (visibility, province_visibility) = prepare_observer_visibility(7).unwrap();
        assert!(!visibility.fow_enabled);
        assert_eq!(visibility.viewer_country, None);
        assert_eq!(visibility.pov_country_id, None);
        assert_eq!(province_visibility, vec![u8::MAX; 7]);

        // The payload's vector is its declared identity, not an operand: under
        // `fow_enabled == false` the intel map reads the same texel from a
        // fully dark vector, so no recorded POV could change this render.
        let eligible = vec![true; 7];
        let dark = vec![0u8; 7];
        let from_declared =
            crate::fow_law::exact_a729_intel_map_texel(&[3], &eligible, &province_visibility, false)
                .unwrap();
        let from_dark =
            crate::fow_law::exact_a729_intel_map_texel(&[3], &eligible, &dark, false).unwrap();
        assert_eq!(from_declared, from_dark);
        assert_eq!(from_declared, !u8::MAX);
        // ... and with fog on the same two vectors disagree, so the equality
        // above is the mode's doing rather than the texel law's.
        assert_ne!(
            crate::fow_law::exact_a729_intel_map_texel(&[3], &eligible, &province_visibility, true)
                .unwrap(),
            crate::fow_law::exact_a729_intel_map_texel(&[3], &eligible, &dark, true).unwrap()
        );
    }

    #[test]
    fn viewer_clock_uses_xmap10_presentation_time_without_selected_present() {
        let presentation = Xmap10ViewerPresentationState::new().unwrap();
        let viewer = ViewerOwnedMapState {
            viewport_width: 1920,
            viewport_height: 1080,
            camera: None,
            observer_country_id: None,
            map_mode_id: "viewer.map_mode.unselected".into(),
            map_mode_selection_epoch: 0,
            selected_object_ids: Vec::new(),
            input_revision: 0,
            animation_time_f32_bits: 0.0f32.to_bits(),
            frame_delta_seconds_f32_bits: 0.0f32.to_bits(),
        };
        let clock =
            Xmap05AuthenticatedMapClock::from_viewer_presentation(42, &viewer, &presentation)
                .unwrap();
        assert_eq!(clock.seconds(), 0.0);
        assert_eq!(clock.fow_opacity(), 0.0);
        assert_eq!(
            clock.viewer_presentation_receipt_sha256,
            presentation.deterministic_sha256
        );

        let drifted = ViewerOwnedMapState {
            animation_time_f32_bits: 1.0f32.to_bits(),
            ..viewer
        };
        assert!(
            Xmap05AuthenticatedMapClock::from_viewer_presentation(42, &drifted, &presentation)
                .is_err()
        );
    }

    #[test]
    fn scene_cb0_patch_rejects_unbound_clock_hour() {
        let frame = scene_patch_frame();
        let camera = scene_patch_camera(&frame, 100.0);
        let clock = Xmap05AuthenticatedMapClock::sealed_test_receipt(43, 1.0);
        assert!(frame.scene_cb0_patch(&camera, &clock).is_err());
    }

    #[test]
    fn scene_cb0_patch_rejects_camera_digest_or_graphics_define_drift() {
        let frame = scene_patch_frame();
        let clock = Xmap05AuthenticatedMapClock::sealed_test_receipt(42, 1.0);
        let mut unsealed = scene_patch_camera(&frame, 100.0);
        unsealed.input_revision += 1;
        assert!(frame.scene_cb0_patch(&unsealed, &clock).is_err());

        let mut other_content = frame.ordered_used_content[0].clone();
        other_content.payload_sha256 = "f".repeat(64);
        let other_defines =
            crate::Hoi4MapCameraDefines::new(1.0, 100.0, 100.0, 100.0, 1.0, 1.0, 1.0).unwrap();
        let other_camera =
            ViewerMapCameraState::new(other_content, other_defines, 1, 1, 1920, 1080, 50.0)
                .unwrap();
        assert!(frame.scene_cb0_patch(&other_camera, &clock).is_err());
    }

    #[test]
    fn authenticated_map_clock_refuses_producer_value_or_receipt_tamper() {
        let clock = Xmap05AuthenticatedMapClock::sealed_test_receipt(42, 1.5);
        clock.validate().unwrap();
        for tampered in [
            {
                let mut value = clock.clone();
                value.hour_index += 1;
                value
            },
            {
                let mut value = clock.clone();
                value.seconds_f32_bits = 2.5f32.to_bits();
                value
            },
            {
                let mut value = clock.clone();
                value.fow_opacity_f32_bits = 0.5f32.to_bits();
                value
            },
            {
                let mut value = clock.clone();
                value.viewer_state_sha256 = "d".repeat(64);
                value
            },
        ] {
            assert!(tampered.validate().is_err());
        }
    }

    #[test]
    fn scene_cb0_patch_refuses_fow_time_or_negative_zero_tamper() {
        let frame = scene_patch_frame();
        let camera = scene_patch_camera(&frame, 100.0);
        let clock = Xmap05AuthenticatedMapClock::sealed_test_receipt(42, 1.5);
        let patch = frame.scene_cb0_patch(&camera, &clock).unwrap();
        let mut wrong_time = patch.clone();
        wrong_time.fow_time = 2.5;
        wrong_time.payload_bytes = scene_cb0_payload_bytes(&wrong_time);
        wrong_time.payload_sha256 = Sha256::hex(&wrong_time.payload_bytes);
        wrong_time.deterministic_sha256 = scene_cb0_patch_digest(&wrong_time);
        assert!(wrong_time.validate().is_err());
        let mut negative_zero = patch;
        negative_zero.min_mesh_alpha = -0.0;
        negative_zero.payload_bytes = scene_cb0_payload_bytes(&negative_zero);
        negative_zero.payload_sha256 = Sha256::hex(&negative_zero.payload_bytes);
        negative_zero.deterministic_sha256 = scene_cb0_patch_digest(&negative_zero);
        assert!(negative_zero.validate().is_err());
    }

    fn channel3_legacy_dds() -> Vec<u8> {
        let mut dds = vec![0u8; 136];
        dds[..4].copy_from_slice(b"DDS ");
        for (offset, value) in [
            (4, 124u32),
            (8, 0x100f),
            (12, 1),
            (16, 2),
            (20, 8),
            (28, 1),
            (76, 32),
            (80, 0x41),
            (88, 32),
            (92, 0x00ff_0000),
            (96, 0x0000_ff00),
            (100, 0x0000_00ff),
            (104, 0xff00_0000),
            (108, 0x1000),
        ] {
            dds[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        dds[128..136].copy_from_slice(&VANILLA_CHANNEL3_BGRA8_SRGB_ORACLE);
        dds
    }

    fn channel3_dx10_dds() -> Vec<u8> {
        let mut dds = vec![0u8; 156];
        dds[..4].copy_from_slice(b"DDS ");
        for (offset, value) in [
            (4, 124u32),
            (8, 0x100f),
            (12, 1),
            (16, 2),
            (20, 8),
            (28, 1),
            (76, 32),
            (80, 0x4),
            (108, 0x1000),
            (128, GRADIENT_BORDER_CHANNEL3_DXGI_FORMAT),
            (132, 3),
            (136, 0),
            (140, 1),
            (144, 0),
        ] {
            dds[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        dds[84..88].copy_from_slice(b"DX10");
        dds[148..156].copy_from_slice(&VANILLA_CHANNEL3_BGRA8_SRGB_ORACLE);
        dds
    }

    #[test]
    fn channel3_dds_accepts_compatible_legacy_and_dx10_bgra8() {
        let legacy = channel3_legacy_dds();
        assert_eq!(
            decode_gradient_border_channel3_dds(&legacy).unwrap(),
            VANILLA_CHANNEL3_BGRA8_SRGB_ORACLE
        );
        let dx10 = channel3_dx10_dds();
        assert_eq!(
            decode_gradient_border_channel3_dds(&dx10).unwrap(),
            VANILLA_CHANNEL3_BGRA8_SRGB_ORACLE
        );
        let mut implicit_single_mip = channel3_legacy_dds();
        implicit_single_mip[28..32].copy_from_slice(&0u32.to_le_bytes());
        implicit_single_mip.extend_from_slice(b"harmless trailing DDS metadata");
        assert_eq!(
            decode_gradient_border_channel3_dds(&implicit_single_mip).unwrap(),
            VANILLA_CHANNEL3_BGRA8_SRGB_ORACLE
        );
        let mut legal_padded_pitch = channel3_dx10_dds();
        legal_padded_pitch[8..12].copy_from_slice(&0x100fu32.to_le_bytes());
        legal_padded_pitch[20..24].copy_from_slice(&16u32.to_le_bytes());
        legal_padded_pitch.extend_from_slice(&[0; 8]);
        assert_eq!(
            decode_gradient_border_channel3_dds(&legal_padded_pitch).unwrap(),
            VANILLA_CHANNEL3_BGRA8_SRGB_ORACLE
        );

        let mut truncated_padded_pitch = channel3_dx10_dds();
        truncated_padded_pitch[8..12].copy_from_slice(&0x100fu32.to_le_bytes());
        truncated_padded_pitch[20..24].copy_from_slice(&16u32.to_le_bytes());
        assert!(decode_gradient_border_channel3_dds(&truncated_padded_pitch).is_err());
    }

    #[test]
    fn naval_dominance_preserves_independent_channel1_and_channel2_gb() {
        let mut channel1 = [11u8, 12, 13, 14, 21, 22, 23, 24];
        let mut channel2 = [31u8, 32, 33, 34, 41, 42, 43, 44];
        clear_derivative_band(
            &mut channel1,
            &mut channel2,
            2,
            1,
            0,
            Xmap05DerivativeEncoding::NavalDominance,
        );
        assert_eq!(channel1, [11, 12, 13, 14, 21, 22, 23, 24]);
        assert_eq!(channel2, [0, 32, 33, 0, 0, 42, 43, 0]);
        write_derivative_pixel(
            &mut channel1,
            &mut channel2,
            4,
            [1, 2, 3, 127],
            [128, 9, 7, 255],
            Xmap05DerivativeEncoding::NavalDominance,
        );
        assert_eq!(channel1, [11, 12, 13, 14, 21, 22, 23, 24]);
        assert_eq!(channel2, [0, 32, 33, 0, 128, 42, 43, 255]);
    }

    #[test]
    fn channel3_dds_refuses_shape_mip_array_dimension_or_format_drift() {
        let mut wrong_width = channel3_legacy_dds();
        wrong_width[16..20].copy_from_slice(&3u32.to_le_bytes());
        assert!(decode_gradient_border_channel3_dds(&wrong_width).is_err());

        for (offset, value) in [(28, 2u32), (128, 87), (132, 4), (140, 2), (144, 5)] {
            let mut drifted = channel3_dx10_dds();
            drifted[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            assert!(decode_gradient_border_channel3_dds(&drifted).is_err());
        }
    }

    #[test]
    fn channel3_dds_refuses_missing_structure_or_legacy_volume_cube_declarations() {
        let mut missing_mandatory_flags = channel3_legacy_dds();
        missing_mandatory_flags[8..12].copy_from_slice(&0x100bu32.to_le_bytes());
        assert!(decode_gradient_border_channel3_dds(&missing_mandatory_flags).is_err());

        let mut missing_texture_cap = channel3_dx10_dds();
        missing_texture_cap[108..112].copy_from_slice(&0u32.to_le_bytes());
        assert!(decode_gradient_border_channel3_dds(&missing_texture_cap).is_err());

        for caps2 in [0x200u32, 0x200000] {
            let mut legacy_non_2d = channel3_legacy_dds();
            legacy_non_2d[112..116].copy_from_slice(&caps2.to_le_bytes());
            assert!(decode_gradient_border_channel3_dds(&legacy_non_2d).is_err());
        }

        let mut legacy_depth = channel3_legacy_dds();
        legacy_depth[8..12].copy_from_slice(&0x80100fu32.to_le_bytes());
        legacy_depth[24..28].copy_from_slice(&1u32.to_le_bytes());
        assert!(decode_gradient_border_channel3_dds(&legacy_depth).is_err());
    }

    #[test]
    fn exact_gradient_byte_uses_center_fade_and_subtract_eight() {
        assert_eq!(exact_a729_gradient_byte(0.0, 2.0, 5.0), 247);
        assert_eq!(exact_a729_gradient_byte(2.0, 2.0, 5.0), 245);
        assert_eq!(exact_a729_gradient_byte(7.0, 2.0, 5.0), 0);
    }

    #[test]
    fn country_thickness_clamps_and_interpolates_engine_fields() {
        let defines = GradientBorderCountryDefines {
            field_low: 300.0,
            field_high: 9000.0,
            thickness_low: 5.0,
            thickness_high: 25.0,
            center_thickness: 2.0,
            refresh_hours: 10,
            outline_cutoff_strategic: 0.98,
            outline_cutoff_country: 0.973,
            camera_override_strategic: 1.0,
            camera_override_country: 0.0,
            draw_fow_cutoff: 400.0,
            draw_fow_fade_length: 350.0,
        };
        assert_eq!(country_gradient_thickness(1, defines).unwrap(), 5.0);
        assert_eq!(country_gradient_thickness(9001, defines).unwrap(), 25.0);
        assert_eq!(country_gradient_thickness(4650, defines).unwrap(), 15.0);
    }

    #[test]
    fn exact_edt_does_not_invent_an_x_seam_edge() {
        let boundary = [true, false, false, false];
        let distances = squared_euclidean_distance(&boundary, 4, 1).unwrap();
        assert_eq!(distances, [0, 1, 4, 9]);
    }

    #[test]
    fn country_edges_ignore_internal_and_diagonal_only_contacts() {
        let without_diagonal = [
            0, 0, 0, 0, 0,
            0, 1, 1, 1, 0,
            0, 1, 1, 1, 0,
            0, 1, 1, 1, 0,
            0, 0, 0, 0, 0,
        ];
        let regions = [
            0, 0, 0, 0, 0,
            0, 1, 1, 1, 0,
            0, 1, 1, 1, 0,
            0, 1, 1, 1, 0,
            0, 0, 0, 0, 2,
        ];
        let province_regions = [0, 1, 2];
        let baseline_edges =
            country_boundary_points(&without_diagonal, &province_regions, 5, 5).unwrap();
        let edges = country_boundary_points(&regions, &province_regions, 5, 5).unwrap();
        assert!(
            !edges[12],
            "same-country interior must not become a candidate"
        );
        assert!(
            !edges[24],
            "physical raster bounds are not province neighbors"
        );
        for index in [6usize, 7, 8, 11, 12, 13, 16, 17, 18] {
            assert_eq!(
                edges[index], baseline_edges[index],
                "a diagonal-only foreign region must not alter region 1's edge candidates"
            );
        }
    }

    #[test]
    fn country_edges_admit_in_bounds_uncontrolled_neighbors_only() {
        let provinces = [
            1, 1, 1,
            1, 1, 2,
            1, 1, 1,
        ];
        let province_regions = [0, 1, 0];
        let edges = country_boundary_points(&provinces, &province_regions, 3, 3).unwrap();
        assert!(
            edges[4],
            "an in-map sea or uncontrolled province is a real neighbor"
        );
        assert!(!edges[0], "a physical map boundary is not an edge record");
    }

    #[test]
    fn gradient_distance_is_built_before_half_resolution_sampling() {
        let mut province_ids = vec![1u16; 16];
        province_ids[2 * 4 + 2] = 2;
        let raster = Xmap05RasterInputs {
            map_width: 4,
            map_height: 4,
            province_ids,
            province_is_eligible_land: vec![false, true, true],
            gradient: crate::political_color_synthesis::GradientBorderPoliticalInputs {
                province_regions: vec![0, 1, 2],
                width: 2,
                height: 2,
                regions: vec![1, 1, 1, 1],
                land_mask: vec![true; 4],
                region_colors: vec![[0, 0, 0], [10, 20, 30], [40, 50, 60]],
                region_areas: vec![0, 15, 1],
            },
        };
        let defines = GradientBorderCountryDefines {
            field_low: 1.0,
            field_high: 20.0,
            thickness_low: 5.0,
            thickness_high: 5.0,
            center_thickness: 2.0,
            refresh_hours: 10,
            outline_cutoff_strategic: 0.98,
            outline_cutoff_country: 0.973,
            camera_override_strategic: 1.0,
            camera_override_country: 0.0,
            draw_fow_cutoff: 400.0,
            draw_fow_fade_length: 350.0,
        };
        let (channel1, _) = synthesize_gradient_border_channels(&raster, defines).unwrap();
        let first_country_texel = ((2 + 1) * 2) * 4;
        assert_eq!(
            &channel1[first_country_texel..first_country_texel + 4],
            &[10, 20, 30, 247]
        );
    }

    #[test]
    fn graphics_defines_use_the_last_qualified_assignment() {
        let source = r#"
            GRADIENT_BORDERS_FIELD_COUNTRY_REFRESH = 10,
            GRADIENT_BORDERS_FIELD_COUNTRY_LOW = 300,
            GRADIENT_BORDERS_FIELD_COUNTRY_HIGH = 9000,
            GRADIENT_BORDERS_THICKNESS_COUNTRY_LOW = 5,
            GRADIENT_BORDERS_COUNTRY_CENTER_THICKNESS = 2,
            GRADIENT_BORDERS_THICKNESS_COUNTRY_HIGH = 25,
            GRADIENT_BORDERS_OUTLINE_CUTOFF_STRATEGIC_REGIONS = 0.98,
            GRADIENT_BORDERS_OUTLINE_CUTOFF_COUNTRY = 0.973,
            GRADIENT_BORDERS_CAMERA_DISTANCE_OVERRIDE_STRATEGIC_REGIONS = 1,
            GRADIENT_BORDERS_CAMERA_DISTANCE_OVERRIDE_COUNTRY = 0,
            DRAW_FOW_CUTOFF = 400,
            DRAW_FOW_FADE_LENGTH = 350,
        "#;
        let parsed = parse_gradient_border_country_defines(source).unwrap();
        assert_eq!(parsed.refresh_hours, 10);
        assert_eq!(parsed.center_thickness, 2.0);
        let overridden = parse_gradient_border_country_defines(&format!(
            "{source}\nNDefines_Graphics.NGraphics.GRADIENT_BORDERS_FIELD_COUNTRY_LOW = 301,"
        ))
        .unwrap();
        assert_eq!(overridden.field_low, 301.0);
    }

    #[test]
    fn scripted_delegate_band_compatibility_is_layout_not_payload_identity() {
        let descriptor = Xmap05TextureDescriptor {
            symbol: "GradientBorderChannel1".into(),
            width: 8,
            height: 18,
            dxgi_format: GRADIENT_BORDER_CHANNEL_DXGI_FORMAT,
            row_pitch: 32,
            mip_levels: 1,
            payload_sha256: "a".repeat(64),
            descriptor_sha256: "b".repeat(64),
        };
        let mut delegate = descriptor.clone();
        delegate.payload_sha256 = "c".repeat(64);
        delegate.descriptor_sha256 = "d".repeat(64);
        assert!(xmap05_texture_layout_compatible(&descriptor, &delegate));
        delegate.width += 1;
        assert!(!xmap05_texture_layout_compatible(&descriptor, &delegate));
    }

    #[test]
    fn scripted_delegate_copies_layer_zero_scene_law_into_selected_band() {
        let mut base = Xmap05SceneCb0Law {
            outline_cutoff_strategic_bits: 1,
            outline_cutoff_country_bits: 2,
            camera_override_strategic_bits: 3,
            camera_override_country_bits: 4,
            draw_fow_cutoff_bits: 5,
            draw_fow_fade_length_bits: 6,
        };
        let delegate = Xmap05SceneCb0Law {
            outline_cutoff_strategic_bits: 11,
            outline_cutoff_country_bits: 12,
            camera_override_strategic_bits: 13,
            camera_override_country_bits: 14,
            draw_fow_cutoff_bits: 15,
            draw_fow_fade_length_bits: 16,
        };
        copy_engine_evaluated_scene_law_band(&mut base, &delegate, Xmap05MapModeBand::Bottom);
        assert_eq!(base.outline_cutoff_strategic_bits, 1);
        assert_eq!(base.camera_override_strategic_bits, 3);
        assert_eq!(base.outline_cutoff_country_bits, 11);
        assert_eq!(base.camera_override_country_bits, 13);
        assert_eq!(base.draw_fow_cutoff_bits, 5);
        assert_eq!(base.draw_fow_fade_length_bits, 6);

        copy_engine_evaluated_scene_law_band(&mut base, &delegate, Xmap05MapModeBand::Top);
        assert_eq!(base.outline_cutoff_strategic_bits, 11);
        assert_eq!(base.camera_override_strategic_bits, 13);
        assert_eq!(base.outline_cutoff_country_bits, 11);
        assert_eq!(base.camera_override_country_bits, 13);
    }

    #[test]
    fn scripted_delegate_moves_top_pixels_and_scene_law_to_bottom_band() {
        let base = scene_patch_frame();
        base.validate().unwrap();
        let mut delegate = base.clone();
        Arc::make_mut(&mut delegate.gradient_border_channel1.bytes)[0..4]
            .copy_from_slice(&[1, 2, 3, 4]);
        Arc::make_mut(&mut delegate.gradient_border_channel2.bytes)[0..4]
            .copy_from_slice(&[5, 6, 7, 8]);
        delegate.scene_cb0_law.outline_cutoff_strategic_bits = 0.5f32.to_bits();
        delegate.scene_cb0_law.camera_override_strategic_bits = 0.25f32.to_bits();
        for texture in [
            &mut delegate.gradient_border_channel1,
            &mut delegate.gradient_border_channel2,
        ] {
            texture.descriptor.payload_sha256 = Sha256::hex(&texture.bytes);
            texture.descriptor.descriptor_sha256 = texture_descriptor_digest(&texture.descriptor);
        }
        delegate.political_family_sha256 = political_family_digest(&delegate);
        delegate.fow_family_sha256 = fow_family_digest(&delegate);
        delegate.deterministic_sha256 = prepared_political_digest(&delegate);
        delegate.validate().unwrap();

        let derived =
            xmap05_copy_engine_evaluated_band(&base, &delegate, Xmap05MapModeBand::Bottom).unwrap();
        assert_eq!(&derived.gradient_border_channel1.bytes[0..4], &[0; 4]);
        assert_eq!(
            &derived.gradient_border_channel1.bytes[8..12],
            &[1, 2, 3, 4]
        );
        assert_eq!(
            &derived.gradient_border_channel2.bytes[8..12],
            &[5, 6, 7, 8]
        );
        assert_eq!(
            derived.scene_cb0_law.outline_cutoff_country_bits,
            0.5f32.to_bits()
        );
        assert_eq!(
            derived.scene_cb0_law.camera_override_country_bits,
            0.25f32.to_bits()
        );
    }

    #[test]
    fn scripted_bottom_hardcoded_delegates_are_isolated_from_prior_top() {
        let authority = scene_patch_frame();
        let mut composite = authority.clone();
        Arc::make_mut(&mut composite.gradient_border_channel1.bytes)[0..4]
            .copy_from_slice(&[21, 22, 23, 24]);
        Arc::make_mut(&mut composite.gradient_border_channel2.bytes)[0..4]
            .copy_from_slice(&[25, 26, 27, 28]);
        for texture in [
            &mut composite.gradient_border_channel1,
            &mut composite.gradient_border_channel2,
        ] {
            texture.descriptor.payload_sha256 = Sha256::hex(&texture.bytes);
            texture.descriptor.descriptor_sha256 = texture_descriptor_digest(&texture.descriptor);
        }
        composite.political_family_sha256 = political_family_digest(&composite);
        composite.fow_family_sha256 = fow_family_digest(&composite);
        composite.deterministic_sha256 = prepared_political_digest(&composite);
        composite.validate().unwrap();

        let political =
            xmap05_copy_engine_evaluated_band(&composite, &authority, Xmap05MapModeBand::Bottom)
                .unwrap();
        assert_eq!(
            &political.gradient_border_channel1.bytes[0..4],
            &[21, 22, 23, 24]
        );
        assert_eq!(&political.gradient_border_channel1.bytes[8..12], &[0; 4]);

        let mut diplomacy_delegate = authority.clone();
        diplomacy_delegate
            .scene_cb0_law
            .outline_cutoff_strategic_bits = 0.75f32.to_bits();
        diplomacy_delegate
            .scene_cb0_law
            .camera_override_strategic_bits = 0.5f32.to_bits();
        diplomacy_delegate.political_family_sha256 = political_family_digest(&diplomacy_delegate);
        diplomacy_delegate.fow_family_sha256 = fow_family_digest(&diplomacy_delegate);
        diplomacy_delegate.deterministic_sha256 = prepared_political_digest(&diplomacy_delegate);
        diplomacy_delegate.validate().unwrap();
        let diplomacy = xmap05_copy_engine_evaluated_band(
            &composite,
            &diplomacy_delegate,
            Xmap05MapModeBand::Bottom,
        )
        .unwrap();
        assert_eq!(
            &diplomacy.gradient_border_channel1.bytes[0..4],
            &[21, 22, 23, 24]
        );
        assert_eq!(&diplomacy.gradient_border_channel1.bytes[8..12], &[0; 4]);
        assert_eq!(
            diplomacy.scene_cb0_law.outline_cutoff_country_bits,
            0.75f32.to_bits()
        );
        assert_eq!(
            diplomacy.scene_cb0_law.camera_override_country_bits,
            0.5f32.to_bits()
        );
    }

    #[test]
    fn effective_graphics_define_uses_last_assignment_in_last_loaded_file() {
        let sources = vec![
            (
                "common/defines/00_graphics.lua".to_string(),
                "GRADIENT_BORDERS_THICKNESS_STATE = 5".to_string(),
            ),
            (
                "common/defines/nested/mod_override.lua".to_string(),
                "GRADIENT_BORDERS_THICKNESS_STATE = 9".to_string(),
            ),
        ];
        assert_eq!(
            effective_define_from_sources(&sources, "GRADIENT_BORDERS_THICKNESS_STATE").unwrap(),
            Some(("common/defines/nested/mod_override.lua", 9.0))
        );
        let malformed = vec![(
            "common/defines/nested/mod_override.lua".to_string(),
            "GRADIENT_BORDERS_THICKNESS_STATE = 9\nGRADIENT_BORDERS_THICKNESS_STATE = 10"
                .to_string(),
        )];
        assert_eq!(
            effective_define_from_sources(&malformed, "GRADIENT_BORDERS_THICKNESS_STATE").unwrap(),
            Some(("common/defines/nested/mod_override.lua", 10.0))
        );
    }

    #[test]
    fn mapc1_requires_exact_same_hour_complete_authority() {
        let authority = serde_json::json!({
            "schema": "MAPC1",
            "schema_version": 3,
            "producer": "presentation-independent owner/controller epoch",
            "executable_sha256": MAPC1_EXECUTABLE_SHA256,
            "province_controller_field_offset": "0x188",
            "country_remap_rva": "0x00ba6240",
            "ghidra_manifest_sha256": MAPC1_GHIDRA_MANIFEST_SHA256,
            "state_identity_order": "state_id_ascending",
            "province_identity_order": "province_id_ascending",
            "presentation_independent": true,
            "state_owner_complete": true,
            "state_controller_complete": true,
            "province_controller_complete": true,
            "frame_hour": 41,
            "capture_sequence": 7
        });
        let authority = authority.as_object().unwrap();
        validate_mapc1_authority_object(authority, 41).unwrap();
        assert!(validate_mapc1_authority_object(authority, 42).is_err());
    }

    #[test]
    fn cache_identity_tracks_mapc1_and_visibility_but_has_no_camera_input() {
        let keying = KeyingTables::default();
        let provinces = BTreeMap::new();
        let map_a = serde_json::json!({"_r2_xmap05_control_authority":{"capture_sequence":1}});
        let map_b = serde_json::json!({"_r2_xmap05_control_authority":{"capture_sequence":2}});
        let countries = serde_json::json!({"GER":{"_r2_tag_db_id":1}});
        let visibility_a = serde_json::json!({"povs":{"GER":{"capture_sequence":3}}});
        let visibility_b = serde_json::json!({"povs":{"GER":{"capture_sequence":4}}});
        let base = xmap05_semantic_cache_digest(
            10,
            Some("GER"),
            &keying,
            &provinces,
            &map_a,
            &countries,
            Some(&visibility_a),
        )
        .unwrap();
        assert_ne!(
            base,
            xmap05_semantic_cache_digest(
                10,
                Some("GER"),
                &keying,
                &provinces,
                &map_b,
                &countries,
                Some(&visibility_a),
            )
            .unwrap()
        );
        assert_ne!(
            base,
            xmap05_semantic_cache_digest(
                10,
                Some("GER"),
                &keying,
                &provinces,
                &map_a,
                &countries,
                Some(&visibility_b),
            )
            .unwrap()
        );
    }

    /// Observer identity must not depend on the visibility domain at all: the
    /// mode reads no recorded vector, so a POV's recorded bytes changing must
    /// not change an observer frame's identity.
    #[test]
    fn observer_cache_identity_is_blind_to_recorded_visibility() {
        let keying = KeyingTables::default();
        let provinces = BTreeMap::new();
        let map = serde_json::json!({"_r2_xmap05_control_authority":{"capture_sequence":1}});
        let countries = serde_json::json!({"GER":{"_r2_tag_db_id":1}});
        let observer = xmap05_semantic_cache_digest(
            10, None, &keying, &provinces, &map, &countries, None,
        )
        .unwrap();
        for recorded in [
            serde_json::json!({"povs":{"GER":{"capture_sequence":3}}}),
            serde_json::json!({"povs":{"SOV":{"capture_sequence":9999}}}),
        ] {
            assert_eq!(
                observer,
                xmap05_semantic_cache_digest(
                    10, None, &keying, &provinces, &map, &countries, Some(&recorded),
                )
                .unwrap(),
                "observer identity must ignore the visibility domain"
            );
        }
        // ... and a POV render must still bind it.
        let pov_a = xmap05_semantic_cache_digest(
            10,
            Some("GER"),
            &keying,
            &provinces,
            &map,
            &countries,
            Some(&serde_json::json!({"povs":{"GER":{"capture_sequence":3}}})),
        )
        .unwrap();
        let pov_b = xmap05_semantic_cache_digest(
            10,
            Some("GER"),
            &keying,
            &provinces,
            &map,
            &countries,
            Some(&serde_json::json!({"povs":{"GER":{"capture_sequence":4}}})),
        )
        .unwrap();
        assert_ne!(pov_a, pov_b);
        assert_ne!(observer, pov_a);
        assert!(xmap05_semantic_cache_digest(
            10,
            Some("GER"),
            &keying,
            &provinces,
            &map,
            &countries,
            None,
        )
        .is_err());
    }

    #[test]
    fn visibility_digest_separates_land_and_taskforce_domains() {
        let row = Xmap05VisibleEntity {
            type_id: 1,
            stable_id: 2,
            disclosure: ExactUnitDisclosure::MapVisible,
            reason: Xmap05VisibilityReason::ExactD95MapVisible,
        };
        let mut visibility = PreparedXmap05Visibility {
            schema: XMAP05_VISIBILITY_SCHEMA.into(),
            viewer_country: Some("GER".into()),
            fow_enabled: true,
            capture_sequence: Some(1),
            pov_country_id: Some(1),
            province_visibility_sha256: "a".repeat(64),
            land_units: vec![row.clone()],
            taskforce_units: Vec::new(),
            air_wing_spotting_unavailable: true,
            land_frame_sequence: Some(2),
            taskforce_frame_sequence: Some(3),
            deterministic_sha256: String::new(),
        };
        let land_digest = visibility_digest(&visibility);
        visibility.taskforce_units = std::mem::take(&mut visibility.land_units);
        assert_ne!(land_digest, visibility_digest(&visibility));
    }

    /// The producer/consumer contract fixture. Byte-identical to the constant of
    /// the same name in
    /// `replay_interface/native/map_viewer/src/production_features/xmap05.rs`;
    /// `runs/jc_gpu15_mapc1/mapc1_identity_agreement.py` fails if the two drift.
    ///
    /// `country_roster_unresolved` is the frame fact the consumer reconstructs the
    /// roster exclusions from: an emitted `countries` key that carries no
    /// `_r2_tag_db_id`. Empty here, and empty on an ordinary hour.
    const MAPC1_PRODUCER_FIXTURE_V3: &str = r#"{
  "authority": {
    "capture_sequence": 1,
    "country_count": 3,
    "country_remap_rva": "0x00ba6240",
    "country_roster_sha256": "0f627ad7b70505da583453178e6930e79cced4057a2737cdb3c163fa98566aba",
    "executable_sha256": "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad",
    "frame_hour": 16959360,
    "ghidra_manifest_sha256": "a69a7b300422c486ee2b76901a5f39e8e8d2cfe8c2baca38e3b353e4892b1990",
    "presentation_identity_excluded": [],
    "presentation_identity_sha256": "8dfa8a63e9d9c57748f62b336dd9e8b6d565a4c4e028c84cd60482c3571d0818",
    "presentation_independent": true,
    "producer": "presentation-independent owner/controller epoch",
    "province_controller_complete": true,
    "province_controller_field_offset": "0x188",
    "province_controller_sha256": "60addcb31d1fd462d5edb5ecc45df91d564358ffcf0d867075c9bf57d22fda07",
    "province_count": 3,
    "province_identity_order": "province_id_ascending",
    "provinces_outside_state": 2,
    "roster_identity_excluded": [],
    "schema": "MAPC1",
    "schema_version": 3,
    "state_controller_complete": true,
    "state_controller_sha256": "fec78f4253a15ebe82ba8d16c64285dacf275542bfdf457b3acc81d4368011b8",
    "state_count": 3,
    "state_identity_order": "state_id_ascending",
    "state_owner_complete": true,
    "state_owner_sha256": "d1a66bc71abb2fb008c008e1036721b5f208c0fb0cfbfaeadda2754392ed85fc"
  },
  "owner_by_state": { "10": "GER", "11": "FRA", "12": "ITA" },
  "control_by_state": { "10": "GER", "11": "GER", "12": "ITA" },
  "control_by_province": { "100": "GER", "101": "GER", "102": "ITA" },
  "country_roster": [
    { "tag": "FRA", "tag_db_id": 9 },
    { "tag": "GER", "tag_db_id": 2 },
    { "tag": "ITA", "tag_db_id": 7 }
  ],
  "country_roster_unresolved": [],
  "presentation_identity": { "FRA": null, "GER": "GER_REICH", "ITA": null }
}"#;

    /// The same contract, for an hour in which ONE country's cosmetic identity
    /// was not measured (D18 marker 0 for ITA).
    ///
    /// This is the case MAPC1 v2 exists for. Under v1 the producer withheld the
    /// whole envelope here and every XMAP05 input refused; under v2 the envelope is
    /// minted, ITA is named in `presentation_identity_excluded`, the other two
    /// countries are certified, and the political fill draws all three - ITA in its
    /// base colour. Note `presentation_identity` has NO key for ITA at all: a
    /// missing key is the unread state (`Leaf::Absent`), which is exactly what a
    /// JSON null is NOT.
    const MAPC1_EXCLUDED_FIXTURE_V3: &str = r#"{
  "authority": {
    "capture_sequence": 1,
    "country_count": 3,
    "country_remap_rva": "0x00ba6240",
    "country_roster_sha256": "0f627ad7b70505da583453178e6930e79cced4057a2737cdb3c163fa98566aba",
    "executable_sha256": "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad",
    "frame_hour": 16959360,
    "ghidra_manifest_sha256": "a69a7b300422c486ee2b76901a5f39e8e8d2cfe8c2baca38e3b353e4892b1990",
    "presentation_identity_excluded": [ "ITA" ],
    "presentation_identity_sha256": "dda72a91706f310b66c758e33dababc4b217d33a75842ac29ae3f3240847b52e",
    "presentation_independent": true,
    "producer": "presentation-independent owner/controller epoch",
    "province_controller_complete": true,
    "province_controller_field_offset": "0x188",
    "province_controller_sha256": "60addcb31d1fd462d5edb5ecc45df91d564358ffcf0d867075c9bf57d22fda07",
    "province_count": 3,
    "province_identity_order": "province_id_ascending",
    "provinces_outside_state": 2,
    "roster_identity_excluded": [],
    "schema": "MAPC1",
    "schema_version": 3,
    "state_controller_complete": true,
    "state_controller_sha256": "fec78f4253a15ebe82ba8d16c64285dacf275542bfdf457b3acc81d4368011b8",
    "state_count": 3,
    "state_identity_order": "state_id_ascending",
    "state_owner_complete": true,
    "state_owner_sha256": "d1a66bc71abb2fb008c008e1036721b5f208c0fb0cfbfaeadda2754392ed85fc"
  },
  "owner_by_state": { "10": "GER", "11": "FRA", "12": "ITA" },
  "control_by_state": { "10": "GER", "11": "GER", "12": "ITA" },
  "control_by_province": { "100": "GER", "101": "GER", "102": "ITA" },
  "country_roster": [
    { "tag": "FRA", "tag_db_id": 9 },
    { "tag": "GER", "tag_db_id": 2 },
    { "tag": "ITA", "tag_db_id": 7 }
  ],
  "country_roster_unresolved": [],
  "presentation_identity": { "FRA": null, "GER": "GER_REICH" },
  "presentation_identity_excluded": [ "ITA" ]
}"#;

    /// The same contract, for an hour in which one emitted country key carries NO
    /// D18 identity at all (`367` - exactly the shape `captures/oz_tip` holds for
    /// 533 of its 535 hours).
    ///
    /// This is the case MAPC1 v3 exists for. Under v2 the producer withheld the
    /// whole envelope here (`roster()` returned the `unresolved tag_db_ids` error),
    /// so replay's every XMAP05 input went Unavailable over one country that owns
    /// and controls nothing. Under v3 the envelope is minted, `367` is named in
    /// `roster_identity_excluded`, and the three countries the hour COULD name are
    /// certified exactly as in the clean fixture.
    ///
    /// What must move and what must not: `country_count` counts the WHOLE roster
    /// and goes to 4, `country_roster_sha256` moves because the excluded key is
    /// inside its preimage, and `presentation_identity_sha256` does NOT move -
    /// the unnameable country never had a presentation identity to certify or to
    /// exclude. The three control digests do not move either: an unresolvable
    /// country cannot appear in a certified control table, which `control_census`
    /// enforces one step earlier by refusing the hour outright.
    const MAPC1_ROSTER_EXCLUDED_FIXTURE_V3: &str = r#"{
  "authority": {
    "capture_sequence": 1,
    "country_count": 4,
    "country_remap_rva": "0x00ba6240",
    "country_roster_sha256": "97e9f979b2d31ce55465659cb9782dd44cbbc67cc70c819a0bf06142d09a04aa",
    "executable_sha256": "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad",
    "frame_hour": 16959360,
    "ghidra_manifest_sha256": "a69a7b300422c486ee2b76901a5f39e8e8d2cfe8c2baca38e3b353e4892b1990",
    "presentation_identity_excluded": [],
    "presentation_identity_sha256": "8dfa8a63e9d9c57748f62b336dd9e8b6d565a4c4e028c84cd60482c3571d0818",
    "presentation_independent": true,
    "producer": "presentation-independent owner/controller epoch",
    "province_controller_complete": true,
    "province_controller_field_offset": "0x188",
    "province_controller_sha256": "60addcb31d1fd462d5edb5ecc45df91d564358ffcf0d867075c9bf57d22fda07",
    "province_count": 3,
    "province_identity_order": "province_id_ascending",
    "provinces_outside_state": 2,
    "roster_identity_excluded": [ "367" ],
    "schema": "MAPC1",
    "schema_version": 3,
    "state_controller_complete": true,
    "state_controller_sha256": "fec78f4253a15ebe82ba8d16c64285dacf275542bfdf457b3acc81d4368011b8",
    "state_count": 3,
    "state_identity_order": "state_id_ascending",
    "state_owner_complete": true,
    "state_owner_sha256": "d1a66bc71abb2fb008c008e1036721b5f208c0fb0cfbfaeadda2754392ed85fc"
  },
  "owner_by_state": { "10": "GER", "11": "FRA", "12": "ITA" },
  "control_by_state": { "10": "GER", "11": "GER", "12": "ITA" },
  "control_by_province": { "100": "GER", "101": "GER", "102": "ITA" },
  "country_roster": [
    { "tag": "FRA", "tag_db_id": 9 },
    { "tag": "GER", "tag_db_id": 2 },
    { "tag": "ITA", "tag_db_id": 7 }
  ],
  "country_roster_unresolved": [ "367" ],
  "presentation_identity": { "FRA": null, "GER": "GER_REICH", "ITA": null }
}"#;

    struct Mapc1Fixture {
        authority: serde_json::Map<String, serde_json::Value>,
        owners: BTreeMap<u32, String>,
        controllers: BTreeMap<u32, String>,
        province_controllers: BTreeMap<u32, String>,
        roster: Vec<replay_core::D18CountryRosterEntry>,
        /// The emitted `countries` keys with no `_r2_tag_db_id`, which is what
        /// `replay_core::d18_roster` hands the consumer as the roster's
        /// excluded half.
        roster_excluded: Vec<String>,
        cosmetic: BTreeMap<String, Leaf<Option<String>>>,
    }

    impl Mapc1Fixture {
        fn load() -> Self {
            Self::parse(MAPC1_PRODUCER_FIXTURE_V3)
        }

        fn load_excluded() -> Self {
            Self::parse(MAPC1_EXCLUDED_FIXTURE_V3)
        }

        fn load_roster_excluded() -> Self {
            Self::parse(MAPC1_ROSTER_EXCLUDED_FIXTURE_V3)
        }

        fn parse(text: &str) -> Self {
            let parsed: serde_json::Value =
                serde_json::from_str(text).expect("fixture is json");
            let table = |key: &str| -> BTreeMap<u32, String> {
                parsed[key]
                    .as_object()
                    .expect("tag table")
                    .iter()
                    .map(|(id, tag)| {
                        (
                            id.parse::<u32>().expect("numeric id key"),
                            tag.as_str().expect("TAG string").to_string(),
                        )
                    })
                    .collect()
            };
            let roster: Vec<replay_core::D18CountryRosterEntry> = parsed["country_roster"]
                .as_array()
                .expect("roster array")
                .iter()
                .map(|row| replay_core::D18CountryRosterEntry {
                    tag: row["tag"].as_str().expect("roster tag").to_string(),
                    tag_db_id: row["tag_db_id"].as_u64().expect("roster id") as u32,
                })
                .collect();
            let roster_excluded: Vec<String> = parsed["country_roster_unresolved"]
                .as_array()
                .expect("unresolved roster key array")
                .iter()
                .map(|row| row.as_str().expect("unresolved key").to_string())
                .collect();
            let overrides = parsed["presentation_identity"]
                .as_object()
                .expect("presentation identity object");
            let cosmetic = roster
                .iter()
                .map(|row| {
                    let leaf = match overrides.get(&row.tag) {
                        Some(serde_json::Value::Null) => Leaf::Measured(None),
                        Some(value) => Leaf::Measured(Some(
                            value.as_str().expect("cosmetic string").to_string(),
                        )),
                        None => Leaf::Absent,
                    };
                    (row.tag.clone(), leaf)
                })
                .collect();
            Self {
                authority: parsed["authority"]
                    .as_object()
                    .expect("authority object")
                    .clone(),
                owners: table("owner_by_state"),
                controllers: table("control_by_state"),
                province_controllers: table("control_by_province"),
                roster,
                roster_excluded,
                cosmetic,
            }
        }

        fn frame_hour(&self) -> i64 {
            self.authority["frame_hour"].as_i64().expect("frame hour")
        }

        fn presentation_census(&self) -> Result<PresentationCensus> {
            mapc1_presentation_census_from(&self.roster, |tag| self.cosmetic.get(tag))
        }

        /// The producer fixture as a recorded frame's `rest` object, spelled the
        /// way the fold writes it: the three tag tables as `map.*` mirrors, the
        /// roster as `countries.<TAG>._r2_tag_db_id`, an unnameable country as a
        /// `countries` entry WITHOUT that key, and the cosmetic identity at the
        /// ROOT field `countries.<TAG>.cosmetic_tag` (`world_fold.rs`, "Cosmetic
        /// tag (D18)"), absent spelled by writing no key at all.
        ///
        /// The root field and not `extra.cosmetic_tag`: `build_country_identities`
        /// takes the first of `extra.cosmetic_tag`, `misc.cosmetic_tag`,
        /// `cosmetic_tag`, so a fixture on an alias would leave the branch the
        /// recorder actually feeds untested (Codex round 2).
        fn snapshot(&self) -> serde_json::Value {
            let mut countries = serde_json::Map::new();
            for row in &self.roster {
                let mut country = serde_json::json!({"_r2_tag_db_id": row.tag_db_id});
                // `Mapc1Fixture::parse` only ever produces `Measured` or
                // `Absent` here, and absent is spelled by writing no key.
                if let Some(Leaf::Measured(cosmetic)) = self.cosmetic.get(&row.tag) {
                    country["cosmetic_tag"] = serde_json::json!(cosmetic);
                }
                countries.insert(row.tag.clone(), country);
            }
            for key in &self.roster_excluded {
                countries.insert(key.clone(), serde_json::json!({}));
            }
            let table = |rows: &BTreeMap<u32, String>| {
                rows.iter()
                    .map(|(id, tag)| (id.to_string(), serde_json::json!(tag)))
                    .collect::<serde_json::Map<_, _>>()
            };
            serde_json::json!({
                "countries": countries,
                "map": {
                    "owner_by_state": table(&self.owners),
                    "control_by_state": table(&self.controllers),
                    "control_by_province": table(&self.province_controllers),
                }
            })
        }

        fn frame(&self) -> std::sync::Arc<WorldFrame> {
            world_frame_at(self.snapshot(), self.frame_hour())
        }

        fn admit(&self) -> Result<()> {
            validate_mapc1_authority_object(&self.authority, self.frame_hour())?;
            validate_mapc1_payload_tables(
                &self.authority,
                &self.owners,
                &self.controllers,
                &self.province_controllers,
                &self.roster,
                &self.roster_excluded,
                &self.presentation_census()?,
            )
        }
    }

    struct FixtureSource {
        hours: Vec<i64>,
        rest: serde_json::Map<String, serde_json::Value>,
    }

    impl replay_core::ReplaySource for FixtureSource {
        fn game_id(&self) -> &str {
            "xmap05-mapc1-contract"
        }
        fn hours(&self) -> &[i64] {
            &self.hours
        }
        fn warnings(&self) -> &[String] {
            &[]
        }
        fn frame(
            &self,
            hour_index: i64,
        ) -> std::result::Result<replay_core::RawFrame, replay_core::SourceError> {
            Ok(replay_core::RawFrame {
                hour_index,
                date: None,
                save_version: None,
                schema_version: None,
                rest: self.rest.clone(),
            })
        }
    }

    fn world_frame_at(rest: serde_json::Value, hour_index: i64) -> std::sync::Arc<WorldFrame> {
        let timeline = replay_core::WorldTimeline::open(Box::new(FixtureSource {
            hours: vec![hour_index],
            rest: rest.as_object().expect("snapshot object").clone(),
        }))
        .expect("timeline");
        timeline.frame(hour_index).expect("frame")
    }

    /// kills: a fixture route that derives its digests from anything other than
    /// the preimages the recorder writes, and any drift in the frame-side
    /// derivation those preimages are taken over.
    ///
    /// The whole public route runs here — `Mapc1Fixture::snapshot` is opened as
    /// an envelope-less recorded frame and `mapc1_control_authority_envelope`
    /// reads its tables back out — so a swap or mis-read anywhere in
    /// `xmap05_certified_tables` moves a digest and fails this, which is the
    /// hole the first version of this test had (Codex round 1: it fed
    /// hand-assembled tables straight to `mapc1_envelope_from_tables`).
    ///
    /// Every minted field must equal the producer contract fixture's own, on the
    /// two shapes the recorder emits - the clean hour and the
    /// presentation-excluded hour. `provinces_outside_state` is the one producer
    /// field with no minted counterpart: the consumer never reads it, and a
    /// frame does not carry state/province membership to recompute it from.
    ///
    /// `MAPC1_ROSTER_EXCLUDED_FIXTURE_V3` is deliberately NOT here. Lane TL made
    /// that shape unreachable from the current recorder by withholding an
    /// unnameable country key from `countries` upstream, and
    /// `world_fold/tests.rs::mapc1_never_sees_an_unresolved_country_key_because_the_fold_withheld_it`
    /// records that as a finding and assigns the retire-or-keep decision to lane
    /// PQ. A test that says "this is what the producer mints" must not name a
    /// shape the producer cannot mint (Codex round 3). The consumer's own
    /// acceptance of it stays covered by
    /// `mapc1_admits_an_hour_whose_roster_could_not_name_one_country`, which is
    /// untouched and is lane PQ's to adjudicate.
    #[test]
    fn mapc1_minted_envelope_is_the_producer_contract_fixtures_own_authority() {
        for fixture in [Mapc1Fixture::load(), Mapc1Fixture::load_excluded()] {
            let capture_sequence = fixture.authority["capture_sequence"]
                .as_u64()
                .expect("capture sequence");
            // The fixture frame must reach the presentation census through the
            // producer's own root field, with no alias shadowing it.
            for country in fixture.snapshot()["countries"].as_object().unwrap().values() {
                for alias in ["extra", "misc"] {
                    assert!(country.get(alias).is_none(), "{alias} cosmetic alias");
                }
            }
            let frame = fixture.frame();
            let minted = mapc1_control_authority_envelope(&frame, capture_sequence)
                .expect("the fixture frame carries a complete XMAP05 census");
            let minted = minted.as_object().expect("envelope object");
            for (key, value) in minted {
                assert_eq!(Some(value), fixture.authority.get(key), "{key}");
            }
            let unminted = fixture
                .authority
                .keys()
                .filter(|key| !minted.contains_key(*key))
                .cloned()
                .collect::<Vec<_>>();
            assert_eq!(unminted, vec!["provinces_outside_state".to_string()]);

            // ... and the frame the envelope was minted from admits under it,
            // which is the property the four XMAP00 ledger rows rest on.
            let mut carrying = fixture.snapshot();
            carrying["map"]["_r2_xmap05_control_authority"] =
                serde_json::Value::Object(minted.clone());
            admitted_epoch_keying(&world_frame_at(carrying, fixture.frame_hour()))
                .expect("the minted envelope admits the frame it was minted from");
        }
    }

    /// kills: a derivation that reads the state-owner table where the
    /// state-controller table belongs, or vice versa. The producer fixture's two
    /// tables differ on state 11 (owner FRA, controller GER), so exchanging them
    /// must move both digests off the producer's.
    #[test]
    fn mapc1_minting_is_sensitive_to_which_control_table_it_read() {
        let fixture = Mapc1Fixture::load();
        let mut swapped = fixture.snapshot();
        let owners = swapped["map"]["owner_by_state"].clone();
        swapped["map"]["owner_by_state"] = swapped["map"]["control_by_state"].clone();
        swapped["map"]["control_by_state"] = owners;
        let minted = mapc1_control_authority_envelope(
            &world_frame_at(swapped, fixture.frame_hour()),
            fixture.authority["capture_sequence"].as_u64().unwrap(),
        )
        .expect("the swapped frame still carries a complete census");
        for key in ["state_owner_sha256", "state_controller_sha256"] {
            assert_ne!(minted[key], fixture.authority[key], "{key}");
        }
    }

    /// kills: certifying a frame whose control tables name a country the D18
    /// roster could not resolve. `validate_xmap05_state_authority` catches it in
    /// the RENDER path, which is after the four XMAP00 rows have already reported
    /// Present, so the frame-pure certifier has to catch it too (Codex round 1).
    #[test]
    fn mapc1_refuses_a_control_row_naming_a_country_outside_the_resolved_roster() {
        let fixture = Mapc1Fixture::load();
        for table in ["owner_by_state", "control_by_state", "control_by_province"] {
            let mut outside = fixture.snapshot();
            let key = outside["map"][table]
                .as_object()
                .unwrap()
                .keys()
                .next()
                .unwrap()
                .clone();
            outside["map"][table][&key] = serde_json::json!("ZZZ");
            let frame = world_frame_at(outside.clone(), fixture.frame_hour());
            let error = mapc1_control_authority_envelope(&frame, 1)
                .expect_err("an unresolvable control TAG cannot be minted for");
            assert!(
                error.contains("ZZZ") && error.contains("resolved D18 roster"),
                "{error}"
            );

            // ... and it is not certifiable even under the producer's own
            // envelope, which is the frame the four ledger rows would admit.
            outside["map"]["_r2_xmap05_control_authority"] =
                serde_json::Value::Object(fixture.authority.clone());
            admitted_epoch_keying(&world_frame_at(outside, fixture.frame_hour()))
                .expect_err("an unresolvable control TAG refuses the frame");
        }
    }

    #[test]
    fn mapc1_admits_the_extractors_minted_envelope() {
        let fixture = Mapc1Fixture::load();
        fixture.admit().expect("the producer's envelope is admitted");
        assert_eq!(fixture.owners.len(), 3);
        assert_eq!(fixture.province_controllers.len(), 3);
        assert_eq!(fixture.owners.keys().collect::<Vec<_>>(), fixture.controllers.keys().collect::<Vec<_>>());
        assert_eq!(fixture.owners[&11], "FRA");
        assert_eq!(fixture.controllers[&11], "GER");
    }

    /// kills: a viewer that cannot ADMIT an hour whose roster could not name one
    /// country - the case v2 had no envelope for at all, and which cost
    /// `captures/oz_tip` 469 of 535 hours of the whole XMAP05 family.
    ///
    /// Scope, stated exactly because an earlier version of this comment
    /// overclaimed it: this test supplies the roster split DIRECTLY to
    /// `validate_mapc1_payload_tables`, so it exercises the envelope contract
    /// and not `exact_d18_country_roster`. Reverting the roster read would NOT
    /// fail this test; `a_numeric_country_key_is_excluded_and_named_not_fatal`
    /// and `a_present_but_unusable_country_identity_is_still_fatal` in
    /// `replay_core` are what cover the read.
    ///
    /// What production change would make THIS fail: dropping
    /// `roster_identity_excluded` from the digest preimage (the two digests
    /// below become equal), reading the declared list leniently or
    /// absent-tolerantly (the mutation arms stop refusing), counting only the
    /// resolved countries in `country_count`, or letting the declared and
    /// derived splits differ without refusing.
    #[test]
    fn mapc1_admits_an_hour_whose_roster_could_not_name_one_country() {
        let fixture = Mapc1Fixture::load_roster_excluded();
        fixture
            .admit()
            .expect("v3 admits an hour with one unnameable country");
        assert_eq!(fixture.roster.len(), 3, "the three named countries stand");
        assert_eq!(fixture.roster_excluded, vec!["367".to_string()]);
        assert_eq!(
            fixture.authority["country_count"].as_u64(),
            Some(4),
            "country_count is the WHOLE roster, named and unnamed alike"
        );

        // The roster digest is what binds the exclusion. Same three resolved
        // rows, different digest, because the excluded key is in the preimage.
        let clean = Mapc1Fixture::load();
        assert_eq!(clean.roster, fixture.roster);
        assert_ne!(
            clean.authority["country_roster_sha256"],
            fixture.authority["country_roster_sha256"],
            "the excluded key must be inside the roster digest preimage"
        );
        // ... and the presentation identity is untouched: a country nobody could
        // name never had one to certify or to exclude.
        assert_eq!(
            clean.authority["presentation_identity_sha256"],
            fixture.authority["presentation_identity_sha256"]
        );
        assert_eq!(
            clean.authority["presentation_identity_excluded"],
            fixture.authority["presentation_identity_excluded"]
        );

        // Declaring a different exclusion set is refused BY NAME, in both
        // directions. This is the check that stops the envelope from choosing
        // which countries the viewer compares.
        let mut under = Mapc1Fixture::load_roster_excluded();
        under
            .authority
            .insert("roster_identity_excluded".into(), serde_json::json!([]));
        let error = under.admit().expect_err("an under-declared list refuses");
        assert!(
            error.to_string().contains("roster-identity exclusions disagree")
                && error.to_string().contains("367"),
            "the refusal must name the country: {error}"
        );

        let mut over = Mapc1Fixture::load();
        over.authority
            .insert("roster_identity_excluded".into(), serde_json::json!(["367"]));
        over.admit()
            .expect_err("an over-declared list refuses too");

        // REQUIRED and strictly typed, exactly like `presentation_identity_excluded`.
        for broken in [
            serde_json::Value::Null,
            serde_json::json!("367"),
            serde_json::json!([1]),
        ] {
            let mut mutated = Mapc1Fixture::load_roster_excluded();
            mutated
                .authority
                .insert("roster_identity_excluded".into(), broken.clone());
            mutated
                .admit()
                .expect_err("a malformed roster_identity_excluded refuses");
        }
        let mut deleted = Mapc1Fixture::load_roster_excluded();
        deleted.authority.remove("roster_identity_excluded");
        deleted
            .admit()
            .expect_err("a missing roster_identity_excluded refuses");
    }

    #[test]
    fn mapc1_refuses_every_authority_field_mutation() {
        let base = Mapc1Fixture::load();
        base.admit().expect("baseline admits");

        let authority_fields = [
            ("schema", serde_json::json!("MAPC2")),
            // 1 is the RETIRED version, which is the mutation that matters: a
            // recording sealed by the old recorder must be refused by name, not
            // read as v2 and rejected later on a digest it never computed.
            ("schema_version", serde_json::json!(1)),
            ("producer", serde_json::json!("something else")),
            ("executable_sha256", serde_json::json!("0".repeat(64))),
            ("province_controller_field_offset", serde_json::json!("0x190")),
            ("country_remap_rva", serde_json::json!("0x00ba6241")),
            ("ghidra_manifest_sha256", serde_json::json!("0".repeat(64))),
            ("state_identity_order", serde_json::json!("state_id_descending")),
            (
                "province_identity_order",
                serde_json::json!("province_id_descending"),
            ),
            ("presentation_independent", serde_json::json!(false)),
            ("state_owner_complete", serde_json::json!(false)),
            ("state_controller_complete", serde_json::json!(false)),
            ("province_controller_complete", serde_json::json!(false)),
            ("capture_sequence", serde_json::json!(0)),
        ];
        for (field, value) in authority_fields {
            let mut mutated = Mapc1Fixture::load();
            assert!(
                mutated.authority.contains_key(field),
                "{field} is not in the producer's envelope at all"
            );
            mutated.authority.insert(field.to_string(), value);
            assert!(
                mutated.admit().is_err(),
                "a frame with a mutated {field} must be refused"
            );
        }

        let payload_fields = [
            "state_count",
            "province_count",
            "country_count",
            "state_owner_sha256",
            "state_controller_sha256",
            "province_controller_sha256",
            "country_roster_sha256",
            "presentation_identity_sha256",
        ];
        for field in payload_fields {
            let mut mutated = Mapc1Fixture::load();
            let current = mutated
                .authority
                .get(field)
                .unwrap_or_else(|| panic!("{field} is not in the producer's envelope at all"))
                .clone();
            let replacement = match current {
                serde_json::Value::Number(number) => {
                    serde_json::json!(number.as_u64().expect("count is a u64") + 1)
                }
                serde_json::Value::String(_) => serde_json::json!("0".repeat(64)),
                other => panic!("{field} has an unexpected shape: {other}"),
            };
            mutated.authority.insert(field.to_string(), replacement);
            assert!(
                mutated.admit().is_err(),
                "a frame with a mutated {field} must be refused"
            );
        }

        let base = Mapc1Fixture::load();
        let hour = base.frame_hour();
        let mut moved = Mapc1Fixture::load();
        moved
            .authority
            .insert("frame_hour".to_string(), serde_json::json!(hour + 1));
        assert!(
            validate_mapc1_authority_object(&moved.authority, hour).is_err(),
            "a certificate stamped with another hour must be refused"
        );

        let mut extra = Mapc1Fixture::load();
        extra
            .authority
            .insert("provinces_outside_state".to_string(), serde_json::json!(99));
        extra
            .admit()
            .expect("an unpinned accounting field does not gate admission");
    }

    #[test]
    fn mapc1_refuses_every_single_fact_mutation_of_the_admitted_frame() {
        let base = Mapc1Fixture::load();
        base.admit().expect("baseline admits");

        let mut flipped = Mapc1Fixture::load();
        flipped
            .province_controllers
            .insert(102, "GER".to_string());
        assert!(flipped.admit().is_err(), "flipped province controller");

        let mut shrunk = Mapc1Fixture::load();
        shrunk.province_controllers.remove(&100);
        assert!(shrunk.admit().is_err(), "dropped province row");

        let mut swapped = Mapc1Fixture::load();
        swapped.owners.insert(11, "GER".to_string());
        assert!(swapped.admit().is_err(), "owner/controller swap");

        let mut renumbered = Mapc1Fixture::load();
        renumbered.roster[0].tag_db_id = 3;
        assert!(renumbered.admit().is_err(), "renumbered roster identity");

        let mut recosmeticked = Mapc1Fixture::load();
        recosmeticked.cosmetic.insert(
            "FRA".to_string(),
            Leaf::Measured(Some("FRA_VICHY".to_string())),
        );
        assert!(recosmeticked.admit().is_err(), "invented cosmetic override");

        let mut decosmeticked = Mapc1Fixture::load();
        decosmeticked
            .cosmetic
            .insert("GER".to_string(), Leaf::Measured(None));
        assert!(decosmeticked.admit().is_err(), "erased cosmetic override");

        let base = Mapc1Fixture::load();
        assert!(
            validate_mapc1_authority_object(&base.authority, base.frame_hour() + 1).is_err(),
            "stale frame_hour"
        );

        // MALFORMED measurements stay whole-frame refusals. Only a MISSING one
        // is excludable at country grain, and that case has its own test below.
        for (label, leaf) in [
            ("unobtained", Leaf::Unobtained),
            ("blank override", Leaf::Measured(Some("   ".to_string()))),
        ] {
            let mut broken = Mapc1Fixture::load();
            broken.cosmetic.insert("FRA".to_string(), leaf);
            assert!(
                broken.presentation_census().is_err(),
                "{label} cosmetic identity must not be certified"
            );
        }
    }

    #[test]
    fn mapc1_certifies_a_measured_absence_of_cosmetic_override_and_excludes_an_unread_one() {
        let fixture = Mapc1Fixture::load();
        assert_eq!(fixture.cosmetic["FRA"], Leaf::Measured(None));
        assert_eq!(fixture.cosmetic["ITA"], Leaf::Measured(None));
        assert_eq!(
            fixture.cosmetic["GER"],
            Leaf::Measured(Some("GER_REICH".to_string()))
        );
        fixture
            .admit()
            .expect("a measured 'no cosmetic override' is a certifiable identity");

        assert_eq!(
            mapc1_presentation_digest(&fixture.presentation_census().expect("census")),
            fixture.authority["presentation_identity_sha256"]
                .as_str()
                .expect("digest string"),
            "the consumer's digest is the value the producer minted"
        );

        assert_eq!(
            admitted_cosmetic_identity(Some(&Leaf::Absent)),
            Ok(CosmeticAdmission::Unmeasured),
            "an unreadable cosmetic field is excludable, not a refusal"
        );
        assert_eq!(
            admitted_cosmetic_identity(Some(&Leaf::Measured(None))),
            Ok(CosmeticAdmission::NoOverride),
            "a measured 'no override' is a presentation identity: the base TAG"
        );

        // The v1 behaviour, and why it is gone: a roster country whose cosmetic
        // field was never carried used to refuse the WHOLE frame, which took the
        // three control tables and all four fog-of-war POV rows down with it.
        let mut unread = Mapc1Fixture::load();
        unread.cosmetic.insert("FRA".to_string(), Leaf::Absent);
        assert!(
            unread.admit().is_err(),
            "the fully-measured envelope must not admit a frame that excludes a country:              its declared exclusion list is empty and its digest says so"
        );
        let census = unread.presentation_census().expect("the census is derivable");
        assert_eq!(census.excluded, vec!["FRA".to_string()]);
        assert_eq!(census.certified.len(), 2);
    }

    /// kills: reverting the consumer to the whole-roster presentation identity.
    ///
    /// This fixture is the producer's own minted output for an hour in which
    /// ITA's cosmetic field was unreadable - the case that refused the entire
    /// XMAP05 family under v1. The assertion is that it ADMITS, that the viewer
    /// derived the same split the recorder declared, and that one accounted row
    /// comes out of it.
    #[test]
    fn mapc1_admits_a_frame_with_one_excluded_presentation_identity() {
        let fixture = Mapc1Fixture::load_excluded();
        assert_eq!(fixture.cosmetic["ITA"], Leaf::Absent);
        fixture
            .admit()
            .expect("one unread cosmetic identity does not refuse the family");

        let census = fixture.presentation_census().expect("census");
        assert_eq!(census.excluded, vec!["ITA".to_string()]);
        assert_eq!(census.certified.len(), 2, "FRA and GER stay certified");

        let rows = excluded_presentation_identities(&census);
        assert_eq!(rows.len(), 1, "one accounted row per excluded country");
        rows[0].validate().expect("the row is a legal diagnostic");
        assert_eq!(rows[0].element_id, "XMAP05:presentation-identity:ITA");
        assert_eq!(rows[0].element_count, 1);
        assert_eq!(rows[0].family, MapVisualFamily::PoliticalFillGradients);

        // The row must say the fill DREW, from a substitute. An earlier draft
        // used `ExcludedUnknownInput`, which says the element never reached the
        // frame - false, because `KeyingTables::cosmetic_by_tag` simply has no
        // entry for ITA and the base colour is painted. It must also not be
        // structural, or the hour would stop counting this against its own
        // completeness.
        assert_eq!(
            rows[0].outcome,
            ElementOutcome::DrawnFromDocumentedSubstitute
        );
        assert!(rows[0].outcome.draws(), "ITA's political fill is drawn");
        assert!(!rows[0].outcome.is_structural());

        // And the substitute really is what the keying table produces: an
        // excluded country contributes no cosmetic override, so the colour
        // lookup falls through to its base colour.
        let keying_source: BTreeMap<String, Leaf<Option<String>>> = fixture.cosmetic.clone();
        let cosmetic_by_tag: BTreeMap<String, String> = keying_source
            .iter()
            .filter_map(|(tag, leaf)| match leaf {
                Leaf::Measured(Some(cosmetic)) => Some((tag.clone(), cosmetic.clone())),
                _ => None,
            })
            .collect();
        assert!(
            !cosmetic_by_tag.contains_key("ITA"),
            "an excluded country must apply no cosmetic override"
        );
        assert_eq!(cosmetic_by_tag.get("GER").map(String::as_str), Some("GER_REICH"));

        // The control tables are untouched: ITA still owns and controls state 12
        // and province 102. The hole is the cosmetic identity, and only that.
        assert_eq!(fixture.owners[&12], "ITA");
        assert_eq!(fixture.province_controllers[&102], "ITA");
    }

    /// kills: reading `presentation_identity_excluded` leniently.
    ///
    /// The first draft used `filter_map(as_str).unwrap_or_default()`, so on an
    /// ordinary frame - the common case, with no exclusions - the field could be
    /// DELETED, or replaced with a number, and the envelope still admitted; and
    /// on a frame with exclusions a null member was silently dropped, so
    /// `["ICE", null, "URG"]` read as `["ICE","URG"]`. A field the certificate
    /// depends on has to be absent-intolerant and strictly typed, or it is not
    /// part of the certificate.
    #[test]
    fn mapc1_refuses_a_missing_or_malformed_exclusion_list() {
        for (label, value) in [
            ("deleted", None),
            ("not an array", Some(serde_json::json!("ITA"))),
            ("null", Some(serde_json::json!(null))),
            (
                "a non-string member",
                Some(serde_json::json!(["ITA", serde_json::Value::Null])),
            ),
            ("a numeric member", Some(serde_json::json!([7]))),
        ] {
            let mut broken = Mapc1Fixture::load_excluded();
            match value {
                None => {
                    broken.authority.remove("presentation_identity_excluded");
                }
                Some(value) => {
                    broken
                        .authority
                        .insert("presentation_identity_excluded".into(), value);
                }
            }
            assert!(
                broken.admit().is_err(),
                "{label} presentation_identity_excluded must be refused"
            );
        }

        // The same, on the ORDINARY frame whose list is empty - the case a
        // lenient reader could not tell from a deleted field at all.
        let mut deleted = Mapc1Fixture::load();
        deleted.authority.remove("presentation_identity_excluded");
        assert!(
            deleted.admit().is_err(),
            "an empty exclusion list still has to be present"
        );
    }

    /// kills: relaxing the exclusion comparison to "whatever the recorder said".
    ///
    /// A recorder that declared ITA excluded while the frame still carried ITA's
    /// cosmetic identity would be describing a different measurement than the one
    /// on the wire. The digest alone already catches it; this asserts the frame
    /// is refused with the countries NAMED, which the digest mismatch cannot do.
    #[test]
    fn mapc1_refuses_when_the_declared_and_derived_exclusions_disagree() {
        let mut fixture = Mapc1Fixture::load_excluded();
        fixture
            .cosmetic
            .insert("ITA".to_string(), Leaf::Measured(None));
        let error = fixture.admit().expect_err("the two splits disagree");
        let text = error.to_string();
        assert!(
            text.contains("exclusions disagree") && text.contains("ITA"),
            "the refusal must name the country: {text}"
        );
    }
}
