#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: engine_heap::Heap = engine_heap::Heap;

pub mod labels;
pub mod left_to_layers;
pub mod map_text;
pub mod mode_fills;
pub mod names;
pub mod naval;
pub mod occupation_marks;
pub mod political;
pub mod political_lines;
pub mod resource_icons;
pub mod scripts;
