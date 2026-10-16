//! engine-scripts ownership; see README.md.

#[cfg(test)]
use engine_native::read_census;
use engine_native::{
    Memory, ccharacter, ccharacter_manager, ccountry, cgame_state, cpdx_array, creference_registry,
    cstate, memory, read, read_slice, walk,
};

pub mod cflag_manager;
pub mod cscript_flag;
pub mod cvariables;
pub mod saved_event_targets;
pub mod variable_hash_node;
