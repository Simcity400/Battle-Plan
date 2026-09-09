//! Entity object returned by FUN142277df0; subtype-dependent field meanings remain unresolved.
//!
//! Harvested from `extractor/native/lanes/production_render_submit_live/src/a729_hook.rs` and its submodules at
//! `archive/pre-rewrite-2026-09-09`. The original C++ class name is unproven.
//! Presentation knowledge only; this module does not enable a recorded table or hook.

/// Entity definition pointer; a729_hook.rs:7767 reads it after the named factory.
pub const DEFINITION: usize = 0x40;
/// Entity mesh-owner pointer; a729_hook.rs:10513 follows it before drawing.
pub const MESH_OWNER: usize = 0x58;
/// Entity skinning-owner pointer; a729_hook.rs:10487 follows it in the mesh path.
pub const SKINNING_OWNER: usize = 0x70;
/// Entity draw-variant u8; a729_hook.rs:10535 reads this in the mesh path; weather reads f64 at this offset.
pub const DRAW_VARIANT: usize = 0x78;
/// Entity material-binding group headers pointer; a729_hook.rs:10513 reads it.
pub const MATERIAL_BINDING_GROUPS: usize = 0xb8;
/// Weather entity playback f64; a729_hook.rs:11096 narrows to f32 in old output; mesh path reads u8 here.
pub const WEATHER_PLAYBACK: usize = 0x78;
/// Weather entity child-slot pointers; a729_hook.rs:11104 follows them through one more pointer.
/// replay_interface/native/map_viewer/src/xmap09_light_lifecycle.rs:2029-2030 says final free leaves transferred descendants alive.
pub const CHILD_SLOTS: usize = 0x80;
/// Weather entity child count u32; a729_hook.rs:11100 reads it.
pub const CHILD_COUNT: usize = 0x8c;
/// Weather entity active byte; a729_hook.rs:11095 compares it with zero.
pub const ACTIVE: usize = 0xe8;
/// Entity transform setter FUN14227ba20; o05_building_capture.rs:14 identifies the 64-byte matrix input.
pub const TRANSFORM_SETTER_RVA: usize = 0x0227_ba20;
/// Entity transform setter bytes checked by o05_building_capture.rs:29.
pub const TRANSFORM_SETTER_SENTINEL: [u8; 16] = [
    0x40, 0x55, 0x53, 0x57, 0x48, 0x8d, 0x6c, 0x24, 0xd0, 0x48, 0x81, 0xec, 0x30, 0x01, 0x00, 0x00,
];
/// Entity factory FUN142277df0; lib.rs:75 and a729_hook.rs:7668 retain its ABI.
pub const ENTITY_FACTORY_RVA: u32 = 0x0227_7DF0;
/// Entity factory entry bytes checked by a729_hook.rs:227.
pub const ENTITY_FACTORY_SENTINEL: [u8; 16] = [
    0x40, 0x55, 0x48, 0x8d, 0xac, 0x24, 0xd0, 0xfd, 0xff, 0xff, 0x48, 0x81, 0xec, 0x30, 0x03, 0x00,
];
/// Entity destruction FUN1422787c0; lib.rs:76 identifies the object lifetime endpoint.
pub const ENTITY_DESTROY_RVA: u32 = 0x0227_87C0;
/// Entity destroy entry bytes checked by a729_hook.rs:248.
pub const ENTITY_DESTROY_SENTINEL: [u8; 16] = [
    0x48, 0x85, 0xc9, 0x74, 0x1f, 0x53, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0xd9, 0xe8, 0x3e, 0x41,
];
/// Weather entity active setter FUN14227b200; lib.rs:96 identifies the operation.
pub const WEATHER_ENTITY_ACTIVE_SETTER_RVA: u32 = 0x0227_B200;
/// Entity active setter entry bytes checked by a729_hook.rs:263.
pub const WEATHER_ACTIVE_SETTER_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x57, 0x48, 0x83, 0xec, 0x20, 0x48,
];
/// Entity alpha setter FUN142271b40; lib.rs:97 identifies the operation.
pub const WEATHER_ENTITY_ALPHA_SETTER_RVA: u32 = 0x0227_1B40;
/// Entity alpha setter entry bytes checked by a729_hook.rs:266.
pub const WEATHER_ALPHA_SETTER_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x57,
];
/// Entity playback setter FUN14227b4a0; lib.rs:98 identifies the operation.
pub const WEATHER_ENTITY_PLAYBACK_SETTER_RVA: u32 = 0x0227_B4A0;
/// Entity playback setter entry bytes equal the alpha setter prefix; a729_hook.rs:269.
pub const WEATHER_PLAYBACK_SETTER_SENTINEL: [u8; 16] = WEATHER_ALPHA_SETTER_SENTINEL;

/// Allocator placement field inherited by animation-event children; replay_interface/native/map_viewer/src/xmap09_lighting_law.rs:574-578.
/// Its width and relationship to the mesh-path MESH_OWNER interpretation remain unproven.
pub const ALLOCATOR_PLACEMENT_FIELD_58: usize = 0x58;
/// Allocator scene mask forwarded to light creation; replay_interface/native/map_viewer/src/xmap09_lighting_law.rs:540-542,574-578.
/// That source says animation-event children inherit the parent's +0x58/+0x5c placement fields verbatim.
pub const SCENE_MASK: usize = 0x5c;
/// Entity allocator; replay_interface/native/map_viewer/src/xmap09_lighting_law.rs:574-578 says it writes the scene mask from argument four.
/// The sole-writer claim is preserved from that source, not newly established by this harvest.
pub const ALLOCATE_RVA: usize = 0x0227_3fb0;
