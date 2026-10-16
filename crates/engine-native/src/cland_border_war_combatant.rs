//! CLandBorderWarCombatant, the border-war side. Its round dispatch directly
//! calls the ordinary land round; replacing a virtual slot alone misses this path.

/// CLandBorderWarCombatant primary vtable.
pub const VTABLE_RVA: usize = 0x029a_5818;
/// CLandBorderWarCombatant round slot.
pub const ROUND_VTABLE_SLOT: usize = 0x98;
/// Border-war combat-side round entry.
pub const ROUND_RVA: usize = 0x013d_8740;
/// Direct ordinary-land-round call inside the border-war round.
pub const LAND_ROUND_CALL_RVA: usize = 0x013d_91f8;
/// CLandBorderWarCombatant lifecycle pointer slot, entry 33.
pub const PARTICIPANT_LIFECYCLE_SLOT_RVA: usize = 0x029a_5920;
/// CLandBorderWarCombatant authenticated vtable entry count.
pub const VTABLE_ENTRY_COUNT: usize = 34;
/// Border-war side country used by the tooltip.
pub const COUNTRY: usize = 0x4a0;
/// Border-war state pointer; the tooltip reads its +0x58 id.
pub const STATE: usize = 0x4a8;
/// Win/lose event MSVC strings; stored length establishes presence.
pub const WIN_EVENT: usize = 0x4e0;
pub const LOSE_EVENT: usize = 0x500;
/// CLandBorderWarCombatant lifecycle slot index.
pub const PARTICIPANT_LIFECYCLE_SLOT_INDEX: usize = 33;
