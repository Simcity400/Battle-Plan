//! Resolved scripted GUI property-binding record.
//! No hourly-state lifetime is established.

/// Property-binding record stride.
pub const STRIDE: usize = 0x88;
/// Binding target pointer.
pub const TARGET: usize = 0;
/// Authored element-name string.
pub const AUTHORED_ELEMENT: usize = 0x08;
/// Authored property-name string.
pub const AUTHORED_PROPERTY: usize = 0x28;
/// Resolved element-name string.
pub const RESOLVED_ELEMENT: usize = 0x48;
/// Resolved property-name string.
pub const RESOLVED_PROPERTY: usize = 0x68;
