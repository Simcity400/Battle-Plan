//! `CGuiObject` base; offsets use its own pointer, not an enclosing window pointer.

/// CGuiObject vtable.
pub const VTABLE_RVA: usize = 0x02b2_d000;
/// CGuiObject authored CGuiType pointer +0x60.
pub const GUI_TYPE: usize = 0x60;
/// GUI interaction-state byte; high bit locks state changes.
pub const INTERACTION_STATE: usize = 0x74;
/// GUI interaction-disable lock, preserved during state updates.
pub const DISABLE_LOCK_MASK: u8 = 0x80;
/// CGuiObject flags byte +0x75.
pub const FLAGS: usize = 0x75;
/// Hidden-suppression bit gates showing the GUI object.
pub const HIDDEN_MASK: u8 = 0x10;
/// GUI active/visible bit, distinct from hidden suppression.
pub const ACTIVE_MASK: u8 = 0x08;
/// CGuiObject graphical-position getter byte slot, output [i32.
pub const GRAPHICAL_POSITION_VSLOT: usize = 0x98;
/// CGuiObject name getter byte slot.
pub const NAME_VSLOT: usize = 0xa8;
