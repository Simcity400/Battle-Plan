//! Render-target object passed directly to the clear operation.
//!
//! Harvested from `extractor/native/lanes/production_render_submit_live/src/a729_hook.rs` and its submodules at
//! `archive/pre-rewrite-2026-09-09`. The original C++ class name is unproven.
//! Presentation knowledge only; this module does not enable a recorded table or hook.

/// Color render-target-view pointer; a729_hook.rs:8619 reads it for clear bit 0.
pub const COLOR_VIEW: usize = 0x10;
/// Depth/stencil-view pointer; a729_hook.rs:8643 reads it for clear bits 1 and 2.
pub const DEPTH_STENCIL_VIEW: usize = 0x18;
/// Target clear FUN1423fca30; lib.rs:81 and hook_render_target_clear identify this receiver.
pub const RENDER_TARGET_CLEAR_RVA: u32 = 0x023F_CA30;
/// Target clear entry bytes checked by a729_hook.rs:330.
pub const RENDER_TARGET_CLEAR_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x57, 0x48, 0x83, 0xec, 0x40, 0x49,
];
