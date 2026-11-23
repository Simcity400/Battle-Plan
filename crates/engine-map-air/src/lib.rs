
use engine_native::{
    cprovince::{buildings as cbuilding_status, self},
    ccountry_supply_system::{self, world as csupply_system},
    cweather_manager::{self, region_weather as sweather_per_region},
};

pub mod foreign_homebase_node;
pub mod map_air_base_reference;
pub mod map_air_carrier_operands;
pub mod map_air_country_region_modifier;
pub mod map_air_rocket_slots;
pub mod map_air_sortie_inputs;
pub mod map_air_supply;
pub mod radar_group;
