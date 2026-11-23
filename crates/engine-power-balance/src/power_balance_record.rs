
pub const STRIDE: usize = 0x190;
pub const VALID: usize = 0x08;
pub const DEFINITION: usize = 0x10;
pub const LEFT_SIDE: usize = 0x18;
pub const RIGHT_SIDE: usize = 0x20;
pub const TRENDING_SIDE: usize = 0x28;
pub const VALUE: usize = 0x30;
pub const FIXED_POINT_SCALE: i64 = 100_000;
pub const MEMBERS: usize = 0x38;
pub const MEMBER_COUNT: usize = 0x44;
pub const MEMBER_STRIDE: usize = 0x04;
pub const RANGES: usize = 0x68;
pub const RANGE_COUNT: usize = 0x74;
pub const RANGE_STRIDE: usize = 0x08;
pub const ACTIVE_RANGE: usize = 0xb0;
pub const GFX_OVERRIDES: usize = 0x178;
pub const GFX_OVERRIDE_CAPACITY: usize = 0x180;
pub const GFX_OVERRIDE_COUNT: usize = 0x184;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
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
    pub country: u32,
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
    if flags & engine_native_parts::modifier_definition::CLAMP_MAXIMUM != 0 {
        raw.clamp(0, FIXED_POINT_SCALE)
    } else {
        raw
    }
}
