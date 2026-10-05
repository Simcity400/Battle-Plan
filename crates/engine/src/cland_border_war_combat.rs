//! `CLandBorderWarCombat`, the border-war subclass of `CLandCombat`.
//!
//! The archived manager walk uses the shared land layout after checking both
//! class vtables. Its handle's type word is runtime minted, not ordinary land's 62.

/// CLandBorderWarCombat primary vtable; archived offsets.rs:758, confirmed by a729 RTTI.
pub const VTABLE_RVA: usize = 0x029a_56e0;
/// CLandBorderWarCombat secondary vtable at +0x10; archived offsets.rs:759, confirmed by a729 RTTI COL offset.
pub const BODY_VTABLE_RVA: usize = 0x029a_57c0;
/// CLandBorderWarCombat secondary-base displacement; archived land manager walk and a729 RTTI COL offset 16.
pub const BODY: usize = 0x10;
/// Transfer-state byte tested by the border tooltip tail at 1413D69D0.
pub const TRANSFER_STATE: usize = 0xf0;
