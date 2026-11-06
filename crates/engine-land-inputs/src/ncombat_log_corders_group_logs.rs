//! One orders group's Combat Log in its country's manager.
//!
//! A writer creates it when no held log resolves to the group; the view only
//! looks it up. Daily pruning uses the loaded maximum-month age, then removes
//! empty logs or logs whose group no longer resolves. Every array below is a
//! pointer CPdxArray with data, capacity, count and allocator fields.

use engine_sites::Rva;
/// Group-log primary vtable.
pub const VTABLE_RVA: Rva = Rva(0x029d_a210);
/// Enemy equipment destroyed by this group, recorded by retained-history events.
pub const ENEMY_LOSSES: usize = 0x08;
/// Recovered equipment after combat, grouped by month with reason zero.
pub const RECOVERED: usize = 0x20;
/// Four own-loss arrays indexed by reason. Reason zero records combat,
/// exile, paradrop and overrun losses. Attrition uses reason two in combat,
/// three while exercising and one otherwise; volunteer transfer also uses one.
/// The view separates combat loss from these three attrition categories.
pub const REASONS: usize = 0x38;
/// Distance between the reason arrays' headers.
pub const REASON_STRIDE: usize = 0x18;
/// Reasons the game keeps: the loader drops a loaded entry whose reason is above 3.
pub const REASON_COUNT: usize = 4;
/// Orders-group full reference, used to resolve and locate its log.
pub const GROUP: usize = 0xe0;
/// Group-log allocation size.
pub const SIZE: usize = 0xe8;
