//! Virtual file returned by VFS opening.
//! Presentation metadata; no recorded table or hook is enabled.

/// Stream handle pointer after opening.
pub const STREAM_HANDLE: usize = 0x38;
/// VFS open constructor.
pub const VFS_OPEN_CONSTRUCTOR_RVA: usize = 0x024C_A2F0;
/// Exact VFS-open entry sentinel.
pub const VFS_OPEN_CONSTRUCTOR_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24, 0x20, 0x48, 0x89, 0x54, 0x24, 0x10, 0x48,
];
