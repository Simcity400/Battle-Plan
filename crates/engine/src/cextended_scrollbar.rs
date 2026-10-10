//! CExtendedScrollbar presentation layout with signed i64 fixed values.
//! Its interfaces are at displacements 0, +0x80 and +0xb0.

/// CExtendedScrollbar primary vtable.
pub const VTABLE_RVA: usize = 0x02b3_2ff0;
/// CExtendedScrollbar value-base vtable at +0x80.
pub const VALUE_VTABLE_RVA: usize = 0x02b3_3270;
/// CExtendedScrollbar third-base vtable at +0xb0.
pub const THIRD_VTABLE_RVA: usize = 0x02b3_3300;
/// CExtendedScrollbar value interface +0x80.
pub const VALUE_INTERFACE: usize = 0x80;
/// CExtendedScrollbar third interface +0xb0.
pub const THIRD_INTERFACE: usize = 0xb0;
/// CExtendedScrollbar lower range i64 +0x1558.
pub const MINIMUM: usize = 0x1558;
/// CExtendedScrollbar upper range i64 +0x1560.
pub const MAXIMUM: usize = 0x1560;
/// CExtendedScrollbar target i64 +0x15f8.
pub const TARGET: usize = 0x15f8;
/// CExtendedScrollbar displayed current i64 +0x1600.
pub const CURRENT: usize = 0x1600;
/// CExtendedScrollbar allocation size.
pub const SIZE: usize = 0x1620;
/// CExtendedScrollbar target mutation function.
pub const SET_TARGET_RVA: usize = 0x022d_6cf0;
/// CExtendedScrollbar animation function.
pub const UPDATE_CURRENT_RVA: usize = 0x022d_6fe0;
/// CExtendedScrollbar geometry/range rescaling function.
pub const RESCALE_RVA: usize = 0x022d_64c0;
/// Scrollbar value-interface dispatch at +0x20 returns f32 for argument
/// one, independently of raw signed i64 fields.
pub const DISPLAY_VALUE_SLOT: usize = 0x20;
