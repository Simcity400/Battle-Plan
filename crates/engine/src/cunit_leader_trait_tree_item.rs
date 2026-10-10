//! Selected-leader trait-tree presentation state, excluded from recorded state.
//! Its visible buttons expose selected-leader presentation, not a whole-world trait census.

/// Trait-definition pointer.
pub const DEFINITION: usize = 0x20;
/// Complete selected-leader reference, u64.
pub const LEADER_REFERENCE: usize = 0x2c;
/// Disabled-state button pointer; one of three mutually exclusive states.
pub const DISABLED_BUTTON: usize = 0x40;
/// Pickable-state button pointer; one of three mutually exclusive states.
pub const PICKABLE_BUTTON: usize = 0x598;
/// Picked-state button pointer; one of three mutually exclusive states.
pub const PICKED_BUTTON: usize = 0xaf0;
/// Item constructor.
pub const CONSTRUCTOR_RVA: usize = 0x01cf_e2c0;
/// Button-state producer.
pub const UPDATE_STATE_RVA: usize = 0x01d0_11a0;
/// Tooltip formatter.
pub const TOOLTIP_FORMATTER_RVA: usize = 0x01cf_f880;
/// Tooltip-formatter entry bytes.
pub const TOOLTIP_FORMATTER_SENTINEL: [u8; 24] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57,
    0x48, 0x8d, 0xac, 0x24, 0x30, 0xf8, 0xff, 0xff,
];
