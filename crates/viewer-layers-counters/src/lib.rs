#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: engine_heap::Heap = engine_heap::Heap;

pub mod air_mission_counters;
pub mod base_counters;
pub mod counter_kinds;
pub mod counters;
pub mod gun_counters;
pub mod land_markers;
pub mod naval_counters;
pub mod naval_markers;
pub mod relations;
