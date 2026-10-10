//! CGraphicalObject base presentation layout, inherited by C2dPositionObject.
//! Lifetime and ancestry remain unproven. The container adds a truncated
//! half-extent before position subtracts a floating half-extent, preserving
//! half-pixel results for odd dimensions. Traversal can propagate an empty
//! clip; drawing later rejects it.

/// CGraphicalObject vtable.
pub const VTABLE_RVA: usize = 0x02b2_d360;
/// CGraphicalObject constructor.
pub const CONSTRUCTOR_RVA: usize = 0x0229_9200;
/// Graphical-object embedded colour.
pub const COLOR: usize = 0x20;
/// CGraphicalObject local matrix, sixteen f32.
pub const LOCAL_MATRIX: usize = 0x50;
/// Local translation X, f32, adjusted using stored width.
pub const LOCAL_TRANSLATION_X: usize = 0x80;
/// CGraphicalObject local translation Y f32.
pub const LOCAL_TRANSLATION_Y: usize = 0x84;
/// CGraphicalObject local translation Z f32.
pub const LOCAL_TRANSLATION_Z: usize = 0x88;
/// CGraphicalObject world matrix, sixteen f32.
pub const WORLD_MATRIX: usize = 0x90;
/// World translation X after parent composition, f32.
pub const WORLD_TRANSLATION_X: usize = 0xc0;
/// World translation Y after parent composition, f32.
pub const WORLD_TRANSLATION_Y: usize = 0xc4;
/// CGraphicalObject child-pointer array.
pub const CHILD_DATA: usize = 0xd0;
/// CGraphicalObject child-array capacity i32.
pub const CHILD_CAPACITY: usize = 0xd8;
/// CGraphicalObject child-array count i32.
pub const CHILD_COUNT: usize = 0xdc;
/// CGraphicalObject child allocator pointer.
pub const CHILD_ALLOCATOR: usize = 0xe0;
/// CGraphicalObject parent pointer.
pub const PARENT: usize = 0x118;
/// CGraphicalObject local scale f32.
pub const LOCAL_SCALE: usize = 0x124;
/// Requested X, f32, converted from the integer setter argument.
pub const REQUESTED_X: usize = 0x128;
/// Requested Y, f32, converted from the integer setter argument.
pub const REQUESTED_Y: usize = 0x12c;
/// Stored width read as u32; sprites use physical and containers logical dimensions.
pub const STORED_WIDTH: usize = 0x130;
/// Stored height read as u32; its representation differs from the integer size result.
pub const STORED_HEIGHT: usize = 0x134;
/// Child-traversal gate byte, checked before descending.
pub const CHILD_TRAVERSAL_GATE: usize = 0x138;
/// Scale-origin branch byte; stronger semantics remain unproven.
pub const SCALE_ORIGIN_BRANCH: usize = 0x144;
/// CGraphicalObject draw/input participation gate byte.
pub const PARTICIPATION_GATE: usize = 0x154;
/// Clip-enabled byte set after coordinates convert to physical pixels.
pub const CLIP_ENABLED: usize = 0x155;
/// CGraphicalObject physical clip left i32.
pub const CLIP_LEFT: usize = 0x158;
/// CGraphicalObject physical clip top i32.
pub const CLIP_TOP: usize = 0x15c;
/// CGraphicalObject physical clip right i32.
pub const CLIP_RIGHT: usize = 0x160;
/// CGraphicalObject physical clip bottom i32.
pub const CLIP_BOTTOM: usize = 0x164;
/// CGraphicalObject clip mode i32.
pub const CLIP_MODE: usize = 0x168;
/// Integer-size getter dispatch displacement; returns a signed i32 pair.
pub const INTEGER_SIZE_GETTER_VSLOT: usize = 0x158;
/// Position setter resets the local matrix while preserving separate
/// integer and f32 conversion steps.
pub const SET_POSITION_RVA: usize = 0x022e_8ee0;
/// CGraphicalObject scale setter.
pub const SET_SCALE_RVA: usize = 0x022e_9050;
/// CGraphicalObject clip setter.
pub const SET_CLIP_RVA: usize = 0x022e_9060;
/// CGraphicalObject parent assignment.
pub const SET_PARENT_RVA: usize = 0x0229_95a0;
/// Parent-transform refresh reads the current parent before composing
/// the world matrix.
pub const REFRESH_TRANSFORM_RVA: usize = 0x0229_9e30;
/// CGraphicalObject transform composition.
pub const COMPOSE_TRANSFORM_RVA: usize = 0x0229_9e60;
/// CGraphicalObject traversal.
pub const TRAVERSE_RVA: usize = 0x0229_98c0;
