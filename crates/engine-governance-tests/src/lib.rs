//! Shared governance fixtures for research and focus reader tests.
#![cfg(feature = "test-support")]

extern crate self as engine;

use engine_executable::modifier_keys;
use engine_focus::{
    ccontinuous_national_focus, cnational_focus, cnational_focus_progress, cnational_focus_tree,
};
use engine_native::{
    Memory, batch_fallback, ccountry, cgame_state, memory, modifier_definition, walk,
};
use engine_research::{
    climited_use_tech_bonus, cresearch_slot, ctechnology, ctechnology_status, ctechnology_template,
};

// Collector and this fixture package compile the same source at its existing include path.
#[path = "../../engine/src/governance_batch/test_kit.rs"]
mod kit;
pub use kit::*;
