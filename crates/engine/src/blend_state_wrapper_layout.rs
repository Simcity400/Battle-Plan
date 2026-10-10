//! Blend-state wrapper; its concrete type is unknown.
//!
//! Presentation-only layout. Creation writes an API object pointer at the wrapper
//! start and copies the descriptor after it.

/// Wrapper allocation size.
pub const ALLOCATION_BYTES: usize = 0x30;
/// API-object pointer written by creation.
pub const API_OBJECT: usize = 0x00;
/// Copied descriptor following the API pointer.
pub const DESCRIPTION: usize = 0x08;
/// Wrapper allocation and population function.
pub const CREATE_RVA: usize = 0x023f_cb30;
/// Binding function dereferencing the wrapper's API object.
pub const BIND_RVA: usize = 0x0240_0090;
