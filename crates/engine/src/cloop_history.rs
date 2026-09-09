//! CLoopHistory layout and its air-statistics channel schema.
//!
//! Harvested from `air_loss_stats_live/src/lib.rs`, whose country walk checks
//! the vtable, self-reference, 89 fields, and five-entry yearly window. The
//! 24 named channels below are a subset of those 89 fields. Values remain i64;
//! display divisors do not justify narrowing or rounding the recorded value.

/// CLoopHistory primary vtable; a729 RTTI identity and old derive_country_row_result vtable check.
pub const VTABLE_RVA: usize = 0x029b_be58;
/// CLoopHistory window pointer at +0x20; old derive_country_row_result follows it before field reads.
pub const WINDOW: usize = 0x20;
/// CLoopHistory self-reference pointer at +0x28; old derive_country_row_result requires object equality.
pub const SELF_REFERENCE: usize = 0x28;
/// CLoopHistory field-count u32 at +0x34; old derive_country_row_result checks the air schema's 89 fields.
pub const FIELD_COUNT: usize = 0x34;

/// Air-statistics semantics attached to a CLoopHistory instance by the old engine walk.
pub mod air_stats {
    /// Air-statistics field count 89; old air_loss_stats_live validates the live CLoopHistory header.
    pub const FIELD_COUNT: u32 = 89;
    /// Air statistics display divisor for non-building channels; old CHANNEL_DISPLAY_DIVISORS uses 100000.
    pub const DISPLAY_DIVISOR: i64 = 100_000;
    /// Building-bombing display divisor; old CHANNEL_DISPLAY_DIVISORS uses 1000 for fields zero and 41.
    pub const BUILDINGS_BOMBED_DISPLAY_DIVISOR: i64 = 1_000;

    /// CLoopHistory air field 0, our buildings bombed; old air_loss_stats_live PINNED_CHANNELS mapping.
    pub const OUR_BUILDINGS_BOMBED: usize = 0;
    /// CLoopHistory air field 1, our troops bombed; old air_loss_stats_live PINNED_CHANNELS mapping.
    pub const OUR_TROOPS_BOMBED: usize = 1;
    /// CLoopHistory air field 2, our ships bombed; old air_loss_stats_live PINNED_CHANNELS mapping.
    pub const OUR_SHIPS_BOMBED: usize = 2;
    /// CLoopHistory air field 3, our fighters lost; old air_loss_stats_live PINNED_CHANNELS mapping.
    pub const OUR_FIGHTERS_LOST: usize = 3;
    /// CLoopHistory air field 10, our bombers lost; old air_loss_stats_live PINNED_CHANNELS mapping.
    pub const OUR_BOMBERS_LOST: usize = 10;
    /// CLoopHistory air field 17, our close-air-support aircraft lost; old PINNED_CHANNELS mapping.
    pub const OUR_CAS_LOST: usize = 17;
    /// CLoopHistory air field 24, our transports lost; old air_loss_stats_live PINNED_CHANNELS mapping.
    pub const OUR_TRANSPORTS_LOST: usize = 24;
    /// CLoopHistory air field 36, our trains lost; old air_loss_stats_live PINNED_CHANNELS mapping.
    pub const OUR_TRAINS_LOST: usize = 36;
    /// CLoopHistory air field 37, our trucks lost; old air_loss_stats_live PINNED_CHANNELS mapping.
    pub const OUR_TRUCKS_LOST: usize = 37;
    /// CLoopHistory air field 38, our railway guns lost; old air_loss_stats_live PINNED_CHANNELS mapping.
    pub const OUR_RAILWAY_GUNS_LOST: usize = 38;
    /// CLoopHistory air field 39, our bombers disrupted; old air_loss_stats_live PINNED_CHANNELS mapping.
    pub const OUR_BOMBERS_DISRUPTED: usize = 39;
    /// CLoopHistory air field 40, our missiles lost; old air_loss_stats_live PINNED_CHANNELS mapping.
    pub const OUR_MISSILES_LOST: usize = 40;
    /// CLoopHistory air field 41, enemy buildings bombed; old air_loss_stats_live PINNED_CHANNELS mapping.
    pub const ENEMY_BUILDINGS_BOMBED: usize = 41;
    /// CLoopHistory air field 42, enemy troops bombed; old air_loss_stats_live PINNED_CHANNELS mapping.
    pub const ENEMY_TROOPS_BOMBED: usize = 42;
    /// CLoopHistory air field 43, enemy ships sunk; old air_loss_stats_live PINNED_CHANNELS mapping.
    pub const ENEMY_SHIPS_SUNK: usize = 43;
    /// CLoopHistory air field 44, enemy fighters lost; old air_loss_stats_live PINNED_CHANNELS mapping.
    pub const ENEMY_FIGHTERS_LOST: usize = 44;
    /// CLoopHistory air field 51, enemy bombers lost; old air_loss_stats_live PINNED_CHANNELS mapping.
    pub const ENEMY_BOMBERS_LOST: usize = 51;
    /// CLoopHistory air field 58, enemy close-air-support aircraft lost; old PINNED_CHANNELS mapping.
    pub const ENEMY_CAS_LOST: usize = 58;
    /// CLoopHistory air field 65, enemy transports lost; old air_loss_stats_live PINNED_CHANNELS mapping.
    pub const ENEMY_TRANSPORTS_LOST: usize = 65;
    /// CLoopHistory air field 77, enemy trains lost; old air_loss_stats_live PINNED_CHANNELS mapping.
    pub const ENEMY_TRAINS_LOST: usize = 77;
    /// CLoopHistory air field 78, enemy trucks lost; old air_loss_stats_live PINNED_CHANNELS mapping.
    pub const ENEMY_TRUCKS_LOST: usize = 78;
    /// CLoopHistory air field 79, enemy railway guns lost; old air_loss_stats_live PINNED_CHANNELS mapping.
    pub const ENEMY_RAILWAY_GUNS_LOST: usize = 79;
    /// CLoopHistory air field 80, enemy bombers disrupted; old air_loss_stats_live PINNED_CHANNELS mapping.
    pub const ENEMY_BOMBERS_DISRUPTED: usize = 80;
    /// CLoopHistory air field 81, enemy missiles lost; old air_loss_stats_live PINNED_CHANNELS mapping.
    pub const ENEMY_MISSILES_LOST: usize = 81;
}
