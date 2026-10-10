//! Equipment-upgrade designer presentation window.

/// Upgrade-selection operation.
pub const SELECTION_RVA: usize = 0x01f5_d420;
/// Upgrade-selection entry bytes.
pub const SELECTION_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x55, 0x57, 0x41, 0x54, 0x41, 0x56,
];
/// Nested-view pointer.
pub const NESTED_VIEW: usize = 0x1bb0;
/// Selected-variant pointer; must agree with the nested-view selection.
pub const SELECTED_VARIANT: usize = 0x1ce0;
