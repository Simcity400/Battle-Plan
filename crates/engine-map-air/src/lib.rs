
#[cfg(test)]
use engine_native::reference_bucket;
use engine_native::{
    Memory,
    batch_fallback,
    cair_wing_pool,
    cbuilding,
    cbuilding_status,
    cbuilding_template,
    ccountry,
    ccountry_supply_system,
    cfleet,
    cgame_state,
    cnavy_leader,
    cpdx_array,
    cprovince,
    cradars_pool,
    creference_registry,
    cship,
    cstate,
    cstrategic_region,
    csub_unit_definition,
    csupply_system,
    ctask_force,
    cunit,
    cunit_leader,
    cweather_manager,
    memory,
    modifier_accumulator,
    modifier_definition,
    msvc_string,
    read,
    read_slice,
    sweather_per_region,
    walk,
};
use engine_native_air::{
    air_mission,
    cair_base,
    ccountry_air_container,
    cstrategic_air,
    cstrategic_air_manager,
};

pub mod foreign_homebase_node;
pub mod map_air_base_reference;
pub mod map_air_carrier_operands;
pub mod map_air_country_region_modifier;
pub mod map_air_rocket_slots;
pub mod map_air_sortie_inputs;
pub mod map_air_supply;
pub mod radar_group;
