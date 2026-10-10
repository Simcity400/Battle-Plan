//! Ability-database functions; the concrete receiver type is unknown.

/// Subunit unlock initialization; looks up subunit names and sets each ability unlock flag.
pub const INITIALIZE_SUBUNIT_UNLOCKS_RVA: usize = 0x0060_71a0;
/// Ability lookup by subunit name.
pub const LOOKUP_RVA: usize = 0x0060_6930;
