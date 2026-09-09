//! `NCombatLog::CStatsObserver`, embedded in `CLandCombatant+0x1b0`.
//!
//! The first leader-tally entry was only a last-non-null cache observation in
//! the old reader; it did not establish a current leader or freshness. The loss
//! reader brackets excluded event callbacks, not the rewrite's hourly boundary.

/// NCombatLog::CStatsObserver primary vtable; archived offsets.rs:819, confirmed by a729 RTTI.
pub const VTABLE_RVA: usize = 0x0294_6f68;
/// CStatsObserver cached leader-tally array; archived land_combat_detail_events_live/passive_modifier_carrier.rs:17,367.
pub const LEADERS: usize = 0x20;
/// CStatsObserver leader-tally u32 capacity; archived passive_modifier_carrier.rs:18,368.
pub const LEADER_CAPACITY: usize = 0x28;
/// CStatsObserver leader-tally u32 count; archived passive_modifier_carrier.rs:19,369.
pub const LEADER_COUNT: usize = 0x2c;
/// CStatsObserver contiguous SDamageDone array; archived offsets.rs:820 and `read_land_combat_side_attribution`.
pub const DAMAGE: usize = 0x38;
/// CStatsObserver damage-array u32 capacity; archived offsets.rs:821 and attribution header reader.
pub const DAMAGE_CAPACITY: usize = 0x40;
/// CStatsObserver damage-array u32 count; archived offsets.rs:822 and attribution header reader.
pub const DAMAGE_COUNT: usize = 0x44;
/// CStatsObserver manpower loss, old i32 read widened to i64; archived x2_carriers.rs:172,5323, full storage width unproved.
pub const MANPOWER_LOSS: usize = 0x60;
/// CStatsObserver embedded equipment-loss map; archived x2_carriers.rs:173,5328, vector at map+0x20.
pub const EQUIPMENT_LOSSES: usize = 0x70;
