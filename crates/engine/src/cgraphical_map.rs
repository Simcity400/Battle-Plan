//! CGraphicalMap, named by UpdateProvinces TBB task RTTI in func_140b4c120.decompile.c:464.
//!
//! Harvested from `extractor/native/lanes/production_render_submit_live/src/a729_hook.rs` and its submodules at
//! `archive/pre-rewrite-2026-09-09`.
//! Presentation knowledge only; this module does not enable a recorded table or hook.

/// CGraphicalMap render-system pointer; a729_hook.rs:8712 reads it in the map-scene snapshot.
pub const RENDER_SYSTEM: usize = 0x18;
/// CGraphicalMap tree/map-render owner pointer; a729_hook.rs:8713 reads it.
pub const TREE_OWNER: usize = 0x58;
/// CGraphicalMap shadow owner pointer; a729_hook.rs:8714 reads it.
pub const SHADOW_OWNER: usize = 0x60;
/// CGraphicalMap state-graphics pointer array; func_140b43b60.decompile.c:114,132 follows it.
pub const STATE_GRAPHICS: usize = 0xb0;
/// State-graphics pointer capacity, native 32 bits (signed comparisons); func_140b43b60.decompile.c:94-105.
pub const STATE_GRAPHICS_CAPACITY: usize = 0xb8;
/// State-graphics pointer count, native 32 bits (signed comparisons); func_140b43b60.decompile.c:93,110-113.
pub const STATE_GRAPHICS_COUNT: usize = 0xbc;
/// State-graphics array allocator pointer; func_140b43b60.decompile.c:99-102 uses allocate/free virtual slots.
pub const STATE_GRAPHICS_ALLOCATOR: usize = 0xc0;
/// CGraphicalMap builds state graphics and writes province graphics +0x1508; func_140b43b60.decompile.c:124-149.
pub const BUILD_STATE_GRAPHICS_RVA: usize = 0x00b4_3b60;
/// CGraphicalMap update FUN140b49340 calls the map+0x58 owner updater; engine_evidence/src/xmap12_ambient_lifecycle.rs:149.
pub const UPDATE_RVA: usize = 0x00b4_9340;
/// CGraphicalMap::UpdateProvinces, confirmed by engine_evidence/artifacts/func_140b4c120.decompile.c:464 RTTI.
pub const UPDATE_PROVINCES_RVA: usize = 0x00b4_c120;
/// UpdateProvinces entry bytes checked by a729_hook/o05_building_capture.rs:20.
pub const UPDATE_PROVINCES_SENTINEL: [u8; 16] = [
    0x48, 0x8b, 0xc4, 0x48, 0x89, 0x58, 0x18, 0x48, 0x89, 0x50, 0x10, 0x55, 0x56, 0x57, 0x41, 0x54,
];
/// Map-scene render root FUN140b481a0; lib.rs:90 and hook_map_scene_render_root receive the graphical-map pointer.
pub const MAP_SCENE_RENDER_ROOT_RVA: u32 = 0x00B4_81A0;
/// Map-scene root entry bytes checked by a729_hook.rs:303.
pub const MAP_SCENE_RENDER_ROOT_SENTINEL: [u8; 16] = [
    0x48, 0x8b, 0xc4, 0x44, 0x88, 0x48, 0x20, 0x48, 0x89, 0x50, 0x10, 0x48, 0x89, 0x48, 0x08, 0x55,
];
/// CGraphicalMap arrow-manager pointer; engine_evidence/artifacts/func_140b481a0.decompile.c:163-168 passes it to the shadow-arrow thunk.
pub const ARROW_MANAGER: usize = 0x6d0;
/// CGraphicalMap supply-flow object pointer checked before drawing; func_140b481a0.decompile.c:194-196 obtains the draw receiver separately.
pub const SUPPLY_FLOW_OBJECT: usize = 0x730;
/// CGraphicalMap first native 32-bit map-text layer selector; func_140b48e50.decompile.c:46-47 reads it before the second selector.
pub const MAP_TEXT_LAYER_FIRST: usize = 0x6a0;
/// CGraphicalMap second native 32-bit map-text layer selector; func_140b48e50.decompile.c:48-49 reads it after the first selector.
pub const MAP_TEXT_LAYER_SECOND: usize = 0x6a4;
/// CGraphicalMap scene-owner pointer; func_140b481a0.decompile.c:400 follows its +0xb8 field for the object-list debug draw.
pub const SCENE_OWNER: usize = 0x10;
/// CGraphicalMap map-text draw; func_140b481a0.decompile.c:390-391 passes the map receiver unchanged to FUN140b48e50.
pub const DRAW_MAP_TEXT_RVA: usize = 0x00b4_8e50;
