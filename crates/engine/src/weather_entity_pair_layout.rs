//! Native primary/fading entity pair in a weather presentation producer.
//!
//! Harvested from `extractor/native/lanes/production_render_submit_live/src/a729_hook.rs` and its submodules at
//! `archive/pre-rewrite-2026-09-09`. The original C++ class name is unproven.
//! Presentation knowledge only; this module does not enable a recorded table or hook.

/// Weather entity-pair stride; a729_hook.rs:10729 indexes it.
pub const STRIDE: usize = 0x10;
/// Primary entity pointer; a729_hook.rs:10730 reads it.
pub const PRIMARY: usize = 0x00;
/// Fading entity pointer; a729_hook.rs:10733 reads it.
pub const FADING: usize = 0x08;
