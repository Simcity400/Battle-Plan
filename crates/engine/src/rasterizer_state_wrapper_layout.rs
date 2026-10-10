//! Presentation rasterizer-state wrapper. The API object pointer is at
//! the allocation start; the retained descriptor follows at byte eight.

/// Wrapper allocation size.
pub const ALLOCATION_BYTES: usize = 0x28;
/// API object output pointer.
pub const API_OBJECT: usize = 0x00;
/// Retained descriptor after the API pointer.
pub const DESCRIPTION: usize = 0x08;
/// Wrapper allocator and initializer.
pub const CREATE_RVA: usize = 0x023f_d2f0;
/// Wrapper binder dereferencing the API object.
pub const BIND_RVA: usize = 0x0240_0330;
