//! Registration definition passed to particle runtime allocation.
//!
//! Harvested from `extractor/native/lanes/production_render_submit_live/src/a729_hook.rs` and its submodules at
//! `archive/pre-rewrite-2026-09-09`. The original C++ class name is unproven.
//! Presentation knowledge only; this module does not enable a recorded table or hook.

/// Registration token, fixed 64-byte NUL-terminated string; a729_hook.rs:11395.
pub const TOKEN: usize = 0x00;
/// Definition source token, fixed 64-byte NUL-terminated string; a729_hook.rs:11396.
pub const SOURCE_TOKEN: usize = 0x40;
/// Each inline token occupies 64 bytes; read_definition at a729_hook.rs:11395-11396.
pub const TOKEN_CAPACITY: usize = 0x40;
/// Registration hash u32 checked against FNV-1a of the token; a729_hook.rs:11397.
pub const TOKEN_HASH: usize = 0x80;
