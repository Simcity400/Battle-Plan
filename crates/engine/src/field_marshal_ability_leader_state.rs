//! Per-leader selected field-marshal ability cache.
//! Presentation-only. A missing activation-elapsed operand supplies no engine field.

/// Per-leader record stride.
pub const STRIDE: usize = 0x28;
/// Unit-leader pointer; null does not authenticate a leader.
pub const LEADER: usize = 0;
/// State code, u8; zero through four, with one active and two cooldown.
pub const STATE: usize = 0x08;
/// Per-leader hours remaining, native i32.
pub const REMAINING_HOURS: usize = 0x0c;
/// Per-leader command-power cost, native i64 /100000.
pub const COMMAND_POWER_COST: usize = 0x10;
/// Per-leader activation command-power cost, native i64 /100000.
pub const ACTIVATION_COMMAND_POWER_COST: usize = 0x18;
/// Per-leader allocation decision, u8 interpreted as nonzero.
pub const WILL_ACTIVATE: usize = 0x20;
