//! Render entity with subtype-dependent field interpretations.
//! Presentation metadata; no recorded table or hook is enabled.

/// Entity definition pointer.
pub const DEFINITION: usize = 0x40;
/// Mesh-owner pointer.
pub const MESH_OWNER: usize = 0x58;
/// Skinning-owner pointer.
pub const SKINNING_OWNER: usize = 0x70;
/// Mesh draw-variant byte; weather entities interpret this offset as f64.
pub const DRAW_VARIANT: usize = 0x78;
/// Material-binding group headers pointer.
pub const MATERIAL_BINDING_GROUPS: usize = 0xb8;
/// Weather playback, f64; mesh entities interpret this offset as u8.
pub const WEATHER_PLAYBACK: usize = 0x78;
/// Weather child-slot pointers, followed through one more pointer.
/// Final destruction leaves transferred descendants alive.
pub const CHILD_SLOTS: usize = 0x80;
/// Weather child count, u32.
pub const CHILD_COUNT: usize = 0x8c;
/// Weather active byte, tested for nonzero.
pub const ACTIVE: usize = 0xe8;
/// Transform setter accepting a 64-byte matrix.
pub const TRANSFORM_SETTER_RVA: usize = 0x0227_ba20;
/// Exact transform-setter entry sentinel.
pub const TRANSFORM_SETTER_SENTINEL: [u8; 16] = [
    0x40, 0x55, 0x53, 0x57, 0x48, 0x8d, 0x6c, 0x24, 0xd0, 0x48, 0x81, 0xec, 0x30, 0x01, 0x00, 0x00,
];
/// Entity factory.
pub const ENTITY_FACTORY_RVA: u32 = 0x0227_7DF0;
/// Exact factory-entry sentinel.
pub const ENTITY_FACTORY_SENTINEL: [u8; 16] = [
    0x40, 0x55, 0x48, 0x8d, 0xac, 0x24, 0xd0, 0xfd, 0xff, 0xff, 0x48, 0x81, 0xec, 0x30, 0x03, 0x00,
];
/// Entity destruction routine.
pub const ENTITY_DESTROY_RVA: u32 = 0x0227_87C0;
/// Exact destruction-entry sentinel.
pub const ENTITY_DESTROY_SENTINEL: [u8; 16] = [
    0x48, 0x85, 0xc9, 0x74, 0x1f, 0x53, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0xd9, 0xe8, 0x3e, 0x41,
];
/// Weather active setter.
pub const WEATHER_ENTITY_ACTIVE_SETTER_RVA: u32 = 0x0227_B200;
/// Exact active-setter entry sentinel.
pub const WEATHER_ACTIVE_SETTER_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x57, 0x48, 0x83, 0xec, 0x20, 0x48,
];
/// Alpha setter.
pub const WEATHER_ENTITY_ALPHA_SETTER_RVA: u32 = 0x0227_1B40;
/// Exact alpha-setter entry sentinel.
pub const WEATHER_ALPHA_SETTER_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x57,
];
/// Playback setter.
pub const WEATHER_ENTITY_PLAYBACK_SETTER_RVA: u32 = 0x0227_B4A0;
/// Playback entry sentinel, sharing the alpha-setter prefix.
pub const WEATHER_PLAYBACK_SETTER_SENTINEL: [u8; 16] = WEATHER_ALPHA_SETTER_SENTINEL;

/// Allocator placement field inherited by animation-event children.
/// Its width and relationship to MESH_OWNER remain unestablished.
pub const ALLOCATOR_PLACEMENT_FIELD_58: usize = 0x58;
/// Allocator scene mask forwarded to light creation. Animation-event
/// children inherit parent +0x58/+0x5c placement fields verbatim.
pub const SCENE_MASK: usize = 0x5c;
/// Entity allocator writing the scene mask from argument four.
pub const ALLOCATE_RVA: usize = 0x0227_3fb0;
