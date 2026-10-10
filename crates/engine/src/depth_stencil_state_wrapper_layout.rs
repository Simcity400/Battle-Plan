//! Depth/stencil state wrapper, used only for presentation.
//! The API object pointer begins the allocation, followed by the retained descriptor.

/// Wrapper allocation size.
pub const ALLOCATION_BYTES: usize = 0x38;
/// API-object output pointer.
pub const API_OBJECT: usize = 0x00;
/// Retained descriptor, immediately after the API pointer.
pub const DESCRIPTION: usize = 0x08;
/// Wrapper allocation and population operation.
pub const CREATE_RVA: usize = 0x023f_cfc0;
