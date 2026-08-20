
use crate::{
    decode_bmp_indexed, extract_block_after, find_assignment, latin1_bytes_to_string,
    map_building_placements_from_source, materialize_xmap12_pdxmesh, parse_bool_value,
    parse_building_entity_bindings, parse_building_pdxmesh_bindings, parse_f32_value, parse_i32s,
    parse_quoted_items, parse_quoted_value, parse_u32_value, show_on_map_buildings_from_source,
    strip_comments, BuildingEntityBinding, BuildingPdxmeshBinding, GameFileResolver, IndexImage,
    L03TypedOwner, MapBuildingPlacement, Result, ShowOnMapBuilding, ViewerMapCameraState,
    Xmap12MeshSettingOverride, Xmap12PdxMeshMaterialFamily, Xmap12PdxMeshMaterialization,
    Xmap12PdxMeshRequest, Xmap12TextureSource, BUILDING_SHADER_SOURCE,
};
use replay_core::{
    BuildingStatus, Leaf, StateBuildings, UsedContentKey, ValidatedContentAdmission, ViewStatus,
    WorldFrame,
};
use serde::{Deserialize, Serialize};
use snapshot::sha256::Sha256;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

pub const XMAP12_BUILDING_FRAME_SCHEMA: &str = "hoi4.replay.xmap12.building-frame.a729.v1";
pub const XMAP12_CITY_FRAME_SCHEMA: &str = "hoi4.replay.xmap12.city-frame.a729.v1";
pub const XMAP12_FIXED_FRAME_SCHEMA: &str = "hoi4.replay.xmap12.fixed-frame.a729.v1";
pub const XMAP12_RENDER_FRAME_SCHEMA: &str = "hoi4.replay.xmap12.render-frame.a729.v2";
pub const XMAP12_DEFAULT_MAP_SOURCE: &str = "map/default.map";
pub const XMAP12_MAP_BUILDINGS_SOURCE: &str = "map/buildings.txt";
pub const XMAP12_COMMON_BUILDINGS_PREFIX: &str = "common/buildings/";
pub const XMAP12_DEFINES_PREFIX: &str = "common/defines/";
pub const XMAP12_PROVINCES_SOURCE: &str = "map/provinces.bmp";
pub const XMAP12_DEFINITION_SOURCE: &str = "map/definition.csv";
pub const XMAP12_CITIES_SOURCE: &str = "map/cities.txt";
pub const XMAP12_TERRAIN_SOURCE: &str = "map/terrain.bmp";
pub const XMAP12_RIVERS_SOURCE: &str = "map/rivers.bmp";
pub const XMAP12_HEIGHTMAP_SOURCE: &str = "map/heightmap.bmp";
pub const XMAP12_TERRAIN_DEFINITION_PREFIX: &str = "common/terrain/";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Xmap12BuildingDisposition {
    HiddenByLevel,
    HiddenByCamera,
    Visible,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap12BuildingSemantic {
    pub stable_typed_id: String,
    pub source_ordinal: u32,
    pub state_id: u32,
    pub province_id: u32,
    pub building_token: String,
    pub slot_index: u32,
    pub level_f32_bits: u32,
    pub max_building_levels: u32,
    pub entity_variant: u32,
    pub destroyed_variant: bool,
    pub under_construction: bool,
    pub disposition: Xmap12BuildingDisposition,
    pub world_matrix_f32_bits: [u32; 16],
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedXmap12BuildingFrame {
    pub schema: String,
    pub content_manifest_sha256: String,
    pub effective_vfs_manifest_sha256: String,
    pub replay_frame_identity_sha256: String,
    pub building_visual_frame_sha256: String,
    pub viewer_state_sha256: String,
    pub map_buildings_source: UsedContentKey,
    pub common_building_sources: Vec<UsedContentKey>,
    pub defines_sources: Vec<UsedContentKey>,
    pub complete_population: Vec<Xmap12BuildingSemantic>,
    pub ordered_visible_draws: Vec<Xmap12PdxMeshMaterialization>,
    pub deterministic_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap12BuildingVisualFrame {
    pub replay_frame_identity_sha256: String,
    pub rows: Vec<Xmap12BuildingVisualRow>,
    pub deterministic_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap12BuildingVisualRow {
    pub source_ordinal: u32,
    pub selected_entity_token: String,
    pub destroyed_variant: bool,
    pub under_construction: bool,
    pub visual_scale_f32_bits: u32,
}

impl Xmap12BuildingVisualFrame {
    pub fn validate(&self) -> Result<()> {
        validate_sha256(
            &self.replay_frame_identity_sha256,
            "XMAP12 building visual replay frame",
        )?;
        for (index, row) in self.rows.iter().enumerate() {
            let scale = f32::from_bits(row.visual_scale_f32_bits);
            if row.source_ordinal as usize != index
                || row.selected_entity_token.trim().is_empty()
                || !scale.is_finite()
                || scale <= 0.0
            {
                return Err("XMAP12 building visual row is invalid".into());
            }
        }
        validate_sha256(&self.deterministic_sha256, "XMAP12 building visual frame")?;
        let mut unsigned = self.clone();
        unsigned.deterministic_sha256.clear();
        if self.deterministic_sha256 != Sha256::hex(&serde_json::to_vec(&unsigned)?) {
            return Err("XMAP12 building visual frame digest mismatch".into());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Xmap12CityDisposition {
    MissingEntity,
    HiddenByCamera,
    Visible,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap12CitySemantic {
    pub stable_typed_id: String,
    pub source_ordinal: u32,
    pub source_group_ordinal: u32,
    pub color_index: u8,
    pub terrain_index: u8,
    pub base_province_id: u32,
    pub displaced_province_id: u32,
    pub base_x: u32,
    pub base_z: u32,
    pub displaced_x: i32,
    pub displaced_z: i32,
    pub urban_depth: u32,
    pub entity_token: Option<String>,
    pub yaw_quadrant: u8,
    pub disposition: Xmap12CityDisposition,
    pub world_matrix_f32_bits: [u32; 16],
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedXmap12CityFrame {
    pub schema: String,
    pub content_manifest_sha256: String,
    pub effective_vfs_manifest_sha256: String,
    pub viewer_state_sha256: String,
    pub cities_source: UsedContentKey,
    pub types_source: UsedContentKey,
    pub terrain_source: UsedContentKey,
    pub rivers_source: UsedContentKey,
    pub heightmap_source: UsedContentKey,
    pub province_source: UsedContentKey,
    pub definition_source: UsedContentKey,
    pub terrain_definition_sources: Vec<UsedContentKey>,
    pub complete_population: Vec<Xmap12CitySemantic>,
    pub ordered_visible_draws: Vec<Xmap12PdxMeshMaterialization>,
    pub deterministic_sha256: String,
}

impl PreparedXmap12CityFrame {
    pub fn validate(&self) -> Result<()> {
        if self.schema != XMAP12_CITY_FRAME_SCHEMA
            || self
                .complete_population
                .iter()
                .enumerate()
                .any(|(index, row)| row.source_ordinal as usize != index)
        {
            return Err("XMAP12 city frame population identity/order is invalid".into());
        }
        validate_sha256(
            &self.content_manifest_sha256,
            "XMAP12 city content manifest",
        )?;
        validate_sha256(
            &self.effective_vfs_manifest_sha256,
            "XMAP12 city effective VFS",
        )?;
        validate_sha256(&self.viewer_state_sha256, "XMAP12 city viewer state")?;
        let visible = self
            .complete_population
            .iter()
            .filter(|row| row.disposition == Xmap12CityDisposition::Visible)
            .count();
        if visible != self.ordered_visible_draws.len() {
            return Err("XMAP12 city visible population/draw count mismatch".into());
        }
        for (order, draw) in self.ordered_visible_draws.iter().enumerate() {
            draw.validate()?;
            if draw.object_draw.visible_order as usize != order {
                return Err("XMAP12 city visible draw order is not contiguous".into());
            }
        }
        validate_sha256(&self.deterministic_sha256, "XMAP12 city frame")?;
        let mut unsigned = self.clone();
        unsigned.deterministic_sha256.clear();
        if self.deterministic_sha256 != Sha256::hex(&serde_json::to_vec(&unsigned)?) {
            return Err("XMAP12 city frame digest mismatch".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap12FixedRuntimeState {
    pub replay_frame_identity_sha256: String,
    pub elapsed_renderer_seconds_f32_bits: u32,
    pub map_object_presentation_enabled: bool,
    pub enabled_dlc_names: Vec<String>,
    pub ambient_types: Vec<Xmap12FixedTypeRuntimeCarrier>,
    pub deterministic_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap12FixedTypeRuntimeCarrier {
    pub type_ordinal: u32,
    pub engine_counter: u32,
    pub objects: Vec<Xmap12FixedObjectRuntimeCarrier>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap12FixedObjectRuntimeCarrier {
    pub object_ordinal: u32,
    pub active: bool,
    pub remaining_duration_f32_bits: Option<u32>,
}

impl Xmap12FixedRuntimeState {
    pub fn validate(&self) -> Result<()> {
        validate_sha256(
            &self.replay_frame_identity_sha256,
            "XMAP12 fixed runtime replay frame",
        )?;
        let elapsed = f32::from_bits(self.elapsed_renderer_seconds_f32_bits);
        if !elapsed.is_finite()
            || elapsed < 0.0
            || self
                .enabled_dlc_names
                .iter()
                .any(|name| name.trim().is_empty())
        {
            return Err("XMAP12 fixed runtime state is invalid".into());
        }
        let mut sorted = self.enabled_dlc_names.clone();
        sorted.sort();
        sorted.dedup();
        if sorted != self.enabled_dlc_names {
            return Err("XMAP12 fixed DLC names are not canonical sorted unique values".into());
        }
        for (type_index, ambient_type) in self.ambient_types.iter().enumerate() {
            if ambient_type.type_ordinal as usize != type_index {
                return Err("XMAP12 fixed runtime type ordinals are not contiguous".into());
            }
            for (object_index, object) in ambient_type.objects.iter().enumerate() {
                if object.object_ordinal as usize != object_index
                    || object.remaining_duration_f32_bits.is_some_and(|bits| {
                        let value = f32::from_bits(bits);
                        !value.is_finite() || value < 0.0
                    })
                {
                    return Err("XMAP12 fixed runtime object carrier is invalid".into());
                }
            }
        }
        validate_sha256(&self.deterministic_sha256, "XMAP12 fixed runtime state")?;
        let mut unsigned = self.clone();
        unsigned.deterministic_sha256.clear();
        if self.deterministic_sha256 != Sha256::hex(&serde_json::to_vec(&unsigned)?) {
            return Err("XMAP12 fixed runtime state digest mismatch".into());
        }
        Ok(())
    }

    pub fn validate_for_replay(&self, replay_frame_identity_sha256: &str) -> Result<()> {
        self.validate()?;
        validate_sha256(
            replay_frame_identity_sha256,
            "XMAP12 expected fixed runtime replay frame",
        )?;
        if self.replay_frame_identity_sha256 != replay_frame_identity_sha256 {
            return Err("XMAP12 fixed runtime state belongs to a different replay frame".into());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Xmap12FixedDisposition {
    InactiveLifecycle,
    MissingEntity,
    HiddenByCamera,
    Visible,
}

pub fn xmap12_fixed_spawn_refused(
    object_active: bool,
    engine_counter: u32,
    max_visible: Option<u32>,
) -> bool {
    match max_visible {
        Some(limit) if limit >= 1 && engine_counter > limit => true,
        _ => object_active,
    }
}

pub fn xmap12_fixed_apply_active_transition(
    engine_counter: &mut u32,
    object_active: &mut bool,
    new_active: bool,
) {
    if *object_active == new_active {
        return;
    }
    if new_active {
        *engine_counter = engine_counter.saturating_sub(1);
    } else {
        *engine_counter = engine_counter.saturating_add(1);
    }
    *object_active = new_active;
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap12FixedSemantic {
    pub stable_typed_id: String,
    pub source_ordinal: u32,
    pub type_ordinal: u32,
    pub object_ordinal: u32,
    pub object_name: String,
    pub entity_token: String,
    pub use_animation: bool,
    pub animation_name: String,
    pub always_visible: bool,
    pub spawn_refused: bool,
    pub disposition: Xmap12FixedDisposition,
    pub world_matrix_f32_bits: [u32; 16],
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedXmap12FixedFrame {
    pub schema: String,
    pub content_manifest_sha256: String,
    pub effective_vfs_manifest_sha256: String,
    pub replay_frame_identity_sha256: String,
    pub viewer_state_sha256: String,
    pub runtime_state_sha256: String,
    pub default_map_source: UsedContentKey,
    pub ambient_source: UsedContentKey,
    pub complete_population: Vec<Xmap12FixedSemantic>,
    pub ordered_visible_draws: Vec<Xmap12PdxMeshMaterialization>,
    pub deterministic_sha256: String,
}

impl PreparedXmap12FixedFrame {
    pub fn validate(&self) -> Result<()> {
        if self.schema != XMAP12_FIXED_FRAME_SCHEMA
            || self
                .complete_population
                .iter()
                .enumerate()
                .any(|(index, row)| row.source_ordinal as usize != index)
        {
            return Err("XMAP12 fixed frame population identity/order is invalid".into());
        }
        for (value, label) in [
            (
                &self.content_manifest_sha256,
                "XMAP12 fixed content manifest",
            ),
            (
                &self.effective_vfs_manifest_sha256,
                "XMAP12 fixed effective VFS",
            ),
            (
                &self.replay_frame_identity_sha256,
                "XMAP12 fixed replay frame",
            ),
            (&self.viewer_state_sha256, "XMAP12 fixed viewer state"),
            (&self.runtime_state_sha256, "XMAP12 fixed runtime state"),
            (&self.deterministic_sha256, "XMAP12 fixed frame"),
        ] {
            validate_sha256(value, label)?;
        }
        let visible = self
            .complete_population
            .iter()
            .filter(|row| row.disposition == Xmap12FixedDisposition::Visible)
            .count();
        if self.complete_population.iter().any(|row| {
            row.disposition != Xmap12FixedDisposition::InactiveLifecycle && !row.spawn_refused
        }) {
            return Err("XMAP12 active fixed object does not refuse a duplicate spawn".into());
        }
        if visible != self.ordered_visible_draws.len() {
            return Err("XMAP12 fixed visible population/draw count mismatch".into());
        }
        for (order, draw) in self.ordered_visible_draws.iter().enumerate() {
            draw.validate()?;
            if draw.object_draw.visible_order as usize != order {
                return Err("XMAP12 fixed visible draw order is not contiguous".into());
            }
        }
        let mut unsigned = self.clone();
        unsigned.deterministic_sha256.clear();
        if self.deterministic_sha256 != Sha256::hex(&serde_json::to_vec(&unsigned)?) {
            return Err("XMAP12 fixed frame digest mismatch".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedXmap12RenderFrame {
    pub schema: String,
    pub content_manifest_sha256: String,
    pub effective_vfs_manifest_sha256: String,
    pub replay_frame_identity_sha256: String,
    pub viewer_state_sha256: String,
    pub city_frame_sha256: String,
    pub building_frame_sha256: String,
    pub fixed_frame_sha256: String,
    pub fow_noise_content: UsedContentKey,
    pub fow_height_content: UsedContentKey,
    pub environment_map_content: UsedContentKey,
    pub ordered_draws: Vec<Xmap12PdxMeshMaterialization>,
    pub ordered_l03_rows: Vec<crate::L03SemanticInputRow>,
    pub deterministic_sha256: String,
}

impl PreparedXmap12RenderFrame {
    pub fn validate(&self) -> Result<()> {
        if self.schema != XMAP12_RENDER_FRAME_SCHEMA
            || self.ordered_draws.len() != self.ordered_l03_rows.len()
        {
            return Err("XMAP12 render frame shape is invalid".into());
        }
        for (value, label) in [
            (
                &self.content_manifest_sha256,
                "XMAP12 render content manifest",
            ),
            (
                &self.effective_vfs_manifest_sha256,
                "XMAP12 render effective VFS",
            ),
            (
                &self.replay_frame_identity_sha256,
                "XMAP12 render replay frame",
            ),
            (&self.viewer_state_sha256, "XMAP12 render viewer state"),
            (&self.city_frame_sha256, "XMAP12 city frame"),
            (&self.building_frame_sha256, "XMAP12 building frame"),
            (&self.fixed_frame_sha256, "XMAP12 fixed frame"),
            (&self.deterministic_sha256, "XMAP12 render frame"),
        ] {
            validate_sha256(value, label)?;
        }
        for (content, expected_path, label) in [
            (&self.fow_noise_content, None, "XMAP12 FOW noise texture"),
            (
                &self.fow_height_content,
                Some("map/terrain/fow_rgb_waterspec_a.dds"),
                "XMAP12 FOW height texture",
            ),
            (
                &self.environment_map_content,
                Some("map/terrain/reflection.dds"),
                "XMAP12 environment map",
            ),
        ] {
            content
                .validate()
                .map_err(|error| format!("{label} identity is invalid: {error}"))?;
            let normalized = content
                .normalized_game_relative_path
                .replace('\\', "/")
                .to_ascii_lowercase();
            if expected_path.is_some_and(|expected| normalized != expected)
                || (label == "XMAP12 FOW noise texture"
                    && !matches!(
                        normalized.as_str(),
                        "map/terrain/fow_noise_0.dds"
                            | "map/terrain/fow_noise_1.dds"
                            | "map/terrain/fow_noise_2.dds"
                    ))
            {
                return Err(format!("{label} has the wrong engine-selected path").into());
            }
        }
        let mut ids = BTreeSet::new();
        for (order, (draw, l03)) in self
            .ordered_draws
            .iter()
            .zip(&self.ordered_l03_rows)
            .enumerate()
        {
            draw.validate()?;
            if draw.object_draw.visible_order as usize != order
                || !ids.insert(draw.stable_typed_id.as_str())
                || *l03 != draw.l03_caster_row()
            {
                return Err("XMAP12 render draw/L03 order or identity is invalid".into());
            }
        }
        let mut unsigned = self.clone();
        unsigned.deterministic_sha256.clear();
        if self.deterministic_sha256 != Sha256::hex(&serde_json::to_vec(&unsigned)?) {
            return Err("XMAP12 render frame digest mismatch".into());
        }
        Ok(())
    }
}

pub fn prepare_xmap12_render_frame(
    cities: &PreparedXmap12CityFrame,
    buildings: &PreparedXmap12BuildingFrame,
    fixed: &PreparedXmap12FixedFrame,
    fow_noise_content: UsedContentKey,
    fow_height_content: UsedContentKey,
    environment_map_content: UsedContentKey,
) -> Result<PreparedXmap12RenderFrame> {
    cities.validate()?;
    buildings.validate()?;
    fixed.validate()?;
    if cities.content_manifest_sha256 != buildings.content_manifest_sha256
        || cities.content_manifest_sha256 != fixed.content_manifest_sha256
        || cities.effective_vfs_manifest_sha256 != buildings.effective_vfs_manifest_sha256
        || cities.effective_vfs_manifest_sha256 != fixed.effective_vfs_manifest_sha256
        || buildings.replay_frame_identity_sha256 != fixed.replay_frame_identity_sha256
        || cities.viewer_state_sha256 != buildings.viewer_state_sha256
        || cities.viewer_state_sha256 != fixed.viewer_state_sha256
    {
        return Err("XMAP12 component frames belong to different content/viewer state".into());
    }
    let mut ordered_draws = cities.ordered_visible_draws.clone();
    ordered_draws.extend(buildings.ordered_visible_draws.clone());
    ordered_draws.extend(fixed.ordered_visible_draws.clone());
    for (order, draw) in ordered_draws.iter_mut().enumerate() {
        draw.object_draw.visible_order = u32::try_from(order)?;
        draw.deterministic_sha256.clear();
        draw.deterministic_sha256 = Sha256::hex(&serde_json::to_vec(draw)?);
        draw.validate()?;
    }
    let ordered_l03_rows = ordered_draws
        .iter()
        .map(Xmap12PdxMeshMaterialization::l03_caster_row)
        .collect();
    let mut frame = PreparedXmap12RenderFrame {
        schema: XMAP12_RENDER_FRAME_SCHEMA.into(),
        content_manifest_sha256: cities.content_manifest_sha256.clone(),
        effective_vfs_manifest_sha256: cities.effective_vfs_manifest_sha256.clone(),
        replay_frame_identity_sha256: buildings.replay_frame_identity_sha256.clone(),
        viewer_state_sha256: cities.viewer_state_sha256.clone(),
        city_frame_sha256: cities.deterministic_sha256.clone(),
        building_frame_sha256: buildings.deterministic_sha256.clone(),
        fixed_frame_sha256: fixed.deterministic_sha256.clone(),
        fow_noise_content,
        fow_height_content,
        environment_map_content,
        ordered_draws,
        ordered_l03_rows,
        deterministic_sha256: String::new(),
    };
    frame.deterministic_sha256 = Sha256::hex(&serde_json::to_vec(&frame)?);
    frame.validate()?;
    Ok(frame)
}

impl PreparedXmap12BuildingFrame {
    pub fn validate(&self) -> Result<()> {
        if self.schema != XMAP12_BUILDING_FRAME_SCHEMA
            || self.complete_population.is_empty()
            || self
                .complete_population
                .iter()
                .enumerate()
                .any(|(index, row)| row.source_ordinal as usize != index)
        {
            return Err("XMAP12 building frame population identity/order is invalid".into());
        }
        validate_sha256(&self.content_manifest_sha256, "XMAP12 content manifest")?;
        validate_sha256(&self.effective_vfs_manifest_sha256, "XMAP12 effective VFS")?;
        validate_sha256(
            &self.replay_frame_identity_sha256,
            "XMAP12 replay frame identity",
        )?;
        validate_sha256(
            &self.building_visual_frame_sha256,
            "XMAP12 building visual frame",
        )?;
        validate_sha256(&self.viewer_state_sha256, "XMAP12 viewer state")?;
        let visible = self
            .complete_population
            .iter()
            .filter(|row| row.disposition == Xmap12BuildingDisposition::Visible)
            .count();
        if visible != self.ordered_visible_draws.len() {
            return Err("XMAP12 building visible population/draw count mismatch".into());
        }
        for (order, draw) in self.ordered_visible_draws.iter().enumerate() {
            draw.validate()?;
            if draw.object_draw.visible_order as usize != order {
                return Err("XMAP12 building visible draw order is not contiguous".into());
            }
        }
        validate_sha256(&self.deterministic_sha256, "XMAP12 building frame")?;
        let mut unsigned = self.clone();
        unsigned.deterministic_sha256.clear();
        if self.deterministic_sha256 != Sha256::hex(&serde_json::to_vec(&unsigned)?) {
            return Err("XMAP12 building frame digest mismatch".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct Xmap12ResolvedEntityChain {
    pub entity: BuildingEntityBinding,
    pub entity_source: UsedContentKey,
    pub pdxmesh: BuildingPdxmeshBinding,
    pub pdxmesh_source: UsedContentKey,
    pub mesh_source: UsedContentKey,
    pub shader_source: UsedContentKey,
    pub ordered_texture_sources: Vec<Xmap12TextureSource>,
}

pub fn xmap12_building_slot_visible(
    slot_index: u32,
    show_on_map: u32,
    level: f32,
    max_building_levels: u32,
) -> Result<bool> {
    if show_on_map == 0 || max_building_levels == 0 || !level.is_finite() || level < 0.0 {
        return Err("XMAP12 building slot law has invalid input".into());
    }
    let ratio = level / max_building_levels as f32;
    Ok(ratio >= 1.0 || slot_index as f32 / (show_on_map as f32) < ratio)
}

pub fn xmap12_building_entity_variant(
    variant_count: u32,
    level: f32,
    max_building_levels: u32,
) -> Result<u32> {
    if variant_count == 0 || max_building_levels == 0 || !level.is_finite() || level < 0.0 {
        return Err("XMAP12 building variant law has invalid input".into());
    }
    if variant_count < 2 || level <= 0.0 {
        return Ok(0);
    }
    let ratio = level / max_building_levels as f32;
    Ok(((variant_count as f32 * ratio) as u32).min(variant_count - 1))
}

pub fn prepare_xmap12_buildings(
    resolver: &GameFileResolver,
    admission: &ValidatedContentAdmission,
    world: &WorldFrame,
    camera: &ViewerMapCameraState,
    visual: &Xmap12BuildingVisualFrame,
) -> Result<PreparedXmap12BuildingFrame> {
    if world.d48_buildings_status() != ViewStatus::Present {
        return Err("XMAP12 buildings require a present D48 building domain".into());
    }
    camera.validate()?;
    visual.validate()?;
    let replay_frame_identity_sha256 = crate::xmap04_replay_frame_identity_sha256(world)?;
    if visual.replay_frame_identity_sha256 != replay_frame_identity_sha256 {
        return Err("XMAP12 building visual carrier belongs to another replay frame".into());
    }
    let map_buildings_key = required_content(admission, XMAP12_MAP_BUILDINGS_SOURCE)?.clone();
    let placements = map_building_placements_from_source(&latin1_bytes_to_string(
        &resolver.read_admitted_used_content(&map_buildings_key)?,
    ))?;
    if placements.is_empty() {
        return Err("XMAP12 map/buildings.txt has no placement population".into());
    }
    if visual.rows.len() != placements.len() {
        return Err("XMAP12 building visual census differs from map/buildings.txt".into());
    }
    let common_sources = admitted_prefix_sources(admission, XMAP12_COMMON_BUILDINGS_PREFIX, "txt")?;
    let show_on_map = merged_show_on_map_buildings(resolver, &common_sources)?;
    let show_by_token = show_on_map
        .iter()
        .map(|row| (row.name.as_str(), row))
        .collect::<BTreeMap<_, _>>();
    let define_sources = admitted_prefix_sources(admission, XMAP12_DEFINES_PREFIX, "lua")?;
    let max_levels = max_building_levels_from_admitted_sources(resolver, &define_sources)?;
    let province_raster = admitted_province_raster(resolver, admission)?;

    let mut per_state_type_slot = BTreeMap::<(u32, String), u32>::new();
    let mut complete_population = Vec::with_capacity(placements.len());
    let mut ordered_visible_draws = Vec::new();
    let mut chains = BTreeMap::<(String, u32), Xmap12ResolvedEntityChain>::new();
    for (source_ordinal, placement) in placements.iter().enumerate() {
        let visual_row = &visual.rows[source_ordinal];
        let show = show_by_token
            .get(placement.building.as_str())
            .ok_or_else(|| {
                format!(
                    "XMAP12 placement {} references building without show_on_map",
                    placement.building
                )
            })?;
        let slot = per_state_type_slot
            .entry((placement.state_id, placement.building.clone()))
            .or_default();
        let slot_index = *slot;
        *slot = slot
            .checked_add(1)
            .ok_or("XMAP12 building slot count overflow")?;
        if slot_index >= show.show_on_map {
            return Err(format!(
                "XMAP12 {} state {} has more placement slots than show_on_map={}",
                placement.building, placement.state_id, show.show_on_map
            )
            .into());
        }
        let province_id = province_raster.province_at_world(placement.pos[0], placement.pos[2])?;
        let status =
            building_status_for_placement(world.buildings_by_state(), placement, province_id)?;
        let level = measured_f64(&status.level, "building level")? as f32;
        for (logical, selected, label) in [
            (
                optional_measured_bool(&status.damaged),
                visual_row.destroyed_variant,
                "damaged",
            ),
            (
                optional_measured_bool(&status.under_construction),
                visual_row.under_construction,
                "under-construction",
            ),
        ] {
            if logical.is_some_and(|logical| logical != selected) {
                return Err(format!(
                    "XMAP12 building visual {label} state contradicts D48 logical state"
                )
                .into());
            }
        }
        let destroyed = visual_row.destroyed_variant;
        let under_construction = visual_row.under_construction;
        let variant_count = show.show_on_map_meshes.unwrap_or(1).max(1);
        let entity_variant = xmap12_building_entity_variant(variant_count, level, max_levels)?;
        let visible =
            xmap12_building_slot_visible(slot_index, show.show_on_map, level, max_levels)?;
        let world_matrix = y_rotation_translation_matrix(
            placement.rotation,
            placement.pos,
            f32::from_bits(visual_row.visual_scale_f32_bits),
        )?;
        let stable_typed_id = format!(
            "xmap12:building:{}:{}:{}:{}",
            placement.state_id, province_id, placement.building, slot_index
        );
        let row = Xmap12BuildingSemantic {
            stable_typed_id: stable_typed_id.clone(),
            source_ordinal: u32::try_from(source_ordinal)?,
            state_id: placement.state_id,
            province_id,
            building_token: placement.building.clone(),
            slot_index,
            level_f32_bits: level.to_bits(),
            max_building_levels: max_levels,
            entity_variant,
            destroyed_variant: destroyed,
            under_construction,
            disposition: if visible {
                Xmap12BuildingDisposition::Visible
            } else {
                Xmap12BuildingDisposition::HiddenByLevel
            },
            world_matrix_f32_bits: world_matrix,
        };
        complete_population.push(row);
        if !visible {
            continue;
        }
        let chain_key = (visual_row.selected_entity_token.clone(), entity_variant);
        if !chains.contains_key(&chain_key) {
            chains.insert(
                chain_key.clone(),
                resolve_xmap12_entity_chain(
                    resolver,
                    admission,
                    &visual_row.selected_entity_token,
                )?,
            );
        }
        let chain = chains.get(&chain_key).expect("inserted chain");
        let request = Xmap12PdxMeshRequest {
            visible_order: u32::try_from(ordered_visible_draws.len())?,
            stable_typed_id,
            owner: L03TypedOwner::O05StaticFixed,
            entity_token: chain.entity.name.clone(),
            pdxmesh_token: chain.pdxmesh.name.clone(),
            entity_source: chain.entity_source.clone(),
            pdxmesh_declaration_source: chain.pdxmesh_source.clone(),
            mesh_source: chain.mesh_source.clone(),
            shader_source: chain.shader_source.clone(),
            ordered_texture_sources: chain.ordered_texture_sources.clone(),
            mesh_settings: xmap12_mesh_settings(&chain.pdxmesh),
            world_matrix_f32_bits: world_matrix,
            entity_scale_f32_bits: chain.entity.scale.to_bits(),
            pdxmesh_scale_f32_bits: chain.pdxmesh.scale.to_bits(),
            material_family: Xmap12PdxMeshMaterialFamily::Standard,
            selected_lod_override: None,
            selected_shape_indices: None,
            skin_palette_3200_bytes: None,
        };
        if let Some(draw) = materialize_xmap12_pdxmesh(resolver, admission, camera, &request)? {
            ordered_visible_draws.push(draw);
        } else {
            complete_population
                .last_mut()
                .expect("just pushed")
                .disposition = Xmap12BuildingDisposition::HiddenByCamera;
        }
    }
    for (order, draw) in ordered_visible_draws.iter_mut().enumerate() {
        draw.object_draw.visible_order = u32::try_from(order)?;
    }
    let mut frame = PreparedXmap12BuildingFrame {
        schema: XMAP12_BUILDING_FRAME_SCHEMA.into(),
        content_manifest_sha256: admission.manifest_digest_sha256().into(),
        effective_vfs_manifest_sha256: map_buildings_key.effective_vfs_manifest_sha256.clone(),
        replay_frame_identity_sha256,
        building_visual_frame_sha256: visual.deterministic_sha256.clone(),
        viewer_state_sha256: camera.deterministic_sha256.clone(),
        map_buildings_source: map_buildings_key,
        common_building_sources: common_sources,
        defines_sources: define_sources,
        complete_population,
        ordered_visible_draws,
        deterministic_sha256: String::new(),
    };
    frame.deterministic_sha256 = Sha256::hex(&serde_json::to_vec(&frame)?);
    frame.validate()?;
    Ok(frame)
}

#[derive(Clone, Debug)]
struct CitySettings {
    types_source: String,
    step_x: u32,
    step_z: u32,
    groups: Vec<CityGroup>,
}

#[derive(Clone, Debug)]
struct CityGroup {
    color_index: u8,
    density: f32,
    buildings: Vec<CityTier>,
}

#[derive(Clone, Debug)]
struct CityTier {
    distance: i32,
    entities: Vec<String>,
}

#[derive(Clone, Debug)]
struct CityCandidate {
    base_x: u32,
    base_z: u32,
    terrain_index: u8,
    base_province_id: u32,
    group_ordinal: usize,
    urban_depth: u32,
    hash: u32,
    entity_token: String,
}

pub fn prepare_xmap12_cities(
    resolver: &GameFileResolver,
    admission: &ValidatedContentAdmission,
    camera: &ViewerMapCameraState,
) -> Result<PreparedXmap12CityFrame> {
    camera.validate()?;
    let cities_source = required_content(admission, XMAP12_CITIES_SOURCE)?.clone();
    let settings = parse_city_settings(&latin1_bytes_to_string(
        &resolver.read_admitted_used_content(&cities_source)?,
    ))?;
    let types_source = required_content(admission, &settings.types_source)?.clone();
    let terrain_source = required_content(admission, XMAP12_TERRAIN_SOURCE)?.clone();
    let rivers_source = required_content(admission, XMAP12_RIVERS_SOURCE)?.clone();
    let heightmap_source = required_content(admission, XMAP12_HEIGHTMAP_SOURCE)?.clone();
    let province_source = required_content(admission, XMAP12_PROVINCES_SOURCE)?.clone();
    let definition_source = required_content(admission, XMAP12_DEFINITION_SOURCE)?.clone();
    let terrain_definition_sources =
        admitted_prefix_sources(admission, XMAP12_TERRAIN_DEFINITION_PREFIX, "txt")?;
    let spawn_city = admitted_spawn_city_indices(resolver, &terrain_definition_sources)?;
    let types = decode_bmp_indexed(
        &resolver.read_admitted_used_content(&types_source)?,
        Path::new(&settings.types_source),
    )?;
    let terrain = decode_bmp_indexed(
        &resolver.read_admitted_used_content(&terrain_source)?,
        Path::new(XMAP12_TERRAIN_SOURCE),
    )?;
    let rivers = decode_bmp_indexed(
        &resolver.read_admitted_used_content(&rivers_source)?,
        Path::new(XMAP12_RIVERS_SOURCE),
    )?;
    let heightmap = decode_bmp_indexed(
        &resolver.read_admitted_used_content(&heightmap_source)?,
        Path::new(XMAP12_HEIGHTMAP_SOURCE),
    )?;
    let province_raster = admitted_province_raster(resolver, admission)?;
    let dimensions = (camera.map_width as usize, camera.map_height as usize);
    for (label, image) in [
        ("types_source", &types),
        ("terrain.bmp", &terrain),
        ("rivers.bmp", &rivers),
        ("heightmap.bmp", &heightmap),
    ] {
        if (image.width, image.height) != dimensions {
            return Err(format!(
                "XMAP12 city {label} dimensions {}x{} differ from active map {}x{}",
                image.width, image.height, dimensions.0, dimensions.1
            )
            .into());
        }
    }
    if (province_raster.width, province_raster.height) != dimensions {
        return Err("XMAP12 city province raster dimensions differ from active map".into());
    }

    let mut candidates = Vec::new();
    let mut candidate_by_lattice = BTreeMap::<(u32, u32), usize>::new();
    for base_z in (0..camera.map_height).step_by(settings.step_z as usize) {
        for base_x in (0..camera.map_width).step_by(settings.step_x as usize) {
            if !city_candidate_eligible(
                &terrain,
                &rivers,
                &spawn_city,
                base_x as i32,
                base_z as i32,
                settings.step_x as i32,
                settings.step_z as i32,
                false,
            )? {
                continue;
            }
            let base_province_id =
                province_raster.province_at_world(base_x as f32, base_z as f32)?;
            let color_index = sample_world_index(&types, base_x as i32, base_z as i32)
                .ok_or("XMAP12 city types_source coordinate is invalid")?;
            let Some((group_ordinal, group)) = settings
                .groups
                .iter()
                .enumerate()
                .find(|(_, group)| group.color_index == color_index)
            else {
                continue;
            };
            let urban_depth =
                city_urban_depth(&terrain, &spawn_city, base_x as i32, base_z as i32)?;
            let Some(tier) = group
                .buildings
                .iter()
                .rev()
                .find(|tier| tier.distance <= urban_depth as i32)
            else {
                continue;
            };
            if tier.entities.is_empty() {
                continue;
            }
            let seed = base_province_id.wrapping_mul(base_x).wrapping_mul(base_z);
            let hash = city_entity_hash(seed);
            let entity_token =
                tier.entities[(hash & 0x7fff_ffff) as usize % tier.entities.len()].clone();
            if try_resolve_xmap12_entity_chain(resolver, admission, &entity_token)?.is_none() {
                continue;
            }
            let terrain_index = sample_world_index(&terrain, base_x as i32, base_z as i32)
                .expect("eligible coordinate is in bounds");
            let index = candidates.len();
            candidates.push(CityCandidate {
                base_x,
                base_z,
                terrain_index,
                base_province_id,
                group_ordinal,
                urban_depth,
                hash,
                entity_token,
            });
            candidate_by_lattice.insert((base_x, base_z), index);
        }
    }

    let mut chains = BTreeMap::<String, Xmap12ResolvedEntityChain>::new();
    let mut complete_population = Vec::with_capacity(candidates.len());
    let mut ordered_visible_draws = Vec::new();
    for candidate in &candidates {
        let group = &settings.groups[candidate.group_ordinal];
        let gradient = city_population_gradient(
            candidate,
            &candidates,
            &candidate_by_lattice,
            camera.map_width,
            camera.map_height,
            settings.step_x,
            settings.step_z,
        );
        let denominator = candidate.urban_depth.max(1) as f32;
        let displaced_x_f32 = candidate.base_x as f32 + gradient[0] * group.density / denominator;
        let displaced_z_f32 = candidate.base_z as f32 + gradient[1] * group.density / denominator;
        let displaced_x = displaced_x_f32 as i32;
        let displaced_z = displaced_z_f32 as i32;
        if !city_candidate_eligible(
            &terrain,
            &rivers,
            &spawn_city,
            displaced_x,
            displaced_z,
            settings.step_x as i32,
            settings.step_z as i32,
            false,
        )? {
            continue;
        }
        let height = sample_xmap12_height(&heightmap, displaced_x_f32, displaced_z_f32)?;
        let yaw_quadrant = (candidate.hash & 3) as u8;
        let yaw = yaw_quadrant as f32 * std::f32::consts::FRAC_PI_2;
        let world_matrix =
            y_rotation_translation_matrix(yaw, [displaced_x_f32, height, displaced_z_f32], 1.0)?;
        let displaced_province_id = province_raster
            .province_at_lattice_any(displaced_x, displaced_z)
            .filter(|province| *province != 0)
            .unwrap_or(candidate.base_province_id);
        let stable_typed_id = format!(
            "xmap12:city:{}:{}:{}",
            settings.types_source, candidate.base_x, candidate.base_z
        );
        if !chains.contains_key(&candidate.entity_token) {
            let chain =
                try_resolve_xmap12_entity_chain(resolver, admission, &candidate.entity_token)?
                    .ok_or("XMAP12 city entity disappeared between production passes")?;
            chains.insert(candidate.entity_token.clone(), chain);
        }
        let chain = chains
            .get(&candidate.entity_token)
            .expect("resolved city entity chain");
        let request = Xmap12PdxMeshRequest {
            visible_order: u32::try_from(ordered_visible_draws.len())?,
            stable_typed_id: stable_typed_id.clone(),
            owner: L03TypedOwner::O04City,
            entity_token: chain.entity.name.clone(),
            pdxmesh_token: chain.pdxmesh.name.clone(),
            entity_source: chain.entity_source.clone(),
            pdxmesh_declaration_source: chain.pdxmesh_source.clone(),
            mesh_source: chain.mesh_source.clone(),
            shader_source: chain.shader_source.clone(),
            ordered_texture_sources: chain.ordered_texture_sources.clone(),
            mesh_settings: xmap12_mesh_settings(&chain.pdxmesh),
            world_matrix_f32_bits: world_matrix,
            entity_scale_f32_bits: chain.entity.scale.to_bits(),
            pdxmesh_scale_f32_bits: chain.pdxmesh.scale.to_bits(),
            material_family: Xmap12PdxMeshMaterialFamily::Standard,
            selected_lod_override: None,
            selected_shape_indices: None,
            skin_palette_3200_bytes: None,
        };
        let draw = materialize_xmap12_pdxmesh(resolver, admission, camera, &request)?;
        let disposition = if draw.is_some() {
            Xmap12CityDisposition::Visible
        } else {
            Xmap12CityDisposition::HiddenByCamera
        };
        if let Some(draw) = draw {
            ordered_visible_draws.push(draw);
        }
        complete_population.push(Xmap12CitySemantic {
            stable_typed_id,
            source_ordinal: u32::try_from(complete_population.len())?,
            source_group_ordinal: u32::try_from(candidate.group_ordinal)?,
            color_index: group.color_index,
            terrain_index: candidate.terrain_index,
            base_province_id: candidate.base_province_id,
            displaced_province_id,
            base_x: candidate.base_x,
            base_z: candidate.base_z,
            displaced_x,
            displaced_z,
            urban_depth: candidate.urban_depth,
            entity_token: Some(candidate.entity_token.clone()),
            yaw_quadrant,
            disposition,
            world_matrix_f32_bits: world_matrix,
        });
    }
    for (order, draw) in ordered_visible_draws.iter_mut().enumerate() {
        draw.object_draw.visible_order = u32::try_from(order)?;
    }
    let mut frame = PreparedXmap12CityFrame {
        schema: XMAP12_CITY_FRAME_SCHEMA.into(),
        content_manifest_sha256: admission.manifest_digest_sha256().into(),
        effective_vfs_manifest_sha256: cities_source.effective_vfs_manifest_sha256.clone(),
        viewer_state_sha256: camera.deterministic_sha256.clone(),
        cities_source,
        types_source,
        terrain_source,
        rivers_source,
        heightmap_source,
        province_source,
        definition_source,
        terrain_definition_sources,
        complete_population,
        ordered_visible_draws,
        deterministic_sha256: String::new(),
    };
    frame.deterministic_sha256 = Sha256::hex(&serde_json::to_vec(&frame)?);
    frame.validate()?;
    Ok(frame)
}

#[derive(Clone, Debug)]
struct FixedType {
    entity_token: String,
    use_animation: bool,
    animation_name: String,
    scale: f32,
    always_visible: bool,
    duration: Option<f32>,
    max_visible: Option<u32>,
    required_dlc_name: Option<String>,
    objects: Vec<FixedObject>,
}

#[derive(Clone, Debug)]
struct FixedObject {
    name: String,
    position: [f32; 3],
    rotation: [f32; 3],
}

pub fn prepare_xmap12_fixed_objects(
    resolver: &GameFileResolver,
    admission: &ValidatedContentAdmission,
    camera: &ViewerMapCameraState,
    runtime: &Xmap12FixedRuntimeState,
) -> Result<PreparedXmap12FixedFrame> {
    camera.validate()?;
    runtime.validate()?;
    let default_map_source = required_content(admission, XMAP12_DEFAULT_MAP_SOURCE)?.clone();
    let default_map = strip_comments(&latin1_bytes_to_string(
        &resolver.read_admitted_used_content(&default_map_source)?,
    ));
    let ambient_relative = parse_quoted_value(&default_map, "ambient_object")
        .ok_or("XMAP12 default.map has no ambient_object source")?;
    let ambient_path = format!("map/{ambient_relative}");
    let ambient_source = required_content(admission, &ambient_path)?.clone();
    let fixed_types = parse_fixed_types(&latin1_bytes_to_string(
        &resolver.read_admitted_used_content(&ambient_source)?,
    ))?;
    if runtime.ambient_types.len() != fixed_types.len()
        || runtime
            .ambient_types
            .iter()
            .zip(&fixed_types)
            .any(|(carrier, authored)| carrier.objects.len() != authored.objects.len())
    {
        return Err(
            "XMAP12 fixed runtime lifecycle census differs from admitted ambient source".into(),
        );
    }
    let heightmap_source = required_content(admission, XMAP12_HEIGHTMAP_SOURCE)?;
    let heightmap = decode_bmp_indexed(
        &resolver.read_admitted_used_content(heightmap_source)?,
        Path::new(XMAP12_HEIGHTMAP_SOURCE),
    )?;
    if (heightmap.width, heightmap.height)
        != (camera.map_width as usize, camera.map_height as usize)
    {
        return Err("XMAP12 fixed-object heightmap dimensions differ from active map".into());
    }
    let enabled_dlcs = runtime
        .enabled_dlc_names
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let mut complete_population = Vec::new();
    let mut ordered_visible_draws = Vec::new();
    let mut chains = BTreeMap::<String, Xmap12ResolvedEntityChain>::new();
    for (type_ordinal, fixed_type) in fixed_types.iter().enumerate() {
        let runtime_type = &runtime.ambient_types[type_ordinal];
        let dlc_enabled = !fixed_type
            .required_dlc_name
            .as_deref()
            .is_some_and(|name| !enabled_dlcs.contains(name));
        let presentation_enabled =
            fixed_type.always_visible || runtime.map_object_presentation_enabled;
        if (!dlc_enabled || !presentation_enabled)
            && runtime_type.objects.iter().any(|object| object.active)
        {
            return Err(
                "XMAP12 fixed runtime retains active objects after DLC/presentation removal".into(),
            );
        }
        if let Some(chain) =
            try_resolve_xmap12_entity_chain(resolver, admission, &fixed_type.entity_token)?
        {
            chains
                .entry(fixed_type.entity_token.clone())
                .or_insert(chain);
        }
        let chain = chains.get(&fixed_type.entity_token);
        let material_family = chain
            .map(|chain| xmap12_chain_material_family(resolver, chain))
            .transpose()?;
        for (object_ordinal, object) in fixed_type.objects.iter().enumerate() {
            let runtime_object = &runtime_type.objects[object_ordinal];
            match (
                fixed_type.duration,
                runtime_object.remaining_duration_f32_bits,
            ) {
                (Some(authored), Some(bits)) => {
                    let remaining = f32::from_bits(bits);
                    if remaining > authored || (runtime_object.active && remaining <= 0.0) {
                        return Err(
                            "XMAP12 fixed runtime duration differs from authored lifecycle".into(),
                        );
                    }
                }
                (Some(_), None) if runtime_object.active => {
                    return Err(
                        "XMAP12 active duration-limited object has no remaining duration".into(),
                    );
                }
                (None, Some(_)) => {
                    return Err("XMAP12 undurated ambient object carries a runtime duration".into());
                }
                _ => {}
            }
            let y = if fixed_type.always_visible {
                object.position[1]
            } else {
                sample_xmap12_height(&heightmap, object.position[0], object.position[2])?
                    + object.position[1]
            };
            let yaw = object.rotation[1] / 360.0 * 6.283_185_5;
            let world_matrix = y_rotation_translation_matrix(
                yaw,
                [object.position[0], y, object.position[2]],
                fixed_type.scale,
            )?;
            let stable_typed_id = format!(
                "xmap12:fixed:{}:{}:{}:{}",
                ambient_path, type_ordinal, object_ordinal, object.name
            );
            let draw = if runtime_object.active {
                if let (Some(chain), Some(material_family)) = (chain, material_family) {
                    let request = Xmap12PdxMeshRequest {
                        visible_order: u32::try_from(ordered_visible_draws.len())?,
                        stable_typed_id: stable_typed_id.clone(),
                        owner: L03TypedOwner::O05StaticFixed,
                        entity_token: chain.entity.name.clone(),
                        pdxmesh_token: chain.pdxmesh.name.clone(),
                        entity_source: chain.entity_source.clone(),
                        pdxmesh_declaration_source: chain.pdxmesh_source.clone(),
                        mesh_source: chain.mesh_source.clone(),
                        shader_source: chain.shader_source.clone(),
                        ordered_texture_sources: chain.ordered_texture_sources.clone(),
                        mesh_settings: xmap12_mesh_settings(&chain.pdxmesh),
                        world_matrix_f32_bits: world_matrix,
                        entity_scale_f32_bits: chain.entity.scale.to_bits(),
                        pdxmesh_scale_f32_bits: chain.pdxmesh.scale.to_bits(),
                        material_family,
                        selected_lod_override: None,
                        selected_shape_indices: None,
                        skin_palette_3200_bytes: None,
                    };
                    materialize_xmap12_pdxmesh(resolver, admission, camera, &request)?
                } else {
                    None
                }
            } else {
                None
            };
            let disposition = if !runtime_object.active {
                Xmap12FixedDisposition::InactiveLifecycle
            } else if chain.is_none() {
                Xmap12FixedDisposition::MissingEntity
            } else if draw.is_some() {
                Xmap12FixedDisposition::Visible
            } else {
                Xmap12FixedDisposition::HiddenByCamera
            };
            if let Some(draw) = draw {
                ordered_visible_draws.push(draw);
            }
            complete_population.push(Xmap12FixedSemantic {
                stable_typed_id,
                source_ordinal: u32::try_from(complete_population.len())?,
                type_ordinal: u32::try_from(type_ordinal)?,
                object_ordinal: u32::try_from(object_ordinal)?,
                object_name: object.name.clone(),
                entity_token: fixed_type.entity_token.clone(),
                use_animation: fixed_type.use_animation,
                animation_name: fixed_type.animation_name.clone(),
                always_visible: fixed_type.always_visible,
                spawn_refused: xmap12_fixed_spawn_refused(
                    runtime_object.active,
                    runtime_type.engine_counter,
                    fixed_type.max_visible,
                ),
                disposition,
                world_matrix_f32_bits: world_matrix,
            });
        }
    }
    for (order, draw) in ordered_visible_draws.iter_mut().enumerate() {
        draw.object_draw.visible_order = u32::try_from(order)?;
    }
    let mut frame = PreparedXmap12FixedFrame {
        schema: XMAP12_FIXED_FRAME_SCHEMA.into(),
        content_manifest_sha256: admission.manifest_digest_sha256().into(),
        effective_vfs_manifest_sha256: ambient_source.effective_vfs_manifest_sha256.clone(),
        replay_frame_identity_sha256: runtime.replay_frame_identity_sha256.clone(),
        viewer_state_sha256: camera.deterministic_sha256.clone(),
        runtime_state_sha256: runtime.deterministic_sha256.clone(),
        default_map_source,
        ambient_source,
        complete_population,
        ordered_visible_draws,
        deterministic_sha256: String::new(),
    };
    frame.deterministic_sha256 = Sha256::hex(&serde_json::to_vec(&frame)?);
    frame.validate()?;
    Ok(frame)
}

pub fn resolve_xmap12_entity_chain(
    resolver: &GameFileResolver,
    admission: &ValidatedContentAdmission,
    entity_token: &str,
) -> Result<Xmap12ResolvedEntityChain> {
    try_resolve_xmap12_entity_chain(resolver, admission, entity_token)?.ok_or_else(|| {
        format!("XMAP12 entity {entity_token:?} is absent from admitted assets").into()
    })
}

fn try_resolve_xmap12_entity_chain(
    resolver: &GameFileResolver,
    admission: &ValidatedContentAdmission,
    entity_token: &str,
) -> Result<Option<Xmap12ResolvedEntityChain>> {
    if entity_token.trim().is_empty() {
        return Err("XMAP12 entity token is blank".into());
    }
    let asset_sources = resolver.require_complete_admitted_dir(
        "",
        Some("asset"),
        true,
        "XMAP12 asset namespace",
    )?;
    let mut entity_hits = Vec::new();
    for key in &asset_sources {
        let text = latin1_bytes_to_string(&resolver.read_admitted_used_content(key)?);
        if !text.contains(entity_token) {
            continue;
        }
        for entity in parse_building_entity_bindings(&text, &key.normalized_game_relative_path)? {
            if entity.name == entity_token {
                entity_hits.push((entity, key.clone()));
            }
        }
    }
    if entity_hits.is_empty() {
        return Ok(None);
    }
    let (entity, entity_source) = unique_semantic_entity(entity_token, entity_hits)?;

    let gfx_sources =
        resolver.require_complete_admitted_dir("", Some("gfx"), true, "XMAP12 GFX namespace")?;
    let mut pdxmesh_hits = Vec::new();
    for key in &gfx_sources {
        let text = latin1_bytes_to_string(&resolver.read_admitted_used_content(key)?);
        if !text.contains(&entity.pdxmesh) {
            continue;
        }
        for pdxmesh in parse_building_pdxmesh_bindings(&text, &key.normalized_game_relative_path)? {
            if pdxmesh.name == entity.pdxmesh {
                pdxmesh_hits.push((pdxmesh, key.clone()));
            }
        }
    }
    let (pdxmesh, pdxmesh_source) = unique_semantic_pdxmesh(&entity.pdxmesh, pdxmesh_hits)?;
    let mesh_source = required_content(admission, &pdxmesh.mesh)?.clone();
    let shader_source = required_content(admission, BUILDING_SHADER_SOURCE)?.clone();
    let mesh_bytes = resolver.read_admitted_used_content(&mesh_source)?;
    let model = crate::extract_pdx_model(&crate::parse_pdx_asset(&mesh_bytes)?)?;
    let ordered_texture_sources =
        admitted_material_texture_sources(admission, &mesh_source, &model, &pdxmesh)?;
    Ok(Some(Xmap12ResolvedEntityChain {
        entity,
        entity_source,
        pdxmesh,
        pdxmesh_source,
        mesh_source,
        shader_source,
        ordered_texture_sources,
    }))
}

fn parse_fixed_types(source: &str) -> Result<Vec<FixedType>> {
    let text = strip_comments(source);
    let mut rows = Vec::new();
    let mut search = 0usize;
    while let Some((body, next)) = extract_block_after(&text, "type", search) {
        let entity_token =
            parse_quoted_value(&body, "type").ok_or("XMAP12 ambient type has no entity token")?;
        let use_animation = parse_bool_value(&body, "use_animation").unwrap_or(true);
        let animation_name =
            parse_quoted_value(&body, "animation").unwrap_or_else(|| "idle".into());
        let scale = parse_f32_value(&body, "scale").unwrap_or(1.0);
        let always_visible = parse_bool_value(&body, "always_visible").unwrap_or(false);
        let duration = parse_f32_value(&body, "duration");
        let max_visible = parse_u32_value(&body, "max_visible");
        if !scale.is_finite()
            || scale <= 0.0
            || duration.is_some_and(|value| !value.is_finite() || value <= 0.0)
        {
            return Err("XMAP12 ambient type scale/duration is invalid".into());
        }
        let required_dlc_name = extract_block_after(&body, "dlc_allowed", 0)
            .and_then(|(trigger, _)| parse_quoted_value(&trigger, "has_dlc"));
        let mut objects = Vec::new();
        let mut object_search = 0usize;
        while let Some((object_body, object_next)) =
            extract_block_after(&body, "object", object_search)
        {
            let name = parse_quoted_value(&object_body, "name")
                .ok_or("XMAP12 ambient object has no name")?;
            let position = parse_fixed_vec3(&object_body, "position")?;
            let rotation = parse_fixed_vec3(&object_body, "rotation")?;
            objects.push(FixedObject {
                name,
                position,
                rotation,
            });
            object_search = object_next;
        }
        rows.push(FixedType {
            entity_token,
            use_animation,
            animation_name,
            scale,
            always_visible,
            duration,
            max_visible,
            required_dlc_name,
            objects,
        });
        search = next;
    }
    if rows.is_empty() {
        return Err("XMAP12 ambient source has no type declarations".into());
    }
    Ok(rows)
}

fn parse_fixed_vec3(body: &str, key: &str) -> Result<[f32; 3]> {
    let values = extract_block_after(body, key, 0)
        .map(|(block, _)| parse_f32_tokens(&block))
        .ok_or_else(|| format!("XMAP12 ambient object has no {key}"))?;
    if values.len() != 3 || values.iter().any(|value| !value.is_finite()) {
        return Err(format!("XMAP12 ambient object {key} is not exactly float3").into());
    }
    Ok([values[0], values[1], values[2]])
}

fn parse_f32_tokens(text: &str) -> Vec<f32> {
    text.split(|character: char| {
        character.is_whitespace() || matches!(character, '{' | '}' | ',' | ';')
    })
    .filter(|token| !token.is_empty())
    .filter_map(|token| token.parse::<f32>().ok())
    .collect()
}

fn xmap12_chain_material_family(
    resolver: &GameFileResolver,
    chain: &Xmap12ResolvedEntityChain,
) -> Result<Xmap12PdxMeshMaterialFamily> {
    let bytes = resolver.read_admitted_used_content(&chain.mesh_source)?;
    let model = crate::extract_pdx_model(&crate::parse_pdx_asset(&bytes)?)?;
    let mut families = BTreeSet::new();
    for (shape_index, shape) in model.shapes.iter().enumerate() {
        let mesh_setting = chain
            .pdxmesh
            .mesh_settings
            .iter()
            .find(|row| row.index == Some(shape_index as u32));
        for mesh in &shape.meshes {
            if mesh.material.is_collision() {
                continue;
            }
            let shader = mesh_setting
                .and_then(|row| row.shader.as_deref())
                .unwrap_or(&mesh.material.shader);
            families.insert(if shader == "PdxMeshBorder" {
                Xmap12PdxMeshMaterialFamily::FixedMapFurnitureBorder
            } else {
                Xmap12PdxMeshMaterialFamily::Standard
            });
        }
    }
    if families.len() != 1 {
        return Err("XMAP12 fixed PdxMesh mixes incompatible material families".into());
    }
    families
        .into_iter()
        .next()
        .ok_or_else(|| "XMAP12 fixed PdxMesh has no drawable material family".into())
}

fn parse_city_settings(source: &str) -> Result<CitySettings> {
    let text = strip_comments(source);
    let types_source =
        parse_quoted_value(&text, "types_source").ok_or("XMAP12 cities.txt has no types_source")?;
    let step_x = parse_u32_value(&text, "pixel_step_x").unwrap_or(1);
    let step_z = parse_u32_value(&text, "pixel_step_y").unwrap_or(1);
    if step_x == 0 || step_z == 0 {
        return Err("XMAP12 cities.txt pixel steps must be positive".into());
    }
    let mut groups = Vec::new();
    let mut group_search = 0usize;
    while let Some((group_body, next)) = extract_block_after(&text, "city_group", group_search) {
        let color_index = parse_u32_value(&group_body, "color_index")
            .ok_or("XMAP12 city_group has no color_index")?;
        let color_index = u8::try_from(color_index)
            .map_err(|_| "XMAP12 city_group color_index exceeds 8-bit mask")?;
        let density =
            parse_f32_value(&group_body, "density").ok_or("XMAP12 city_group has no density")?;
        if !density.is_finite() || density < 0.0 {
            return Err("XMAP12 city_group density is invalid".into());
        }
        let mut buildings = Vec::new();
        let mut building_search = 0usize;
        while let Some((building_body, building_next)) =
            extract_block_after(&group_body, "building", building_search)
        {
            let distance_at = find_assignment(&building_body, "distance", 0)
                .ok_or("XMAP12 city building tier has no distance")?;
            let distance = parse_i32s(&building_body[distance_at..])
                .first()
                .copied()
                .ok_or("XMAP12 city building distance is invalid")?;
            let entities = extract_block_after(&building_body, "mesh", 0)
                .map(|(body, _)| parse_quoted_items(&body))
                .unwrap_or_default();
            buildings.push(CityTier { distance, entities });
            building_search = building_next;
        }
        groups.push(CityGroup {
            color_index,
            density,
            buildings,
        });
        group_search = next;
    }
    if groups.is_empty() {
        return Err("XMAP12 cities.txt has no city_group declarations".into());
    }
    Ok(CitySettings {
        types_source,
        step_x,
        step_z,
        groups,
    })
}

fn admitted_spawn_city_indices(
    resolver: &GameFileResolver,
    sources: &[UsedContentKey],
) -> Result<BTreeSet<u8>> {
    let mut by_name = BTreeMap::<String, (u8, bool)>::new();
    for source in sources {
        let text = strip_comments(&latin1_bytes_to_string(
            &resolver.read_admitted_used_content(source)?,
        ));
        for (name, body) in top_level_assignment_blocks(&text)? {
            let Some(color_body) = extract_block_after(&body, "color", 0).map(|row| row.0) else {
                continue;
            };
            let Some(color) = parse_i32s(&color_body).first().copied() else {
                continue;
            };
            let Ok(color) = u8::try_from(color) else {
                return Err(format!(
                    "XMAP12 terrain {name} color is outside the 8-bit terrain raster"
                )
                .into());
            };
            let spawn = parse_bool_value(&body, "spawn_city").unwrap_or(false);
            by_name.insert(name, (color, spawn));
        }
    }
    let result = by_name
        .into_values()
        .filter_map(|(color, spawn)| spawn.then_some(color))
        .collect::<BTreeSet<_>>();
    if result.is_empty() {
        return Err("XMAP12 effective terrain definitions have no spawn_city terrain".into());
    }
    Ok(result)
}

fn top_level_assignment_blocks(text: &str) -> Result<Vec<(String, String)>> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut cursor = 0usize;
    while cursor < bytes.len() {
        while cursor < bytes.len() && (bytes[cursor].is_ascii_whitespace() || bytes[cursor] == b'}')
        {
            cursor += 1;
        }
        let name_start = cursor;
        while cursor < bytes.len()
            && (bytes[cursor].is_ascii_alphanumeric()
                || bytes[cursor] == b'_'
                || bytes[cursor] == b'-')
        {
            cursor += 1;
        }
        if cursor == name_start {
            cursor += 1;
            continue;
        }
        let name = text[name_start..cursor].to_string();
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if bytes.get(cursor) != Some(&b'=') {
            continue;
        }
        cursor += 1;
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if bytes.get(cursor) != Some(&b'{') {
            while cursor < bytes.len() && bytes[cursor] != b'\n' {
                cursor += 1;
            }
            continue;
        }
        let body_start = cursor + 1;
        let mut depth = 1i32;
        cursor += 1;
        while cursor < bytes.len() && depth > 0 {
            match bytes[cursor] {
                b'{' => depth += 1,
                b'}' => depth -= 1,
                _ => {}
            }
            cursor += 1;
        }
        if depth != 0 {
            return Err(format!("XMAP12 terrain block {name} is unterminated").into());
        }
        out.push((name, text[body_start..cursor - 1].to_string()));
    }
    Ok(out)
}

fn sample_world_index(image: &IndexImage, x: i32, z: i32) -> Option<u8> {
    if x < 0 || z < 0 || x >= image.width as i32 || z >= image.height as i32 {
        return None;
    }
    let row = image.height - 1 - z as usize;
    Some(image.pixels[row * image.width + x as usize])
}

fn city_candidate_eligible(
    terrain: &IndexImage,
    rivers: &IndexImage,
    spawn_city: &BTreeSet<u8>,
    x: i32,
    z: i32,
    step_x: i32,
    step_z: i32,
    perimeter_only: bool,
) -> Result<bool> {
    if terrain.width != rivers.width || terrain.height != rivers.height {
        return Err("XMAP12 terrain/river dimensions differ".into());
    }
    let Some(terrain_index) = sample_world_index(terrain, x, z) else {
        return Ok(false);
    };
    if !spawn_city.contains(&terrain_index) {
        return Ok(false);
    }
    if !perimeter_only
        && step_x > 0
        && step_z > 0
        && step_x < x
        && step_z < z
        && x < terrain.width as i32 - step_x
        && z < terrain.height as i32 - step_z
    {
        for dx in -step_x..step_x {
            for dz in -step_z..step_z {
                if sample_world_index(rivers, x + dx, z + dz).is_none_or(|value| value < 254) {
                    return Ok(false);
                }
            }
        }
    }
    Ok(true)
}

fn city_urban_depth(
    terrain: &IndexImage,
    spawn_city: &BTreeSet<u8>,
    x: i32,
    z: i32,
) -> Result<u32> {
    let dummy_rivers = terrain;
    for radius in 1i32..=99 {
        for edge_x in (x - radius)..=(x + radius) {
            for edge_z in [z - radius, z + radius] {
                if !city_candidate_eligible(
                    terrain,
                    dummy_rivers,
                    spawn_city,
                    edge_x,
                    edge_z,
                    0,
                    0,
                    true,
                )? {
                    return Ok(radius as u32);
                }
            }
        }
        for edge_z in (z - radius + 1)..=(z + radius - 1) {
            for edge_x in [x - radius, x + radius] {
                if !city_candidate_eligible(
                    terrain,
                    dummy_rivers,
                    spawn_city,
                    edge_x,
                    edge_z,
                    0,
                    0,
                    true,
                )? {
                    return Ok(radius as u32);
                }
            }
        }
    }
    Ok(100)
}

pub fn city_entity_hash(seed: u32) -> u32 {
    let mut h1 = seed.wrapping_mul(0x7eb8_da44);
    h1 = ((h1 >> 8) ^ h1).wrapping_add(0x10d8_365e);
    h1 = ((h1 << 8) ^ h1).wrapping_mul(0x6d22_f31b);
    h1 = (h1 >> 8) ^ h1;
    let mut h2 = seed.wrapping_mul(0x205a_d647);
    h2 = ((h2 >> 8) ^ h2).wrapping_add(0xb4f3_c315);
    h2 = ((h2 << 8) ^ h2).wrapping_mul(0x387a_9f3a);
    h2 = (h2 >> 8) ^ h2;
    let mut hash = h1.wrapping_sub(h2.wrapping_mul(0x4ad6_85b3));
    hash = ((hash >> 8) ^ hash).wrapping_add(0x68e3_1da4);
    hash = ((hash << 8) ^ hash).wrapping_mul(0x1b56_c4e9);
    hash = ((hash >> 8) ^ hash).wrapping_mul(0x92d6_8ca2);
    (hash >> 8) ^ hash
}

fn city_population_gradient(
    candidate: &CityCandidate,
    candidates: &[CityCandidate],
    by_lattice: &BTreeMap<(u32, u32), usize>,
    map_width: u32,
    map_height: u32,
    step_x: u32,
    step_z: u32,
) -> [f32; 2] {
    let mut gradient = [0.0f32; 2];
    for (dx, dz) in [
        (-(step_x as i32), -(step_z as i32)),
        (0, -(step_z as i32)),
        (step_x as i32, -(step_z as i32)),
        (-(step_x as i32), 0),
        (step_x as i32, 0),
        (-(step_x as i32), step_z as i32),
        (0, step_z as i32),
        (step_x as i32, step_z as i32),
    ] {
        let z = candidate.base_z as i32 + dz;
        if z < 0 || z >= map_height as i32 {
            continue;
        }
        let x = (candidate.base_x as i32 + dx).rem_euclid(map_width as i32) as u32;
        if let Some(neighbor) = by_lattice
            .get(&(x, z as u32))
            .map(|index| &candidates[*index])
        {
            gradient[0] += dx as f32 * neighbor.urban_depth as f32;
            gradient[1] += dz as f32 * neighbor.urban_depth as f32;
        }
    }
    let length = (gradient[0] * gradient[0] + gradient[1] * gradient[1]).sqrt();
    if length > 0.0 {
        gradient[0] /= length;
        gradient[1] /= length;
    }
    gradient
}

fn sample_xmap12_height(heightmap: &IndexImage, x: f32, z: f32) -> Result<f32> {
    if heightmap.width < 2 || heightmap.height < 2 || !x.is_finite() || !z.is_finite() {
        return Err("XMAP12 height sample input is invalid".into());
    }
    let image_x = x - 0.5;
    let image_z = z - 0.5;
    let x0 = (image_x as i32).clamp(0, heightmap.width as i32 - 2) as usize;
    let z0 = (image_z as i32).clamp(0, heightmap.height as i32 - 2) as usize;
    let tx = image_x - x0 as f32;
    let tz = image_z - z0 as f32;
    let top_row = heightmap.height - 1 - z0;
    let bottom_row = heightmap.height - 2 - z0;
    let a = heightmap.pixels[top_row * heightmap.width + x0] as f32;
    let b = heightmap.pixels[top_row * heightmap.width + x0 + 1] as f32;
    let c = heightmap.pixels[bottom_row * heightmap.width + x0] as f32;
    let d = heightmap.pixels[bottom_row * heightmap.width + x0 + 1] as f32;
    let top = (b - a) * tx + a;
    let bottom = (d - c) * tx + c;
    Ok(((bottom - top) * tz + top) * 0.1)
}

fn xmap12_mesh_settings(binding: &BuildingPdxmeshBinding) -> Vec<Xmap12MeshSettingOverride> {
    binding
        .mesh_settings
        .iter()
        .filter_map(|row| {
            row.index.map(|shape_index| Xmap12MeshSettingOverride {
                shape_index,
                texture_diffuse: row.texture_diffuse.clone(),
                texture_normal: row.texture_normal.clone(),
                texture_specular: row.texture_specular.clone(),
                shader: row.shader.clone(),
            })
        })
        .collect()
}

fn building_status_for_placement<'a>(
    states: &'a BTreeMap<u32, StateBuildings>,
    placement: &MapBuildingPlacement,
    province_id: u32,
) -> Result<&'a BuildingStatus> {
    let state = states
        .get(&placement.state_id)
        .ok_or_else(|| format!("XMAP12 D48 has no building state {}", placement.state_id))?;
    if state.status != ViewStatus::Present {
        return Err(format!("XMAP12 D48 state {} is not present", placement.state_id).into());
    }
    if let Some(status) = state
        .province_buildings
        .get(&province_id)
        .and_then(|rows| rows.get(&placement.building))
    {
        return Ok(status);
    }
    state
        .state_buildings
        .get(&placement.building)
        .ok_or_else(|| {
            format!(
                "XMAP12 D48 state/province has no {} status for placement",
                placement.building
            )
            .into()
        })
}

fn measured_f64(value: &Leaf<f64>, label: &str) -> Result<f64> {
    match value {
        Leaf::Measured(value) if value.is_finite() && *value >= 0.0 => Ok(*value),
        Leaf::Measured(_) => Err(format!("XMAP12 {label} is invalid").into()),
        Leaf::Absent => Err(format!("XMAP12 {label} is absent").into()),
        Leaf::Unobtained => Err(format!("XMAP12 {label} is unobtained").into()),
    }
}

fn optional_measured_bool(value: &Leaf<bool>) -> Option<bool> {
    match value {
        Leaf::Measured(value) => Some(*value),
        Leaf::Absent | Leaf::Unobtained => None,
    }
}

fn max_building_levels_from_admitted_sources(
    resolver: &GameFileResolver,
    sources: &[UsedContentKey],
) -> Result<u32> {
    let mut values = Vec::new();
    for source in sources {
        let text = strip_comments(&latin1_bytes_to_string(
            &resolver.read_admitted_used_content(source)?,
        ));
        if let Some(value) = parse_u32_value(&text, "MAX_BUILDING_LEVELS") {
            values.push((value, source.normalized_game_relative_path.clone()));
        }
    }
    let Some((value, _)) = values.last() else {
        return Err(
            "XMAP12 admitted effective defines have no NBuildings.MAX_BUILDING_LEVELS".into(),
        );
    };
    if *value == 0 {
        return Err("XMAP12 MAX_BUILDING_LEVELS is zero".into());
    }
    Ok(*value)
}

fn merged_show_on_map_buildings(
    resolver: &GameFileResolver,
    sources: &[UsedContentKey],
) -> Result<Vec<ShowOnMapBuilding>> {
    let mut by_name = BTreeMap::new();
    for source in sources {
        let text = latin1_bytes_to_string(&resolver.read_admitted_used_content(source)?);
        for row in show_on_map_buildings_from_source(&text)? {
            if let Some(previous) = by_name.insert(row.name.clone(), row.clone()) {
                if previous != row {
                    return Err(format!(
                        "XMAP12 effective common/buildings repeats {} incompatibly",
                        row.name
                    )
                    .into());
                }
            }
        }
    }
    Ok(by_name.into_values().collect())
}

fn unique_semantic_entity(
    token: &str,
    mut hits: Vec<(BuildingEntityBinding, UsedContentKey)>,
) -> Result<(BuildingEntityBinding, UsedContentKey)> {
    let Some(first) = hits.pop() else {
        return Err(format!("XMAP12 entity {token:?} is absent from admitted assets").into());
    };
    for hit in &hits {
        if !first.0.semantically_eq(&hit.0) {
            return Err(
                format!("XMAP12 entity {token:?} has conflicting admitted definitions").into(),
            );
        }
    }
    Ok(first)
}

fn unique_semantic_pdxmesh(
    token: &str,
    mut hits: Vec<(BuildingPdxmeshBinding, UsedContentKey)>,
) -> Result<(BuildingPdxmeshBinding, UsedContentKey)> {
    let Some(first) = hits.pop() else {
        return Err(format!("XMAP12 PdxMesh {token:?} is absent from admitted gfx").into());
    };
    for hit in &hits {
        if !first.0.semantically_eq(&hit.0) {
            return Err(
                format!("XMAP12 PdxMesh {token:?} has conflicting admitted definitions").into(),
            );
        }
    }
    Ok(first)
}

fn admitted_material_texture_sources(
    admission: &ValidatedContentAdmission,
    mesh_source: &UsedContentKey,
    model: &crate::PdxModel,
    pdxmesh: &BuildingPdxmeshBinding,
) -> Result<Vec<Xmap12TextureSource>> {
    let mesh_dir = mesh_source
        .normalized_game_relative_path
        .rsplit_once('/')
        .map(|(dir, _)| dir)
        .unwrap_or("");
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for mesh in model.shapes.iter().flat_map(|shape| &shape.meshes) {
        for (slot, authored) in [
            ("diffuse", mesh.material.diffuse.as_deref()),
            ("normal", mesh.material.normal_map.as_deref()),
            ("specular", mesh.material.specular.as_deref()),
        ] {
            let Some(authored) = authored else {
                continue;
            };
            let authored = authored.replace('\\', "/");
            let mut candidates = Vec::new();
            if !mesh_dir.is_empty() && !authored.to_ascii_lowercase().starts_with("gfx/") {
                candidates.push(format!("{mesh_dir}/{authored}"));
            }
            candidates.push(authored.clone());
            let authored_base = authored
                .rsplit('/')
                .next()
                .unwrap_or(&authored)
                .to_ascii_lowercase();
            let exact = candidates
                .iter()
                .find_map(|path| admission.used_content(path));
            let content = if let Some(exact) = exact {
                exact.clone()
            } else {
                let matches = admission
                    .required_paths()
                    .filter(|path| {
                        path.rsplit('/').next().map(str::to_ascii_lowercase)
                            == Some(authored_base.clone())
                    })
                    .collect::<Vec<_>>();
                if matches.len() != 1 {
                    return Err(format!(
                        "XMAP12 texture {authored:?} resolves to {} admitted paths",
                        matches.len()
                    )
                    .into());
                }
                admission
                    .require_used_content(matches[0])
                    .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?
                    .clone()
            };
            let identity = (slot.to_string(), content.clone());
            if seen.insert(identity.clone()) {
                out.push(Xmap12TextureSource {
                    slot: identity.0,
                    content: identity.1,
                });
            }
        }
    }
    for override_row in &pdxmesh.mesh_settings {
        for (slot, authored) in [
            ("diffuse", override_row.texture_diffuse.as_deref()),
            ("normal", override_row.texture_normal.as_deref()),
            ("specular", override_row.texture_specular.as_deref()),
        ] {
            let Some(authored) = authored else {
                continue;
            };
            let authored = authored.replace('\\', "/");
            let mut candidates = Vec::new();
            if !mesh_dir.is_empty() && !authored.to_ascii_lowercase().starts_with("gfx/") {
                candidates.push(format!("{mesh_dir}/{authored}"));
            }
            candidates.push(authored.clone());
            let authored_base = authored
                .rsplit('/')
                .next()
                .unwrap_or(&authored)
                .to_ascii_lowercase();
            let content = if let Some(exact) = candidates
                .iter()
                .find_map(|path| admission.used_content(path))
            {
                exact.clone()
            } else {
                let matches = admission
                    .required_paths()
                    .filter(|path| {
                        path.rsplit('/').next().map(str::to_ascii_lowercase)
                            == Some(authored_base.clone())
                    })
                    .collect::<Vec<_>>();
                if matches.len() != 1 {
                    return Err(format!(
                        "XMAP12 meshsettings texture {authored:?} resolves to {} admitted paths",
                        matches.len()
                    )
                    .into());
                }
                admission
                    .require_used_content(matches[0])
                    .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?
                    .clone()
            };
            let identity = (slot.to_string(), content);
            if seen.insert(identity.clone()) {
                out.push(Xmap12TextureSource {
                    slot: identity.0,
                    content: identity.1,
                });
            }
        }
    }
    Ok(out)
}

fn admitted_prefix_sources(
    admission: &ValidatedContentAdmission,
    prefix: &str,
    extension: &str,
) -> Result<Vec<UsedContentKey>> {
    let suffix = format!(".{}", extension.trim_start_matches('.'));
    let mut sources = admission
        .required_paths()
        .filter(|path| path.starts_with(prefix) && path.ends_with(&suffix))
        .map(|path| {
            admission
                .require_used_content(path)
                .map(Clone::clone)
                .map_err(|error| -> Box<dyn std::error::Error> { error.into() })
        })
        .collect::<Result<Vec<_>>>()?;
    sources.sort_by(|a, b| {
        a.normalized_game_relative_path
            .cmp(&b.normalized_game_relative_path)
    });
    if sources.is_empty() {
        return Err(format!("XMAP12 admission has no {prefix}*{suffix} sources").into());
    }
    Ok(sources)
}

fn required_content<'a>(
    admission: &'a ValidatedContentAdmission,
    path: &str,
) -> Result<&'a UsedContentKey> {
    admission
        .require_used_content(path)
        .map_err(|error| -> Box<dyn std::error::Error> { error.into() })
}

#[derive(Clone, Debug)]
struct ProvinceRaster {
    width: usize,
    height: usize,
    ids: Vec<u32>,
    land: BTreeSet<u32>,
}

impl ProvinceRaster {
    fn province_at_world(&self, x: f32, z: f32) -> Result<u32> {
        if !x.is_finite() || !z.is_finite() || self.width == 0 || self.height == 0 {
            return Err("XMAP12 province lookup input is invalid".into());
        }
        let x = (x as i64).rem_euclid(self.width as i64) as usize;
        let world_z = z as i64;
        if world_z < 0 || world_z >= self.height as i64 {
            return Err("XMAP12 province lookup z is outside the map".into());
        }
        let row = self.height - 1 - world_z as usize;
        let province = self.ids[row * self.width + x];
        if province == 0 || !self.land.contains(&province) {
            return Err("XMAP12 object placement is not in a land province".into());
        }
        Ok(province)
    }

    fn province_at_lattice_any(&self, x: i32, z: i32) -> Option<u32> {
        if self.width == 0 || self.height == 0 || z < 0 || z >= self.height as i32 {
            return None;
        }
        let x = x.rem_euclid(self.width as i32) as usize;
        let row = self.height - 1 - z as usize;
        self.ids.get(row * self.width + x).copied()
    }
}

fn admitted_province_raster(
    resolver: &GameFileResolver,
    admission: &ValidatedContentAdmission,
) -> Result<ProvinceRaster> {
    let definition = required_content(admission, XMAP12_DEFINITION_SOURCE)?;
    let provinces = required_content(admission, XMAP12_PROVINCES_SOURCE)?;
    let (colors, land) =
        parse_definition_csv_bytes(&resolver.read_admitted_used_content(definition)?)?;
    decode_province_bmp(
        &resolver.read_admitted_used_content(provinces)?,
        &colors,
        land,
    )
}

fn parse_definition_csv_bytes(bytes: &[u8]) -> Result<(BTreeMap<[u8; 3], u32>, BTreeSet<u32>)> {
    let mut colors = BTreeMap::new();
    let mut land = BTreeSet::new();
    for (index, raw) in latin1_bytes_to_string(bytes).lines().enumerate() {
        let raw = raw.trim();
        if raw.is_empty() {
            continue;
        }
        let fields = raw.split(';').collect::<Vec<_>>();
        if fields.len() < 8 {
            return Err(format!("XMAP12 definition.csv line {} is truncated", index + 1).into());
        }
        let id = fields[0].parse::<u32>()?;
        let rgb = [
            fields[1].parse::<u8>()?,
            fields[2].parse::<u8>()?,
            fields[3].parse::<u8>()?,
        ];
        if colors.insert(rgb, id).is_some() {
            return Err("XMAP12 definition.csv repeats an RGB identity".into());
        }
        if fields[4] == "land" {
            land.insert(id);
        }
    }
    Ok((colors, land))
}

fn decode_province_bmp(
    bytes: &[u8],
    colors: &BTreeMap<[u8; 3], u32>,
    land: BTreeSet<u32>,
) -> Result<ProvinceRaster> {
    if bytes.get(..2) != Some(b"BM") || bytes.len() < 54 {
        return Err("XMAP12 provinces.bmp is not a complete BMP".into());
    }
    let offset = u32::from_le_bytes(bytes[10..14].try_into()?) as usize;
    let width_raw = i32::from_le_bytes(bytes[18..22].try_into()?);
    let height_raw = i32::from_le_bytes(bytes[22..26].try_into()?);
    let bpp = u16::from_le_bytes(bytes[28..30].try_into()?);
    let compression = u32::from_le_bytes(bytes[30..34].try_into()?);
    if width_raw <= 0 || height_raw == 0 || bpp != 24 || compression != 0 {
        return Err("XMAP12 provinces.bmp format is not uncompressed RGB24".into());
    }
    let width = width_raw as usize;
    let height = height_raw.unsigned_abs() as usize;
    let stride = (width * 3 + 3) & !3;
    if bytes.len() < offset + stride * height {
        return Err("XMAP12 provinces.bmp payload is truncated".into());
    }
    let bottom_up = height_raw > 0;
    let mut ids = vec![0u32; width * height];
    for y in 0..height {
        let source_y = if bottom_up { height - 1 - y } else { y };
        for x in 0..width {
            let at = offset + source_y * stride + x * 3;
            let rgb = [bytes[at + 2], bytes[at + 1], bytes[at]];
            ids[y * width + x] = *colors
                .get(&rgb)
                .ok_or_else(|| format!("XMAP12 provinces.bmp contains undefined RGB {rgb:?}"))?;
        }
    }
    Ok(ProvinceRaster {
        width,
        height,
        ids,
        land,
    })
}

fn y_rotation_translation_matrix(yaw: f32, position: [f32; 3], scale: f32) -> Result<[u32; 16]> {
    if !yaw.is_finite()
        || !scale.is_finite()
        || scale <= 0.0
        || position.iter().any(|value| !value.is_finite())
    {
        return Err("XMAP12 object transform input is invalid".into());
    }
    let (sin, cos) = yaw.sin_cos();
    Ok([
        cos * scale,
        0.0,
        sin * scale,
        0.0,
        0.0,
        scale,
        0.0,
        0.0,
        -sin * scale,
        0.0,
        cos * scale,
        0.0,
        position[0],
        position[1],
        position[2],
        1.0,
    ]
    .map(f32::to_bits))
}

pub fn xmap12_unjoined_placement_tokens(
    placements: &[MapBuildingPlacement],
    known_building_tokens: &BTreeMap<String, usize>,
) -> BTreeMap<String, usize> {
    let mut unjoined = BTreeMap::new();
    for placement in placements {
        if !known_building_tokens.contains_key(placement.building.as_str()) {
            *unjoined.entry(placement.building.clone()).or_default() += 1;
        }
    }
    unjoined
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn o05_slot_visibility_uses_define_denominator_and_strict_fraction() {
        assert!(xmap12_building_slot_visible(0, 6, 1.0, 15).unwrap());
        assert!(!xmap12_building_slot_visible(1, 6, 1.0, 15).unwrap());
        assert!(xmap12_building_slot_visible(5, 6, 15.0, 15).unwrap());
    }

    #[test]
    #[cfg_attr(
        not(gate_hoi4_install),
        ignore = "the game install: set BATTLE_PLAN_GAME_ROOT (or BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA) to it, or install it at the default Steam path"
    )]
    fn gate_vanilla_shared_spawn_slots_defeat_the_building_placement_join() {
        // PINS A PRE-EXISTING DEFECT, measured against the pinned install: map/buildings.txt
        // column 2 is a SPAWN-POINT name, not always a common/buildings token. 17,095 of 66,664
        // vanilla placements (25.6%) name a slot that several building types share via
        // `spawn_point = <name>`; symmetrically, the 15 show_on_map buildings that never appear in
        // map/buildings.txt are exactly those declaring a spawn_point. a729 0x141663e70 indexes
        // the entity table by that spawn-point handle (type_obj+0x58).
        //
        // prepare_xmap12_buildings joins placement.building straight against show_on_map and
        // errors per placement, so the XMAP12 building population cannot be built for vanilla
        // content at all - with or without a recorded Xmap12BuildingVisualFrame. Closing it needs
        // the shared-slot occupant law plus separate laws for landmark_spawn and floating_harbor;
        // this test must then be inverted to assert an EMPTY unjoined set.
        //
        // kills: "fixing" the join by inventing a show_on_map row for a spawn slot, which would
        // name an occupant Replay has no recorded basis to choose; and any silent narrowing of the
        // slot set that would hide part of the gap.
        let game_dir = replay_test_gates::require_vanilla_game_root();
        let resolver = crate::GameFileResolver::vanilla(game_dir).expect("vanilla resolver");
        let placements =
            crate::map_building_placements_from_resolver(&resolver).expect("map/buildings.txt");
        let show_on_map =
            crate::show_on_map_buildings_from_resolver(&resolver).expect("common/buildings");
        let known = show_on_map
            .iter()
            .enumerate()
            .map(|(index, row)| (row.name.clone(), index))
            .collect::<BTreeMap<_, _>>();
        let unjoined = xmap12_unjoined_placement_tokens(&placements, &known);
        assert_eq!(
            unjoined.keys().map(String::as_str).collect::<Vec<_>>(),
            vec![
                "dam_spawn",
                "floating_harbor",
                "landmark_spawn",
                "locks_spawn",
                "naval_base_spawn",
                "nuclear_reactor_spawn",
                "rocket_site_spawn",
                "special_project_facility_spawn",
            ],
            "the unjoined placement-token set drifted"
        );
        assert!(
            unjoined.values().sum::<usize>() * 4 > placements.len(),
            "the unjoined share collapsed; re-measure before trusting the census"
        );
    }

    #[test]
    fn o05_variant_is_floor_then_clamped_to_last_authored_entity() {
        assert_eq!(xmap12_building_entity_variant(4, 0.0, 15).unwrap(), 0);
        assert_eq!(xmap12_building_entity_variant(4, 7.5, 15).unwrap(), 2);
        assert_eq!(xmap12_building_entity_variant(4, 99.0, 15).unwrap(), 3);
    }

    #[test]
    fn o04_city_entity_hash_matches_exact_wrapping_vectors() {
        assert_eq!(city_entity_hash(0), 2_903_825_947);
        assert_eq!(city_entity_hash(1), 2_831_080_502);
        assert_eq!(city_entity_hash(2), 2_513_096_664);
        assert_eq!(city_entity_hash(u32::MAX), 1_569_505_716);
        assert_eq!(city_entity_hash(0x1234_5678), 1_833_627_211);
    }

    #[test]
    fn o04_and_o05_y_rotation_is_row_vector_and_keeps_translation() {
        let matrix =
            y_rotation_translation_matrix(std::f32::consts::FRAC_PI_2, [10.0, 20.0, 30.0], 1.0)
                .unwrap()
                .map(f32::from_bits);
        assert!((matrix[2] - 1.0).abs() < 1e-6);
        assert!((matrix[8] + 1.0).abs() < 1e-6);
        assert_eq!(&matrix[12..15], &[10.0, 20.0, 30.0]);
    }

    #[test]
    fn fixed_max_visible_uses_exact_counter_and_active_law() {
        assert!(!xmap12_fixed_spawn_refused(false, 0, Some(1)));
        assert!(xmap12_fixed_spawn_refused(false, 2, Some(1)));
        assert!(xmap12_fixed_spawn_refused(true, 0, Some(1)));
        assert!(!xmap12_fixed_spawn_refused(false, u32::MAX, Some(0)));

        let mut counter = 0;
        let mut active = false;
        xmap12_fixed_apply_active_transition(&mut counter, &mut active, true);
        assert_eq!((counter, active), (0, true));
        xmap12_fixed_apply_active_transition(&mut counter, &mut active, false);
        assert_eq!((counter, active), (1, false));
        xmap12_fixed_apply_active_transition(&mut counter, &mut active, true);
        assert_eq!((counter, active), (0, true));
    }

    #[test]
    fn fixed_runtime_identity_is_digest_bound_and_replay_scoped() {
        let mut runtime = Xmap12FixedRuntimeState {
            replay_frame_identity_sha256: "a".repeat(64),
            elapsed_renderer_seconds_f32_bits: 0.0f32.to_bits(),
            map_object_presentation_enabled: true,
            enabled_dlc_names: Vec::new(),
            ambient_types: Vec::new(),
            deterministic_sha256: String::new(),
        };
        runtime.deterministic_sha256 = Sha256::hex(&serde_json::to_vec(&runtime).unwrap());
        runtime
            .validate_for_replay(&"a".repeat(64))
            .expect("current replay frame");
        assert!(runtime.validate_for_replay(&"b".repeat(64)).is_err());
        runtime.replay_frame_identity_sha256 = "c".repeat(64);
        assert!(runtime.validate().is_err());
    }

    #[test]
    fn fixed_parser_preserves_type_and_object_source_order_and_defaults() {
        let rows = parse_fixed_types(
            r#"
            type = {
                type = "first_entity"
                object = { name = "b" position = { 1 2 3 } rotation = { 4 5 6 } }
                object = { name = "a" position = { 7 8 9 } rotation = { 10 11 12 } }
            }
            type = {
                type = "second_entity"
                scale = 2
                always_visible = yes
                max_visible = 3
                object = { name = "c" position = { 0 0 0 } rotation = { 0 90 0 } }
            }
            "#,
        )
        .unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].entity_token, "first_entity");
        assert_eq!(rows[0].animation_name, "idle");
        assert!(rows[0].use_animation);
        assert_eq!(
            rows[0]
                .objects
                .iter()
                .map(|row| row.name.as_str())
                .collect::<Vec<_>>(),
            ["b", "a"]
        );
        assert_eq!(rows[1].entity_token, "second_entity");
        assert_eq!(rows[1].scale, 2.0);
        assert!(rows[1].always_visible);
        assert_eq!(rows[1].max_visible, Some(3));
    }
}
