//! A restart: a table's whole state at a block's first hour, decodable with nothing
//! earlier, so a walker can start at any block.
//!
//! Layout: status | acquisition (last read hour, availability start, refusal reason) |
//! the table ([`write_table`]: slot count (varint) | live bits (one per slot) | the live rows'
//! keys in slot order ([`Writer::keys`]) | every column's values for the live slots in slot
//! order, as a run carries them | every column's unrecorded live slots ([`Writer::slots`])) |
//! for a truncated table, the slots its view holds | its last complete read ([`Last`]): 0 when
//! there is none or the view is it, read at its last read hour; 1 and the read's hour when the
//! table's rows are it; 2, the read's hour and its table when reads since changed the rows.

use crate::{
    leaf::{Reader, Writer},
    manifest::Status,
    section::{Scratch, read_values},
    state::{Bits, Kind, Table, WeakTable},
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

/// A kept table's last complete read, while its view may not be it: what a walk starting at
/// a restart knows of the table without walking the hours before.
#[derive(Clone, Debug, PartialEq)]
pub enum Last {
    /// The table's rows are the read, made at the hour when known: no hour since changed them.
    Held(Option<i32>),
    /// A partial read since changed the rows: the read's table apart, and its hour when known.
    Apart(Box<Table>, Option<i32>),
}

impl Last {
    pub fn read(&self) -> Option<i32> {
        match self {
            Self::Held(read) | Self::Apart(_, read) => *read,
        }
    }

    /// The bytes its own table holds.
    pub fn held_bytes(&self) -> usize {
        match self {
            Self::Held(_) => 0,
            Self::Apart(table, _) => table.held_bytes(),
        }
    }

    /// Before an hour of `status` applies to `table`, the rows a partial read is about to
    /// change are kept apart while they are the last complete read: sharing the table's
    /// pages, so only those the hour writes are copied.
    pub fn before(last: &mut Option<Self>, table: &Table, status: Status) -> io::Result<()> {
        if let (Status::Truncated { .. }, Some(Self::Held(read))) = (status, &*last) {
            let read = *read;
            *last = Some(Self::Apart(engine_allocation::boxed(table.share()?)?, read));
        }
        Ok(())
    }

    /// A copy sharing its table's pages.
    pub fn share(&self) -> io::Result<Self> {
        Ok(match self {
            Self::Held(read) => Self::Held(*read),
            Self::Apart(table, read) => {
                Self::Apart(engine_allocation::boxed(table.share()?)?, *read)
            }
        })
    }

    /// After an hour of `status`, which wrote a section when `sectioned`, left the view
    /// sampled at `sampled`: a complete view is the last complete read, at its sample hour or,
    /// when it changed no row, at the hour of the read it still holds.
    pub fn after(last: &mut Option<Self>, status: Status, sectioned: bool, sampled: Option<i32>) {
        if status != Status::Complete {
            return;
        }
        let kept = match last {
            Some(Self::Held(read)) if !sectioned => *read,
            _ => None,
        };
        *last = Some(Self::Held(sampled.or(kept)));
    }
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

/// `table`'s slots, live rows, keys, values and unrecorded cells.
fn write_table(writer: &mut Writer, table: &Table) -> io::Result<()> {
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
    table.write_columns(writer, &slots)?;
    // A column's unrecorded slots are live slots, so one reservation holds every column's.
    let mut unrecorded = engine_allocation::vec(slots.len())?;
    for column in 0..table.kinds().count() {
        unrecorded.clear();
        unrecorded.extend(table.unrecorded_slots(column));
        writer.slots(&unrecorded)?;
    }
    Ok(())
}

/// What [`write_table`] wrote.
fn read_table(reader: &mut Reader<'_>, kinds: &[Kind], scratch: &mut Scratch) -> io::Result<Table> {
    let slots = reader.count()?;
    let mut live: Vec<u64> = Vec::new();
    reader.values(slots.div_ceil(64), &mut live)?;
    let live = Bits::from_words(live)?;
    let mut keys = Vec::new();
    reader.keys(live.count(), &mut keys)?;
    let mut table = Table::with_rows(kinds, slots, live, &keys)?;
    let live_slots = engine_allocation::collect_vec(table.live().ones())?;
    for (column, &kind) in kinds.iter().enumerate() {
        let values = read_values(reader, scratch, kind, live_slots.len())?;
        table.set(column, &live_slots, values)?;
    }
    let mut unrecorded = Vec::new();
    for column in 0..kinds.len() {
        reader.slots(&mut unrecorded)?;
        table.unrecord(column, &unrecorded)?;
    }
    Ok(table)
}

/// `view` holds the slots of a truncated table's rows read (none otherwise).
pub fn write(
    table: &Table,
    status: Status,
    acquisition: &Acquisition,
    view: &[u32],
) -> io::Result<Vec<u8>> {
    write_with(table, status, acquisition, view, None)
}

/// [`write`], with the table's last complete read when it is kept.
pub fn write_with(
    table: &Table,
    status: Status,
    acquisition: &Acquisition,
    view: &[u32],
    last: Option<&Last>,
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
    if status == Status::Complete && !matches!(last, None | Some(Last::Held(_))) {
        return Err(crate::invalid(
            "A complete table's rows are its last complete read",
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
    write_table(&mut writer, table)?;
    if truncated {
        writer.slots(view)?;
    }
    match last {
        // A complete view sampled at its read is that read.
        Some(Last::Held(read)) if status == Status::Complete && *read == acquisition.last_read => {
            writer.varint(0)?
        }
        None => writer.varint(0)?,
        Some(Last::Held(read)) => {
            writer.varint(1)?;
            put_hour(&mut writer, *read)?;
        }
        Some(Last::Apart(apart, read)) => {
            if !apart.kinds().eq(table.kinds()) {
                return Err(crate::invalid(
                    "A last complete read's columns differ from its table's",
                ));
            }
            writer.varint(2)?;
            put_hour(&mut writer, *read)?;
            write_table(&mut writer, apart)?;
        }
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
    /// Its last complete read: the view itself when complete, else what the restart kept.
    pub last: Option<Last>,
}

/// A [`Restored`] whose tables' pages are held weakly: what walks starting at one restart
/// find of it again while one of them still shares its pages.
#[derive(Debug)]
pub struct WeakRestored {
    table: WeakTable,
    status: Status,
    acquisition: Acquisition,
    view: Vec<u32>,
    last: Option<(Option<WeakTable>, Option<i32>)>,
}

impl Restored {
    /// A copy sharing its tables' pages.
    pub fn share(&self) -> io::Result<Self> {
        Ok(Self {
            table: self.table.share()?,
            status: self.status,
            acquisition: self.acquisition,
            view: engine_allocation::copy_vec(&self.view)?,
            last: self.last.as_ref().map(Last::share).transpose()?,
        })
    }

    pub fn downgrade(&self) -> io::Result<WeakRestored> {
        Ok(WeakRestored {
            table: self.table.downgrade()?,
            status: self.status,
            acquisition: self.acquisition,
            view: engine_allocation::copy_vec(&self.view)?,
            last: match &self.last {
                None => None,
                Some(Last::Held(read)) => Some((None, *read)),
                Some(Last::Apart(table, read)) => Some((Some(table.downgrade()?), *read)),
            },
        })
    }
}

impl WeakRestored {
    /// The restored table again, sharing its pages, while every one is still held.
    pub fn upgrade(&self) -> io::Result<Option<Restored>> {
        let Some(table) = self.table.upgrade()? else {
            return Ok(None);
        };
        let last = match &self.last {
            None => None,
            Some((None, read)) => Some(Last::Held(*read)),
            Some((Some(apart), read)) => match apart.upgrade()? {
                Some(apart) => Some(Last::Apart(engine_allocation::boxed(apart)?, *read)),
                None => return Ok(None),
            },
        };
        Ok(Some(Restored {
            table,
            status: self.status,
            acquisition: self.acquisition,
            view: engine_allocation::copy_vec(&self.view)?,
            last,
        }))
    }

    /// Whether some of its pages may still be held.
    pub fn alive(&self) -> bool {
        self.table.alive()
    }

    /// The bytes it holds of its own.
    pub fn held_bytes(&self) -> usize {
        self.table.held_bytes()
            + self.view.capacity() * size_of::<u32>()
            + self
                .last
                .as_ref()
                .and_then(|(apart, _)| apart.as_ref())
                .map_or(0, WeakTable::held_bytes)
    }
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
    let table = read_table(&mut reader, kinds, scratch)?;
    let mut view = Vec::new();
    if matches!(status, Status::Truncated { .. }) {
        reader.slots(&mut view)?;
        if view.iter().any(|&slot| !table.live().get(slot)) {
            return Err(crate::invalid(
                "A truncated view names a slot holding no row",
            ));
        }
    }
    let last = match reader.varint()? {
        0 => (status == Status::Complete).then_some(Last::Held(last_read)),
        1 => Some(Last::Held(get_hour(&mut reader)?)),
        2 if status != Status::Complete => {
            let read = get_hour(&mut reader)?;
            Some(Last::Apart(
                Box::new(read_table(&mut reader, kinds, scratch)?),
                read,
            ))
        }
        _ => {
            return Err(crate::invalid(
                "A restart's last complete read is of no known form",
            ));
        }
    };
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
        last,
    })
}
