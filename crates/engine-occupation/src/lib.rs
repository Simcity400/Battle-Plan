//! Native occupation readers and their operands, layouts and synthetic tests.

use engine_deployment::manpower_value_entry;
#[cfg(test)]
use engine_native::carmy_manpower_values;
use engine_native::{
    Memory, allocation_equipment_row, ccountry, cequipment_type, cequipment_variant_pool,
    cgame_state, cpdx_array, creferenced_division_template, cstate, memory, msvc_string, read,
    read_slice, walk,
};

pub mod ccountry_occupation_data;
pub mod ccountry_occupation_status;
pub mod coccupation_law;
pub mod cresistance;
pub mod cstate_garrison_data;
pub mod garrison_equipment_pool;
pub mod pdx_robin_hood_bucket;
pub mod pdx_robin_hood_table;
pub mod required_equipment_row;
