
use engine_native::{
    Memory, air_fuel_defines, air_mission, allocation, cace, cair_base, cair_wing, cair_wing_pool,
    carmy, carmy_combat_inputs, carmy_group, ccountry, ccountry_air_container, cfleet, cgame_state,
    corders_group, cpdx_array, cprovince, creference_registry, cship, cstate, cstrategic_air,
    cstrategic_air_manager, cstrategic_region, csub_unit_definition, cunit, cweather_manager,
    game_score_inputs, memory, modifier_accumulator, msvc_string, read, read_slice, walk,
};

pub mod ccountry_fuel_status;
pub mod fuel_awards;
pub mod fuel_consumer_data;
pub mod fuel_demand;
pub mod fuel_history_entry;
