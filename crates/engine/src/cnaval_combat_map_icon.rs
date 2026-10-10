//! Naval-combat map icon, distinct from simulation combat state.
//! It uses province stack slot zero and returns no direction output.

/// CNavalCombatMapIcon primary vtable.
pub const VTABLE_RVA: usize = 0x02a0_1ac8;
/// CNavalCombatMapIcon world-position method.
pub const WORLD_POSITION_RVA: usize = 0x018e_2d30;
/// CNavalCombatMapIcon category 9.
pub const MAP_ICON_BUCKET: u32 = 9;
/// CNavalCombatMapIcon packed reference u64.
pub const REFERENCE: usize = 0x88;
/// CNavalCombatMapIcon CProvince pointer.
pub const PROVINCE: usize = 0x90;
/// CNavalCombatMapIcon anchor slot.
pub const STACK_SLOT: usize = 0;
