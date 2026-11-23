use super::province_tooltips;
use content_reader_script::sub_units::SubUnits;
use content_reader_division_names::division_names::Database;
use content_reader_localisation::localisation::Localisation;
use std::{collections::BTreeMap, sync::Arc};
use viewer_foundation::{
    building_facts::BuildingFacts,
    movement_timing::{EtaCache, MovementContent},
    tooltip_text::TextIcons,
};

#[derive(Clone)]
pub struct Content {
    pub anchors: Arc<BTreeMap<(u32, u32), [f32; 3]>>,
    pub default_anchors: Arc<BTreeMap<u32, [f32; 3]>>,
    pub terrain: Arc<viewer_gpu::terrain_content::TerrainContent>,
    pub region_of: Arc<BTreeMap<u32, u32>>,
    pub kinds: Arc<BTreeMap<u32, content_reader_map::map::ProvinceKind>>,
    pub ability_sprites: Arc<content_reader_abilities::abilities::AbilitySprites>,
    pub fog: Arc<Result<viewer_foundation::visibility::FogContent, String>>,
    pub aggression: Arc<Vec<i64>>,
    pub naval_counts: viewer_unit_reads::naval_count_approx::Defines,
    pub port_mission_defines: viewer_unit_reads::port_mission_defines::Defines,
    pub supply_grace: Result<i32, String>,
    pub comms_max_distance: Result<i32, String>,
    pub dig_in_factor: Result<i64, String>,
    pub attrition: Result<Arc<viewer_unit_reads::counter_attrition::Content>, String>,
    pub ability_comms_scaling: Result<Arc<Vec<i64>>, String>,
    pub ability_no_hq_scaling: Result<i64, String>,
    pub province_hover: Arc<province_tooltips::Content>,
    pub equipment: Arc<content_reader_key_names::equipment::Database>,
    pub strategic_regions: Arc<BTreeMap<u32, content_reader_regions::regions::StrategicRegion>>,
    pub localisation: Arc<Localisation>,
    pub recorded_language: String,
    pub scripted: Arc<Result<dashboard_text::scripted_text::ScriptedContent, String>>,
    pub state_name_keys: Arc<BTreeMap<u32, Result<String, String>>>,
    pub text_icons: Arc<dyn TextIcons + Send + Sync>,
    pub division_names: Arc<Database>,
    pub sub_units: Result<Arc<SubUnits>, String>,
    pub movement_terrain: Result<Arc<content_reader_key_names::terrain::TerrainDefinitions>, String>,
    pub building_facts: Arc<BuildingFacts>,
    pub air_base_first_provinces: Arc<viewer_foundation::building_facts::FirstProvinces>,
    pub movement: Option<Arc<MovementContent>>,
    pub eta_cache: Arc<EtaCache>,
}

impl Content {
    pub(crate) fn ability_definition(
        &self,
        token: &[u8],
    ) -> Result<&content_reader_abilities::abilities::Definition, String> {
        self.ability_sprites.definition(token).ok_or_else(|| {
            format!(
                "ability {} is absent from the recorded content stack",
                String::from_utf8_lossy(token)
            )
        })
    }

    pub(crate) fn province_is_land(&self, province: u32) -> Option<bool> {
        self.kinds
            .get(&province)
            .map(|kind| *kind == content_reader_map::map::ProvinceKind::Land)
    }
}
