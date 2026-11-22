#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: engine_heap::Heap = engine_heap::Heap;

pub mod allegiance;
pub mod border_power;
pub mod building_facts;
pub mod color_inheritance;
pub mod country_index;
pub mod division_panel;
pub mod fold_tables;
pub mod mode_visibility;
pub mod modifier_names;
pub mod naval_mine_feature;
pub mod movement_timing;
pub mod perspective;
pub mod tables;
pub mod templates;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
pub mod tooltip_text;
pub mod trigger_content;
pub mod unit_rules;
pub mod visibility;
