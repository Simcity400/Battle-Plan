#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: engine_heap::Heap = engine_heap::Heap;

pub mod air_grid;
pub mod air_missions;
pub mod air_volunteers;
pub mod base_windows;
pub mod bases;
pub mod naval;
pub mod naval_hovers;
pub mod naval_markers;
pub mod naval_members;
pub mod naval_mine_badges;
pub mod naval_mission_inputs;
pub mod naval_missions;
pub mod naval_port_hovers;
pub mod naval_regional;
pub mod naval_repairs;
pub mod naval_windows;
