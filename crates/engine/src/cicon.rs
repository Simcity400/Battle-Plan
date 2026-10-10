//! `CIcon`, a CButton-derived widget with a separately owned drawable.
//!
//! Presentation layout only; no hourly walk or presentation hook is installed.

/// CIcon primary vtable.
pub const VTABLE_RVA: usize = 0x02b3_41a0;
/// CIcon observable-secondary vtable.
pub const OBSERVABLE_VTABLE_RVA: usize = 0x02b3_4558;
/// CIcon allocation size.
pub const SIZE: usize = 0x160;
/// CIcon constructor.
pub const CONSTRUCTOR_RVA: usize = 0x022e_5d40;
/// CIcon drawable pointer.
pub const DRAWABLE: usize = 0xe8;
/// CIcon retained drawable-parent argument.
pub const DRAWABLE_PARENT: usize = 0xf0;
/// CIcon graphics pointer.
pub const GRAPHICS: usize = 0xf8;
/// CIcon retained position, three native f32 values.
pub const POSITION: usize = 0x100;
/// CIcon constructor's integer x/y position pair.
pub const INITIAL_INTEGER_POSITION: usize = 0x10c;
/// Latest applied icon signed i32 position pair, stored before centering.
pub const APPLIED_POSITION: usize = 0x114;
/// CIcon native f32 scale operand.
pub const SCALE: usize = 0x11c;
/// Icon signed i32 orientation selector; positioning recognizes zero through six.
pub const ORIENTATION: usize = 0x120;
/// CIcon parent extent, native i32 x/y.
pub const PARENT_EXTENT: usize = 0x124;
/// CIcon flags byte.
pub const FLAGS: usize = 0x154;
/// Icon 2D-path mask, checked before drawable construction and positioning.
pub const TWO_DIMENSIONAL_MASK: u8 = 1;
/// Half-size centering mask, subtracting half the integer drawable size.
pub const CENTER_MASK: u8 = 0x40;
/// Full-size alignment mask, subtracting the complete drawable size.
pub const FULL_SIZE_ALIGNMENT_MASK: u8 = 0x80;
/// CIcon notification flags byte.
pub const NOTIFICATION_FLAGS: usize = 0x155;
/// Hover-notified mask, cleared after leave and set after hover.
pub const NOTIFIED_OVER_MASK: u8 = 1;
/// CIcon state setter byte slot.
pub const SET_STATE_VSLOT: usize = 0x280;
/// CIcon enable byte slot.
pub const ENABLE_VSLOT: usize = 0x288;
/// CIcon disable byte slot.
pub const DISABLE_VSLOT: usize = 0x290;
/// Icon position-setter dispatch displacement for its orientation-adjusted i32 pair.
pub const SET_POSITION_VSLOT: usize = 0x2a8;
/// CIcon observer callback byte slot.
pub const STATE_CALLBACK_VSLOT: usize = 0x378;
/// CIcon state/effect setter.
pub const SET_STATE_RVA: usize = 0x022e_6e30;
/// CIcon enable operation.
pub const ENABLE_RVA: usize = 0x022e_6490;
/// CIcon disable operation.
pub const DISABLE_RVA: usize = 0x022e_6460;
/// Icon observer callback retests state after leave before considering hover.
pub const STATE_CALLBACK_RVA: usize = 0x022e_5fa0;
/// CIcon drawable replacement.
pub const REPLACE_DRAWABLE_RVA: usize = 0x022e_6050;
/// CIcon alternate drawable replacement.
pub const REPLACE_DRAWABLE_ALTERNATE_RVA: usize = 0x022e_6180;
/// Drawable construction converts stored f32 positions to i32 before dispatch.
pub const BUILD_DRAWABLE_RVA: usize = 0x022e_6b90;
/// Icon position setter applies centering before drawable dispatch.
pub const SET_POSITION_RVA: usize = 0x022e_66f0;
/// CIcon observer Over event.
pub const OBSERVER_OVER_EVENT: i32 = 3;
/// CIcon observer Leave event.
pub const OBSERVER_LEAVE_EVENT: i32 = 4;
