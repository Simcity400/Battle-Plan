#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: engine_heap::Heap = engine_heap::Heap;

pub mod builder_pool;
pub mod count_math;
pub mod debug_fields;
pub mod device_shared;
pub mod diagnostics_panel;
pub mod display_refresh;
pub mod errors;
pub mod frame_budget;
pub mod frame_limit;
pub mod frame_writer;
pub mod geometry;
pub mod hour_check;
pub mod install_check;
pub mod label_reads;
pub mod landed;
pub mod load_record;
pub mod memo;
pub mod msvc_stable_sort;
pub mod name_draw;
pub mod parallel;
pub mod presentation;
pub mod profile_writer;
pub mod profiling;
pub mod publication_check;
pub mod reads;
pub mod recorded_labels;
pub mod redraws;
pub mod resource_origins;
pub mod retirement;
pub mod simulation_rate;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
pub mod unit_class;
pub mod unit_handle;
pub mod view_record;
pub mod worker_priority;
