//! `CGui` manager; retained registration knowledge, excluded from hourly state.

/// CGui primary vtable.
pub const VTABLE_RVA: usize = 0x02bb_3fa8;
/// CGui secondary vtable.
pub const SECONDARY_VTABLE_RVA: usize = 0x02bb_4080;
/// CGui singleton pointer.
pub const SINGLETON_RVA: usize = 0x0350_0fc0;
/// CGui constructor.
pub const CONSTRUCTOR_RVA: usize = 0x022b_8770;
/// CGui graphics pointer.
pub const GRAPHICS: usize = 0xb8;
