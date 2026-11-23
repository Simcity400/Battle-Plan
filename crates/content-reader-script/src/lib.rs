#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: engine_heap::Heap = engine_heap::Heap;

pub mod color_reads;
pub mod effect_keys;
pub mod effect_names;
pub mod equipment_stats;
pub mod idea_fields;
pub mod key_names;
pub mod localization_binding;
pub mod modifiers;
pub mod scope;
pub mod scoped_variable;
pub mod scripted_keys;
pub mod scripted_trigger_grammar;
pub mod native;
pub mod sub_units;
#[cfg(any(test, feature = "test-support"))]
pub mod test_game;
pub mod trigger_grammar;
pub mod trigger_keys;
pub mod triggers;
pub mod unit_stats;
pub mod variable_builtins;
