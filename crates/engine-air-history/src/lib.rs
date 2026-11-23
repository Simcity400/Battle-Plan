
#[cfg(test)]
use engine_native::cland_combat;
use engine_native::{
    Memory,
    allocation,
    cair_wing_pool,
    ccountry,
    cfleet,
    cgame_state,
    cnaval_combat,
    cpdx_array,
    cprovince,
    cstrategic_region,
    csub_unit_definition,
    ctask_force,
    cunit,
    memory,
    read,
    walk,
};
use engine_native_politics::cdiplomacy_status;
use engine_native_air::{cair_wing, cstrategic_air, cstrategic_air_manager};

pub mod air_history_mutations;
pub mod air_kamikaze;
pub mod air_loss_sites;
pub mod air_participant_vectors;
pub mod air_record_mutations;
pub mod cair_region_combat_data;
pub mod cloop_history;
pub mod loop_history_record_fields;
pub mod loop_history_window;
pub mod menu_history;
pub mod sair_wing_combat_data;
pub mod sdamage_entry;
