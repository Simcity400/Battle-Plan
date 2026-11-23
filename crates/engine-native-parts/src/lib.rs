#[cfg(test)]
#[global_allocator]
static ALLOCATOR: engine_allocation::test_support::DeniedAllocator =
    engine_allocation::test_support::DeniedAllocator;

pub mod acclimatization_category;
pub mod acclimatization_data;
pub mod acclimatization_entry;
pub mod air_fuel_defines;
pub mod autonomy_state_definition;
pub mod carmy_group;
pub mod carmy_manpower_values;
pub mod cbuilding;
pub mod cbuilding_template;
pub mod ccontroller_area;
pub mod ccountry_database;
pub mod cdynamic_modifier;
pub mod cequipment_module;
pub mod cequipment_type;
pub mod cequipment_upgrade;
pub mod cfex_group;
pub mod character_advisor_node;
pub mod cidea;
pub mod cideology_group;
pub mod cland_border_war_combat;
pub mod cland_border_war_combatant;
pub mod cmap;
pub mod cnaval_mission;
pub mod cnavy_leader;
pub mod cnull_combat_tactic;
pub mod crailway_gun;
pub mod crailway_gun_production_line;
pub mod creference_registry;
pub mod cship_refit_production_line;
pub mod cstate_template;
pub mod cstrategic_region;
pub mod csub_unit_definition;
pub mod cterrain_type;
pub mod cunit_supply_inputs;
pub mod land_combat_air_entry;
pub mod modifier_entry;
pub mod province_neighbour;
pub mod reference_bucket;
pub mod supply_companion;
