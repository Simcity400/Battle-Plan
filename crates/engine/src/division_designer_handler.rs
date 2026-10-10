//! Division-designer handler presentation layout.

/// Selected-template thunk.
pub const SELECTION_RVA: usize = 0x0167_aee0;
/// Selected-template thunk entry bytes.
pub const SELECTION_SENTINEL: [u8; 16] = [
    0x4c, 0x8b, 0xc2, 0xc7, 0x01, 0x02, 0x00, 0x00, 0x00, 0x48, 0x83, 0xc2, 0x18, 0xe9, 0x7e, 0x12,
];
/// Selected-template pointer.
pub const SELECTED_TEMPLATE: usize = 0x8d0;
