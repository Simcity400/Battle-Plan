#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: engine_heap::Heap = engine_heap::Heap;

pub mod atlas_pages;
pub mod camera;
pub mod frame;
pub mod gpu_contract;
pub mod gpu_frame_time;
pub mod gpu_passes;
pub mod gradient_math;
pub mod icon_tracks;
pub mod layers;
pub mod pass_key;
pub mod province_admission;
pub mod province_geometry;
pub mod province_map;
pub mod province_pick;
pub mod province_schedule;
pub mod ribbon_content;
pub mod ribbon_fixed;
pub mod ribbon_geometry;
pub mod ribbon_mesh;
pub mod ribbon_native;
pub mod stack_anchors;
pub mod terrain_content;
pub mod terrain_sight;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
pub mod textures;
pub mod ui_scale;
pub mod unit_pose;
