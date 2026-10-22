//! CLandBorderWarCombatant, the border-war side. Its round dispatch directly
//! calls the ordinary land round; replacing a virtual slot alone misses this path.

/// CLandBorderWarCombatant primary vtable.
pub const VTABLE_RVA: usize = 0x029a_5818;
/// Border-war side country used by the tooltip.
pub const COUNTRY: usize = 0x4a0;
/// Border-war state pointer; the tooltip reads its id.
pub const STATE: usize = 0x4a8;
/// Win/lose event MSVC strings; stored length establishes presence.
pub const WIN_EVENT: usize = 0x4e0;
pub const LOSE_EVENT: usize = 0x500;
