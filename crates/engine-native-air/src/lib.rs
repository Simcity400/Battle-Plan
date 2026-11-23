#[cfg(test)]
#[global_allocator]
static ALLOCATOR: engine_allocation::test_support::DeniedAllocator =
    engine_allocation::test_support::DeniedAllocator;

pub mod air_fuel_inputs;
pub mod air_mission;
pub mod air_power_menu;
pub mod cace;
pub mod cair_base;
pub mod cair_regional_mission;
pub mod cair_wing;
pub mod ccountry_air_container;
pub mod ccountry_raid_status;
pub mod command_inputs;
pub mod craid_instance;
pub mod craid_source;
pub mod craid_system;
pub mod craid_target;
pub mod cstrategic_air;
pub mod cstrategic_air_manager;
pub mod game_score_inputs;
pub mod raid_database;
pub mod raid_equipment;
pub mod raid_launch_inputs;
pub mod raid_location_scope_layout;
pub mod raid_speed_inputs;
pub mod raid_tooltip_operands;
pub mod transport_spotting_graph;
