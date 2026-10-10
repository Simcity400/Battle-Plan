//! National-focus presentation view, excluded from hourly acquisition.
//! Its vtable placement is unresolved between the observed view pointer and GUI
//! subobject. These offsets are relative to the observed view argument.

/// View constructor entry.
pub const CONSTRUCTOR_RVA: usize = 0x0136_aec0;
/// View rebuild-owner entry.
pub const REBUILD_RVA: usize = 0x0137_1b70;
/// View vector-publisher entry.
pub const VECTOR_PUBLISHER_RVA: usize = 0x0136_f750;
/// View vector-publisher entry sentinel.
pub const VECTOR_PUBLISHER_SENTINEL: [u8; 16] = [
    0x4c, 0x89, 0x4c, 0x24, 0x20, 0x4c, 0x89, 0x44, 0x24, 0x18, 0x48, 0x89, 0x54, 0x24, 0x10, 0x48,
];
/// View dynamic-refresh entry.
pub const DYNAMIC_REFRESH_RVA: usize = 0x0137_a780;
/// View dynamic-refresh entry sentinel.
pub const DYNAMIC_REFRESH_SENTINEL: [u8; 16] = [
    0x48, 0x8b, 0xc4, 0x48, 0x89, 0x58, 0x18, 0x48, 0x89, 0x68, 0x20, 0x56, 0x57, 0x41, 0x54, 0x41,
];
/// View primary vtable.
pub const VTABLE_RVA: usize = 0x0299_d680;
/// View GUI-subobject adjustment.
pub const GUI_SUBOBJECT: usize = 0x30;
/// View runtime canvas-root pointer.
pub const CANVAS_ROOT: usize = 0x588;
/// View canvas-position widget pointer.
pub const CANVAS_POSITION_WIDGET: usize = 0x590;
/// View focus-item pointer array.
pub const ITEMS: usize = 0x2058;
/// View item-array u32 capacity.
pub const ITEM_CAPACITY: usize = 0x2060;
/// View item-array u32 count.
pub const ITEM_COUNT: usize = 0x2064;
/// View item-array allocator pointer, included in header-stability checks.
pub const ITEM_ALLOCATOR: usize = 0x2068;
/// View canvas-scale f32 bits, shared by both axes.
pub const CANVAS_SCALE: usize = 0x2118;
