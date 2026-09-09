//! CSunkConvoyInfo aggregate history records reached through CGameState +0x5a8.
//!
//! `read_compact_sunk_convoy_history` in old collector/attach_log.rs:140220-140440
//! (month, killer, owner) observation and increments that bucket afterwards.
//! Pointer-vector order is first-seen order, not chronological or sorted order.
//! An hourly walk still needs the Milestone 2.1 consistency and cost checks.
//! Terminal convoy route/combat attribution required a separate old event hook;
//! those transient identities cannot be recovered from this aggregate record.

/// CSunkConvoyInfo vtable; old attach_log.rs:140226/140345 and convoy_terminal_hook_windows.rs:67 authenticate it.
pub const VTABLE_RVA: usize = 0x0295_d008;
/// CSunkConvoyInfo complete read extent; old attach_log.rs:140228 copies 0x18 bytes for every aggregate row.
pub const READ_EXTENT: usize = 0x18;
/// CSunkConvoyInfo native i32 month bucket; old attach_log.rs:140372 decodes the full signed word.
pub const MONTH: usize = 0x08;
/// CSunkConvoyInfo native u32 sunk-convoy count; old attach_log.rs:140373 preserves the aggregate counter.
pub const CONVOYS: usize = 0x0c;
/// CSunkConvoyInfo native u32 killer country link; old attach_log.rs:140375 reads its database identity.
pub const KILLER_COUNTRY: usize = 0x10;
/// CSunkConvoyInfo native u32 owner country link; old attach_log.rs:140376 reads its database identity.
pub const OWNER_COUNTRY: usize = 0x14;
