//! Native packet copied from the particle builder's normal and ribbon arrays.
//!
//! Harvested from `extractor/native/lanes/production_render_submit_live/src/a729_hook.rs` and its submodules at
//! `archive/pre-rewrite-2026-09-09`. The original C++ class name is unproven.
//! Presentation knowledge only; this module does not enable a recorded table or hook.

/// Particle world matrix, 16 f32 words; a729_hook.rs:11355 decodes the leading 64 bytes.
pub const WORLD_TRANSFORM: usize = 0x00;
/// Live particle count u32; a729_hook.rs:11340 reads the packet field.
pub const LIVE_COUNT: usize = 0x40;
/// GPU instance data pointer; a729_hook.rs:11341 reads it.
pub const RECORDS: usize = 0x48;
/// GPU instance stride u32; a729_hook.rs:11342 reads and checks it.
pub const RECORD_STRIDE: usize = 0x50;
/// Particle capacity u32; a729_hook.rs:11343 reads it.
pub const CAPACITY: usize = 0x54;
/// Elapsed time f32; a729_hook.rs:11387 retains its native bits.
pub const ELAPSED: usize = 0x58;
/// Packet flags u16; a729_hook.rs:11389 decodes them.
pub const FLAGS: usize = 0x5c;
/// Subsystem selector u32; a729_hook.rs:11359 uses it to select shader and atlas rows.
pub const SUBSYSTEM_SELECTOR: usize = 0x60;
/// Native particle packet size; lib.rs:104 and a729_hook.rs:11338 copy exactly 0x70 bytes.
pub const PACKET_BYTES: usize = 0x70;
/// Native GPU instance row stride; lib.rs:105 and a729_hook.rs:11344 check 0x30 bytes.
pub const GPU_INSTANCE_STRIDE: u32 = 0x30;
