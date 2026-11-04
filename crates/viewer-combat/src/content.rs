//! Immutable content operands for combat windows and marker hover text.
use super::{Assets, modifier_text::NameIndexCache};
use installed_content_core::map::MapData;
use installed_content_localisation::localisation::Localisation;
use installed_content_regions::regions::StrategicRegion;
use std::{collections::BTreeMap, sync::Arc};
use viewer_foundation::{
    land_combat_math::ScoreDefines,
    map::{
        lazy_icons::{LazyFlags, LazyTextures},
        memo::Keyed,
        native_windows::WindowCache,
        perspective::{Vision, VisionKey},
        province_geometry::ProvinceGeometry,
        terrain_content::TerrainContent,
        visibility::FogContent,
    },
    movement_timing::{EtaCache, MovementContent},
    trigger_content::TriggerContent,
};

pub struct Content {
    pub air_context: Option<Arc<viewer_counter_products::map::counter_content::Content>>,
    pub attrition: std::sync::OnceLock<
        Result<viewer_foundation::map::counter_attrition::PanelContent, String>,
    >,
    pub medals: std::sync::OnceLock<Result<installed_content_core::unit_medals::Medals, String>>,
    pub assets: Arc<Assets>,
    pub localisation: Arc<Localisation>,
    pub recorded_language: String,
    /// The naval source labels' recovery into the display language.
    pub recorded_labels: Arc<super::naval_labels::RecordedLabels>,
    pub scripted: Arc<Result<dashboard_text::scripted_text::ScriptedContent, String>>,
    pub map: Arc<MapData>,
    pub strategic_regions: Arc<BTreeMap<u32, StrategicRegion>>,
    pub battle_name_provinces: Arc<BTreeMap<u32, Vec<u32>>>,
    pub state_name_keys: Arc<BTreeMap<u32, Result<String, String>>>,
    pub province_geometry: Arc<ProvinceGeometry>,
    pub region_of: Arc<BTreeMap<u32, u32>>,
    pub terrain: Arc<TerrainContent>,
    pub scores: ScoreDefines,
    pub fog: Arc<Result<FogContent, String>>,
    pub vision_memo: Arc<Keyed<Vision, VisionKey>>,
    pub eta_cache: Arc<EtaCache>,
    pub modifier_names: Arc<NameIndexCache>,
    pub movement: Option<Arc<MovementContent>>,
    pub window_cache: Arc<WindowCache>,
    pub small_flags: Arc<LazyFlags>,
    pub large_flags: Arc<LazyFlags>,
    pub lazy: Arc<LazyTextures>,
    pub trigger_content: Arc<TriggerContent>,
}
