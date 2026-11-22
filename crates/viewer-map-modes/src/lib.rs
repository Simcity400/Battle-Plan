#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: engine_heap::Heap = engine_heap::Heap;


pub mod air_assignment;
pub mod map_mode;
pub mod naval_dominance;
pub mod naval_selection;
pub mod occupation_stripes;
pub mod political;
pub mod political_gpu;
