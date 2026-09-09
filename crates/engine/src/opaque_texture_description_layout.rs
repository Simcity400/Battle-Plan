//! Description passed to FUN1420fda10 before opaque target allocation.
//!
//! Harvested from `extractor/native/lanes/production_render_submit_live/src/a729_hook.rs` and its submodules at
//! `archive/pre-rewrite-2026-09-09`. The original C++ class name is unproven.
//! Presentation knowledge only; this module does not enable a recorded table or hook.

/// Pixel format u32; a729_hook.rs:8147 snapshots it before allocation.
pub const PIXEL_FORMAT: usize = 0x08;
/// Creation flags u32; a729_hook.rs:8148 reads it and tests bit 1 for a render target.
pub const FLAGS: usize = 0x0c;
/// Requested width u32; a729_hook.rs:8145 compares it with the created texture.
pub const WIDTH: usize = 0x10;
/// Requested height u32; a729_hook.rs:8146 compares it with the created texture.
pub const HEIGHT: usize = 0x14;
/// Created wrapper pointer; a729_hook.rs:8164 reads it after allocation.
pub const WRAPPER: usize = 0x70;
