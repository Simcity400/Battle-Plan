//! Command-ability definition. These constants do not establish an hourly
//! world walk or a complete availability evaluator.

/// Primary definition vtable.
pub const VTABLE_RVA: usize = 0x0296_e6c8;
/// Secondary definition vtable.
pub const SECONDARY_VTABLE_RVA: usize = 0x0296_e718;
/// Secondary subobject offset.
pub const SECONDARY_BASE: usize = 0x08;
/// Valid-definition byte tested before setting the subunit unlock marker.
pub const VALID: usize = 0x10;
/// Embedded allowed-condition trigger.
pub const ALLOWED_TRIGGER: usize = 0x18;
/// Allowed-trigger child count, i32.
pub const ALLOWED_CHILD_COUNT: usize = 0x2c;
/// Token string with inline or heap storage.
pub const TOKEN: usize = 0x70;
/// Token length, u64.
pub const TOKEN_LENGTH: usize = 0x80;
/// Token capacity, u64, selecting inline or heap storage.
pub const TOKEN_CAPACITY: usize = 0x88;
/// Target kind, i32; the country-wide value uses no caster or formation match.
pub const TARGET_KIND: usize = 0x90;
/// Country-wide target-kind value.
pub const COUNTRY_TARGET_KIND: i32 = 2;
/// Allocated-cost flag byte selecting persistent cost allocation.
pub const ALLOCATE_COST: usize = 0x94;
/// First command-power cost, signed i64 fixed5.
pub const COMMAND_POWER_COST_A: usize = 0x98;
/// Second command-power cost, signed i64 fixed5; added after scaling.
pub const COMMAND_POWER_COST_B: usize = 0xa0;
/// Duration hours, i32; the indefinite value retains an active record.
pub const DURATION_HOURS: usize = 0xa8;
/// Cooldown hours, i32; values below one remove the record.
pub const COOLDOWN_HOURS: usize = 0xac;
/// Cancelable byte.
pub const CANCELABLE: usize = 0x2e8;
/// Authored deployed-HQ requirement byte.
pub const REQUIRES_DEPLOYED_HQ: usize = 0x2e9;
/// Subunit-unlock clause byte.
pub const REQUIRES_SUB_UNIT_UNLOCK: usize = 0x2ea;
/// Allowed-trigger scope kind, i32, initialized to four.
pub const ALLOWED_SCOPE_KIND: usize = 0x2ec;
/// Fixed-point cost scale for i64 products and divisions.
pub const COST_SCALE: i64 = 100_000;
/// Indefinite duration value.
pub const INDEFINITE_DURATION: i32 = -1;
/// Constructor installing both vtables and the allowed-condition trigger.
pub const CONSTRUCTOR_RVA: usize = 0x00ff_4250;
/// Allowed evaluator applying subunit and trigger clauses.
pub const ALLOWED_RVA: usize = 0x00ff_5c30;
/// Per-key parser for allowed and deployed-HQ requirements.
pub const PARSE_KEY_RVA: usize = 0x00ff_5ec0;
/// Effective command-power cost getter returning signed i64.
pub const EFFECTIVE_COMMAND_POWER_COST_RVA: usize = 0x00ff_4c30;
/// Effective-cost getter entry sentinel.
pub const EFFECTIVE_COMMAND_POWER_COST_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24, 0x20, 0x57,
];
