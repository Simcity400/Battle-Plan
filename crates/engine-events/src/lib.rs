
use engine_deployment::{carmy_upgrades_requests, cupgrade_delivery, manpower_value_entry};
use engine_native::{
    cprovince::{buildings as cbuilding_status, self},
    cequipment_variant::pool as cequipment_variant_pool,
    crelation::{self, status as crelation_status},
};
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
