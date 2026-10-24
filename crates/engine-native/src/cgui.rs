//! `CGui` manager; retained registration knowledge, excluded from hourly state.

/// CGui primary vtable.
pub const VTABLE_RVA: usize = 0x02b3_f4e0;
/// CGui secondary vtable.
pub const SECONDARY_VTABLE_RVA: usize = 0x02b3_f5b8;
/// CGui singleton pointer.
pub const SINGLETON_RVA: usize = 0x0345_3230;
/// CGui constructor.
pub const CONSTRUCTOR_RVA: usize = 0x0225_9b30;
/// CGui graphics pointer.
pub const GRAPHICS: usize = 0xb8;
