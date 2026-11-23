#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: engine_heap::Heap = engine_heap::Heap;

pub mod cursors;
pub mod document;
pub mod engaged_index;
pub mod highlight;
pub mod layer_sources;
pub mod limitations;
pub mod map_binding;
pub mod map_icons;
pub mod map_open;
pub mod map_source;
pub mod map_views;
pub mod overlay;
pub mod perf_log;
pub mod playback;
pub mod presentation;
pub mod proofs;
pub mod render;
pub mod side;
pub mod visual_fallback;
pub mod worker_pool;

#[cfg(test)]
mod p6_tests;
