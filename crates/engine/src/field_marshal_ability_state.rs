//! GUI field-marshal ability aggregate record; exact engine type name unproven.
//!
//! Source: `extractor/native/lanes/active_command_abilities_live/src/a729_hook.rs`.
//! This 0x68-byte selected-controller cache is separate from the country's 0x58-byte
//! persistent active-ability record. Costs retain the native signed i64 /100000 value.

/// Aggregate record stride; stable_snapshot and parse_field_marshal_states use it (source:100,2426,2551).
pub const STRIDE: usize = 0x68;
/// Aggregate ability-definition pointer; old parse_field_marshal_states reads it first (source:2552).
pub const DEFINITION: usize = 0;
/// Aggregate per-leader-state vector data pointer; old parser follows 0x28-byte records (source:2553,2576).
pub const LEADERS: usize = 0x08;
/// Aggregate per-leader-state vector capacity, u32; old parser compares it with count (source:2554).
pub const LEADER_CAPACITY: usize = 0x10;
/// Aggregate per-leader-state vector count, u32; old parser reads this many records (source:2555).
pub const LEADER_COUNT: usize = 0x14;
/// Aggregate available leader count, u32; confirmed by old parser's field read (source:2608).
pub const AVAILABLE_COUNT: usize = 0x20;
/// Aggregate active leader count, u32; confirmed by old parser's field read (source:2609).
pub const ACTIVE_COUNT: usize = 0x24;
/// Aggregate cooldown leader count, u32; confirmed by old parser's field read (source:2610).
pub const COOLDOWN_COUNT: usize = 0x28;
/// Aggregate allocated leader count, u32; confirmed by old parser's field read (source:2611).
pub const ALLOCATED_COUNT: usize = 0x2c;
/// Aggregate candidate leader count, u32; confirmed by old parser's field read (source:2612).
pub const CANDIDATE_LEADER_COUNT: usize = 0x30;
/// Aggregate definition-allowed leader count, u32; confirmed by old parser's field read (source:2613).
pub const DEFINITION_ALLOWED_LEADER_COUNT: usize = 0x34;
/// Aggregate minimum active hours remaining, i32; old parser preserves signed timer representation (source:2614).
pub const MINIMUM_ACTIVE_REMAINING_HOURS: usize = 0x38;
/// Aggregate minimum cooldown hours remaining, i32; old parser preserves signed timer representation (source:2615).
pub const MINIMUM_COOLDOWN_REMAINING_HOURS: usize = 0x3c;
/// Aggregate group activation elapsed hours, i32; confirmed by old parser's signed read (source:2616).
pub const GROUP_ACTIVATION_ELAPSED_HOURS: usize = 0x40;
/// Aggregate allocated command-power cost, i64 /100000; confirmed by old parser's native read (source:2617).
pub const ALLOCATED_COMMAND_POWER_COST: usize = 0x48;
/// Aggregate allocated activation cost, i64 /100000; confirmed by old parser's native read (source:2618).
pub const ALLOCATED_ACTIVATION_COMMAND_POWER_COST: usize = 0x50;
/// Aggregate allocated daily command-power cost, i64 /100000; confirmed by old parser's native read (source:2619).
pub const ALLOCATED_DAILY_COMMAND_POWER_COST: usize = 0x58;
/// Aggregate allocated affected division count, u32; confirmed by old parser's field read (source:2620).
pub const ALLOCATED_AFFECTED_DIVISION_COUNT: usize = 0x60;
/// Aggregate group-already-active flag, u8 interpreted as nonzero; confirmed by old parser (source:2621-2624).
pub const GROUP_ALREADY_ACTIVE: usize = 0x64;
