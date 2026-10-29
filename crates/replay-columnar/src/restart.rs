//! A restart: a table's whole state at a block's first hour, decodable with nothing
//! earlier, so a walker can start at any block.
//!
//! Layout: status | acquisition (last read hour, availability start, refusal reason) |
//! slot count (varint) | live bits (one per slot) | the live rows' keys in slot order
//! ([`Writer::keys`]) | every column's values for the live slots in slot order, as a run
//! carries them | every column's unrecorded live slots ([`Writer::slots`]) | for a
//! truncated table, the slots its view holds.

use crate::{
    leaf::{Reader, Writer},
    manifest::Status,
    section::{Scratch, read_values},
    state::{Bits, Kind, Table},
};
use std::io;

#[cfg(test)]
mod tests;

/// What a table's reads had established when the block began.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Acquisition {
    /// Its sample hour: the hour of the acquisition its view holds, none when that is not
    /// known (never read, or an untimed snapshot).
    pub last_read: Option<i32>,
    /// The first hour of its current availability.
    pub available_from: Option<i32>,
    /// Why it is not available, in the dictionary.
    pub refusal: Option<u32>,
}

fn put_hour(writer: &mut Writer, hour: Option<i32>) -> io::Result<()> {
    writer.varint(hour.map_or(0, |hour| u64::from(((hour << 1) ^ (hour >> 31)) as u32) + 1))
}

fn get_hour(reader: &mut Reader<'_>) -> io::Result<Option<i32>> {
    Ok(match reader.varint()? {
        0 => None,
        value => {
            let value = u32::try_from(value - 1)
                .map_err(|_| crate::invalid("Restart hour exceeds 32 bits"))?;
            Some(((value >> 1) as i32) ^ -((value & 1) as i32))
        }
    })
}

/// `view` holds the slots of a truncated table's rows read (none otherwise).
pub fn write(
    table: &Table,
    status: Status,
    acquisition: &Acquisition,
    view: &[u32],
) -> io::Result<Vec<u8>> {
    let truncated = matches!(status, Status::Truncated { .. });
    if !truncated && !view.is_empty() {
        return Err(crate::invalid(
            "Only a truncated table has a view of read rows",
        ));
    }
    if view.iter().any(|&slot| !table.live().get(slot)) {
        return Err(crate::invalid(
            "A truncated view names a slot holding no row",
        ));
    }
    let mut writer = Writer::new();
    status.write(&mut writer)?;
    put_hour(&mut writer, acquisition.last_read)?;
    put_hour(&mut writer, acquisition.available_from)?;
    writer.varint(
        acquisition
            .refusal
            .map_or(0, |reason| u64::from(reason) + 1),
    )?;
    writer.varint(table.slots() as u64)?;
    let mut live = engine_allocation::filled(table.slots().div_ceil(64), 0u64)?;
    let slots = engine_allocation::collect_vec(table.live().ones())?;
    for &slot in &slots {
        live[slot as usize / 64] |= 1 << (slot % 64);
    }
    writer.values(&live)?;
    let keys = engine_allocation::collect_vec(
        slots
            .iter()
            .map(|&slot| table.key(slot).expect("live slot")),
    )?;
    writer.keys(&keys)?;
    table.write_columns(&mut writer, &slots)?;
    // A column's unrecorded slots are live slots, so one reservation holds every column's.
    let mut unrecorded = engine_allocation::vec(slots.len())?;
    for column in 0..table.kinds().count() {
        unrecorded.clear();
        unrecorded.extend(table.unrecorded_slots(column));
        writer.slots(&unrecorded)?;
    }
    if truncated {
        writer.slots(view)?;
    }
    Ok(writer.into_bytes())
}

/// A table as a restart holds it.
#[derive(Debug)]
pub struct Restored {
    pub table: Table,
    pub status: Status,
    pub acquisition: Acquisition,
    /// The slots a truncated table's view holds.
    pub view: Vec<u32>,
}

pub fn read(bytes: &[u8], kinds: &[Kind], scratch: &mut Scratch) -> io::Result<Restored> {
    let mut reader = Reader::new(bytes);
    let status = Status::read(&mut reader)?;
    let last_read = get_hour(&mut reader)?;
    let available_from = get_hour(&mut reader)?;
    let refusal = match reader.varint()? {
        0 => None,
        reason => Some(
            u32::try_from(reason - 1)
                .map_err(|_| crate::invalid("Restart refusal exceeds 32 bits"))?,
        ),
    };
    let slots = reader.count()?;
    let mut live: Vec<u64> = Vec::new();
    reader.values(slots.div_ceil(64), &mut live)?;
    let live = Bits::from_words(live)?;
    let mut keys = Vec::new();
    reader.keys(live.count(), &mut keys)?;
    let mut table = Table::with_rows(kinds, slots, live, &keys)?;
    let live_slots = engine_allocation::collect_vec(table.live().ones())?;
    for (column, &kind) in kinds.iter().enumerate() {
        let values = read_values(&mut reader, scratch, kind, live_slots.len())?;
        table.set(column, &live_slots, values)?;
    }
    let mut unrecorded = Vec::new();
    for column in 0..kinds.len() {
        reader.slots(&mut unrecorded)?;
        table.unrecord(column, &unrecorded)?;
    }
    let mut view = Vec::new();
    if matches!(status, Status::Truncated { .. }) {
        reader.slots(&mut view)?;
        if view.iter().any(|&slot| !table.live().get(slot)) {
            return Err(crate::invalid(
                "A truncated view names a slot holding no row",
            ));
        }
    }
    reader.finish()?;
    Ok(Restored {
        table,
        status,
        acquisition: Acquisition {
            last_read,
            available_from,
            refusal,
        },
        view,
    })
}
