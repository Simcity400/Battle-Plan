//! Native map air readers and their operands, layouts and synthetic tests.

#[cfg(test)]
use engine_native::reference_bucket;
use engine_native::{
    Memory, air_mission, batch_fallback, cair_base, cair_wing_pool, cbuilding, cbuilding_status,
    cbuilding_template, ccountry, ccountry_air_container, ccountry_supply_system, cfleet,
    cgame_state, cnavy_leader, cpdx_array, cprovince, cradars_pool, creference_registry, cship,
    cstate, cstrategic_air, cstrategic_air_manager, cstrategic_region, csub_unit_definition,
    csupply_system, ctask_force, cunit, cunit_leader, cweather_manager, memory,
    modifier_accumulator, modifier_definition, msvc_string, read, read_slice, sweather_per_region,
    walk,
};

pub mod foreign_homebase_node;
pub mod map_air_base_reference;
pub mod map_air_carrier_operands;
pub mod map_air_country_region_modifier;
pub mod map_air_rocket_slots;
pub mod map_air_sortie_inputs;
pub mod radar_group;
