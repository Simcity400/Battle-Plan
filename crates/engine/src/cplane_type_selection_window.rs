//! Quick wing deployment plane-type selection window.
//! This presentation controller materializes selected rows outside the
//! simulation acquisition boundary.

/// Window constructor.
pub const CONSTRUCTOR_RVA: usize = 0x01f4_a6c0;
/// CPlaneTypeSelectionWindow primary vtable.
pub const VTABLE_RVA: usize = 0x02a8_4ee0;
/// CPlaneTypeSelectionWindow +0x10 interface vtable.
pub const INTERFACE_10_VTABLE_RVA: usize = 0x02a8_4f58;
/// CPlaneTypeSelectionWindow +0x38 GUI-interface vtable.
pub const GUI_VTABLE_RVA: usize = 0x02a8_4f80;
/// CPlaneTypeSelectionWindow secondary interface location.
pub const INTERFACE_10: usize = 0x10;
/// CPlaneTypeSelectionWindow GUI interface location.
pub const GUI: usize = 0x38;
/// Selection-row materializer entry.
pub const MATERIALIZE_RVA: usize = 0x01f4_a980;
/// Materializer entry sentinel bytes.
pub const MATERIALIZE_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24, 0x20, 0x57,
];
/// Quick-wing-deployment controller pointer.
pub const UI_CONTROLLER: usize = 0x05a0;
/// Row-pointer vector with pointer-sized entries.
pub const ROWS: usize = 0x05b0;
/// CPlaneTypeSelectionWindow row-vector i32 capacity.
pub const ROW_CAPACITY: usize = 0x05b8;
/// Signed row-vector count, checked against selected-option count.
pub const ROW_COUNT: usize = 0x05bc;
/// CPlaneTypeSelectionWindow row-vector allocator pointer.
pub const ROW_ALLOCATOR: usize = 0x05c0;
/// Retained standard-list pointer, authenticated through both runtime vtables.
pub const LIST: usize = 0x05c8;
