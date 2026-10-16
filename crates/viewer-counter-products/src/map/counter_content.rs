//! Immutable content for unit/province hover and selected-order queries.
use super::province_tooltips;
use installed_content::{division_names::Database, localisation::Localisation, unit_leader::SubUnits};
use std::{collections::BTreeMap, sync::Arc};
use viewer_foundation::{
    map::building_facts::BuildingFacts,
    movement_timing::{EtaCache, MovementContent},
    tooltip_text::TextIcons,
};

#[derive(Clone)]
pub struct Content {
    pub anchors: Arc<BTreeMap<(u32, u32), [f32; 3]>>,
    pub default_anchors: Arc<BTreeMap<u32, [f32; 3]>>,
    pub terrain: Arc<viewer_foundation::map::terrain_content::TerrainContent>,
    pub region_of: Arc<BTreeMap<u32, u32>>,
    pub kinds: Arc<BTreeMap<u32, installed_content::map::ProvinceKind>>,
    pub ability_sprites: Arc<installed_content::abilities::AbilitySprites>,
    pub fog: Arc<Result<viewer_foundation::map::visibility::FogContent, String>>,
    pub aggression: Arc<Vec<i64>>,
    pub naval_counts: viewer_foundation::map::naval_count_approx::Defines,
    pub port_mission_defines: viewer_foundation::map::port_mission_defines::Defines,
    pub supply_grace: Result<i32, String>,
    pub comms_max_distance: Result<i32, String>,
    pub dig_in_factor: Result<i64, String>,
    pub attrition: Result<Arc<viewer_foundation::map::counter_attrition::Content>, String>,
    pub ability_comms_scaling: Result<Arc<Vec<i64>>, String>,
    pub ability_no_hq_scaling: Result<i64, String>,
    pub province_hover: Arc<province_tooltips::Content>,
    pub equipment: Arc<installed_content::equipment::Database>,
    pub strategic_regions: Arc<BTreeMap<u32, installed_content::regions::StrategicRegion>>,
    pub localisation: Arc<Localisation>,
    pub recorded_language: String,
    pub scripted: Arc<Result<dashboard_text::scripted_text::ScriptedContent, String>>,
    pub state_name_keys: Arc<BTreeMap<u32, Result<String, String>>>,
    pub text_icons: Arc<dyn TextIcons + Send + Sync>,
    pub division_names: Arc<Database>,
    pub sub_units: Result<Arc<SubUnits>, String>,
    pub movement_terrain: Result<Arc<installed_content::terrain::TerrainDefinitions>, String>,
    pub building_facts: Arc<BuildingFacts>,
    pub movement: Option<Arc<MovementContent>>,
    pub eta_cache: Arc<EtaCache>,
}

impl Content {
    /// The same recorded-stack ability definition that selected this grid's sprite.
    pub(crate) fn ability_definition(
        &self,
        token: &[u8],
    ) -> Result<&installed_content::abilities::Definition, String> {
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
            .map(|kind| *kind == installed_content::map::ProvinceKind::Land)
    }
}
