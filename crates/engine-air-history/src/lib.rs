//! Native air history readers and their operands, layouts and synthetic tests.

use engine_native::{
    Memory, allocation, cair_wing, cair_wing_pool, ccountry, cdiplomacy_status, cfleet,
    cgame_state, cnaval_combat, cpdx_array, cprovince, cstrategic_air, cstrategic_air_manager,
    cstrategic_region, csub_unit_definition, ctask_force, cunit, memory, read, walk,
};
#[cfg(test)]
use engine_native::{cland_combat, read_census};

pub mod air_history_mutations;
pub mod air_kamikaze;
pub mod air_loss_sites;
pub mod air_participant_vectors;
pub mod air_record_mutations;
pub mod cair_region_combat_data;
pub mod cloop_history;
pub mod loop_history_record_fields;
pub mod loop_history_window;
pub mod sair_wing_combat_data;
pub mod sdamage_entry;
