use super::{
    super::{RaidDefines, unit_inputs::SpeedDefines},
    intel,
};
use content_reader_defines::defines::Defines;
use content_reader_states::states::StateDefinition;
use std::collections::BTreeMap;

#[derive(Debug)]
pub struct Content {
    pub intel: Result<intel::Defines, String>,
    pub speed: Result<SpeedDefines, String>,
    pub prepare_days: Result<i32, String>,
    pub max_wing_xp: Result<i64, String>,
    pub night_threshold: Result<i64, String>,
    pub thresholds: Result<[i64; 2], String>,
    pub state_names: BTreeMap<u32, Result<String, String>>,
    pub terrain_names: Result<Vec<String>, String>,
    pub unit_names: Result<content_reader_raids::raids::names::Groups, String>,
    pub projects: Result<content_reader_military::special_projects::Catalog, String>,
}
impl Default for Content {
    fn default() -> Self {
        Self {
            intel: Err("raid intel defines unavailable".into()),
            speed: Err("raid speed defines unavailable".into()),
            prepare_days: Err("BASE_DAYS_TO_PREPARE unavailable".into()),
            max_wing_xp: Err("AIR_WING_XP_LEVELS unavailable".into()),
            night_threshold: Err("DAY_NIGHT_COVERAGE_FACTOR unavailable".into()),
            thresholds: Err("success modifier thresholds unavailable".into()),
            state_names: BTreeMap::new(),
            terrain_names: Err("terrain definitions unavailable".into()),
            unit_names: Err("unit name content unavailable".into()),
            projects: Err("special project content unavailable".into()),
        }
    }
}
impl Content {
    pub fn read(
        d: &Defines,
        states: &BTreeMap<u32, StateDefinition>,
        layers: &content_reader_layers::ResolvedLayers,
    ) -> Self {
        let speed = (|| {
            let r = RaidDefines::read(d).map_err(|e| e.to_string())?;
            let [land, naval, transfer] =
                r.unit_speeds.ok_or("raid unit speed defines unavailable")?;
            Ok(SpeedDefines {
                land_speed_modifier: land,
                naval_speed_modifier: naval,
                naval_transfer_base_speed: transfer,
                raid_unit_speed_multiplier: d
                    .fixed5("NRaids", "RAID_UNIT_SPEED_MULTIPLIER")
                    .map_err(|e| e.to_string())?,
                ace_wing_size: r.ace_wing_size,
                ace_wing_size_max_bonus: r.ace_wing_size_max_bonus,
            })
        })();
        Self {
            intel: intel::Defines::read(d),
            speed,
            unit_names: content_reader_raids::raids::names::Groups::read(layers)
                .map_err(|e| e.to_string()),
            projects: content_reader_military::special_projects::Catalog::read(layers)
                .map_err(|e| e.to_string()),
            prepare_days: d
                .int32("NRaids", "BASE_DAYS_TO_PREPARE")
                .map_err(|e| e.to_string()),
            max_wing_xp: d
                .fixed5_array("NAir", "AIR_WING_XP_LEVELS")
                .map_err(|e| e.to_string())
                .and_then(|v| {
                    v.last()
                        .copied()
                        .ok_or("AIR_WING_XP_LEVELS is empty".into())
                }),
            night_threshold: d
                .fixed5("NAir", "DAY_NIGHT_COVERAGE_FACTOR")
                .map_err(|e| e.to_string()),
            thresholds: (|| {
                Ok([
                    d.fixed5("NRaids", "RAID_SUCCESS_MODIFIER_THRESHOLD_BAD")
                        .map_err(|e| e.to_string())?,
                    d.fixed5("NRaids", "RAID_SUCCESS_MODIFIER_THRESHOLD_NEUTRAL")
                        .map_err(|e| e.to_string())?,
                ])
            })(),
            state_names: states
                .iter()
                .map(|(&id, s)| (id, s.name_key.clone()))
                .collect(),
            terrain_names: content_reader_key_names::terrain::read_definitions(layers)
                .map_err(|e| e.to_string())
                .and_then(|d| {
                    d.categories
                        .into_iter()
                        .map(|c| String::from_utf8(c.name).map_err(|e| e.to_string()))
                        .collect()
                }),
        }
    }
}
