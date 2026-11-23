#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: engine_heap::Heap = engine_heap::Heap;

pub mod mesh;
pub mod particles;

#[cfg(test)]
mod layer_tests;
