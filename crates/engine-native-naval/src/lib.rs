#[cfg(test)]
#[global_allocator]
static ALLOCATOR: engine_allocation::test_support::DeniedAllocator =
    engine_allocation::test_support::DeniedAllocator;

pub mod clast_target_info;
pub mod cnaval_base;
pub mod cstrategic_navy;
pub mod cstrategic_navy_manager;
pub mod naval_air_details;
pub mod naval_air_efficiency_inputs;
pub mod naval_combat_air_hit;
pub mod naval_combat_details;
pub mod naval_combat_hit;
pub mod naval_combat_summary_inputs;
pub mod naval_commander_inputs;
pub mod naval_management;
pub mod naval_member_details;
pub mod naval_mine_inputs;
pub mod naval_mission_inputs;
pub mod naval_patrol_inputs;
pub mod naval_ship_modifier_inputs;
pub mod naval_side_details;
