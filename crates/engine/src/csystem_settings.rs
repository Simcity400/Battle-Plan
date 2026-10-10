//! System settings own a separate graphics-settings allocation.

/// Primary vtable.
pub const VTABLE_RVA: usize = 0x02b2_4be0;
/// Constructor allocating graphics settings with independent identity.
pub const CONSTRUCTOR_RVA: usize = 0x0221_4f80;
/// Graphics-settings pointer.
pub const GRAPHICS_SETTINGS: usize = 0x98;
/// Token dispatcher forwarding the graphics block to its virtual reader.
pub const READ_TOKEN_RVA: usize = 0x0221_5ee0;
/// Graphics-block token selecting the separately allocated reader.
pub const GRAPHICS_TOKEN: u32 = 0xc1;
