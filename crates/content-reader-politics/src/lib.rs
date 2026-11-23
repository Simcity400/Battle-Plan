#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: engine_heap::Heap = engine_heap::Heap;

pub mod autonomy;
pub mod collections;
pub mod country_scorers;
pub mod decisions;
pub mod diplomacy;
pub mod dynamic_modifiers;
pub mod faction_goals;
pub mod faction_templates;
pub mod faction_upgrades;
pub mod ideas;
pub mod names;
pub mod occupation_laws;
pub mod occupation_modifiers;
pub mod opinion_modifiers;
pub mod power_balance;
pub mod script_constants;
pub mod static_modifiers;
