#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: engine_heap::Heap = engine_heap::Heap;

pub mod anchored;
pub mod counter_hovers;
pub mod frame_head;
pub mod free_icons;
pub mod gui_resources;
pub mod icon_windows;
pub mod lazy_icons;
pub mod native_effects;
pub mod native_flags;
pub mod native_gui;
pub mod native_resources;
pub mod native_shader;
pub mod native_text;
pub mod native_windows;
pub mod overlay_count;
pub mod sprites;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
pub mod text;
pub mod tooltip_painter;
