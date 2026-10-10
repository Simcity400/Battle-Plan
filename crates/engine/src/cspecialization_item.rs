//! Project-specialization presentation row.

/// Row constructor assigning its specialization token.
pub const CONSTRUCTOR_RVA: usize = 0x01c9_a6c0;
/// Specialization-row allocation size.
pub const SIZE: usize = 0x0b68;
/// Primary vtable required before reading specialization rows.
pub const VTABLE_RVA: usize = 0x02a4_9938;
/// GUI-interface vtable required for the +0x38 subobject.
pub const GUI_VTABLE_RVA: usize = 0x02a4_99a8;
/// GUI subobject displacement, subtracted from the list-item pointer.
pub const GUI: usize = 0x38;
/// Bound lexer token, u32, at +0x70.
pub const TOKEN: usize = 0x70;
