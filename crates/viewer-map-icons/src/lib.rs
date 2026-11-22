#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: engine_heap::Heap = engine_heap::Heap;


pub mod adjacency_icons;
pub mod labels;
pub mod occupation_icons;
pub mod resource_icons;
