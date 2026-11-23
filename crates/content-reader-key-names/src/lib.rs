#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: engine_heap::Heap = engine_heap::Heap;

pub mod autonomy_definitions;
pub mod buildings;
pub mod characters;
pub mod country_tags;
pub mod equipment;
pub mod equipment_facts;
pub mod ideologies;
pub mod key_name_products;
pub mod modifier_definitions;
pub mod resources;
pub mod scripted_effects;
pub mod scripted_triggers;
pub mod tag_aliases;
pub mod terrain;
pub mod unit_leader;

#[cfg(test)]
mod effect_template_tests;
