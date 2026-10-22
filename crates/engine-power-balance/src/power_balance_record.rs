//! Inline power-balance state record with raw country-reference membership.
//! The owner supplies the observation boundary; record reads do not retry.

/// Power-balance record stride.
pub const STRIDE: usize = 0x190;
/// Validity byte required to be nonzero.
pub const VALID: usize = 0x08;
/// Definition pointer.
pub const DEFINITION: usize = 0x10;
/// Left-side definition pointer.
pub const LEFT_SIDE: usize = 0x18;
/// Right-side definition pointer.
pub const RIGHT_SIDE: usize = 0x20;
/// Trending-side pointer, updated independently of value sign.
pub const TRENDING_SIDE: usize = 0x28;
/// Signed i64 fixed-point balance value.
pub const VALUE: usize = 0x30;
/// Fixed-point unit and positive endpoint.
pub const FIXED_POINT_SCALE: i64 = 100_000;
/// Raw u32 country-reference member array.
pub const MEMBERS: usize = 0x38;
/// Signed member count.
pub const MEMBER_COUNT: usize = 0x44;
/// Member-entry width.
pub const MEMBER_STRIDE: usize = 0x04;
/// Range-pointer array.
pub const RANGES: usize = 0x68;
/// Signed range-pointer count.
pub const RANGE_COUNT: usize = 0x74;
/// Range-pointer entry width.
pub const RANGE_STRIDE: usize = 0x08;
/// Active-range pointer.
pub const ACTIVE_RANGE: usize = 0xb0;
/// Side-sprite override array.
pub const GFX_OVERRIDES: usize = 0x178;
/// Signed override capacity.
pub const GFX_OVERRIDE_CAPACITY: usize = 0x180;
/// Signed override count.
pub const GFX_OVERRIDE_COUNT: usize = 0x184;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    /// The engine's stored manager position, not a synthetic persistent handle.
    pub ordinal: u32,
    pub value_raw: i64,
    pub balance: crate::power_balance_definition::Token,
    pub left: crate::power_balance_definition::Token,
    pub right: crate::power_balance_definition::Token,
    pub trending: crate::power_balance_definition::Token,
    pub active_range: crate::power_balance_definition::Token,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MemberRow {
    pub record: u32,
    pub ordinal: u32,
    /// Exact raw reference in the member array; aliases are not replaced.
    pub country: u32,
    /// Stored modifier integers, before getter flags are applied.
    pub daily_raw: i64,
    pub weekly_raw: i64,
    pub daily_present: bool,
    pub weekly_present: bool,
    pub daily_flags: u32,
    pub weekly_flags: u32,
}

impl MemberRow {
    pub fn daily_effective(self) -> i64 {
        apply_flags(self.daily_raw, self.daily_flags)
    }

    pub fn weekly_effective(self) -> i64 {
        apply_flags(self.weekly_raw, self.weekly_flags)
    }
}

fn apply_flags(raw: i64, flags: u32) -> i64 {
    if flags & crate::modifier_definition::CLAMP_MAXIMUM != 0 {
        raw.clamp(0, FIXED_POINT_SCALE)
    } else {
        raw
    }
}
