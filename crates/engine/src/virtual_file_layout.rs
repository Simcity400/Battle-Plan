//! Virtual-file object returned by the VFS open constructor.
//!
//! Harvested from `extractor/native/lanes/production_render_submit_live/src/a729_hook.rs` and its submodules at
//! `archive/pre-rewrite-2026-09-09`. The original C++ class name is unproven.
//! Presentation knowledge only; this module does not enable a recorded table or hook.

/// Stream handle pointer; a729_hook.rs:7812 reads it after VFS open.
pub const STREAM_HANDLE: usize = 0x38;
/// VFS open constructor FUN1424ca2f0; a729_hook.rs:199 and hook_vfs_open_constructor retain the ABI.
pub const VFS_OPEN_CONSTRUCTOR_RVA: usize = 0x024C_A2F0;
/// VFS open entry bytes checked by a729_hook.rs:236.
pub const VFS_OPEN_CONSTRUCTOR_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24, 0x20, 0x48, 0x89, 0x54, 0x24, 0x10, 0x48,
];
