//! Eight-byte border bucket element; its concrete type is unknown.

/// Bucket-row size.
pub const SIZE: usize = 8;
/// Topology-row index, u16.
pub const TOPOLOGY_ROW: usize = 0;
/// Raw selector-reason byte, sorted unsigned.
pub const REASON: usize = 2;
/// View-mask tag, u8, selected from candidate camera tags.
pub const VIEW_TAG: usize = 3;
/// Camera fade, f32.
pub const FADE: usize = 4;
