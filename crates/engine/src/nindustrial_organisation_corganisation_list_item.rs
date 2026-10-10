//! Organisation list item owning the MIO detail window; excluded from hourly recording.

/// List-item primary vtable.
pub const VTABLE_RVA: usize = 0x02a7_d6e0;
/// List-item GUI vtable.
pub const GUI_VTABLE_RVA: usize = 0x02a7_d750;
/// List-item third vtable.
pub const THIRD_VTABLE_RVA: usize = 0x02a7_d818;
/// List-item fourth vtable.
pub const FOURTH_VTABLE_RVA: usize = 0x02a7_d830;
/// List-item GUI interface.
pub const GUI: usize = 0x38;
/// List-item third interface.
pub const THIRD_INTERFACE: usize = 0x68;
/// List-item fourth interface.
pub const FOURTH_INTERFACE: usize = 0x70;
/// List-item organisation reference, two u32 words.
pub const ORGANISATION_REFERENCE: usize = 0x84;
/// List-item signed i32 constructor variant.
pub const VARIANT: usize = 0x8c;
/// Embedded detail window owned by this list item.
pub const DETAIL_WINDOW: usize = 0x90;
