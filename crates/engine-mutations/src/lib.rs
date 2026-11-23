
use engine_deployment::cdeployment_status;
#[cfg(test)]
use engine_events::politics_sites;
use engine_focus::cnational_focus_progress;
#[cfg(test)]
use engine_native::carmy;
use engine_native::{
    Memory,
    allocation,
    cair_wing_pool,
    ccharacter,
    ccountry,
    cequipment_production_license,
    cfaction,
    cgame_state,
    clicensed_production_status,
    cpdx_array,
    cprovince,
    creferenced_division_template,
    crelation,
    crelation_status,
    cstate,
    province_definition,
    read,
};
use engine_native_politics::{
    ccountry_characters,
    cdiplomacy_status,
    cpolitical_party,
    cpolitical_status,
};
use engine_native_air::{
    cair_base,
    ccountry_air_container,
    cstrategic_air,
    cstrategic_air_manager,
};
use engine_projects::{nproject_cprogram_status, nproject_cproject};
use engine_railways::{cprovince_railway_info, crailway_manager};
use engine_research::{
    cresearch_slot, ctechnology, ctechnology_sharing_group, ctechnology_status,
    ctechnology_template,
};
use engine_resources::{ccountry_resources, cresource_origin, resource_map};
use engine_stability::{cdynamic_modifier_container, cworld_threat};

pub mod advisor_mutations;
pub mod advisor_pool_mutations;
pub mod air_base_mutations;
pub mod air_wing_equipment_mutations;
pub mod army_stat_mutations;
pub mod building_mutations;
pub mod census_mutations;
pub mod character_role_mutations;
pub mod character_roster_mutations;
pub mod combat_engagement_mutations;
pub mod country_input_mutations;
pub mod division_panel_mutations;
pub mod doctrine_mutations;
pub mod faction_companion_mutations;
pub mod faction_goal_mutations;
pub mod local_subunit_mutations;
pub mod naval_tree_mutations;
pub mod navy_ledger_mutations;
pub mod opinion_mutations;
pub mod orders_group_mutations;
pub mod order_membership_mutations;
pub mod party_leader_mutations;
pub mod political_party_mutations;
pub mod political_status_mutations;
pub mod province_combat_mutations;
pub mod province_unit_mutations;
pub mod raid_mutations;
pub mod railway_mutations;
pub mod regional_mission_mutations;
pub mod script_flag_mutations;
pub mod script_variable_mutations;
pub mod ship_model_mutations;
pub mod template_mutations;
pub mod territory_mutations;
pub mod unit_leader_mutations;
pub mod world_input_mutations;
pub mod world_lifetime_sites;

#[cfg(test)]
use engine_native::memory;

pub mod war_participation_sites;
