//! Model-geometry receiver used by bounds and collision routines.
//! The full allocation size is unknown. This layout is distinct from
//! `render_entity_layout`: its transform translation occupies the field used
//! there for a definition pointer.

/// Model transform matrix, [f32; 16].
pub const TRANSFORM: usize = 0x10;
/// Collision entry receiving the model mesh and transform.
pub const COLLISION_RVA: usize = 0x0226_27e0;
/// World-bounds entry selecting static or animated bounds.
pub const WORLD_BOUNDS_RVA: usize = 0x0226_1df0;
