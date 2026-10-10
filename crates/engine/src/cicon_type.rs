//! `CIconType` authored definition; presentation-only retained layout.

/// CIconType primary vtable.
pub const VTABLE_RVA: usize = 0x02b3_cda0;
/// Icon-type constructor defaults frame one, 2D mode and zero position.
pub const CONSTRUCTOR_RVA: usize = 0x0235_8490;
/// Icon factory selects construction arguments using the mode byte.
pub const OBJECT_FACTORY_RVA: usize = 0x0235_8760;
/// CIconType object-factory byte slot.
pub const OBJECT_FACTORY_VSLOT: usize = 0x68;
/// CIconType property reader.
pub const PROPERTY_READER_RVA: usize = 0x0235_8910;
/// CIconType graphical resource-name MSVC string.
pub const GRAPHICAL_RESOURCE_NAME: usize = 0x100;
/// Icon-type position, three f32 values converted from authored i32
/// x/y coordinates; z initializes to zero.
pub const POSITION: usize = 0x120;
/// CIconType native f32 scale.
pub const SCALE: usize = 0x12c;
/// CIconType native i32 orientation.
pub const ORIENTATION: usize = 0x130;
/// CIconType native i32 frame.
pub const FRAME: usize = 0x134;
/// CIconType native f32 token-0x156 operand.
pub const TOKEN_156_FLOAT: usize = 0x138;
/// Icon 2D mode byte: 2D position sets it, 3D position clears it.
pub const TWO_DIMENSIONAL: usize = 0x13c;
/// CIconType native byte for tokens 0x13F/0x1D6.
pub const TOKEN_13F_1D6_FLAG: usize = 0x13d;
/// CIconType half-size centering byte.
pub const CENTER: usize = 0x13e;
/// CIconType full-size alignment byte.
pub const FULL_SIZE_ALIGNMENT: usize = 0x13f;
/// CIconType position property token.
pub const POSITION_TOKEN: u32 = 0x4c;
/// CIconType position3D property token.
pub const POSITION_3D_TOKEN: u32 = 0xaf;
/// CIconType scale property token.
pub const SCALE_TOKEN: u32 = 0x5d;
/// CIconType frame property token.
pub const FRAME_TOKEN: u32 = 0xfc;
/// CIconType default frame.
pub const DEFAULT_FRAME: i32 = 1;
