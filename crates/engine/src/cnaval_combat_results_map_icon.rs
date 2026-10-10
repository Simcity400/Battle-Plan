//! Retained naval-result map icon, distinct from the result record.
//! Province bit zero at +0xd2 selects stack slot 19 or zero. When clear,
//! active combat raises the third position component by five. Constants
//! name these bit branches without inferring land/sea semantics.

/// CNavalCombatResultsMapIcon primary vtable.
pub const VTABLE_RVA: usize = 0x029f_d560;
/// CNavalCombatResultsMapIcon world-position method.
pub const WORLD_POSITION_RVA: usize = 0x018a_9740;
/// CNavalCombatResultsMapIcon category 0xa.
pub const MAP_ICON_BUCKET: u32 = 0xa;
/// CNavalCombatResultsMapIcon province id u32.
pub const PROVINCE_ID: usize = 0x88;
/// CNavalCombatResultsMapIcon bit-0-clear anchor slot.
pub const TYPE_BIT_CLEAR_STACK_SLOT: usize = 0;
/// CNavalCombatResultsMapIcon bit-0-set anchor slot.
pub const TYPE_BIT_SET_STACK_SLOT: usize = 19;
/// CNavalCombatResultsMapIcon bit-clear active-combat offset storage.
pub const ACTIVE_COMBAT_HEIGHT_OFFSET_RVA: usize = 0x0270_e55c;
