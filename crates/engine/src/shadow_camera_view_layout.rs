//! Map-scene camera pointer read by the old S1 shadow snapshot; aliases with camera root/carrier are unproven.
//!
//! Harvested from `extractor/native/lanes/production_render_submit_live/src/a729_hook.rs` and its submodules at
//! `archive/pre-rewrite-2026-09-09`. The original C++ class name is unproven.
//! Presentation knowledge only; this module does not enable a recorded table or hook.

/// Camera focus X scale f32; a729_hook.rs:8741; camera-root reader instead interprets +0x08 as owner pointer.
pub const FOCUS_X_SCALE: usize = 0x08;
/// Camera focus ratio denominator f32; a729_hook.rs:8746.
pub const FOCUS_RATIO_DENOMINATOR: usize = 0x18;
/// Camera focus Z scale f32; a729_hook.rs:8743.
pub const FOCUS_Z_SCALE: usize = 0x28;
/// Camera focus X offset f32; a729_hook.rs:8742.
pub const FOCUS_X_OFFSET: usize = 0x180;
/// Camera focus ratio numerator f32; a729_hook.rs:8745.
pub const FOCUS_RATIO_NUMERATOR: usize = 0x184;
/// Camera focus Z offset f32; a729_hook.rs:8744.
pub const FOCUS_Z_OFFSET: usize = 0x188;
/// Camera Y f32; a729_hook.rs:8740.
pub const CAMERA_Y: usize = 0x194;
/// West wrap gate byte; a729_hook.rs:8726 reads it.
pub const WRAP_WEST_ENABLED: usize = 0x670;
/// East wrap gate byte; a729_hook.rs:8729 reads it.
pub const WRAP_EAST_ENABLED: usize = 0x671;
/// Overall wrap gate byte; a729_hook.rs:8724 reads it.
pub const WRAP_ENABLED: usize = 0x6f8;
