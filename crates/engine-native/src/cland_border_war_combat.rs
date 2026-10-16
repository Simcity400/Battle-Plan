//! CLandBorderWarCombat, the border-war subclass of CLandCombat.
//! The shared layout requires both vtables to authenticate. Its reference
//! type is runtime minted, independently of ordinary land-combat type 62.

/// CLandBorderWarCombat primary vtable.
pub const VTABLE_RVA: usize = 0x029a_56e0;
/// CLandBorderWarCombat secondary vtable (`BODY`).
pub const BODY_VTABLE_RVA: usize = 0x029a_57c0;
/// CLandBorderWarCombat secondary-base displacement.
pub const BODY: usize = 0x10;
/// Signed fixed5 configured base combat width, initially zero. Save and load use this value.
pub const COMBAT_WIDTH: usize = 0xd8;
/// Transfer-state byte used by the border-war tooltip.
pub const TRANSFER_STATE: usize = 0xf0;

/// Finalization tests three u8 flags in priority order: forced attacker
/// victory, forced defender victory, cancellation. Otherwise both sides
/// are tested for participation.
pub const END_FLAGS: usize = 0xf1;
