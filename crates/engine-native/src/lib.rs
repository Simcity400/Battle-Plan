#[cfg(test)]
#[global_allocator]
static ALLOCATOR: engine_allocation::test_support::DeniedAllocator =
    engine_allocation::test_support::DeniedAllocator;

pub mod cadvisor;
pub mod carmy;
pub mod carmy_combat_inputs;
pub mod carmy_leader;
pub mod ccharacter;
pub mod cconvoy_ship;
pub mod ccountry;
pub mod ccountry_supply_system;
pub mod cequipment_variant;
pub mod cfaction;
pub mod cfex_air;
pub mod cfex_damage_entry;
pub mod cfex_member;
pub mod cfleet;
pub mod cgame_state;
pub mod cland_combat;
pub mod cland_combatant;
pub mod cname_group_member;
pub mod cnaval_combat;
pub mod cnaval_combatant;
pub mod corder_instance;
pub mod corders_group;
pub mod corders_group_member;
pub mod cproduction_status;
pub mod cprovince;
pub mod creferenced_division_template;
pub mod crelation;
pub mod cship;
pub mod cstate;
pub mod ctask_force;
pub mod ctheatre;
pub mod cunit;
pub mod cunit_leader;
pub mod cwar_relation;
pub mod cweather_manager;
