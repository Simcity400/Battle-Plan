//! `NCombatLog::CStatsObserver::SDamageDone` attribution entry.
//!
//! The old collector reads damage as u32 then widens it to u64. That does not
//! prove the engine field is only four bytes; verify its producer before recording
//! this value in the new native-precision format.

/// SDamageDone primary vtable; archived offsets.rs:823, confirmed by a729 RTTI.
pub const VTABLE_RVA: usize = 0x0294_6ec8;
/// SDamageDone in-memory array stride; archived offsets.rs:824 and attribution element addressing.
pub const STRIDE: usize = 0x20;
/// SDamageDone u32 dealer country tag; archived offsets.rs:825 and attribution reader.
pub const DEALER: usize = 0x08;
/// SDamageDone u32 dealer echo checked against +0x08; archived offsets.rs:826 and attribution reader.
pub const DEALER_ECHO: usize = 0x0c;
/// SDamageDone u32 victim country tag; archived offsets.rs:827 and attribution reader.
pub const VICTIM: usize = 0x10;
/// SDamageDone damage at +0x18, old u32 read with unproved full width; archived offsets.rs:828 and attribution reader.
pub const DAMAGE: usize = 0x18;
