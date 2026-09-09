//! Skinning owner reached from a mesh entity at +0x70.
//!
//! Harvested from `extractor/native/lanes/production_render_submit_live/src/a729_hook.rs` and its submodules at
//! `archive/pre-rewrite-2026-09-09`. The original C++ class name is unproven.
//! Presentation knowledge only; this module does not enable a recorded table or hook.

/// Skinning vector-count pointer, dereferenced as u32; a729_hook.rs:10491-10495.
pub const VECTOR_COUNT_POINTER: usize = 0x08;
/// Skinning palette byte pointer; a729_hook.rs:10491 follows it.
pub const PALETTE_DATA: usize = 0x18;
/// Skinning count is multiplied by 16 bytes in a729_hook.rs:10496; old capture accepted only 3,200-byte palettes.
pub const VECTOR_STRIDE: usize = 0x10;
