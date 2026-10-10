//! Damage attribution entry. The full damage-field width is not established;
//! a four-byte read does not authorize recording a narrower value.

/// Attribution-entry vtable.
pub const VTABLE_RVA: usize = 0x0294_6ec8;
/// In-memory array stride.
pub const STRIDE: usize = 0x20;
/// Dealer country tag, u32.
pub const DEALER: usize = 0x08;
/// Dealer echo, u32, checked against +0x08.
pub const DEALER_ECHO: usize = 0x0c;
/// Victim country tag, u32.
pub const VICTIM: usize = 0x10;
/// Damage at +0x18; full width remains unestablished.
pub const DAMAGE: usize = 0x18;
