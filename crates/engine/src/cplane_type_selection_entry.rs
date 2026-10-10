//! Quick wing deployment plane-type selection row.

/// Row constructor.
pub const CONSTRUCTOR_RVA: usize = 0x01fc_49d0;
/// CPlaneTypeSelectionEntry allocation size.
pub const SIZE: usize = 0x0b10;
/// Primary vtable required for row identity.
pub const VTABLE_RVA: usize = 0x02a9_1800;
/// GUI-interface vtable required for the +0x38 subobject identity.
pub const GUI_VTABLE_RVA: usize = 0x02a9_1870;
/// CPlaneTypeSelectionEntry +0x48 interface vtable.
pub const INTERFACE_48_VTABLE_RVA: usize = 0x02a9_1938;
/// GUI subobject displacement, subtracted from list-item pointers.
pub const GUI: usize = 0x38;
/// CPlaneTypeSelectionEntry secondary interface location.
pub const INTERFACE_48: usize = 0x48;
/// Bound air-option pointer, compared with the selected bucket record.
pub const OPTION: usize = 0x50;
