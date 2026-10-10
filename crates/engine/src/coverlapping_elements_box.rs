//! Overlapping-elements presentation layout.

/// Layout entry.
pub const LAYOUT_RVA: usize = 0x0237_2190;
/// Layout entry sentinel bytes.
pub const LAYOUT_SENTINEL: [u8; 16] = [
    0x41, 0x56, 0x48, 0x81, 0xec, 0x80, 0x00, 0x00, 0x00, 0x48, 0x8b, 0x01, 0x4c, 0x8b, 0xf1, 0xff,
];
/// Embedded child-list interface subobject.
pub const LIST_INTERFACE: usize = 0xa8;
/// Child-list accessor virtual-dispatch byte offset.
pub const CHILD_LIST_SLOT: usize = 0x58;
/// Reverse-order byte selecting forward or reverse list traversal.
pub const REVERSED: usize = 0x13c;
