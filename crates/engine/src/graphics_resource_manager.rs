//! Texture-resource manager reached from the bitmap-font graphics context.
//! It owns the entry array and separate path and resource-ID indexes.

/// Texture-resource entry array pointer.
pub const ENTRIES: usize = 8;
/// Texture-resource manager free-list head/resource index, u32.
pub const FREE_LIST_HEAD: usize = 0x20;
/// Texture-resource manager embedded path index.
pub const PATH_INDEX: usize = 0x28;
/// Texture-resource manager embedded resource-ID index.
pub const RESOURCE_ID_INDEX: usize = 0x48;
/// Texture-resource path registration.
pub const REGISTER_PATH_RVA: usize = 0x023f_17e0;
/// Resource-registration entry bytes.
pub const REGISTER_PATH_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x44, 0x88, 0x4c, 0x24, 0x20, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41,
];
/// Resource release; removes the name after the last owning reference.
pub const RELEASE_PATH_RVA: usize = 0x023f_4160;
