//! Graphical GUI render-node receiver; exact C++ type and subobject base unproven.
//! The old bridge recorded candidate clip rectangles without proving their meaning.

/// Graphical GUI node render virtual slot; old production_render_submit_live/src/a729_hook/a25_gui_bridge.rs:1979 reads vtable+50.
pub const RENDER_VSLOT: usize = 0x50;
/// Graphical GUI node child pointer-vector data; old a25_gui_bridge.rs:3153 traverses +D0 while building the ancestry path.
pub const CHILD_DATA: usize = 0xd0;
/// Graphical GUI node child i32 count; old a25_gui_bridge.rs:3153 traverses +DC with the child pointer vector.
pub const CHILD_COUNT: usize = 0xdc;
/// Graphical GUI node parent pointer; old a25_gui_bridge.rs:2045,2088 follows +118 while authenticating owner paths.
pub const PARENT: usize = 0x118;
/// Graphical GUI node runtime-widget pointer; old a25_gui_bridge.rs:2116,2245 reads +148 for authored identity.
pub const RUNTIME_WIDGET: usize = 0x148;
/// Graphical GUI node candidate clip i32[4]; old a25_gui_bridge.rs:1421,1605 reads +158 without proving a scissor law.
pub const CLIP_CANDIDATE: usize = 0x158;
/// Graphical GUI node candidate rectangle first i32; old a25_gui_bridge.rs:3288 reads +210 without proving placement semantics.
pub const RECT_CANDIDATE_0: usize = 0x210;
/// Graphical GUI node candidate rectangle second i32; old a25_gui_bridge.rs:3289 reads +214.
pub const RECT_CANDIDATE_1: usize = 0x214;
/// Graphical GUI node candidate rectangle third i32; old a25_gui_bridge.rs:3290 reads +220.
pub const RECT_CANDIDATE_2: usize = 0x220;
/// Graphical GUI node candidate rectangle fourth i32; old a25_gui_bridge.rs:3291 reads +224.
pub const RECT_CANDIDATE_3: usize = 0x224;
