
#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: test_allocator::Allocator = test_allocator::Allocator;
#[cfg(test)]
mod test_allocator;
#[cfg(test)]
mod script_test_game;
#[cfg(any(test, feature = "test-support"))]
pub mod test_country_links;

pub mod air_wing_names;
pub mod artwork;
pub mod catalogue;
pub mod character_scope;
pub mod cryptology;
pub mod division_history;
pub mod faction_names;
pub mod flag_choices;
pub mod idea_names;
pub mod identity;
pub mod metadata;
pub mod occupation_law;
pub mod modifier_text;
pub mod naval_battle_name;
pub mod organisation_quote;
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
