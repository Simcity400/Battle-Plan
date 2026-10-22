//! `CGui` manager; retained registration knowledge, excluded from hourly state.

/// CGui primary vtable.
pub const VTABLE_RVA: usize = 0x02b2_8720;
/// CGui secondary vtable.
pub const SECONDARY_VTABLE_RVA: usize = 0x02b2_87f8;
/// CGui singleton pointer.
pub const SINGLETON_RVA: usize = 0x0343_a2d0;
/// CGui constructor.
pub const CONSTRUCTOR_RVA: usize = 0x0224_4bf0;
/// CGui graphics pointer.
pub const GRAPHICS: usize = 0xb8;
