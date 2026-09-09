//! Particle runtime type descriptor with matched shader and atlas arrays.
//!
//! Harvested from `extractor/native/lanes/production_render_submit_live/src/a729_hook.rs` and its submodules at
//! `archive/pre-rewrite-2026-09-09`. The original C++ class name is unproven.
//! Presentation knowledge only; this module does not enable a recorded table or hook.

/// Program-index array pointer; a729_hook.rs:11249 reads it.
pub const PROGRAM_INDICES: usize = 0x48;
/// Program-index count u32; a729_hook.rs:11250 reads it.
pub const PROGRAM_COUNT: usize = 0x54;
/// Atlas-row array pointer; a729_hook.rs:11251 reads it.
pub const ATLAS_ROWS: usize = 0x60;
/// Atlas-row count u32; a729_hook.rs:11252 requires it to match program count.
pub const ATLAS_COUNT: usize = 0x6c;
