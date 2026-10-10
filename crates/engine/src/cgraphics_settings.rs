//! CGraphicsSettings, a separate allocation from CSystemSettings and CSettings.
//! Display settings are presentation state; no hourly walker is enabled here.

/// CGraphicsSettings vtable.
pub const VTABLE_RVA: usize = 0x02b2_4940;
/// CGraphicsSettings allocation size.
pub const SIZE: usize = 0x1b0;
/// CGraphicsSettings constructor.
pub const CONSTRUCTOR_RVA: usize = 0x0221_2bd0;
/// CGraphicsSettings physical width i32.
pub const WIDTH: usize = 0xbc;
/// CGraphicsSettings physical height i32.
pub const HEIGHT: usize = 0xc0;
/// CGraphicsSettings minimum GUI width i32.
pub const MINIMUM_GUI_WIDTH: usize = 0xcc;
/// CGraphicsSettings minimum GUI height i32.
pub const MINIMUM_GUI_HEIGHT: usize = 0xd0;
/// Effective GUI scale f32, finalized from the authored value.
pub const GUI_SCALE: usize = 0xd4;
/// CGraphicsSettings full-screen byte.
pub const FULL_SCREEN: usize = 0xe0;
/// CGraphicsSettings borderless byte.
pub const BORDERLESS: usize = 0xe1;
/// CGraphicsSettings block-reader byte slot.
pub const READ_BLOCK_VSLOT: usize = 0x18;
/// CGraphicsSettings successful-read finalizer byte slot.
pub const FINALIZE_READ_VSLOT: usize = 0x40;
/// CGraphicsSettings GUI-scale finalizer.
pub const FINALIZE_READ_RVA: usize = 0x0221_4530;
/// CGraphicsSettings scale getter.
pub const GUI_SCALE_GETTER_RVA: usize = 0x0221_3830;
/// CGraphicsSettings minimum GUI dimensions getter.
pub const MINIMUM_GUI_SIZE_GETTER_RVA: usize = 0x0221_3840;
/// CGraphicsSettings physical dimensions getter.
pub const PHYSICAL_SIZE_GETTER_RVA: usize = 0x0221_3890;
