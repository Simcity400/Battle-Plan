
use crate::{
    build_mapname_vertices_with_height, compute_controller_label_placements_with_text,
    compute_explicit_area_label_placements_with_text, decode_bmp_indexed,
    draw_country_names_cutoff_from_resolver, mapname_tracked_advance_px, mapname_tracking_px,
    merge_localisation_source_exact, parse_bmfont, sample_map_height_a729,
    ControllerLabelLawInputs, CountryLabelPlacement, ExplicitAreaLabelLawInputs,
    FoundationFeatureContext, FoundationFeatureOutput, GameFileResolver, IndexImage,
    LabelTextMetrics, MapnameVertex, ProvinceKind, Result,
};
use portable_content_manifest::UsedContentKey;
use portable_render_stream::FamilyDisposition;
use replay_core::{Leaf, ValidatedContentAdmission, ViewStatus, WorldFrame};
use serde::{Deserialize, Serialize};
use snapshot::sha256::Sha256;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;

pub(crate) const XMAP17_LABEL_SCHEMA: &str = "hoi4.xmap17.map-names.a729.v2";
pub const XMAP17_PREPARE_STAGE: &str = "xmap17.labels.prepare";
pub const XMAP17_PREPARE_OPERATION: &str = "labels.prepare";
pub const XMAP17_EXECUTE_STAGE: &str = "xmap17.labels.execute";
pub const XMAP17_EXECUTE_OPERATION: &str = "labels.draw";
const MAPNAME_EFFECT_FILE: &str = "gfx/FX/mapname.shader";
const MAPNAME_EFFECT_NAME: &str = "mapname";
const MAPNAME_PLATFORM_HEADER: &str = "gfx/FX/defines_hlsl_dx11.fxh";
const MAPNAME_GRAPHICS_DEFINES: &str = "common/defines/00_graphics.lua";
const MAPNAME_CORE_GFX: &str = "interface/core.gfx";
const MAPNAME_FONT_TOKEN: &str = "tahoma_60";
const MAPNAME_PROVINCES: &str = "map/provinces.bmp";
const MAPNAME_DEFINITION: &str = "map/definition.csv";
const MAPNAME_HEIGHTMAP: &str = "map/heightmap.bmp";
const MAPNAME_ADJACENCIES: &str = "map/adjacencies.csv";
const MAPNAME_VERTEX_STRIDE: usize = 20;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MapNameAlphaState {
    pub near_alpha: f32,
    pub far_alpha: f32,
}

impl MapNameAlphaState {
    pub fn new(near_alpha: f32, far_alpha: f32) -> Result<Self> {
        if !near_alpha.is_finite()
            || !far_alpha.is_finite()
            || !(0.0..=1.0).contains(&near_alpha)
            || !(0.0..=1.0).contains(&far_alpha)
        {
            return Err("XMAP17 alpha state is outside [0,1]".into());
        }
        Ok(Self {
            near_alpha,
            far_alpha,
        })
    }

