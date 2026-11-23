
use recorder_derive_framework::framework::{Hour, columns};
use replay_data::{
    format::FormatVersion,
    rows::{LandCombatDetailsRow, LandCombatResultsRow, NavalCombatDetailsRow, NavalCombatResultDetailsRow, NavalResultRow, NavalResultSourceRow, RowCodec, TableId},
};
use std::io;

pub(super) fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

pub(super) const LAND_SIDES: [usize; 1] = columns::<LandCombatDetailsRow, 1>(["sides"]);
pub(super) const NAVAL_SIDES: [usize; 1] = columns::<NavalCombatDetailsRow, 1>(["sides"]);
pub(super) const WINNER: [usize; 1] = columns::<NavalResultRow, 1>(["winning_side"]);
const LAND_RESULT: [usize; 4] = columns::<LandCombatResultsRow, 4>(["combat", "end_hour", "winners", "sides"]);

fn mask(columns: &[usize]) -> u64 {
    columns.iter().fold(0, |mask, column| mask | 1 << column)
}

fn row<R: RowCodec>(hour: &Hour<'_>, table: TableId, slot: u32, mask: u64) -> io::Result<R> {
    let held = hour.table(table);
    let key = held.key(slot).ok_or_else(|| invalid(format!("{table:?} slot {slot} holds no row")))?;
    let format = FormatVersion::CURRENT.version();
    R::from_wire_fields(key, mask, &mut |column| replay_frame::frame::field(&held, slot, column, format))
}

pub(super) fn commanders(hour: &Hour<'_>, naval: bool, slot: u32) -> io::Result<[u64; 2]> {
    Ok(if naval {
        let row: NavalCombatDetailsRow = row(hour, TableId::NavalCombatDetails, slot, mask(&NAVAL_SIDES))?;
        row.sides.map(|side| side.commander)
    } else {
        let row: LandCombatDetailsRow = row(hour, TableId::LandCombatDetails, slot, mask(&LAND_SIDES))?;
        row.sides.map(|side| side.commander)
    })
}

pub(super) struct LandResult {
    pub combat: u64,
    pub hour: i32,
    pub winners: u8,
    pub commanders: [u64; 2],
}

pub(super) fn land_result(hour: &Hour<'_>, slot: u32) -> io::Result<LandResult> {
    let row: LandCombatResultsRow = row(hour, TableId::LandCombatResults, slot, mask(&LAND_RESULT))?;
    Ok(LandResult { combat: row.combat, hour: row.end_hour, winners: row.winners, commanders: row.sides.map(|side| side.commander) })
}

pub(super) fn naval_source(hour: &Hour<'_>, slot: u32) -> io::Result<NavalResultSourceRow> {
    row(hour, TableId::NavalResultSources, slot, u64::MAX)
}

pub(super) fn naval_winner(hour: &Hour<'_>, result: u64) -> Option<Option<u8>> {
    if !hour.complete(TableId::NavalResults) {
        return None;
    }
    let view = hour.table(TableId::NavalResults);
    let slot = view.slot(u128::from(result))?;
    Some(view.number(WINNER[0], slot).map(|side| side as u8))
}

pub(super) fn naval_result_leaders(hour: &Hour<'_>, result: u64) -> io::Result<[u64; 2]> {
    if !hour.complete(TableId::NavalCombatResultDetails) {
        return Ok([0; 2]);
    }
    let view = hour.table(TableId::NavalCombatResultDetails);
    let Some(slot) = view.slot(u128::from(result)) else { return Ok([0; 2]) };
    let row: NavalCombatResultDetailsRow = row(hour, TableId::NavalCombatResultDetails, slot, u64::MAX)?;
    Ok(row.sides.map(|side| side.leader))
}
