use super::*;

#[cfg(any(test, feature = "test-support"))]
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CountryExperienceUnsayableRow {
    pub country: u32,
    pub army_raw: Recorded<i64>,
    pub army_daily_raw: Recorded<i64>,
    pub army_daily_training_raw: Recorded<i64>,
    pub navy_raw: Recorded<i64>,
    pub navy_daily_raw: Recorded<i64>,
    pub navy_daily_training_raw: Recorded<i64>,
    pub air_raw: Recorded<i64>,
    pub air_daily_raw: Recorded<i64>,
    pub armies_for_training_raw: Recorded<i64>,
}

#[cfg(any(test, feature = "test-support"))]
row_codec!(CountryExperienceUnsayableRow, 32, |row| row.country as u128, |key| { country: key as u32 }, {0 => army_raw: I64, 1 => army_daily_raw: I64, 2 => army_daily_training_raw: I64, 3 => navy_raw: I64, 4 => navy_daily_raw: I64, 5 => navy_daily_training_raw: I64, 6 => air_raw: I64, 7 => air_daily_raw: I64, 8 => armies_for_training_raw: I64}, |_row| Ok(()));
