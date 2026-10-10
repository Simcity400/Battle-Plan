//! Graphical GUI render-node receiver; exact C++ type and subobject base unproven.
//! Candidate rectangles do not establish clipping or placement semantics.

/// Graphical GUI node render virtual slot.
pub const RENDER_VSLOT: usize = 0x50;
/// Graphical GUI node child pointer-vector data.
pub const CHILD_DATA: usize = 0xd0;
/// Graphical GUI node child i32 count.
pub const CHILD_COUNT: usize = 0xdc;
/// Graphical GUI node parent pointer.
pub const PARENT: usize = 0x118;
/// Graphical GUI node runtime-widget pointer.
pub const RUNTIME_WIDGET: usize = 0x148;
/// Candidate clip rectangle, four i32 values; no scissor semantics are implied.
pub const CLIP_CANDIDATE: usize = 0x158;
/// Candidate rectangle first i32; placement semantics are unspecified.
pub const RECT_CANDIDATE_0: usize = 0x210;
/// Graphical GUI node candidate rectangle second i32.
pub const RECT_CANDIDATE_1: usize = 0x214;
/// Graphical GUI node candidate rectangle third i32.
pub const RECT_CANDIDATE_2: usize = 0x220;
/// Graphical GUI node candidate rectangle fourth i32.
pub const RECT_CANDIDATE_3: usize = 0x224;