    pub fn update(&mut self, camera_height: f32, cutoff: f32, delta: f32) -> Result<()> {
        if !camera_height.is_finite() || !cutoff.is_finite() || !delta.is_finite() || delta < 0.0 {
            return Err("XMAP17 alpha transition received invalid binary32 input".into());
        }
        if camera_height >= cutoff {
            self.far_alpha = (self.far_alpha + delta).clamp(0.0, 1.0);
            self.near_alpha = (self.near_alpha - delta).clamp(0.0, 1.0);
        } else {
            self.far_alpha = (self.far_alpha - delta).clamp(0.0, 1.0);
            self.near_alpha = (self.near_alpha + delta).clamp(0.0, 1.0);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedMapLabelProgram {
    pub effect_file: String,
    pub effect_name: String,
    pub ordered_shader_content: Vec<UsedContentKey>,
    pub vertex_bytecode: Arc<[u8]>,
    pub pixel_bytecode: Arc<[u8]>,
    pub vertex_bytecode_sha256: String,
    pub pixel_bytecode_sha256: String,
    pub font_descriptor: UsedContentKey,
    pub font_atlas: UsedContentKey,
    pub deterministic_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedMapLabelRow {
    pub key: String,
    pub localized_text: String,
    pub component_id: u32,
    pub first_vertex: u32,
    pub vertex_count: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedMapLabelSet {
    pub selector: u8,
    pub rows: Vec<PreparedMapLabelRow>,
    pub vertex_bytes: Arc<[u8]>,
    pub deterministic_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedMapLabelFrame {
    pub schema: String,
    pub hour_index: i64,
    pub content_manifest_sha256: String,
    pub product_modset_sha256: String,
    pub effective_vfs_manifest_sha256: String,
    pub foundation_content_operation_sha256: String,
    pub viewer_state_sha256: String,
    pub map_width: u32,
    pub map_height: u32,
    pub viewer_camera_sha256: String,
    pub camera_height_f32_bits: u32,
    pub cutoff_f32_bits: u32,
    pub map_copy_receipt_set_sha256: String,
    pub wrap_offset_f32_bits: Vec<u32>,
    pub ordered_used_content: Vec<UsedContentKey>,
    pub program: PreparedMapLabelProgram,
    pub country: PreparedMapLabelSet,
    pub state: PreparedMapLabelSet,
    pub faction: Option<PreparedMapLabelSet>,
    pub faction_source_sha256: Option<String>,
    pub player: Option<PreparedMapLabelSet>,
    pub player_carrier_record_sha256: Option<String>,
    pub deterministic_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MapLabelFinalImageDependencies {
    pub scene_cb0_sha256: String,
    pub final_image_target_sha256: String,
    pub xmap10_operation_order_sha256: String,
    pub near_selector: u8,
    pub far_selector: u8,
    pub update_daily: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadyMapLabelDrawContract {
    pub hour_index: i64,
    pub label_frame_sha256: String,
    pub scene_cb0_sha256: String,
    pub final_image_target_sha256: String,
    pub xmap10_operation_order_sha256: String,
    pub near_selector: u8,
    pub far_selector: u8,
    pub update_daily: bool,
    pub near_alpha_f32_bits: u32,
    pub far_alpha_f32_bits: u32,
    pub ordered_set_sha256: Vec<String>,
    pub deterministic_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Xmap17ExecutionIdentity {
    pub hour_index: i64,
    pub content_manifest_sha256: String,
    pub foundation_content_operation_sha256: String,
    pub viewer_state_sha256: String,
    pub main_scene_target_sha256: String,
    pub map_copy_receipt_set_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Xmap17DrawDisposition {
    Present,
    Empty { reason: String },
    NotApplicable { reason: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Xmap17DrawReceipt {
    pub identity: Xmap17ExecutionIdentity,
    pub assigned_stage: portable_render_stream::AssignedStage,
    pub label_frame_sha256: String,
    pub ready_draw_sha256: String,
    pub xmap10_operation_order_sha256: String,
    pub near_selector: u8,
    pub far_selector: u8,
    pub update_daily: bool,
    pub near_alpha_f32_bits: u32,
    pub far_alpha_f32_bits: u32,
    pub draw_count: u32,
    pub submitted_vertex_count: u64,
    pub disposition: Xmap17DrawDisposition,
    pub packet_sha256: String,
}

impl ReadyMapLabelDrawContract {
    pub fn validate(&self) -> Result<()> {
        for (value, label) in [
            (&self.label_frame_sha256, "XMAP17 ready label frame"),
            (&self.scene_cb0_sha256, "XMAP17 ready scene CB0"),
            (
                &self.final_image_target_sha256,
                "XMAP17 ready final-image target",
            ),
            (
                &self.xmap10_operation_order_sha256,
                "XMAP17 ready XMAP10 order",
            ),
        ] {
            validate_sha256(value, label)?;
        }
        if self.near_selector > 4
            || self.far_selector > 4
            || self.ordered_set_sha256.len() != 2
            || self
                .ordered_set_sha256
                .iter()
                .any(|digest| validate_sha256(digest, "XMAP17 ready label set").is_err())
        {
            return Err("XMAP17 ready draw ordering is invalid".into());
        }
        for bits in [self.near_alpha_f32_bits, self.far_alpha_f32_bits] {
            let alpha = f32::from_bits(bits);
            if !alpha.is_finite() || !(0.0..=1.0).contains(&alpha) {
                return Err("XMAP17 ready draw alpha is outside [0,1]".into());
            }
        }
        if self.deterministic_sha256 != ready_map_label_draw_digest(self) {
            return Err("XMAP17 ready draw digest mismatch".into());
        }
        Ok(())
    }

    pub fn next_alpha_state(&self) -> Result<MapNameAlphaState> {
        self.validate()?;
        MapNameAlphaState::new(
            f32::from_bits(self.near_alpha_f32_bits),
            f32::from_bits(self.far_alpha_f32_bits),
        )
    }
}

impl PreparedMapLabelProgram {
    fn validate(&self) -> Result<()> {
        if self.effect_file != MAPNAME_EFFECT_FILE.to_ascii_lowercase()
            || self.effect_name != MAPNAME_EFFECT_NAME
            || self.ordered_shader_content.is_empty()
            || self.vertex_bytecode.is_empty()
            || self.pixel_bytecode.is_empty()
            || self.vertex_bytecode_sha256 != Sha256::hex(&self.vertex_bytecode)
            || self.pixel_bytecode_sha256 != Sha256::hex(&self.pixel_bytecode)
            || self.deterministic_sha256 != map_label_program_digest(self)
        {
            return Err("XMAP17 executable mapname program is invalid".into());
        }
        self.font_descriptor
            .validate()
            .map_err(|error| format!("XMAP17 font descriptor identity: {error}"))?;
        self.font_atlas
            .validate()
            .map_err(|error| format!("XMAP17 font atlas identity: {error}"))?;
        Ok(())
    }
}

impl PreparedMapLabelSet {
    fn validate(&self) -> Result<()> {
        if !matches!(self.selector, 0..=3)
            || self.vertex_bytes.len() % MAPNAME_VERTEX_STRIDE != 0
            || self.rows.is_empty() != self.vertex_bytes.is_empty()
            || self.deterministic_sha256 != map_label_set_digest(self)
        {
            return Err("XMAP17 prepared label set is invalid".into());
        }
        let total_vertices = u32::try_from(self.vertex_bytes.len() / MAPNAME_VERTEX_STRIDE)?;
        let mut cursor = 0u32;
        for row in &self.rows {
            if row.key.trim().is_empty()
                || row.localized_text.trim().is_empty()
                || row.vertex_count == 0
                || row.vertex_count % 6 != 0
                || row.first_vertex != cursor
            {
                return Err("XMAP17 label row order/range is invalid".into());
            }
            cursor = cursor
                .checked_add(row.vertex_count)
                .ok_or("XMAP17 label vertex range overflow")?;
        }
        if cursor != total_vertices {
            return Err("XMAP17 label rows do not cover the vertex payload".into());
        }
        Ok(())
    }
}

impl PreparedMapLabelFrame {
    pub fn validate(&self) -> Result<()> {
        if self.schema != XMAP17_LABEL_SCHEMA
            || self.map_width == 0
            || self.map_height == 0
            || self.wrap_offset_f32_bits.is_empty()
            || self.wrap_offset_f32_bits[0] != 0.0f32.to_bits()
            || self.ordered_used_content.is_empty()
            || self.country.selector != 0
            || self.state.selector != 2
            || self.deterministic_sha256 != map_label_frame_digest(self)
        {
            return Err("XMAP17 prepared label frame is incomplete or unsealed".into());
        }
        validate_sha256(&self.viewer_camera_sha256, "XMAP17 viewer camera")?;
        validate_sha256(&self.content_manifest_sha256, "XMAP17 content manifest")?;
        validate_sha256(&self.product_modset_sha256, "XMAP17 product modset")?;
        validate_sha256(
            &self.effective_vfs_manifest_sha256,
            "XMAP17 effective VFS manifest",
        )?;
        validate_sha256(
            &self.foundation_content_operation_sha256,
            "XMAP17 foundation content operation",
        )?;
        validate_sha256(&self.viewer_state_sha256, "XMAP17 viewer state")?;
        validate_sha256(
            &self.map_copy_receipt_set_sha256,
            "XMAP17 map-copy receipt set",
        )?;
        let height = f32::from_bits(self.camera_height_f32_bits);
        let cutoff = f32::from_bits(self.cutoff_f32_bits);
        if !height.is_finite() || !cutoff.is_finite() || cutoff <= 0.0 {
            return Err("XMAP17 camera/cutoff payload is invalid".into());
        }
        let mut wraps = BTreeSet::new();
        for bits in &self.wrap_offset_f32_bits {
            let offset = f32::from_bits(*bits);
            if !offset.is_finite() || !wraps.insert(*bits) {
                return Err("XMAP17 wrap offsets are non-finite or duplicated".into());
            }
        }
        let mut paths = BTreeSet::new();
        for content in &self.ordered_used_content {
            content
                .validate()
                .map_err(|error| format!("XMAP17 used-content identity: {error}"))?;
            if !paths.insert(content.normalized_game_relative_path.as_str()) {
                return Err("XMAP17 used-content path is duplicated".into());
            }
        }
        self.program.validate()?;
        self.country.validate()?;
        self.state.validate()?;
        if let Some(faction) = &self.faction {
            faction.validate()?;
            if faction.selector != 1 {
                return Err("XMAP17 faction label set has the wrong selector".into());
            }
            validate_sha256(
                self.faction_source_sha256
                    .as_deref()
                    .ok_or("XMAP17 faction set has no source identity")?,
                "XMAP17 faction source",
            )?;
        } else if self.faction_source_sha256.is_some() {
            return Err("XMAP17 unavailable faction set retained a source identity".into());
        }
        if let Some(player) = &self.player {
            player.validate()?;
            if player.selector != 3 {
                return Err("XMAP17 player label set has the wrong selector".into());
            }
            validate_sha256(
                self.player_carrier_record_sha256
                    .as_deref()
                    .ok_or("XMAP17 player set has no carrier identity")?,
                "XMAP17 player carrier",
            )?;
        } else if self.player_carrier_record_sha256.is_some() {
            return Err("XMAP17 unavailable player set retained a carrier identity".into());
        }
        Ok(())
    }

    pub fn selected_label_set(&self, selector: u8) -> Result<Option<&PreparedMapLabelSet>> {
        Ok(match selector {
            0 => Some(&self.country),
            1 => Some(self.faction.as_ref().ok_or(
                "XMAP17 faction labels require a complete exact faction roster/name join",
            )?),
            2 => Some(&self.state),
            3 => Some(self.player.as_ref().ok_or(
                "XMAP17 player labels require the passive exact player identity/text carrier",
            )?),
            4 => None,
            other => return Err(format!("XMAP17 unsupported EMapNameMode selector {other}").into()),
        })
    }

    pub fn admit_final_image_dependencies(
        &self,
        dependencies: MapLabelFinalImageDependencies,
        alpha_state: MapNameAlphaState,
        render_delta: f32,
    ) -> Result<ReadyMapLabelDrawContract> {
        self.validate()?;
        validate_sha256(&dependencies.scene_cb0_sha256, "XMAP17 scene CB0")?;
        validate_sha256(
            &dependencies.final_image_target_sha256,
            "XMAP17 final-image target",
        )?;
        validate_sha256(
            &dependencies.xmap10_operation_order_sha256,
            "XMAP17 XMAP10 operation order",
        )?;
        let near = self.selected_label_set(dependencies.near_selector)?;
        let far = self.selected_label_set(dependencies.far_selector)?;
        let ordered_set_sha256 = vec![
            near.map_or_else(map_label_none_digest, |set| {
                set.deterministic_sha256.clone()
            }),
            far.map_or_else(map_label_none_digest, |set| {
                set.deterministic_sha256.clone()
            }),
        ];
        let mut next_alpha_state = alpha_state;
        next_alpha_state.update(
            f32::from_bits(self.camera_height_f32_bits),
            f32::from_bits(self.cutoff_f32_bits),
            render_delta,
        )?;
        let mut ready = ReadyMapLabelDrawContract {
            hour_index: self.hour_index,
            label_frame_sha256: self.deterministic_sha256.clone(),
            scene_cb0_sha256: dependencies.scene_cb0_sha256,
            final_image_target_sha256: dependencies.final_image_target_sha256,
            xmap10_operation_order_sha256: dependencies.xmap10_operation_order_sha256,
            near_selector: dependencies.near_selector,
            far_selector: dependencies.far_selector,
            update_daily: dependencies.update_daily,
            near_alpha_f32_bits: next_alpha_state.near_alpha.to_bits(),
            far_alpha_f32_bits: next_alpha_state.far_alpha.to_bits(),
            ordered_set_sha256,
            deterministic_sha256: String::new(),
        };
        ready.deterministic_sha256 = ready_map_label_draw_digest(&ready);
        ready.validate()?;
        Ok(ready)
    }

    pub fn preflight_execution(
        &self,
        ready: &ReadyMapLabelDrawContract,
        identity: &Xmap17ExecutionIdentity,
        assigned_stage: &portable_render_stream::AssignedStage,
    ) -> Result<(u32, u64)> {
        self.validate()?;
        ready.validate()?;
        assigned_stage
            .validate()
            .map_err(|error| format!("XMAP17 assigned execute stage: {error}"))?;
        let canonical_schedule = portable_render_stream::AssignedStageSchedule::from_topology(
            &crate::product_foundation_assigned_stage_topology()?,
        )
        .map_err(|error| format!("XMAP17 canonical product schedule: {error}"))?;
        let canonical_stage = canonical_schedule
            .require(XMAP17_EXECUTE_STAGE, XMAP17_EXECUTE_OPERATION)
            .map_err(|error| format!("XMAP17 canonical execute stage: {error}"))?;
        if ready.label_frame_sha256 != self.deterministic_sha256
            || ready.hour_index != self.hour_index
            || ready.xmap10_operation_order_sha256 != canonical_schedule.schedule_sha256
            || identity.hour_index != self.hour_index
            || identity.content_manifest_sha256 != self.content_manifest_sha256
            || identity.foundation_content_operation_sha256
                != self.foundation_content_operation_sha256
            || identity.viewer_state_sha256 != self.viewer_state_sha256
            || identity.main_scene_target_sha256 != ready.final_image_target_sha256
            || identity.map_copy_receipt_set_sha256 != self.map_copy_receipt_set_sha256
        {
            return Err("XMAP17 execution identities disagree with the prepared frame".into());
        }
        for (value, label) in [
            (&identity.content_manifest_sha256, "XMAP17 content manifest"),
            (
                &identity.foundation_content_operation_sha256,
                "XMAP17 foundation content operation",
            ),
            (&identity.viewer_state_sha256, "XMAP17 viewer state"),
            (
                &identity.main_scene_target_sha256,
                "XMAP17 main-scene target",
            ),
            (
                &identity.map_copy_receipt_set_sha256,
                "XMAP17 map-copy receipt set",
            ),
        ] {
            validate_sha256(value, label)?;
        }
        if assigned_stage != canonical_stage {
            return Err(
                "XMAP17 execute assignment differs from the canonical product schedule".into(),
            );
        }
        let near = self.selected_label_set(ready.near_selector)?;
        let far = self.selected_label_set(ready.far_selector)?;
        let copies = u64::try_from(self.wrap_offset_f32_bits.len())?;
        let expected_draws = u32::try_from(
            (near.map_or(0, |set| set.rows.len()) + far.map_or(0, |set| set.rows.len()))
                .checked_mul(self.wrap_offset_f32_bits.len())
                .ok_or("XMAP17 draw-count overflow")?,
        )?;
        let expected_vertices = near
            .into_iter()
            .flat_map(|set| &set.rows)
            .chain(far.into_iter().flat_map(|set| &set.rows))
            .map(|row| u64::from(row.vertex_count))
            .sum::<u64>()
            .checked_mul(copies)
            .ok_or("XMAP17 submitted-vertex overflow")?;
        Ok((expected_draws, expected_vertices))
    }

    pub fn seal_execution_receipt(
        &self,
        ready: &ReadyMapLabelDrawContract,
        identity: Xmap17ExecutionIdentity,
        assigned_stage: portable_render_stream::AssignedStage,
        draw_count: u32,
        submitted_vertex_count: u64,
    ) -> Result<Xmap17DrawReceipt> {
        let (expected_draws, expected_vertices) =
            self.preflight_execution(ready, &identity, &assigned_stage)?;
        if draw_count != expected_draws || submitted_vertex_count != expected_vertices {
            return Err("XMAP17 native draw totals disagree with prepared label rows".into());
        }
        let disposition = if expected_draws == 0 {
            Xmap17DrawDisposition::Empty {
                reason: "selected near and far label vectors contain no drawable glyph rows".into(),
            }
        } else {
            Xmap17DrawDisposition::Present
        };
        let mut receipt = Xmap17DrawReceipt {
            identity,
            assigned_stage,
            label_frame_sha256: self.deterministic_sha256.clone(),
            ready_draw_sha256: ready.deterministic_sha256.clone(),
            xmap10_operation_order_sha256: ready.xmap10_operation_order_sha256.clone(),
            near_selector: ready.near_selector,
            far_selector: ready.far_selector,
            update_daily: ready.update_daily,
            near_alpha_f32_bits: ready.near_alpha_f32_bits,
            far_alpha_f32_bits: ready.far_alpha_f32_bits,
            draw_count,
            submitted_vertex_count,
            disposition,
            packet_sha256: String::new(),
        };
        receipt.packet_sha256 = xmap17_draw_receipt_digest(&receipt)?;
        Ok(receipt)
    }
}

struct Xmap17Content {
    content_manifest_sha256: String,
    product_modset_sha256: String,
    ordered_used_content: Vec<UsedContentKey>,
    province_ids: Vec<u32>,
    width: usize,
    height: usize,
    province_to_state: BTreeMap<u32, u32>,
    impassable_states: BTreeSet<u32>,
    province_importance: BTreeMap<u32, u64>,
    province_type: BTreeMap<u32, ProvinceKind>,
    province_adjacencies: BTreeSet<(u32, u32)>,
    state_name_keys: BTreeMap<u32, String>,
    area_by_province: BTreeMap<u32, String>,
    localization: BTreeMap<String, String>,
    font: crate::BmFont,
    heightmap: IndexImage,
    cutoff: f32,
    program: PreparedMapLabelProgram,
}

impl Xmap17Content {
    fn admit(
        resolver: &GameFileResolver,
        admission: &ValidatedContentAdmission,
        localisation_language: &str,
    ) -> Result<Self> {
        let mut ordered = BTreeMap::<String, UsedContentKey>::new();
        let mut require = |path: &str| -> Result<(UsedContentKey, Vec<u8>)> {
            let key = admission
                .require_used_content(path)
                .map_err(|error| format!("XMAP17 requires admitted {path}: {error}"))?
                .clone();
            let bytes = resolver.read_admitted_used_content(&key)?;
            ordered.insert(path.to_ascii_lowercase(), key.clone());
            Ok((key, bytes))
        };

        let (_, definition_bytes) = require(MAPNAME_DEFINITION)?;
        let (_, provinces_bytes) = require(MAPNAME_PROVINCES)?;
        let (_, heightmap_bytes) = require(MAPNAME_HEIGHTMAP)?;
        let (_, adjacency_bytes) = require(MAPNAME_ADJACENCIES)?;
        let (_, core_gfx_bytes) = require(MAPNAME_CORE_GFX)?;
        let font_base = selected_mapname_font_base(
            &crate::latin1_bytes_to_string(&core_gfx_bytes),
            localisation_language,
        )?;
        let font_descriptor_path = format!("{font_base}.fnt");
        let font_atlas_path = format!("{font_base}.dds");
        let (font_descriptor, font_bytes) = require(&font_descriptor_path)?;
        let (font_atlas, _) = require(&font_atlas_path)?;
        let (_, graphics_bytes) = require(MAPNAME_GRAPHICS_DEFINES)?;

        if definition_bytes.is_empty() {
            return Err("XMAP17 admitted definition.csv is empty".into());
        }
        let (definitions, color_to_id) = crate::decode_definition_csv(
            &crate::latin1_bytes_to_string(&definition_bytes),
            Path::new(MAPNAME_DEFINITION),
        )?;
        if provinces_bytes.is_empty() {
            return Err("XMAP17 admitted provinces.bmp is empty".into());
        }
        let province_bitmap = crate::decode_provinces_bmp(
            &provinces_bytes,
            Path::new(MAPNAME_PROVINCES),
            &color_to_id,
        )?;
        let heightmap = decode_bmp_indexed(&heightmap_bytes, Path::new(MAPNAME_HEIGHTMAP))?;
        if heightmap.width != province_bitmap.width || heightmap.height != province_bitmap.height {
            return Err("XMAP17 heightmap/province raster dimensions disagree".into());
        }

        resolver.require_complete_admitted_dir(
            "history/states",
            Some("txt"),
            false,
            "XMAP17 effective history/states namespace",
        )?;
        let state_files = resolver.resolve_dir_files("history/states", Some("txt"))?;
        if state_files.is_empty() {
            return Err("XMAP17 effective history/states is empty".into());
        }
        let mut state_sources = Vec::with_capacity(state_files.len());
        for (relative, _) in &state_files {
            let (_, bytes) = require(relative)?;
            state_sources.push((
                relative.clone(),
                relative.clone(),
                crate::latin1_bytes_to_string(&bytes),
            ));
        }
        let (states, province_to_state, _) = crate::decode_states(&state_sources)?;
        let mut impassable_states = BTreeSet::new();
        let mut province_importance = BTreeMap::new();
        let mut state_name_keys = BTreeMap::new();
        let mut area_by_province = BTreeMap::new();
        for (&state_id, state) in &states {
            if state.impassable {
                impassable_states.insert(state_id);
            }
            state_name_keys.insert(state_id, state.name_key.clone());
            for &province in &state.provinces {
                let key = format!("STATE:{state_id}");
                if area_by_province.insert(province, key).is_some() {
                    return Err(format!("XMAP17 province {province} belongs to two states").into());
                }
                province_importance.insert(
                    province,
                    state
                        .victory_points
                        .get(&province)
                        .copied()
                        .unwrap_or_default()
                        .max(0) as u64,
                );
            }
        }
        let province_type = definitions
            .into_iter()
            .map(|(id, definition)| {
                (
                    id,
                    ProvinceKind::from_type_str(definition.province_type.as_str()),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let province_adjacencies = parse_province_adjacencies(&adjacency_bytes)?;

        let language_directory = mapname_language_directory(localisation_language)?;
        resolver.require_complete_admitted_dir(
            &format!("localisation/{language_directory}"),
            Some("yml"),
            false,
            "XMAP17 selected-language localisation namespace",
        )?;
        let mut localization = BTreeMap::new();
        let localization_files = resolver
            .resolve_dir_files(&format!("localisation/{language_directory}"), Some("yml"))?;
        if localization_files.is_empty() {
            return Err(
                format!("XMAP17 effective {language_directory} localization is empty").into(),
            );
        }
        for (relative, _) in localization_files {
            let (_, bytes) = require(&relative)?;
            let source = localisation_source_utf8(&bytes, &relative)?;
            merge_localisation_source_exact(
                &mut localization,
                &source,
                &relative,
                &format!("l_{language_directory}:"),
            )?;
        }
        let font = parse_bmfont(&crate::latin1_bytes_to_string(&font_bytes))?;
        if font.page_count != 1 || font.glyphs.values().any(|glyph| glyph.page != 0) {
            return Err(
                "XMAP17 exact mapname executor currently requires one admitted bitmap-font page"
                    .into(),
            );
        }
        let cutoff = crate::parse_lua_number_define(
            &crate::latin1_bytes_to_string(&graphics_bytes),
            crate::DRAW_COUNTRY_NAMES_CUTOFF_KEY,
        )?;
        if cutoff != draw_country_names_cutoff_from_resolver(resolver)? {
            return Err("XMAP17 admitted cutoff disagrees with effective resolver".into());
        }
        let program = prepare_mapname_program(
            resolver,
            admission,
            province_bitmap.width as u32,
            province_bitmap.height as u32,
            font_descriptor,
            font_atlas,
            &mut require,
        )?;
        drop(require);
        Ok(Self {
            content_manifest_sha256: admission.manifest_digest_sha256().to_owned(),
            product_modset_sha256: admission.product_modset_sha256().to_owned(),
            ordered_used_content: ordered.into_values().collect(),
            province_ids: province_bitmap.ids,
            width: province_bitmap.width,
            height: province_bitmap.height,
            province_to_state,
            impassable_states,
            province_importance,
            province_type,
            province_adjacencies,
            state_name_keys,
            area_by_province,
            localization,
            font,
            heightmap,
            cutoff,
            program,
        })
    }
}

pub(crate) struct Xmap17LabelAdapter {
    content: Arc<Xmap17Content>,
}

pub(crate) struct Xmap17FeaturePreparation {
    pub(crate) output: FoundationFeatureOutput,
    pub(crate) prepared: PreparedMapLabelFrame,
}

impl Xmap17LabelAdapter {
    pub(crate) fn new(
        resolver: &GameFileResolver,
        admission: &ValidatedContentAdmission,
        localisation_language: &str,
    ) -> Result<Self> {
        Ok(Self {
            content: Arc::new(Xmap17Content::admit(
                resolver,
                admission,
                localisation_language,
            )?),
        })
    }

    pub(crate) fn prepare(
        &self,
        context: &FoundationFeatureContext<'_>,
    ) -> Result<Xmap17FeaturePreparation> {
        let assigned_stage =
            context.require_assigned_stage(XMAP17_PREPARE_STAGE, XMAP17_PREPARE_OPERATION)?;
        if self.content.product_modset_sha256 != context.product_modset_sha256 {
            return Err("XMAP17 admitted content belongs to another product modset".into());
        }
        let camera = context
            .viewer
            .camera
            .as_ref()
            .ok_or("XMAP17 requires the promoted XMAP03 viewer camera")?;
        if camera.map_width as usize != self.content.width
            || camera.map_height as usize != self.content.height
        {
            return Err(
                "XMAP17 viewer camera dimensions disagree with admitted map content".into(),
            );
        }
        let terrain = context
            .prepared_terrain
            .ok_or("XMAP17 requires the earlier same-hour XMAP04 terrain frame")?;
        if terrain.camera_frame_sha256 != camera.deterministic_sha256
            || terrain.map_width as usize != self.content.width
            || terrain.map_height as usize != self.content.height
        {
            return Err("XMAP17 XMAP04 map-copy provider belongs to another frame".into());
        }
        let wrap_offsets = terrain
            .ordered_map_copy_receipts()?
            .into_iter()
            .map(|row| f32::from_bits(row.map_copy_offset_f32_bits))
            .collect::<Vec<_>>();
        if wrap_offsets.is_empty() {
            return Err("XMAP17 camera produced no admitted map copy".into());
        }
        let map_copy_receipt_set_sha256 = terrain.map_copy_receipt_set_sha256()?;

        let (_, controllers) =
            crate::production_features::xmap05::admitted_epoch_keying(context.world_frame)?;
        let country_names = country_display_names(
            context.world_frame,
            controllers.values().map(String::as_str),
            &self.content.localization,
        )?;
        let country_metrics = label_metrics(&self.content.font, &country_names)?;
        let country_placements = compute_controller_label_placements_with_text(
            &ControllerLabelLawInputs {
                province_ids: &self.content.province_ids,
                width: self.content.width,
                height: self.content.height,
                province_to_state: &self.content.province_to_state,
                controllers: &controllers,
                impassable_states: &self.content.impassable_states,
                province_importance: &self.content.province_importance,
                province_type: &self.content.province_type,
                province_adjacencies: &self.content.province_adjacencies,
            },
            &country_metrics,
        );

        let state_names =
            state_display_names(&self.content.state_name_keys, &self.content.localization)?;
        let state_metrics = label_metrics(&self.content.font, &state_names)?;
        let state_placements = compute_explicit_area_label_placements_with_text(
            &ExplicitAreaLabelLawInputs {
                province_ids: &self.content.province_ids,
                width: self.content.width,
                height: self.content.height,
                area_by_province: &self.content.area_by_province,
                province_to_state: &self.content.province_to_state,
                impassable_states: &self.content.impassable_states,
                province_importance: &self.content.province_importance,
                province_type: &self.content.province_type,
            },
            &state_metrics,
        );

        let country = build_label_set(
            0,
            &country_placements,
            &country_names,
            &country_metrics,
            &self.content.font,
            &self.content.heightmap,
            self.content.height as f32,
        )?;
        let state = build_label_set(
            2,
            &state_placements,
            &state_names,
            &state_metrics,
            &self.content.font,
            &self.content.heightmap,
            self.content.height as f32,
        )?;
        let (faction, faction_source_sha256) =
            faction_label_set(context.world_frame, &controllers, &self.content)?;
        let (player, player_carrier_record_sha256) =
            player_label_set(context.world_frame, &controllers, &self.content)?;
        let mut prepared = PreparedMapLabelFrame {
            schema: XMAP17_LABEL_SCHEMA.into(),
            hour_index: context.world_frame.hour_index(),
            content_manifest_sha256: self.content.content_manifest_sha256.clone(),
            product_modset_sha256: self.content.product_modset_sha256.clone(),
            effective_vfs_manifest_sha256: context.effective_vfs_manifest_sha256.to_owned(),
            foundation_content_operation_sha256: context
                .foundation_content_operation_sha256
                .to_owned(),
            viewer_state_sha256: context.viewer_state_sha256.to_owned(),
            map_width: self.content.width as u32,
            map_height: self.content.height as u32,
            viewer_camera_sha256: camera.deterministic_sha256.clone(),
            camera_height_f32_bits: camera.current_eye()[1].to_bits(),
            cutoff_f32_bits: self.content.cutoff.to_bits(),
            map_copy_receipt_set_sha256,
            wrap_offset_f32_bits: wrap_offsets.into_iter().map(f32::to_bits).collect(),
            ordered_used_content: self.content.ordered_used_content.clone(),
            program: self.content.program.clone(),
            country,
            state,
            faction,
            faction_source_sha256,
            player,
            player_carrier_record_sha256,
            deterministic_sha256: String::new(),
        };
        prepared.deterministic_sha256 = map_label_frame_digest(&prepared);
        prepared.validate()?;
        let registration = Sha256::hex(
            format!(
                "XMAP17\0{XMAP17_LABEL_SCHEMA}\0{}\0{}",
                "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad",
                "20260730T124521Z-xmap17-exact-a729-label-topology-placeme-14024e5a"
            )
            .as_bytes(),
        );
        let ordered_resources_sha256 = Sha256::hex(
            format!(
                "{}\0{}\0{}\0{}\0{}\0{}\0{}\0{}",
                prepared.program.font_descriptor.payload_sha256,
                prepared.program.font_atlas.payload_sha256,
                prepared.country.deterministic_sha256,
                prepared.state.deterministic_sha256,
                prepared
                    .faction
                    .as_ref()
                    .map_or_else(map_label_none_digest, |set| set
                        .deterministic_sha256
                        .clone()),
                prepared
                    .faction_source_sha256
                    .clone()
                    .unwrap_or_else(map_label_none_digest),
                prepared
                    .player
                    .as_ref()
                    .map_or_else(map_label_none_digest, |set| set
                        .deterministic_sha256
                        .clone()),
                prepared
                    .player_carrier_record_sha256
                    .clone()
                    .unwrap_or_else(map_label_none_digest)
            )
            .as_bytes(),
        );
        let target_sha256 = Sha256::hex(
            format!(
                "{}\0{}\0{}\0{}",
                prepared.map_width,
                prepared.map_height,
                context.viewer.viewport_width,
                context.viewer.viewport_height
            )
            .as_bytes(),
        );
        Ok(Xmap17FeaturePreparation {
            output: FoundationFeatureOutput {
                disposition: FamilyDisposition::Present {
                    payload_schema: XMAP17_LABEL_SCHEMA.into(),
                    producer_registration_sha256: registration,
                    payload_sha256: prepared.deterministic_sha256.clone(),
                    ordered_operations: vec![assigned_stage
                        .foundation_operation_identity(
                            prepared.program.deterministic_sha256.clone(),
                            ordered_resources_sha256,
                            target_sha256,
                        )
                        .map_err(|error| format!("XMAP17 assigned prepare stage: {error}"))?],
                    ordered_used_content: prepared.ordered_used_content.clone(),
                },
                prepared_terrain: None,
                prepared_political: None,
                prepared_thin_borders: None,
                prepared_lighting: None,
                prepared_weather: None,
                prepared_rivers_straits: None,
                prepared_water: None,
                prepared_effects: None,
                prepared_units: None,
                prepared_combat: None,
                prepared_orders: None,
                prepared_rail_supply: None,
                prepared_map_modes: None,
                consumed_gap_ids: Vec::new(),
            },
            prepared,
        })
    }
}

fn grouped_label_set(
    selector: u8,
    controllers: &BTreeMap<u32, String>,
    tag_to_group: &BTreeMap<String, String>,
    group_names: &BTreeMap<String, String>,
    content: &Xmap17Content,
) -> Result<PreparedMapLabelSet> {
    let grouped = controllers
        .iter()
        .filter_map(|(&province, tag)| {
            tag_to_group
                .get(tag)
                .cloned()
                .map(|group| (province, group))
        })
        .collect::<BTreeMap<_, _>>();
    let metrics = label_metrics(&content.font, group_names)?;
    let placements = compute_controller_label_placements_with_text(
        &ControllerLabelLawInputs {
            province_ids: &content.province_ids,
            width: content.width,
            height: content.height,
            province_to_state: &content.province_to_state,
            controllers: &grouped,
            impassable_states: &content.impassable_states,
            province_importance: &content.province_importance,
            province_type: &content.province_type,
            province_adjacencies: &content.province_adjacencies,
        },
        &metrics,
    );
    build_label_set(
        selector,
        &placements,
        group_names,
        &metrics,
        &content.font,
        &content.heightmap,
        content.height as f32,
    )
}

fn faction_label_set(
    world: &WorldFrame,
    controllers: &BTreeMap<u32, String>,
    content: &Xmap17Content,
) -> Result<(Option<PreparedMapLabelSet>, Option<String>)> {
    let diplomacy = world.diplomacy();
    let Leaf::Measured(roster) = &diplomacy.faction_members else {
        return Ok((None, None));
    };
    let label_view = world.xmap17_faction_labels_exact();
    if label_view.status != ViewStatus::Present {
        return Ok((None, None));
    }
    let label_carrier = label_view
        .carrier
        .as_ref()
        .ok_or("XMAP17 present faction-label view has no carrier")?;
    let display_by_id = label_carrier
        .factions
        .iter()
        .map(|row| (row.faction_id.as_str(), row.engine_display_text.as_str()))
        .collect::<BTreeMap<_, _>>();
    let mut tag_to_group = BTreeMap::new();
    let mut group_names = BTreeMap::new();
    for (faction_id, members) in roster {
        if faction_id.trim().is_empty() || members.is_empty() {
            return Err(
                "XMAP17 measured faction roster contains an empty identity/member set".into(),
            );
        }
        let group = format!("FACTION:{faction_id}");
        let display_text = display_by_id.get(faction_id.as_str()).ok_or_else(|| {
            format!("XMAP17 exact faction-label census omitted faction {faction_id}")
        })?;
        group_names.insert(group.clone(), display_text.to_uppercase());
        for tag in members {
            let country = diplomacy.countries.get(tag).ok_or_else(|| {
                format!("XMAP17 faction {faction_id} member {tag} has no diplomacy row")
            })?;
            if !matches!(&country.faction_name, Leaf::Measured(name) if name == faction_id) {
                return Err(format!(
                    "XMAP17 faction roster/name disagreement for {tag} in {faction_id}"
                )
                .into());
            }
            if tag_to_group.insert(tag.clone(), group.clone()).is_some() {
                return Err(format!("XMAP17 faction member {tag} appears more than once").into());
            }
        }
    }
    for (tag, country) in &diplomacy.countries {
        if let Leaf::Measured(faction_id) = &country.faction_name {
            if !faction_id.is_empty() && !tag_to_group.contains_key(tag) {
                return Err(format!(
                    "XMAP17 country {tag} has measured faction {faction_id} outside the complete roster"
                )
                .into());
            }
        }
    }
    let source_sha256 = label_carrier.record_sha256.clone();
    Ok((
        Some(grouped_label_set(
            1,
            controllers,
            &tag_to_group,
            &group_names,
            content,
        )?),
        Some(source_sha256),
    ))
}

fn player_label_set(
    world: &WorldFrame,
    controllers: &BTreeMap<u32, String>,
    content: &Xmap17Content,
) -> Result<(Option<PreparedMapLabelSet>, Option<String>)> {
    let view = world.xmap17_player_labels_exact();
    if view.status != ViewStatus::Present {
        return Ok((None, None));
    }
    let carrier = view
        .carrier
        .as_ref()
        .ok_or("XMAP17 present player-label view has no carrier")?;
    let mut tag_to_group = BTreeMap::new();
    let mut group_names = BTreeMap::new();
    for row in &carrier.countries {
        let (Some(player_id), Some(display_text)) =
            (row.player_object_id, row.engine_display_text.as_deref())
        else {
            continue;
        };
        let group = format!("PLAYER:{player_id:016x}");
        if let Some(previous) = group_names.insert(group.clone(), display_text.to_uppercase()) {
            if previous != display_text.to_uppercase() {
                return Err(format!(
                    "XMAP17 player object {player_id} has inconsistent display text"
                )
                .into());
            }
        }
        tag_to_group.insert(row.country_tag.clone(), group);
    }
    Ok((
        Some(grouped_label_set(
            3,
            controllers,
            &tag_to_group,
            &group_names,
            content,
        )?),
        Some(carrier.record_sha256.clone()),
    ))
}

fn country_display_names<'a>(
    frame: &WorldFrame,
    controllers: impl Iterator<Item = &'a str>,
    localization: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>> {
    let mut names = BTreeMap::new();
    for tag in controllers {
        if names.contains_key(tag) {
            continue;
        }
        let identity = frame
            .country_identity(tag)
            .ok_or_else(|| format!("XMAP17 controller {tag} has no country identity"))?;
        let ideology = match &identity.ruling_ideology {
            Leaf::Measured(value) if !value.trim().is_empty() => value.as_str(),
            _ => {
                return Err(
                    format!("XMAP17 controller {tag} has no measured ruling ideology").into(),
                )
            }
        };
        let cosmetic = super::xmap05::admitted_cosmetic_identity(Some(&identity.cosmetic_tag))
            .map_err(|detail| {
                format!("XMAP17 controller {tag} has incomplete cosmetic identity: {detail}")
            })?;
        let mut candidates = Vec::new();
        if let Some(cosmetic) = cosmetic {
            candidates.push(format!("{cosmetic}_{ideology}"));
            candidates.push(cosmetic.to_string());
        }
        candidates.push(format!("{tag}_{ideology}"));
        candidates.push(tag.to_string());
        let key = candidates
            .iter()
            .find(|key| localization.contains_key(*key))
            .ok_or_else(|| {
                format!(
                    "XMAP17 controller {tag} has no effective localization in chain {}",
                    candidates.join(" -> ")
                )
            })?;
        names.insert(
            tag.to_string(),
            resolve_static_mapname_text(localization, key)?,
        );
    }
    Ok(names)
}

fn mapname_language_directory(localisation_language: &str) -> Result<&str> {
    let language = localisation_language
        .trim()
        .strip_prefix("l_")
        .unwrap_or(localisation_language.trim());
    if language.is_empty()
        || !language
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
    {
        return Err(format!(
            "XMAP17 localization language {localisation_language:?} is not a canonical the game language"
        )
        .into());
    }
    Ok(language)
}

fn localisation_source_utf8<'a>(bytes: &'a [u8], relative: &str) -> Result<&'a str> {
    std::str::from_utf8(bytes).map_err(|error| {
        format!("XMAP17 localization {relative} is not valid UTF-8: {error}").into()
    })
}

fn selected_mapname_font_base(core_gfx: &str, localisation_language: &str) -> Result<String> {
    let language = if localisation_language.trim().starts_with("l_") {
        localisation_language.trim().to_ascii_lowercase()
    } else {
        format!("l_{}", localisation_language.trim().to_ascii_lowercase())
    };
    let bindings = crate::unit_counter_assets::parse_bitmap_fonts(core_gfx, MAPNAME_CORE_GFX);
    let mut base = None;
    let mut matching_override = None;
    for binding in bindings
        .iter()
        .filter(|binding| binding.name.eq_ignore_ascii_case(MAPNAME_FONT_TOKEN))
    {
        let selected_file = binding
            .path
            .clone()
            .or_else(|| {
                binding
                    .languages
                    .iter()
                    .position(|candidate| candidate.eq_ignore_ascii_case(&language))
                    .and_then(|index| binding.font_files.get(index).cloned())
            })
            .or_else(|| binding.font_files.first().cloned());
        if binding.is_override {
            if binding
                .languages
                .iter()
                .any(|candidate| candidate.eq_ignore_ascii_case(&language))
            {
                matching_override = selected_file;
            }
        } else if selected_file.is_some() {
            base = selected_file;
        }
    }
    let mut selected = matching_override.or(base).ok_or_else(|| {
        format!(
            "XMAP17 effective {MAPNAME_CORE_GFX} has no {MAPNAME_FONT_TOKEN} font for {language}"
        )
    })?;
    selected = selected.trim().replace('\\', "/");
    for suffix in [".fnt", ".dds", ".tga"] {
        if selected.to_ascii_lowercase().ends_with(suffix) {
            selected.truncate(selected.len() - suffix.len());
            break;
        }
    }
    if selected.is_empty() || selected.starts_with('/') || selected.contains("../") {
        return Err("XMAP17 effective map-font path is unsafe or empty".into());
    }
    Ok(selected)
}

fn parse_province_adjacencies(bytes: &[u8]) -> Result<BTreeSet<(u32, u32)>> {
    let source = crate::latin1_bytes_to_string(bytes);
    let mut pairs = BTreeSet::new();
    for (line_index, raw) in source.lines().enumerate() {
        let line = raw.trim().trim_start_matches('\u{feff}');
        if line.is_empty() || line.starts_with('#') || line.starts_with("From;") {
            continue;
        }
        let fields = line.split(';').collect::<Vec<_>>();
        if fields.len() < 3 {
            return Err(format!(
                "XMAP17 adjacencies.csv line {} has fewer than three fields",
                line_index + 1
            )
            .into());
        }
        let from = fields[0].trim().parse::<i64>().map_err(|error| {
            format!(
                "XMAP17 adjacencies.csv line {} has invalid from province: {error}",
                line_index + 1
            )
        })?;
        let to = fields[1].trim().parse::<i64>().map_err(|error| {
            format!(
                "XMAP17 adjacencies.csv line {} has invalid to province: {error}",
                line_index + 1
            )
        })?;
        if from <= 0 || to <= 0 {
            continue;
        }
        let (from, to) = (u32::try_from(from)?, u32::try_from(to)?);
        if from != to {
            pairs.insert((from.min(to), from.max(to)));
        }
    }
    Ok(pairs)
}

fn state_display_names(
    keys: &BTreeMap<u32, String>,
    localization: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>> {
    let mut names = BTreeMap::new();
    for (&state_id, key) in keys {
        if !localization.contains_key(key) {
            return Err(format!("XMAP17 state {state_id} has no localization key {key}").into());
        }
        names.insert(
            format!("STATE:{state_id}"),
            resolve_static_mapname_text(localization, key)?,
        );
    }
    Ok(names)
}

fn resolve_static_mapname_text(
    localization: &BTreeMap<String, String>,
    key: &str,
) -> Result<String> {
    fn expand(
        localization: &BTreeMap<String, String>,
        key: &str,
        stack: &mut Vec<String>,
    ) -> Result<String> {
        if stack.len() >= 32 || stack.iter().any(|active| active == key) {
            return Err(format!("XMAP17 localization {key} has a cycle/depth overflow").into());
        }
        let source = localization
            .get(key)
            .ok_or_else(|| format!("XMAP17 localization key {key} is absent"))?;
        stack.push(key.to_string());
        let mut output = String::new();
        let mut chars = source.chars().peekable();
        while let Some(ch) = chars.next() {
            match ch {
                '$' => {
                    let mut token = String::new();
                    let mut closed = false;
                    for next in chars.by_ref() {
                        if next == '$' {
                            closed = true;
                            break;
                        }
                        token.push(next);
                    }
                    if !closed || token.is_empty() || token.contains('|') {
                        return Err(format!(
                            "XMAP17 localization {key} requires a runtime/formatted variable"
                        )
                        .into());
                    }
                    output.push_str(&expand(localization, &token, stack)?);
                }
                '\u{00a7}' | '\u{0011}' => {
                    let code = chars.next().ok_or_else(|| {
                        format!("XMAP17 localization {key} has an unterminated color token")
                    })?;
                    if code == '[' {
                        return Err(format!(
                            "XMAP17 localization {key} requires a runtime color expression"
                        )
                        .into());
                    }
                }
                '[' | ']' | '\u{00a3}' => {
                    return Err(format!(
                        "XMAP17 localization {key} requires runtime data or inline-icon layout"
                    )
                    .into())
                }
                _ => output.push(ch),
            }
        }
        stack.pop();
        Ok(output)
    }
    let text = expand(localization, key, &mut Vec::new())?;
    let text = text.trim();
    if text.is_empty() {
        return Err(format!("XMAP17 localization {key} resolves empty").into());
    }
    Ok(text.to_uppercase())
}

fn label_metrics(
    font: &crate::BmFont,
    names: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, LabelTextMetrics>> {
    names
        .iter()
        .map(|(key, text)| Ok((key.clone(), font.label_text_metrics(text)?)))
        .collect()
}

fn build_label_set(
    selector: u8,
    placements: &[CountryLabelPlacement],
    names: &BTreeMap<String, String>,
    metrics_by_key: &BTreeMap<String, LabelTextMetrics>,
    font: &crate::BmFont,
    heightmap: &IndexImage,
    map_height: f32,
) -> Result<PreparedMapLabelSet> {
    let mut rows = Vec::new();
    let mut vertices = Vec::new();
    for placement in placements {
        let text = names
            .get(&placement.tag)
            .ok_or_else(|| format!("XMAP17 placement {} has no localized text", placement.tag))?;
        let metrics = metrics_by_key
            .get(&placement.tag)
            .ok_or_else(|| format!("XMAP17 placement {} has no font metrics", placement.tag))?;
        let tracking = mapname_tracking_px(placement, metrics);
        let tracked_advance = mapname_tracked_advance_px(metrics, tracking);
        let natural_width = metrics
            .advances
            .iter()
            .map(|(advance, _)| *advance)
            .sum::<f32>();
        let glyphs = font.layout_mapname_tracked(text, tracking)?;
        let law_vertices = build_mapname_vertices_with_height(
            placement,
            &glyphs,
            natural_width,
            tracked_advance,
            metrics.line_height_px,
            map_height,
            |x, z| sample_map_height_a729(heightmap, x, z),
        )?;
        if law_vertices.is_empty() {
            continue;
        }
        let first_vertex = u32::try_from(vertices.len())?;
        let vertex_count = u32::try_from(law_vertices.len())?;
        vertices.extend_from_slice(&law_vertices);
        rows.push(PreparedMapLabelRow {
            key: placement.tag.clone(),
            localized_text: text.clone(),
            component_id: placement.component_id,
            first_vertex,
            vertex_count,
        });
    }
    let mut bytes = Vec::with_capacity(vertices.len() * MAPNAME_VERTEX_STRIDE);
    for MapnameVertex { x, y, z, u, v } in vertices {
        for value in [x, y, z, u, v] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
    }
    let mut set = PreparedMapLabelSet {
        selector,
        rows,
        vertex_bytes: bytes.into(),
        deterministic_sha256: String::new(),
    };
    set.deterministic_sha256 = map_label_set_digest(&set);
    set.validate()?;
    Ok(set)
}

fn prepare_mapname_program<F>(
    resolver: &GameFileResolver,
    admission: &ValidatedContentAdmission,
    map_width: u32,
    map_height: u32,
    font_descriptor: UsedContentKey,
    font_atlas: UsedContentKey,
    require: &mut F,
) -> Result<PreparedMapLabelProgram>
where
    F: FnMut(&str) -> Result<(UsedContentKey, Vec<u8>)>,
{
    let translation_unit =
        crate::shader_compile::effect_translation_unit_files(resolver, MAPNAME_EFFECT_FILE)?;
    let effect = translation_unit
        .last()
        .and_then(|(_, file)| file.effect(MAPNAME_EFFECT_NAME))
        .ok_or("effective mapname.shader has no mapname Effect")?;
    if effect.vertex_shader.is_none() || effect.pixel_shader.is_none() {
        return Err("effective mapname Effect omits VS or PS".into());
    }
    validate_mapname_pipeline(
        &translation_unit
            .last()
            .ok_or("effective mapname translation unit is empty")?
            .1,
        effect,
    )?;
    let constants =
        crate::derive_a729_engine_map_shader_constants(map_width, map_height, false, 0)?;
    let defines = crate::engine_map_shader_defines(constants);
    let vertex_hlsl = crate::shader_compile::build_stage_hlsl_with_defines(
        resolver,
        MAPNAME_EFFECT_FILE,
        MAPNAME_EFFECT_NAME,
        crate::ShaderStage::Vertex,
        &defines,
    )?;
    let pixel_hlsl = crate::shader_compile::build_stage_hlsl_with_defines(
        resolver,
        MAPNAME_EFFECT_FILE,
        MAPNAME_EFFECT_NAME,
        crate::ShaderStage::Pixel,
        &defines,
    )?;
    let (vertex_bytecode, pixel_bytecode) =
        compile_mapname_program_stages(&vertex_hlsl, &pixel_hlsl)?;
    let mut ordered_shader_content = Vec::new();
    for path in translation_unit
        .iter()
        .map(|(path, _)| path.as_str())
        .chain([MAPNAME_PLATFORM_HEADER])
    {
        let (key, _) = require(path)?;
        ordered_shader_content.push(key);
    }
    for content in &ordered_shader_content {
        admission
            .require_used_content(&content.normalized_game_relative_path)
            .map_err(|error| format!("XMAP17 shader content drift: {error}"))?;
    }
    let mut program = PreparedMapLabelProgram {
        effect_file: MAPNAME_EFFECT_FILE.to_ascii_lowercase(),
        effect_name: MAPNAME_EFFECT_NAME.into(),
        ordered_shader_content,
        vertex_bytecode_sha256: Sha256::hex(&vertex_bytecode),
        pixel_bytecode_sha256: Sha256::hex(&pixel_bytecode),
        vertex_bytecode: vertex_bytecode.into(),
        pixel_bytecode: pixel_bytecode.into(),
        font_descriptor,
        font_atlas,
        deterministic_sha256: String::new(),
    };
    program.deterministic_sha256 = map_label_program_digest(&program);
    program.validate()?;
    Ok(program)
}

fn validate_mapname_pipeline(
    file: &crate::shader_compile::EffectFile,
    effect: &crate::shader_compile::EffectDef,
) -> Result<()> {
    fn state_map<'a>(
        rows: &'a [(String, String)],
        label: &str,
    ) -> Result<BTreeMap<&'a str, &'a str>> {
        let mut map = BTreeMap::new();
        for (key, value) in rows {
            if map.insert(key.as_str(), value.as_str()).is_some() {
                return Err(format!("XMAP17 effective {label} repeats {key}").into());
            }
        }
        Ok(map)
    }

    if effect.blend_state.is_some()
        || effect.depth_stencil_state.as_deref() != Some("DepthStencilState")
        || effect.rasterizer_state.is_some()
        || !file.rasterizer_states.is_empty()
    {
        return Err("XMAP17 effective mapname effect changes the proven pipeline selection".into());
    }
    let blend = state_map(
        file.blend_states
            .get("BlendState")
            .ok_or("XMAP17 effective mapname shader has no implicit BlendState")?,
        "BlendState",
    )?;
    let expected_blend = BTreeMap::from([
        ("AlphaTest", "no"),
        ("BlendEnable", "yes"),
        ("DestBlend", "inv_src_alpha"),
        ("SourceBlend", "src_alpha"),
        ("WriteMask", "RED|GREEN|BLUE"),
    ]);
    if blend != expected_blend || file.blend_states.len() != 1 {
        return Err("XMAP17 effective mapname BlendState has no exact-a729 translation".into());
    }
    let depth = state_map(
        file.depth_stencil_states
            .get("DepthStencilState")
            .ok_or("XMAP17 effective mapname shader has no DepthStencilState")?,
        "DepthStencilState",
    )?;
    let expected_depth = BTreeMap::from([
        ("DepthEnable", "no"),
        ("DepthFunction", "comparison_less_equal"),
        ("DepthWriteMask", "depth_write_all"),
        ("FrontStencilDepthFailOp", "stencil_op_keep"),
        ("FrontStencilFailOp", "stencil_op_keep"),
        ("FrontStencilFunc", "comparison_not_equal"),
        ("FrontStencilPassOp", "stencil_op_keep"),
        ("StencilEnable", "yes"),
        ("StencilReadMask", "4"),
        ("StencilRef", "4"),
    ]);
    if depth != expected_depth || file.depth_stencil_states.len() != 1 {
        return Err(
            "XMAP17 effective mapname DepthStencilState has no exact-a729 translation".into(),
        );
    }
    let sampler = file
        .pixel_stage
        .samplers
        .iter()
        .find(|sampler| sampler.name == "DiffuseTexture")
        .ok_or("XMAP17 effective mapname shader has no DiffuseTexture sampler")?;
    if file.pixel_stage.samplers.len() != 1
        || sampler.index != 0
        || sampler.mag_filter.as_deref() != Some("Linear")
        || sampler.min_filter.as_deref() != Some("Linear")
        || sampler.mip_filter.as_deref() != Some("Linear")
        || sampler.address_u.as_deref() != Some("Clamp")
        || sampler.address_v.as_deref() != Some("Clamp")
        || sampler.mip_map_lod_bias.as_deref() != Some("-0.4")
    {
        return Err("XMAP17 effective mapname sampler has no exact-a729 translation".into());
    }
    Ok(())
}

#[cfg(windows)]
fn compile_mapname_program_stages(
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
fn compile_mapname_program_stages(
    _vertex_hlsl: &str,
    _pixel_hlsl: &str,
) -> Result<(Vec<u8>, Vec<u8>)> {
    Err("XMAP17 executable mapname program requires Windows D3DCompile".into())
}

fn map_label_program_digest(program: &PreparedMapLabelProgram) -> String {
    let mut bytes = program.effect_file.as_bytes().to_vec();
    bytes.extend_from_slice(program.effect_name.as_bytes());
    for content in &program.ordered_shader_content {
        bytes.extend_from_slice(
            serde_json::to_string(content)
                .expect("UsedContentKey serialization")
                .as_bytes(),
        );
    }
    bytes.extend_from_slice(program.vertex_bytecode_sha256.as_bytes());
    bytes.extend_from_slice(program.pixel_bytecode_sha256.as_bytes());
    bytes.extend_from_slice(
        serde_json::to_string(&program.font_descriptor)
            .expect("UsedContentKey serialization")
            .as_bytes(),
    );
    bytes.extend_from_slice(
        serde_json::to_string(&program.font_atlas)
            .expect("UsedContentKey serialization")
            .as_bytes(),
    );
    Sha256::hex(&bytes)
}

fn map_label_set_digest(set: &PreparedMapLabelSet) -> String {
    let mut bytes = vec![set.selector];
    for row in &set.rows {
        bytes.extend_from_slice(row.key.as_bytes());
        bytes.push(0);
        bytes.extend_from_slice(row.localized_text.as_bytes());
        bytes.push(0);
        bytes.extend_from_slice(&row.component_id.to_le_bytes());
        bytes.extend_from_slice(&row.first_vertex.to_le_bytes());
        bytes.extend_from_slice(&row.vertex_count.to_le_bytes());
    }
    bytes.extend_from_slice(Sha256::hex(&set.vertex_bytes).as_bytes());
    Sha256::hex(&bytes)
}

fn map_label_none_digest() -> String {
    Sha256::hex(b"XMAP17_EMAPNAMEMODE_NONE_A729_V1")
}

fn map_label_frame_digest(frame: &PreparedMapLabelFrame) -> String {
    let mut bytes = frame.schema.as_bytes().to_vec();
    bytes.extend_from_slice(&frame.hour_index.to_le_bytes());
    bytes.extend_from_slice(frame.content_manifest_sha256.as_bytes());
    bytes.extend_from_slice(frame.product_modset_sha256.as_bytes());
    bytes.extend_from_slice(frame.effective_vfs_manifest_sha256.as_bytes());
    bytes.extend_from_slice(frame.foundation_content_operation_sha256.as_bytes());
    bytes.extend_from_slice(frame.viewer_state_sha256.as_bytes());
    bytes.extend_from_slice(&frame.map_width.to_le_bytes());
    bytes.extend_from_slice(&frame.map_height.to_le_bytes());
    bytes.extend_from_slice(frame.viewer_camera_sha256.as_bytes());
    bytes.extend_from_slice(&frame.camera_height_f32_bits.to_le_bytes());
    bytes.extend_from_slice(&frame.cutoff_f32_bits.to_le_bytes());
    bytes.extend_from_slice(frame.map_copy_receipt_set_sha256.as_bytes());
    for bits in &frame.wrap_offset_f32_bits {
        bytes.extend_from_slice(&bits.to_le_bytes());
    }
    for content in &frame.ordered_used_content {
        bytes.extend_from_slice(
            serde_json::to_string(content)
                .expect("UsedContentKey serialization")
                .as_bytes(),
        );
    }
    bytes.extend_from_slice(frame.program.deterministic_sha256.as_bytes());
    bytes.extend_from_slice(frame.country.deterministic_sha256.as_bytes());
    bytes.extend_from_slice(frame.state.deterministic_sha256.as_bytes());
    bytes.extend_from_slice(
        frame
            .faction
            .as_ref()
            .map_or_else(map_label_none_digest, |set| {
                set.deterministic_sha256.clone()
            })
            .as_bytes(),
    );
    bytes.extend_from_slice(
        frame
            .faction_source_sha256
            .as_deref()
            .unwrap_or("XMAP17_NO_FACTION_SOURCE")
            .as_bytes(),
    );
    bytes.extend_from_slice(
        frame
            .player
            .as_ref()
            .map_or_else(map_label_none_digest, |set| {
                set.deterministic_sha256.clone()
            })
            .as_bytes(),
    );
    bytes.extend_from_slice(
        frame
            .player_carrier_record_sha256
            .as_deref()
            .unwrap_or("XMAP17_NO_PLAYER_CARRIER")
            .as_bytes(),
    );
    Sha256::hex(&bytes)
}

fn validate_sha256(value: &str, label: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        || value.bytes().all(|byte| byte == b'0')
    {
        return Err(format!("{label} is not a canonical nonzero SHA-256").into());
    }
    Ok(())
}

fn xmap17_draw_receipt_digest(receipt: &Xmap17DrawReceipt) -> Result<String> {
    let mut sealed = receipt.clone();
    sealed.packet_sha256.clear();
    Ok(Sha256::hex(&serde_json::to_vec(&sealed)?))
}

fn ready_map_label_draw_digest(ready: &ReadyMapLabelDrawContract) -> String {
    let mut identity = ready.hour_index.to_le_bytes().to_vec();
    identity.extend_from_slice(ready.label_frame_sha256.as_bytes());
    identity.extend_from_slice(ready.scene_cb0_sha256.as_bytes());
    identity.extend_from_slice(ready.final_image_target_sha256.as_bytes());
    identity.extend_from_slice(ready.xmap10_operation_order_sha256.as_bytes());
    identity.push(ready.near_selector);
    identity.push(ready.far_selector);
    identity.push(u8::from(ready.update_daily));
    identity.extend_from_slice(&ready.near_alpha_f32_bits.to_le_bytes());
    identity.extend_from_slice(&ready.far_alpha_f32_bits.to_le_bytes());
    for digest in &ready.ordered_set_sha256 {
        identity.extend_from_slice(digest.as_bytes());
    }
    Sha256::hex(&identity)
}

#[cfg(test)]
mod tests {
    use super::*;
    use portable_content_manifest::ContentLayerKind;
    use std::collections::HashMap;

    fn sha(byte: u8) -> String {
        format!("{byte:02x}").repeat(32)
    }

    fn content(path: &str, payload: u8) -> UsedContentKey {
        UsedContentKey {
            effective_vfs_manifest_sha256: sha(1),
            normalized_game_relative_path: path.to_ascii_lowercase(),
            winning_layer_ordinal: 0,
            layer_kind: ContentLayerKind::Base,
            layer_id: "base".into(),
            source_sha256: sha(payload),
            payload_sha256: sha(payload),
            payload_bytes: 1,
        }
    }

    fn empty_label_set(selector: u8) -> PreparedMapLabelSet {
        let mut set = PreparedMapLabelSet {
            selector,
            rows: Vec::new(),
            vertex_bytes: Arc::from([]),
            deterministic_sha256: String::new(),
        };
        set.deterministic_sha256 = map_label_set_digest(&set);
        set
    }

    fn test_frame() -> PreparedMapLabelFrame {
        let shader = content(MAPNAME_EFFECT_FILE, 2);
        let font_descriptor = content("gfx/fonts/hoi_mapfont4.fnt", 3);
        let font_atlas = content("gfx/fonts/hoi_mapfont4.dds", 4);
        let mut program = PreparedMapLabelProgram {
            effect_file: MAPNAME_EFFECT_FILE.to_ascii_lowercase(),
            effect_name: MAPNAME_EFFECT_NAME.into(),
            ordered_shader_content: vec![shader.clone()],
            vertex_bytecode: Arc::from([1u8]),
            pixel_bytecode: Arc::from([2u8]),
            vertex_bytecode_sha256: Sha256::hex(&[1]),
            pixel_bytecode_sha256: Sha256::hex(&[2]),
            font_descriptor: font_descriptor.clone(),
            font_atlas: font_atlas.clone(),
            deterministic_sha256: String::new(),
        };
        program.deterministic_sha256 = map_label_program_digest(&program);
        let mut frame = PreparedMapLabelFrame {
            schema: XMAP17_LABEL_SCHEMA.into(),
            hour_index: 17,
            content_manifest_sha256: sha(5),
            product_modset_sha256: sha(6),
            effective_vfs_manifest_sha256: sha(1),
            foundation_content_operation_sha256: sha(7),
            viewer_state_sha256: sha(8),
            map_width: 1,
            map_height: 1,
            viewer_camera_sha256: sha(9),
            camera_height_f32_bits: 300.0f32.to_bits(),
            cutoff_f32_bits: 260.0f32.to_bits(),
            map_copy_receipt_set_sha256: sha(10),
            wrap_offset_f32_bits: vec![0.0f32.to_bits()],
            ordered_used_content: vec![shader, font_descriptor, font_atlas],
            program,
            country: empty_label_set(0),
            state: empty_label_set(2),
            faction: None,
            faction_source_sha256: None,
            player: None,
            player_carrier_record_sha256: None,
            deterministic_sha256: String::new(),
        };
        frame.deterministic_sha256 = map_label_frame_digest(&frame);
        frame.validate().unwrap();
        frame
    }

    fn grouping_content() -> Xmap17Content {
        let width = 24usize;
        let height = 12usize;
        let province_ids = (0..height)
            .flat_map(|_| (0..width).map(|x| if x < width / 2 { 1 } else { 2 }))
            .collect::<Vec<_>>();
        let glyph = crate::BmFontGlyph {
            x: 0,
            y: 0,
            width: 8,
            height: 10,
            xoffset: 0,
            yoffset: 0,
            xadvance: 8,
            page: 0,
        };
        Xmap17Content {
            content_manifest_sha256: sha(1),
            product_modset_sha256: sha(2),
            ordered_used_content: Vec::new(),
            province_ids,
            width,
            height,
            province_to_state: BTreeMap::from([(1, 1), (2, 2)]),
            impassable_states: BTreeSet::new(),
            province_importance: BTreeMap::from([(1, 1), (2, 1)]),
            province_type: BTreeMap::from([(1, ProvinceKind::Land), (2, ProvinceKind::Land)]),
            province_adjacencies: BTreeSet::new(),
            state_name_keys: BTreeMap::new(),
            area_by_province: BTreeMap::new(),
            localization: BTreeMap::new(),
            font: crate::BmFont {
                line_height: 10,
                base: 8,
                scale_w: 16,
                scale_h: 16,
                page_count: 1,
                glyphs: HashMap::from([(u32::from('A'), glyph), (u32::from(' '), glyph)]),
                kernings: HashMap::new(),
            },
            heightmap: IndexImage {
                width,
                height,
                pixels: vec![0; width * height],
            },
            cutoff: 260.0,
            program: test_frame().program,
        }
    }

    fn ready_and_identity(
        frame: &PreparedMapLabelFrame,
        alpha: MapNameAlphaState,
    ) -> (ReadyMapLabelDrawContract, Xmap17ExecutionIdentity) {
        let schedule = portable_render_stream::AssignedStageSchedule::from_topology(
            &crate::product_foundation_assigned_stage_topology().unwrap(),
        )
        .unwrap();
        let ready = frame
            .admit_final_image_dependencies(
                MapLabelFinalImageDependencies {
                    scene_cb0_sha256: sha(11),
                    final_image_target_sha256: sha(12),
                    xmap10_operation_order_sha256: schedule.schedule_sha256,
                    near_selector: 0,
                    far_selector: 0,
                    update_daily: true,
                },
                alpha,
                0.25,
            )
            .unwrap();
        let identity = Xmap17ExecutionIdentity {
            hour_index: frame.hour_index,
            content_manifest_sha256: frame.content_manifest_sha256.clone(),
            foundation_content_operation_sha256: frame.foundation_content_operation_sha256.clone(),
            viewer_state_sha256: frame.viewer_state_sha256.clone(),
            main_scene_target_sha256: ready.final_image_target_sha256.clone(),
            map_copy_receipt_set_sha256: frame.map_copy_receipt_set_sha256.clone(),
        };
        (ready, identity)
    }

    #[test]
    fn alpha_transition_is_exact_persistent_crossfade() {
        assert!(MapNameAlphaState::new(-0.1, 0.0).is_err());
        assert!(MapNameAlphaState::new(0.0, f32::NAN).is_err());
        let mut state = MapNameAlphaState::new(0.0, 0.0).unwrap();
        state.update(300.0, 260.0, 0.4).unwrap();
        assert_eq!(state.near_alpha, 0.0);
        assert_eq!(state.far_alpha, 0.4);
        state.update(200.0, 260.0, 0.25).unwrap();
        assert_eq!(state.near_alpha, 0.25);
        assert_eq!(state.far_alpha, 0.15);
        state.update(200.0, 260.0, 2.0).unwrap();
        assert_eq!(state.near_alpha, 1.0);
        assert_eq!(state.far_alpha, 0.0);
    }

    #[test]
    fn static_localization_expands_and_runtime_tokens_fail_closed() {
        let localization = BTreeMap::from([
            ("ADJ".into(), "French".into()),
            ("GOOD".into(), "§Y$ADJ$ Empire§!".into()),
            ("BAD".into(), "$RULER_ADJ$ Empire".into()),
        ]);
        assert_eq!(
            resolve_static_mapname_text(&localization, "GOOD").unwrap(),
            "FRENCH EMPIRE"
        );
        assert!(resolve_static_mapname_text(&localization, "BAD").is_err());
        assert_eq!(
            resolve_static_mapname_text(
                &BTreeMap::from([("GER".into(), "German Reich".into())]),
                "GER",
            )
            .unwrap(),
            "GERMAN REICH"
        );
        assert!(localisation_source_utf8(&[0xff], "localisation/english/bad.yml").is_err());
    }

    #[test]
    fn execution_preflight_joins_foundation_viewer_and_stage_before_draw() {
        let frame = test_frame();
        let (ready, identity) =
            ready_and_identity(&frame, MapNameAlphaState::new(0.0, 0.0).unwrap());
        let schedule = portable_render_stream::AssignedStageSchedule::from_topology(
            &crate::product_foundation_assigned_stage_topology().unwrap(),
        )
        .unwrap();
        let stage = schedule
            .require(XMAP17_EXECUTE_STAGE, XMAP17_EXECUTE_OPERATION)
            .unwrap()
            .clone();
        assert_eq!(
            frame
                .preflight_execution(&ready, &identity, &stage)
                .unwrap(),
            (0, 0)
        );

        let mut wrong_foundation = identity.clone();
        wrong_foundation.foundation_content_operation_sha256 = sha(15);
        assert!(frame
            .preflight_execution(&ready, &wrong_foundation, &stage)
            .is_err());

        let mut wrong_viewer = identity.clone();
        wrong_viewer.viewer_state_sha256 = sha(16);
        assert!(frame
            .preflight_execution(&ready, &wrong_viewer, &stage)
            .is_err());

        let wrong_stage = portable_render_stream::AssignedStage::seal(
            sha(14),
            XMAP17_EXECUTE_STAGE,
            XMAP17_EXECUTE_OPERATION,
            stage.assigned_ordinal,
        )
        .unwrap();
        assert!(frame
            .preflight_execution(&ready, &identity, &wrong_stage)
            .is_err());

        let wrong_ordinal = portable_render_stream::AssignedStage::seal(
            stage.topology_sha256.clone(),
            XMAP17_EXECUTE_STAGE,
            XMAP17_EXECUTE_OPERATION,
            stage.assigned_ordinal + 1,
        )
        .unwrap();
        assert!(frame
            .preflight_execution(&ready, &identity, &wrong_ordinal)
            .is_err());

        let mut wrong_assignment_digest = stage.clone();
        wrong_assignment_digest.assignment_sha256 = sha(15);
        assert!(frame
            .preflight_execution(&ready, &identity, &wrong_assignment_digest)
            .is_err());

        let mut wrong_schedule = ready.clone();
        wrong_schedule.xmap10_operation_order_sha256 = sha(16);
        wrong_schedule.deterministic_sha256 = ready_map_label_draw_digest(&wrong_schedule);
        assert!(frame
            .preflight_execution(&wrong_schedule, &identity, &stage)
            .is_err());
    }

    #[test]
    fn exact_name_mode_selectors_distinguish_unavailable_empty_and_none() {
        let mut frame = test_frame();
        assert!(frame.selected_label_set(1).is_err());
        assert!(frame.selected_label_set(3).is_err());
        assert_eq!(frame.selected_label_set(4).unwrap(), None);

        frame.faction = Some(empty_label_set(1));
        frame.faction_source_sha256 = Some(sha(30));
        frame.player = Some(empty_label_set(3));
        frame.player_carrier_record_sha256 = Some(sha(31));
        frame.deterministic_sha256 = map_label_frame_digest(&frame);
        frame.validate().unwrap();
        assert_eq!(frame.selected_label_set(1).unwrap().unwrap().selector, 1);
        assert_eq!(frame.selected_label_set(3).unwrap().unwrap().selector, 3);
        frame.player = Some(empty_label_set(2));
        frame.deterministic_sha256 = map_label_frame_digest(&frame);
        assert!(frame.validate().is_err());
        frame.faction = None;
        frame.faction_source_sha256 = None;
        frame.player = None;
        frame.player_carrier_record_sha256 = None;
        frame.deterministic_sha256 = map_label_frame_digest(&frame);
        frame.validate().unwrap();
        assert!(frame.selected_label_set(3).is_err());
        assert!(frame.selected_label_set(5).is_err());
    }

    #[test]
    fn scripted_player_compose_groups_by_stable_identity_not_country_tag() {
        let content = grouping_content();
        let controllers = BTreeMap::from([(1, "ENG".into()), (2, "USA".into())]);
        let same_player = grouped_label_set(
            3,
            &controllers,
            &BTreeMap::from([
                ("ENG".into(), "PLAYER:0001".into()),
                ("USA".into(), "PLAYER:0001".into()),
            ]),
            &BTreeMap::from([("PLAYER:0001".into(), "A".into())]),
            &content,
        )
        .unwrap();
        assert_eq!(same_player.selector, 3);
        assert_eq!(same_player.rows.len(), 1);
        assert_eq!(same_player.rows[0].key, "PLAYER:0001");

        let distinct_players = grouped_label_set(
            3,
            &controllers,
            &BTreeMap::from([
                ("ENG".into(), "PLAYER:0001".into()),
                ("USA".into(), "PLAYER:0002".into()),
            ]),
            &BTreeMap::from([
                ("PLAYER:0001".into(), "A".into()),
                ("PLAYER:0002".into(), "A".into()),
            ]),
            &content,
        )
        .unwrap();
        assert_eq!(distinct_players.rows.len(), 2);

        let faction = grouped_label_set(
            1,
            &controllers,
            &BTreeMap::from([
                ("ENG".into(), "FACTION:allies".into()),
                ("USA".into(), "FACTION:allies".into()),
            ]),
            &BTreeMap::from([("FACTION:allies".into(), "A".into())]),
            &content,
        )
        .unwrap();
        assert_eq!(faction.selector, 1);
        assert_eq!(faction.rows.len(), 1);
    }

    #[test]
    fn rejected_execution_cannot_advance_persistent_alpha() {
        let frame = test_frame();
        let alpha = MapNameAlphaState::new(0.0, 0.0).unwrap();
        let (ready, mut identity) = ready_and_identity(&frame, alpha);
        assert_eq!(alpha, MapNameAlphaState::new(0.0, 0.0).unwrap());
        assert_eq!(ready.next_alpha_state().unwrap().far_alpha, 0.25);
        identity.viewer_state_sha256 = sha(20);
        let schedule = portable_render_stream::AssignedStageSchedule::from_topology(
            &crate::product_foundation_assigned_stage_topology().unwrap(),
        )
        .unwrap();
        let stage = schedule
            .require(XMAP17_EXECUTE_STAGE, XMAP17_EXECUTE_OPERATION)
            .unwrap()
            .clone();
        assert!(frame
            .preflight_execution(&ready, &identity, &stage)
            .is_err());
        assert_eq!(alpha, MapNameAlphaState::new(0.0, 0.0).unwrap());
    }
}
