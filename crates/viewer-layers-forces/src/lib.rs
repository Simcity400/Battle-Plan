#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: engine_heap::Heap = engine_heap::Heap;

pub mod aircraft;
pub mod convoys;
pub mod intel_icons;
pub mod orders;
pub mod ships;
pub mod units;
