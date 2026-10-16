//! CGraphicalMap presentation layout; these constants enable no table or hook.

/// CGraphicalMap render-system pointer.
pub const RENDER_SYSTEM: usize = 0x18;
/// CGraphicalMap province-indexed CProvinceGraphics pointer array (with its count).
pub const PROVINCE_GRAPHICS: usize = 0x68;
/// CGraphicalMap tree/map-render owner pointer.
pub const TREE_OWNER: usize = 0x58;
/// CGraphicalMap shadow owner pointer.
pub const SHADOW_OWNER: usize = 0x60;
/// CGraphicalMap state-graphics pointer array.
pub const STATE_GRAPHICS: usize = 0xb0;
/// State-graphics pointer capacity, native 32 bits (signed comparisons).
pub const STATE_GRAPHICS_CAPACITY: usize = 0xb8;
/// State-graphics pointer count, native 32 bits (signed comparisons).
pub const STATE_GRAPHICS_COUNT: usize = 0xbc;
/// State-graphics array allocator pointer.
pub const STATE_GRAPHICS_ALLOCATOR: usize = 0xc0;
/// CGraphicalMap builds state graphics and writes province graphics.
pub const BUILD_STATE_GRAPHICS_RVA: usize = 0x00b4_3b60;
/// Graphical-map update calls the owner updater.
pub const UPDATE_RVA: usize = 0x00b4_9340;
/// Graphical-map province updater.
pub const UPDATE_PROVINCES_RVA: usize = 0x00b4_c120;
/// Exact province-updater entry bytes.
pub const UPDATE_PROVINCES_SENTINEL: [u8; 16] = [
    0x48, 0x8b, 0xc4, 0x48, 0x89, 0x58, 0x18, 0x48, 0x89, 0x50, 0x10, 0x55, 0x56, 0x57, 0x41, 0x54,
];
/// Map-scene render root receiving the graphical-map pointer.
pub const MAP_SCENE_RENDER_ROOT_RVA: u32 = 0x00B4_81A0;
/// Exact map-scene render-root entry bytes.
pub const MAP_SCENE_RENDER_ROOT_SENTINEL: [u8; 16] = [
    0x48, 0x8b, 0xc4, 0x44, 0x88, 0x48, 0x20, 0x48, 0x89, 0x50, 0x10, 0x48, 0x89, 0x48, 0x08, 0x55,
];
/// CGraphicalMap arrow-manager pointer.
pub const ARROW_MANAGER: usize = 0x6d0;
/// Province-visibility owner, a 0x40-byte object holding the current byte buffer.
pub const PROVINCE_VISIBILITY: usize = 0x6f0;
/// CGraphicalMap supply-flow object pointer checked before drawing.
pub const SUPPLY_FLOW_OBJECT: usize = 0x730;
/// CGraphicalMap first native 32-bit map-text layer selector.
pub const MAP_TEXT_LAYER_FIRST: usize = 0x6a0;
/// CGraphicalMap second native 32-bit map-text layer selector.
pub const MAP_TEXT_LAYER_SECOND: usize = 0x6a4;
/// CGraphicalMap scene-owner pointer.
pub const SCENE_OWNER: usize = 0x10;
/// CGraphicalMap map-text draw.
pub const DRAW_MAP_TEXT_RVA: usize = 0x00b4_8e50;
