use super::*;
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct NavalAccidentReportRow {
    pub country: u32,
    pub ordinal: u32,
    pub kind: u8,
    pub mine_owner: u32,
    pub region: u32,
    pub equipment: u64,
    pub damage: i64,
    pub sunk: u8,
    pub ship: u64,
    pub owner: u32,
    pub expire_hour: i32,
}
typed_codec!(NavalAccidentReportRow, 65, |row| (row.country as u128) << 33 | (row.kind as u128) << 32 | row.ordinal as u128,
 |key| Self { country: (key >> 33) as u32, kind: ((key >> 32) & 1) as u8, ordinal: key as u32, ..Default::default() },
 { 0 => (region): U32, 1 => (equipment): U64, 2 => (damage): I64, 3 => (sunk): U8, 4 => (ship): U64, 5 => (owner): U32, 6 => (expire_hour): I32, 7 => (mine_owner): U32 }, |row| if row.kind > 1 { Err(invalid("unknown naval incident report family")) } else { Ok(()) });
