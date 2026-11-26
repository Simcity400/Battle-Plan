
#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: engine_heap::Heap = engine_heap::Heap;

pub mod catalogue;
pub mod latest;
pub mod content;
pub mod facts;
pub mod presentation;
pub mod products;
pub mod queries;
pub mod ranking;
pub mod research_dto;
pub mod map_tables;
pub mod source;
pub mod span;
#[cfg(any(test, feature = "test-support"))]
pub mod test_fixtures;

pub use dashboard_core::invalid;
pub use dashboard_core::{blocks, config, dto::*, findings, map_settings, protocol, transport};
pub use dashboard_text::{air_wing_names, faction_names, idea_names, party_names, variant_names};
pub use dashboard_text::{
    character_scope, cryptology, flag_choices, identity, metadata,
    modifier_text, naval_battle_name, scope, scripted_text, state_resources, triggers, unit_names,
};

pub use research_dto::*;
pub use source::{Source, SourceKey};

pub fn gap_error(reason: &str, text: impl Into<String>) -> std::io::Error {
    if reason.starts_with("unreadable block") || reason.starts_with("invalid frame baseline at hour ") {
        invalid(reason)
    } else {
        dashboard_core::refused(text)
    }
}

#[cfg(test)]
mod endpoint_alias_tests;
