//! Mesh owner reached through a render entity.
//! Draw groups, LOD thresholds, bounds and collision fields describe presentation
//! operands; these layouts do not enable a recorded table or hook.

/// Draw-group array pointer.
pub const DRAW_GROUPS: usize = 0x48;
/// Signed i32 draw-group count, selected by the outer entity row.
pub const DRAW_GROUP_COUNT: usize = 0x54;
/// Draw-group header stride.
pub const GROUP_STRIDE: usize = 0x18;
/// LOD threshold array pointer.
pub const LOD_THRESHOLDS: usize = 0x138;
/// LOD threshold signed i32 count.
pub const LOD_THRESHOLD_COUNT: usize = 0x144;
/// LOD threshold f32 stride.
pub const LOD_THRESHOLD_STRIDE: usize = 0x04;

/// Mesh local bounds as two xyz corners, [f32; 6].
pub const LOCAL_BOUNDS: usize = 0xa0;
/// Mesh collision [f32; 3] vertex-array pointer.
pub const COLLISION_VERTICES: usize = 0x108;
/// Mesh collision u16 index-array pointer.
pub const COLLISION_INDICES: usize = 0x120;
/// Mesh collision signed i32 index count.
pub const COLLISION_INDEX_COUNT: usize = 0x12c;
