#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: engine_heap::Heap = engine_heap::Heap;


pub mod animated_map_text;
pub mod name_pass;
pub mod name_shader;
pub mod names;
