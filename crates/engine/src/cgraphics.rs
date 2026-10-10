//! CGraphics presentation layout; no live state walk is enabled here.
//! Construction fixes extents and projection; ordinary SDL resize events do
//! not rebuild them. Minimum-size products wrap as i32 before division.
//! Projection and world-matrix upload preserve separate f32 operations.

/// CGraphics vtable.
pub const VTABLE_RVA: usize = 0x02b2_6838;
/// CGraphics constructor.
pub const CONSTRUCTOR_RVA: usize = 0x0222_06c0;
/// CGraphics backend handle.
pub const BACKEND: usize = 0x80;
/// CGraphics ordinary GUI raster-state handle.
pub const GUI_RASTER_STATE: usize = 0xa8;
/// CGraphics clipped GUI raster-state handle.
pub const GUI_CLIPPED_RASTER_STATE: usize = 0xb8;
/// CGraphics embedded sprite-type registry.
pub const SPRITE_TYPE_REGISTRY: usize = 0x100;
/// CGraphics texture-manager pointer.
pub const TEXTURE_MANAGER: usize = 0x170;
/// CGraphics physical width i32.
pub const PHYSICAL_WIDTH: usize = 0x178;
/// CGraphics physical height i32.
pub const PHYSICAL_HEIGHT: usize = 0x17c;
/// CGraphics logical width i32.
pub const LOGICAL_WIDTH: usize = 0x180;
/// CGraphics logical height i32.
pub const LOGICAL_HEIGHT: usize = 0x184;
/// CGraphics projection matrix, sixteen f32.
pub const PROJECTION_MATRIX: usize = 0x190;
/// CGraphics logical/physical width ratio f32.
pub const PHYSICAL_TO_LOGICAL_X: usize = 0x1d0;
/// CGraphics logical/physical height ratio f32.
pub const PHYSICAL_TO_LOGICAL_Y: usize = 0x1d4;
/// CGraphics reciprocal X ratio f32.
pub const LOGICAL_TO_PHYSICAL_X: usize = 0x1d8;
/// CGraphics reciprocal Y ratio f32.
pub const LOGICAL_TO_PHYSICAL_Y: usize = 0x1dc;
/// CGraphics embedded root C2dPositionObject.
pub const GUI_ROOT: usize = 0x1f0;
/// Second embedded position object; distinct role remains unknown.
pub const SECOND_POSITION_ROOT: usize = 0x360;
/// CGraphics CGraphicsSettings pointer.
pub const SETTINGS: usize = 0x4e0;
/// CGraphics effective GUI scale f32.
pub const EFFECTIVE_GUI_SCALE: usize = 0x325d8;
/// CGraphics draw-time clip upper Y i32.
pub const CLIP_MAX_Y: usize = 0x32594;
/// CGraphics draw-time clip upper X i32.
pub const CLIP_MAX_X: usize = 0x32598;
/// Logical-to-physical integer coordinate conversion multiplies each
/// signed coordinate by its f32 ratio, then truncates.
pub const CONVERT_COORDINATES_RVA: usize = 0x0222_25b0;
/// GUI draw-list consumer validates clipping before drawable dispatch.
pub const DRAW_GUI_LIST_RVA: usize = 0x0222_8c20;
/// CGraphics viewport reset.
pub const RESET_VIEWPORT_RVA: usize = 0x0222_9480;
/// CGraphics sprite-instance factory.
pub const CREATE_SPRITE_RVA: usize = 0x0222_2720;
/// CGraphics sprite-type lookup wrapper.
pub const LOOKUP_SPRITE_TYPE_RVA: usize = 0x0222_3eb0;
/// CGraphics lookup-wrapper entry bytes.
pub const LOOKUP_SPRITE_TYPE_SENTINEL: [u8; 16] = [
    0x48, 0x81, 0xc1, 0x00, 0x01, 0x00, 0x00, 0xe9, 0x14, 0x92, 0x80, 0xfe, 0xcc, 0xcc, 0xcc, 0xcc,
];

/// Exact GUI draw-list consumer entry bytes.
pub const DRAW_GUI_LIST_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x54, 0x24, 0x10, 0x55, 0x56, 0x41, 0x56, 0x41, 0x57, 0x48, 0x83, 0xec, 0x58, 0x48,
];
