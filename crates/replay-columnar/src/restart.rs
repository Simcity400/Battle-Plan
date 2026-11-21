
use crate::{
    leaf::{Reader, Writer},
    manifest::Status,
    section::{Scratch, read_values},
    state::{Bits, Kind, Table, WeakTable},
};
use std::io;

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Acquisition {
    pub last_read: Option<i32>,
    pub available_from: Option<i32>,
    pub refusal: Option<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Last {
    Held(Option<i32>),
    Apart(Box<Table>, Option<i32>),
}

impl Last {
    pub fn read(&self) -> Option<i32> {
        match self {
            Self::Held(read) | Self::Apart(_, read) => *read,
        }
    }

    pub fn held_bytes(&self) -> usize {
        match self {
            Self::Held(_) => 0,
            Self::Apart(table, _) => table.held_bytes(),
        }
    }

    pub fn before(last: &mut Option<Self>, table: &Table, status: Status) -> io::Result<()> {
        if let (Status::Truncated { .. }, Some(Self::Held(read))) = (status, &*last) {
            let read = *read;
            *last = Some(Self::Apart(engine_allocation::boxed(table.share()?)?, read));
        }
        Ok(())
    }

    pub fn share(&self) -> io::Result<Self> {
        Ok(match self {
            Self::Held(read) => Self::Held(*read),
            Self::Apart(table, read) => {
                Self::Apart(engine_allocation::boxed(table.share()?)?, *read)
            }
        })
    }

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
    let mut columns = engine_allocation::vec(table.kinds().count())?;
    for column in 0..table.kinds().count() {
        let mut bytes = Writer::new();
        table.write_column(column, &mut bytes, &slots)?;
        columns.push(bytes.into_bytes());
    }
    for bytes in &columns {
        writer.varint(bytes.len() as u64)?;
    }
    for bytes in &columns {
        writer.raw(bytes)?;
    }
    let mut unrecorded = engine_allocation::vec(slots.len())?;
    for column in 0..table.kinds().count() {
        unrecorded.clear();
        unrecorded.extend(table.unrecorded_slots(column));
        writer.slots(&unrecorded)?;
    }
    Ok(())
}

fn read_table(
    reader: &mut Reader<'_>,
    kinds: &[Kind],
    scratch: &mut Scratch,
    wanted: Option<&[bool]>,
) -> io::Result<Table> {
    let slots = reader.count()?;
    let mut live: Vec<u64> = Vec::new();
    reader.values(slots.div_ceil(64), &mut live)?;
    let live = Bits::from_words(live)?;
    let mut keys = Vec::new();
    reader.keys(live.count(), &mut keys)?;
    let mut table = Table::with_rows(kinds, slots, live, &keys)?;
    let live_slots = engine_allocation::collect_vec(table.live().ones())?;
    let mut lengths = engine_allocation::vec(kinds.len())?;
    for _ in kinds {
        lengths.push(reader.count()?);
    }
    for (column, &kind) in kinds.iter().enumerate() {
        let bytes = reader.raw(lengths[column])?;
        if wanted.is_some_and(|wanted| !wanted.get(column).copied().unwrap_or(false)) {
            table.unrecord(column, &live_slots)?;
            continue;
        }
        let mut values = Reader::new(bytes);
        let read = read_values(&mut values, scratch, kind, live_slots.len())?;
        table.set(column, &live_slots, read)?;
        values.finish()?;
    }
    let mut unrecorded = Vec::new();
    for column in 0..kinds.len() {
        reader.slots(&mut unrecorded)?;
        table.unrecord(column, &unrecorded)?;
    }
    Ok(table)
}

pub fn write(
    table: &Table,
    status: Status,
    acquisition: &Acquisition,
    view: &[u32],
) -> io::Result<Vec<u8>> {
    write_with(table, status, acquisition, view, None)
}

pub fn write_with(
    table: &Table,
    status: Status,
    acquisition: &Acquisition,
    view: &[u32],
    last: Option<&Last>,
) -> io::Result<Vec<u8>> {
    write_named(table, status, acquisition, view, last, &[])
}

pub fn write_named(
    table: &Table,
    status: Status,
    acquisition: &Acquisition,
    view: &[u32],
    last: Option<&Last>,
    strings: &[(u32, &[u8])],
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
    writer.varint(strings.len() as u64)?;
    let mut next = 0_u64;
    for &(id, text) in strings {
        if u64::from(id) < next {
            return Err(crate::invalid("A restore's strings are not ascending"));
        }
        writer.varint(u64::from(id) - next)?;
        next = u64::from(id) + 1;
        writer.varint(text.len() as u64)?;
        writer.raw(text)?;
    }
    write_table(&mut writer, table)?;
    if truncated {
        writer.slots(view)?;
    }
    match last {
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

#[derive(Debug)]
pub struct Restored {
    pub table: Table,
    pub status: Status,
    pub acquisition: Acquisition,
    pub view: Vec<u32>,
    pub last: Option<Last>,
    pub strings: Vec<(u32, Box<[u8]>)>,
}

#[derive(Debug)]
pub struct WeakRestored {
    table: WeakTable,
    status: Status,
    acquisition: Acquisition,
    view: Vec<u32>,
    last: Option<(Option<WeakTable>, Option<i32>)>,
}

impl Restored {
    pub fn share(&self) -> io::Result<Self> {
        Ok(Self {
            table: self.table.share()?,
            status: self.status,
            acquisition: self.acquisition,
            view: engine_allocation::copy_vec(&self.view)?,
            last: self.last.as_ref().map(Last::share).transpose()?,
            strings: Vec::new(),
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
            strings: Vec::new(),
        }))
    }

    pub fn alive(&self) -> bool {
        self.table.alive()
    }

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

pub fn acquisition(bytes: &[u8]) -> io::Result<(Status, Acquisition)> {
    head(&mut Reader::new(bytes))
}

fn head(reader: &mut Reader<'_>) -> io::Result<(Status, Acquisition)> {
    let status = Status::read(reader)?;
    let last_read = get_hour(reader)?;
    let available_from = get_hour(reader)?;
    let refusal = match reader.varint()? {
        0 => None,
        reason => Some(
            u32::try_from(reason - 1)
                .map_err(|_| crate::invalid("Restart refusal exceeds 32 bits"))?,
        ),
    };
    Ok((
        status,
        Acquisition {
            last_read,
            available_from,
            refusal,
        },
    ))
}

pub fn read(bytes: &[u8], kinds: &[Kind], scratch: &mut Scratch) -> io::Result<Restored> {
    read_projected(bytes, kinds, scratch, None)
}

fn read_strings(reader: &mut Reader<'_>) -> io::Result<Vec<(u32, Box<[u8]>)>> {
    let count = reader.count()?;
    if count > reader.left() {
        return Err(crate::invalid("A restore counts more strings than it holds"));
    }
    let mut strings = engine_allocation::vec(count)?;
    let mut next = 0_u64;
    for _ in 0..count {
        let id = u32::try_from(next + reader.varint()?)
            .map_err(|_| crate::invalid("A restore's string id passes 2^32"))?;
        next = u64::from(id) + 1;
        let length = reader.count()?;
        strings.push((id, engine_allocation::copy_vec(reader.raw(length)?)?.into_boxed_slice()));
    }
    Ok(strings)
}

pub fn read_projected(
    bytes: &[u8],
    kinds: &[Kind],
    scratch: &mut Scratch,
    wanted: Option<&[bool]>,
) -> io::Result<Restored> {
    let mut reader = Reader::new(bytes);
    let (status, acquisition) = head(&mut reader)?;
    let last_read = acquisition.last_read;
    let strings = read_strings(&mut reader)?;
    let table = read_table(&mut reader, kinds, scratch, wanted)?;
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
                Box::new(read_table(&mut reader, kinds, scratch, wanted)?),
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
        acquisition,
        view,
        last,
        strings,
    })
}
