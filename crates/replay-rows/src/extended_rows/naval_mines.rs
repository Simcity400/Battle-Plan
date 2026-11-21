use super::*;

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct NavalRegionalMineRow {
    pub country: u32,
    pub kind: u8,
    pub region: u32,
    pub amount: i64,
}
typed_codec!(NavalRegionalMineRow, 65, |row| (row.country as u128) << 33 | (row.kind as u128) << 32 | row.region as u128,
 |key| Self { country: (key >> 33) as u32, kind: ((key >> 32) & 1) as u8, region: key as u32, ..Default::default() },
 { 0 => (amount): I64 }, |row| if row.kind > 1 || (row.kind == 0 && row.amount != 0) || (row.kind == 1 && row.amount == 0) { Err(invalid("invalid sparse regional mine row")) } else { Ok(()) });
