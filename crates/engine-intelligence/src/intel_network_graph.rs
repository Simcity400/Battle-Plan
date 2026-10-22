//! Graph object referenced by CLocalIntelNetwork's graph pointer; its C++ type name is unknown.
//! Offsets describe game memory, separate from replay rows.

/// Begin pointer for graph rows.
pub const ROWS_BEGIN: usize = 0x10;
/// End pointer for graph rows.
pub const ROWS_END: usize = 0x18;
/// Vertex-array data pointer.
pub const VERTICES_DATA: usize = 0x38;
/// Graph allocation size.
pub const SIZE: usize = 0x50;
