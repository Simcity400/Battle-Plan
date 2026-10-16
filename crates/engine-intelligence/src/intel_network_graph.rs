//! Graph object referenced by CLocalIntelNetwork's graph pointer; its C++ type name is unknown.
//! Offsets describe game memory, separate from replay rows.

/// Begin pointer for graph rows.
pub const ROWS_BEGIN: usize = 0x10;
/// End pointer for graph rows.
pub const ROWS_END: usize = 0x18;
/// Vertex-array data pointer.
pub const VERTICES_DATA: usize = 0x38;
/// Signed i32 vertex count.
pub const VERTICES_COUNT: usize = 0x44;
/// Graph-row copy helper.
pub const COPY_ROWS_RVA: usize = 0x11b_2420;
/// Graph allocation size.
pub const SIZE: usize = 0x50;
