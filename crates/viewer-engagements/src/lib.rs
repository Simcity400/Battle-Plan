#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: engine_heap::Heap = engine_heap::Heap;

pub mod combat_selection;
pub mod engaged;
pub mod engagement_history;
pub mod engagement_progress;
pub mod icon_frames;
pub mod land_combat_math;
pub mod naval_check;
pub mod naval_icon_frames;
pub mod naval_sides;
