//! engine-resources ownership; see README.md.

#[cfg(test)]
use engine_native::crelation_status;
use engine_native::{
    Memory, batch_fallback, ccountry, cequipment_variant, cequipment_variant_pool, cgame_state,
    cpdx_array, cstate, cstate_template, equipment_entry, memory, read, read_slice, walk,
};

pub mod ccountry_resources;
pub mod clend_lease_exchange;
pub mod cresource_exchange;
pub mod cresource_origin;
pub mod cstrategic_resource;
pub mod cstrategic_resource_database;
pub mod cstrategic_resource_pool;
pub mod state_resource_entry;
