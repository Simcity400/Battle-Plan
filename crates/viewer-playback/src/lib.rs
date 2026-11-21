#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: engine_heap::Heap = engine_heap::Heap;

pub mod comparison;
pub mod cursors;
pub mod presentation;
pub mod document;
pub mod engaged_index;
pub mod highlight;
pub mod lane_rows;
pub mod layers;
pub mod map_assets;
pub mod map_binding;
pub mod map_icons;
pub mod map_source;
pub mod map_views;
pub mod perf_log;
pub mod playback;
pub mod playhead_host;
pub mod render;
pub mod visual_fallback;
pub mod worker_pool;
