use super::{Assets, modifier_text::NameIndexCache};
use content_reader_map::map::MapData;
use content_reader_localisation::localisation::Localisation;
use content_reader_regions::regions::StrategicRegion;
use std::{collections::BTreeMap, sync::Arc};
use viewer_engagements::land_combat_math::ScoreDefines;
use viewer_native_gui::{lazy_icons::{LazyFlags, LazyTextures}, native_windows::WindowCache};
use viewer_runtime::memo::Keyed;
use viewer_foundation::{
    perspective::{Vision, VisionKey},
    visibility::FogContent,
    movement_timing::{EtaCache, MovementContent},
    trigger_content::TriggerContent,
};
use viewer_gpu::{province_geometry::ProvinceGeometry, terrain_content::TerrainContent};

pub struct Content {
    pub air_context: Option<Arc<viewer_counter_products::map::counter_content::Content>>,
    pub attrition: std::sync::OnceLock<
        Result<viewer_unit_reads::counter_attrition::PanelContent, String>,
    >,
    pub medals: std::sync::OnceLock<Result<content_reader_military::unit_medals::Medals, String>>,
    pub assets: Arc<Assets>,
    pub localisation: Arc<Localisation>,
    pub recorded_language: String,
    pub recorded_labels: Arc<super::naval_labels::RecordedLabels>,
    pub scripted: Arc<Result<dashboard_text::scripted_text::ScriptedContent, String>>,
    pub map: Arc<MapData>,
    pub strategic_regions: Arc<BTreeMap<u32, StrategicRegion>>,
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
    pub occupation: Arc<Result<Occupation, String>>,
}

pub struct Occupation {
    pub laws: content_reader_politics::occupation_laws::Laws,
    pub modifiers: content_reader_politics::occupation_modifiers::Definitions,
}

impl Occupation {
    pub fn load(layers: &content_reader_layers::ResolvedLayers) -> Result<Self, String> {
        Ok(Self {
            laws: content_reader_politics::occupation_laws::read(layers).map_err(|why| why.to_string())?,
            modifiers: content_reader_politics::occupation_modifiers::read(layers).map_err(|why| why.to_string())?,
        })
    }
}
