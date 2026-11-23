
use recorder_derive_framework::framework::{Hour, columns};
use replay_columnar::store::TableView;
use replay_data::{
    format::FormatVersion,
    rows::{CountryLinkRow, CountryRow, RowCodec, SunkConvoyRow, SunkShipEntry, SunkShipsRow, TableId},
};
use std::io;

pub(super) fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

pub(super) const COUNTRY_SLOT: [usize; 1] = columns::<CountryRow, 1>(["canonical_slot"]);
pub(super) const LINK_SLOT: [usize; 1] = columns::<CountryLinkRow, 1>(["canonical_slot"]);
pub(super) const CONVOY_COUNT: [usize; 1] = columns::<SunkConvoyRow, 1>(["convoys"]);

pub(super) fn slotted(view: &TableView<'_>, slot: u32, column: usize) -> Option<(u32, u32)> {
    Some((view.key(slot)? as u32, view.number(column, slot)? as u32))
}

pub(super) fn sunk_ships(hour: &Hour<'_>) -> io::Result<Vec<SunkShipEntry>> {
    let view = hour.table(TableId::SunkShips);
    let Some(slot) = view.rows().next() else { return Ok(Vec::new()) };
    let key = view.key(slot).ok_or_else(|| invalid("SunkShips holds no row in its slot"))?;
    let format = FormatVersion::CURRENT.version();
    Ok(SunkShipsRow::from_wire_fields(key, u64::MAX, &mut |column| replay_frame::frame::field(&view, slot, column, format))?.entries)
}

pub(super) fn convoy(view: &TableView<'_>, slot: u32) -> io::Result<(u128, u32, u32, i32)> {
    let key = view.key(slot).ok_or_else(|| invalid(format!("SunkConvoys slot {slot} holds no row")))?;
    let count = view.number(CONVOY_COUNT[0], slot).ok_or_else(|| invalid(format!("SunkConvoys slot {slot} does not hold its count")))?;
    Ok((key, (key >> 32) as u32, key as u32, count as i32))
}
