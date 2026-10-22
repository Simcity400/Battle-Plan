//! The dashboard's text interpreter and the content facts it reads: recorded
//! triggers and scopes, scripted localisation, effect descriptions, modifier
//! text, the game's name rules, the content catalogue and the recording
//! metadata helpers products and map tooltips share. Nothing here names a
//! product or a source; the `dashboard` loads the
//! catalogue for each source and re-exports every module at its old path.
//!
//! The `test-support` feature compiles the fixtures and test instrumentation the
//! tests of crates above this one use.

/// Tests run on the allocator the viewer ships with.
#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

pub mod air_wing_names;
pub mod catalogue;
pub mod character_scope;
pub mod cryptology;
pub mod division_history;
pub mod effect_description;
pub mod faction_names;
pub mod flag_choices;
pub mod idea_names;
pub mod identity;
pub mod metadata;
pub mod modifier_text;
pub mod naval_battle_name;
pub mod party_names;
pub mod scope;
pub mod scripted_text;
pub mod state_resources;
pub mod template_facts;
pub mod table_access;
pub mod subunit_files;
pub mod subunit_requirements;
pub mod triggers;
pub mod unit_names;
pub mod variant_names;

pub use dashboard_core::invalid;
#[cfg(test)]
pub(crate) use dashboard_core::test_fixtures;
pub(crate) use dashboard_core::{dto::*, map_settings, transport};

pub mod stability;
