#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: engine_heap::Heap = engine_heap::Heap;

pub mod buildings;
pub mod intel;
pub mod radar;
pub mod rails;
pub mod snow;
pub mod sounds;
pub mod strikes;
pub mod trains;
pub mod weather;
