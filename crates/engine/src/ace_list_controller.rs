//! Ace-list presentation controller; its concrete type is unknown.

/// Selected-wing reference, u64.
pub const SELECTED_WING_REFERENCE: usize = 0x5a8;
/// Plane-type selection mask, u32.
pub const PLANE_TYPE_MASK: usize = 0x5e4;
/// Modifier context; its width is unknown.
pub const MODIFIER_CONTEXT: usize = 0x5f0;
/// Ace-list refresh function.
pub const REFRESH_RVA: usize = 0x0201_e420;
/// Refresh entry sentinel.
pub const REFRESH_SENTINEL: [u8; 21] = [
    0x40, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57, 0x48, 0x8d, 0xac,
    0x24, 0x28, 0xf0, 0xff, 0xff,
];
/// Context-sensitive ace sorting comparator.
pub const SORT_COMPARATOR_RVA: usize = 0x0201_d470;
