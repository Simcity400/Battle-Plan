//! engine-deployment ownership; see README.md.

use engine_native::{
    Memory, carmy, carmy_manpower_values, ccountry, cequipment_type, cequipment_variant,
    cequipment_variant_pool, cname_group_member, cpdx_array, creferenced_division_template,
    csub_unit_definition, equipment_entry, memory, msvc_string, read, read_slice, walk,
};
#[cfg(test)]
use engine_native::{cgame_state, cunit};

pub mod army_upgrade_request_row;
pub mod carmy_upgrades_requests;
pub mod carmy_upgrades_status;
pub mod cdeployment_status;
pub mod cmilitary_deployment;
pub mod cmilitary_deployment_conveyor;
pub mod cmilitary_deployment_line;
pub mod cupgrade_delivery;
pub mod manpower_value_entry;
pub mod unit_bonuses;
