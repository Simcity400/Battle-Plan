//! Per-leader entry in a selected field-marshal ability cache; exact engine name unproven.
//!
//! Source: `extractor/native/lanes/active_command_abilities_live/src/a729_hook.rs`.
//! Presentation data only. The old row's activation_elapsed_hours=-1 is a collector
//! placeholder with no read offset and is deliberately not made an engine field.

/// Per-leader record stride; old parse_field_marshal_states reads chunks of this size (source:101,2583).
pub const STRIDE: usize = 0x28;
/// Per-leader record unit-leader pointer; old parser rejects null identity (source:2588-2591).
pub const LEADER: usize = 0;
/// Per-leader state code, u8; old parser accepts 0..4, later interprets 1 active and 2 cooldown (source:2584-2587,2272-2273).
pub const STATE: usize = 0x08;
/// Per-leader hours remaining, native i32; confirmed by old parser's signed read (source:2595).
pub const REMAINING_HOURS: usize = 0x0c;
/// Per-leader command-power cost, native i64 /100000; confirmed by old parser's read (source:2597).
pub const COMMAND_POWER_COST: usize = 0x10;
/// Per-leader activation command-power cost, native i64 /100000; confirmed by old parser's read (source:2598).
pub const ACTIVATION_COMMAND_POWER_COST: usize = 0x18;
/// Per-leader allocation decision, u8 interpreted as nonzero; old parser calls it will_activate (source:2599-2602).
pub const WILL_ACTIVATE: usize = 0x20;
