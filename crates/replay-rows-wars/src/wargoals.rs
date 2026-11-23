use super::*;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CountryWargoalRow {
    pub country: u32,
    pub ordinal: u32,
    pub handle: u64,
    pub actor: u32,
    pub recipient: u32,
    pub goal_type: u32,
    pub name_token: u32,
    pub expires: i32,
}
typed_codec!(CountryWargoalRow, 64, |r| (r.country as u128) << 32 | r.ordinal as u128,
 |key| Self { country: (key >> 32) as u32, ordinal: key as u32, ..Default::default() },
 { 0 => (handle): U64, 1 => (actor): U32, 2 => (recipient): U32, 3 => (goal_type): U32, 4 => (expires): I32, 5 => (name_token): U32 }, |_r| Ok(()));

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct WargoalJustificationRow {
    pub country: u32,
    pub ordinal: u32,
    pub actor: u32,
    pub recipient: u32,
    pub goal_type: u32,
    pub total_cost_raw: i64,
    pub daily_cost_raw: i64,
    pub progress_raw: i64,
    pub start: i32,
    pub end: i32,
    pub pay_upfront: u8,
    pub threat_raw: i64,
}
typed_codec!(WargoalJustificationRow, 64, |r| (r.country as u128) << 32 | r.ordinal as u128,
 |key| Self { country: (key >> 32) as u32, ordinal: key as u32, ..Default::default() },
 { 0 => (actor): U32, 1 => (recipient): U32, 2 => (goal_type): U32, 3 => (total_cost_raw): I64, 4 => (daily_cost_raw): I64, 5 => (progress_raw): I64, 6 => (start): I32, 7 => (end): I32, 8 => (pay_upfront): U8, 9 => (threat_raw): I64 }, |_r| Ok(()));

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct WargoalTargetRow {
    pub country: u32,
    pub kind: u8,
    pub goal: u32,
    pub ordinal: u32,
    pub value: u64,
}
typed_codec!(WargoalTargetRow, 104,
|r| (r.country as u128) << 72 | (r.kind as u128) << 64 | (r.goal as u128) << 32 | r.ordinal as u128,
|key| Self { country: (key >> 72) as u32, kind: (key >> 64) as u8, goal: (key >> 32) as u32, ordinal: key as u32, ..Default::default() },
{ 0 => (value): U64 }, |r| {
   if r.kind > 4 || (r.kind != 3 && r.value > u32::MAX as u64) || (r.kind == 3 && r.goal != 0) {
       return Err(invalid("invalid war-goal target kind or value"));
   }
   Ok(())
});

impl WargoalJustificationRow {
    pub const INVALID_HOUR: i32 = 43_791_240;
    pub fn completion_hour(&self, hour: i32) -> i32 {
        if self.end != Self::INVALID_HOUR {
            return self.end;
        }
        let mut days = i32::MAX;
        if self.daily_cost_raw > 0 {
            let remaining = self.total_cost_raw.wrapping_sub(self.progress_raw);
            let quotient =
                if (remaining as u64).wrapping_add(0x7fff_ffff_ffff_fffe) < 0xffff_ffff_ffff_fffd {
                    remaining
                        .wrapping_mul(100_000)
                        .wrapping_div(self.daily_cost_raw)
                } else {
                    u32::MAX as i64
                };
            days = (quotient.wrapping_add(if quotient < 0 { -50_000 } else { 50_000 }) / 100_000)
                as i32;
        }
        hour.wrapping_add(days.wrapping_mul(24))
    }
}

impl CountryWargoalRow {
    pub fn serialized_expiry(&self) -> Option<i32> {
        const CALENDAR_ORIGIN: i32 = 43_800_000;
        const LAST_OMITTED_DAY_HOUR: i32 = 17_519;
        (self.expires.wrapping_sub(CALENDAR_ORIGIN) > LAST_OMITTED_DAY_HOUR).then_some(self.expires)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wargoal_rows_round_trip_full_handles_target_kinds_and_completion_rounding() {
        let goal = CountryWargoalRow {
            country: 5000,
            ordinal: 99,
            handle: u64::MAX,
            actor: 5000,
            recipient: 4999,
            goal_type: u32::MAX,
            name_token: 13,
            expires: 43820000,
        };
        replay_rows::rows::round_trip(&goal);
        for (kind, value) in [
            (0, u32::MAX as u64),
            (1, 5000),
            (2, 4999),
            (3, u64::MAX),
            (4, 42),
        ] {
            replay_rows::rows::round_trip(&WargoalTargetRow {
                country: 5000,
                kind,
                goal: if kind == 3 { 0 } else { 99 },
                ordinal: 88,
                value,
            });
        }
        let mut progress = WargoalJustificationRow {
            country: 5000,
            total_cost_raw: 9000000,
            daily_cost_raw: 100000,
            progress_raw: 1750000,
            end: 43791240,
            ..Default::default()
        };
        replay_rows::rows::round_trip(&progress);
        assert_eq!(progress.completion_hour(100), 100 + 73 * 24);
        progress.end = 80;
        assert_eq!(progress.completion_hour(100), 80);
    }
}
