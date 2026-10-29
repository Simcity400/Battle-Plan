//! `CGui` manager; retained registration knowledge, excluded from hourly state.

/// CGui primary vtable.
pub const VTABLE_RVA: usize = 0x02bb_3fc8;
/// CGui secondary vtable.
pub const SECONDARY_VTABLE_RVA: usize = 0x02bb_40a0;
/// CGui singleton pointer.
pub const SINGLETON_RVA: usize = 0x0350_0fc0;
/// CGui constructor.
pub const CONSTRUCTOR_RVA: usize = 0x022b_8ac0;
/// CGui graphics pointer.
pub const GRAPHICS: usize = 0xb8;
