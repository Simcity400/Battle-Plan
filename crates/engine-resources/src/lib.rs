
#[cfg(test)]
use engine_native::crelation_status;
use engine_native::{
    Memory,
    allocation,
    batch_fallback,
    ccountry,
    cequipment_variant,
    cequipment_variant_pool,
    cgame_state,
    cpdx_array,
    cstate,
    cstate_template,
    equipment_entry,
    memory,
    modifier_accumulator,
    read,
    read_slice,
    walk,
};
use engine_native_naval::{cstrategic_navy, cstrategic_navy_manager};

pub mod ccountry_resources;
pub mod clend_lease_exchange;
pub mod cresource_exchange;
pub mod cresource_origin;
pub mod cstrategic_resource;
pub mod cstrategic_resource_database;
pub mod cstrategic_resource_pool;
pub mod resource_map;
pub mod state_resource_entry;
pub mod trade_opinion;
