//! Unit-counter grid reached from the stack; exact C++ class unproven.

/// Unit-counter grid's graphical container pointer; old production_render_submit_live/src/a729_hook/a25_gui_bridge.rs:2003 reads +140.
pub const CONTAINER: usize = 0x140;
/// Unit-counter grid's CUnitCounterItem pointer-vector data; old a25_gui_bridge.rs:2006 reads +1A0 and walks pointers.
pub const OWNER_DATA: usize = 0x1a0;
/// Unit-counter grid's i32 owner-vector count; old a25_gui_bridge.rs:2009 reads +1AC.
pub const OWNER_COUNT: usize = 0x1ac;
