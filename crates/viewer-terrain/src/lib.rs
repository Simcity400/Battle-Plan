#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: engine_heap::Heap = engine_heap::Heap;


pub mod river_shader;
pub mod rivers;
pub mod sky_shader;
pub mod terrain;
mod terrain_coast;
pub mod terrain_shader;
pub mod tree_order;
pub mod tree_receiver;
pub mod tree_shader;
pub mod trees;
pub mod water;
pub mod water_shader;
pub mod weather;
