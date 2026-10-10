//! Selected-controller field-marshal ability aggregate cache, separate from
//! the country's persistent active-ability record. Costs retain signed fixed5 values.

/// Aggregate record stride.
pub const STRIDE: usize = 0x68;
/// Aggregate ability-definition pointer.
pub const DEFINITION: usize = 0;
/// Aggregate per-leader-state vector data pointer.
pub const LEADERS: usize = 0x08;
/// Per-leader vector capacity, u32; must cover its count.
pub const LEADER_CAPACITY: usize = 0x10;
/// Aggregate per-leader-state vector count, u32.
pub const LEADER_COUNT: usize = 0x14;
/// Aggregate available leader count, u32.
pub const AVAILABLE_COUNT: usize = 0x20;
/// Aggregate active leader count, u32.
pub const ACTIVE_COUNT: usize = 0x24;
/// Aggregate cooldown leader count, u32.
pub const COOLDOWN_COUNT: usize = 0x28;
/// Aggregate allocated leader count, u32.
pub const ALLOCATED_COUNT: usize = 0x2c;
/// Aggregate candidate leader count, u32.
pub const CANDIDATE_LEADER_COUNT: usize = 0x30;
/// Aggregate definition-allowed leader count, u32.
pub const DEFINITION_ALLOWED_LEADER_COUNT: usize = 0x34;
/// Aggregate minimum active hours remaining, i32.
pub const MINIMUM_ACTIVE_REMAINING_HOURS: usize = 0x38;
/// Aggregate minimum cooldown hours remaining, i32.
pub const MINIMUM_COOLDOWN_REMAINING_HOURS: usize = 0x3c;
/// Aggregate group activation elapsed hours, i32.
pub const GROUP_ACTIVATION_ELAPSED_HOURS: usize = 0x40;
/// Aggregate allocated command-power cost, i64 /100000.
pub const ALLOCATED_COMMAND_POWER_COST: usize = 0x48;
/// Aggregate allocated activation cost, i64 /100000.
pub const ALLOCATED_ACTIVATION_COMMAND_POWER_COST: usize = 0x50;
/// Aggregate allocated daily command-power cost, i64 /100000.
pub const ALLOCATED_DAILY_COMMAND_POWER_COST: usize = 0x58;
/// Aggregate allocated affected division count, u32.
pub const ALLOCATED_AFFECTED_DIVISION_COUNT: usize = 0x60;
/// Aggregate group-already-active flag, u8 interpreted as nonzero.
pub const GROUP_ALREADY_ACTIVE: usize = 0x64;
