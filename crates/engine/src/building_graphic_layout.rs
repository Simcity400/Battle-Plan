//! Map-building visual record; its concrete type is unknown.
//! This presentation state is excluded from hourly recording.

/// Record allocator using a separate manager receiver.
pub const CONSTRUCTOR_RVA: usize = 0x0165_d5c0;
/// Record allocator entry sentinel.
pub const CONSTRUCTOR_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x4c, 0x89, 0x4c, 0x24, 0x20, 0x89, 0x54, 0x24, 0x10, 0x55, 0x56,
];
/// Visual record byte extent, independent of pointer-array stride.
pub const SIZE: usize = 0x48;
/// Building type, u32.
pub const BUILDING_TYPE: usize = 0;
/// Object tag, u32.
pub const OBJECT_TAG: usize = 4;
/// Required object tag value.
pub const OBJECT_TAG_VALUE: u32 = 0x165;
/// Selected definition pointer.
pub const SELECTED_DEFINITION: usize = 8;
/// Selected entity pointer.
pub const SELECTED_ENTITY: usize = 0x10;
/// Building kind, i32; zero means province building.
pub const BUILDING_KIND: usize = 0x18;
/// Building order, u8.
pub const BUILDING_ORDER: usize = 0x1c;
/// Selected entity ordinal, u8.
pub const SELECTED_ENTITY_ORDINAL: usize = 0x1d;
/// Position X, f32.
pub const POSITION_X: usize = 0x20;
/// Position Y, f32.
pub const POSITION_Y: usize = 0x24;
/// Position Z, f32.
pub const POSITION_Z: usize = 0x28;
/// Yaw, f32.
pub const YAW: usize = 0x2c;
/// Primary province handle, i32.
pub const PRIMARY_PROVINCE: usize = 0x30;
/// Secondary province handle, i32.
pub const SECONDARY_PROVINCE: usize = 0x34;
/// State handle, u16.
pub const STATE: usize = 0x38;
/// Remembered destroyed-selector byte, written when creating the selected entity.
pub const DESTROYED_VARIANT: usize = 0x3a;
/// Entity visual state, u8.
pub const ENTITY_VISUAL_STATE: usize = 0x3b;
/// Type visibility flag, u8.
pub const TYPE_VISIBILITY: usize = 0x3d;
/// Presentation scale, f32; popup growth clamps it to one.
pub const PRESENTATION_SCALE: usize = 0x40;
/// Per-slot occupant, variant and damage updater; stops at the first built candidate.
pub const UPDATE_SLOT_RVA: usize = 0x0166_48d0;
/// Entity selection and creation; recreates an instance when variant or destroyed selector changes.
pub const SELECT_ENTITY_RVA: usize = 0x0166_3e70;
