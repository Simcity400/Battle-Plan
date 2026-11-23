
use engine_deployment::{carmy_upgrades_requests, cupgrade_delivery, manpower_value_entry};
#[cfg(test)]
use engine_native::cequipment_variant;
use engine_native::{
    Memory,
    cair_wing_pool,
    carmy,
    cbuilding,
    cbuilding_status,
    cbuilding_template,
    ccharacter,
    ccountry,
    cequipment_variant_pool,
    cgame_state,
    cideology_group,
    cpdx_array,
    cpdx_lexer,
    cproduction_status,
    cprovince,
    crailway_gun,
    creferenced_division_template,
    crelation,
    crelation_status,
    cstate,
    cunit,
    cwar_relation,
    equipment_entry,
    memory,
    msvc_string,
    read,
    read_slice,
    walk,
};
use engine_native_politics::{cdiplomacy_status, cpolitical_party, cpolitical_status};
use engine_native_air::{cair_wing, cstrategic_air};
use engine_native_intelligence::{coperation, coperation_instance};
use engine_peace::peace_conference;
use engine_production::{
    industrial_organisation_line_entry, nindustrial_organisation_corganisation,
};

pub mod building_damage_sites;
pub mod ccountry_experience_status;
pub mod cevent_scope;
pub mod cincoming_diplomatic_action;
pub mod cnuke;
pub mod diplomacy_mutations;
pub mod diplomatic_action_body;
pub mod equipment_destruction;
pub mod event_feed_sites;
pub mod experience_gain_sites;
pub mod incoming_diplomatic_action_queue;
pub mod map_event_sites;
pub mod nuclear_strike_sites;
pub mod operation_outcome_sites;
pub mod politics_sites;
pub mod script_event_registry;
pub mod script_event_sites;
