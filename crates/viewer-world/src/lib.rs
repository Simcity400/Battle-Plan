#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: engine_heap::Heap = engine_heap::Heap;


pub mod mapframe;
pub mod movement_arrows;
pub mod scene;
pub mod scene_content;
#[cfg(any(test, feature = "test-support"))]
pub mod scene_faults;

pub mod map_assets;
