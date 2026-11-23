use replay_columnar::{container, envelope, state::Kind as Column, unit::{FrameEntry, reader::Extents}};
use replay_data::rows::{TABLE_COUNT, TableId};
use std::io;

pub const LAST_COMPLETE: [TableId; 3] = [TableId::Divisions, TableId::DivisionEquipmentDeletions, TableId::SunkShips];

pub fn keeps_last() -> [bool; TABLE_COUNT] {
    let mut keeps = [false; TABLE_COUNT];
    for table in LAST_COMPLETE {
        keeps[table.index()] = true;
    }
    keeps
}

pub fn names() -> replay_columnar::store::journal::Names {
    replay_columnar::store::journal::Names {
        table: |table| TableId::ALL.get(table).map_or_else(|| format!("table {table}"), |id| format!("{id:?}")),
        column: |table, column| TableId::ALL.get(table).and_then(|id| id.field_name(column)).map_or_else(|| format!("{column}"), str::to_owned),
    }
}

pub fn schema() -> io::Result<Vec<Vec<Column>>> {
    schema_at(replay_data::format::FormatVersion::CURRENT.version())
}

pub fn schema_at(format: u16) -> io::Result<Vec<Vec<Column>>> {
    let mut schema = engine_allocation::vec(TableId::ALL.len())?;
    for table in TableId::ALL {
        schema.push(crate::column_kind::columns(table.schema_at(format))?);
    }
    Ok(schema)
}

pub fn columns_at(format: u16) -> io::Result<Vec<usize>> {
    engine_allocation::collect_vec(TableId::ALL.into_iter().take_while(|table| table.last_change().introduced <= format).map(|table| table.last_change().framed_fields(table, format)))
}

pub fn withheld_at(format: u16) -> io::Result<Vec<(usize, String)>> {
    let mut withheld = Vec::new();
    for table in TableId::ALL {
        let change = table.last_change();
        if change.retired.is_some() {
            continue;
        }
        if let Some(gap) = change.gap(table, format) {
            withheld.push((table.index(), gap.to_string()));
        }
    }
    Ok(withheld)
}

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

pub fn strings_frame(file: &dyn Extents, frame: &FrameEntry) -> io::Result<(Vec<u8>, Vec<(u32, u32)>)> {
    let mut stored = Vec::new();
    file.read_at(frame.offset, frame.stored as usize, &mut stored)?;
    if envelope::digest(&stored) != frame.digest {
        return Err(invalid("A block's strings differ from their digest"));
    }
    let raw = container::decompress(&stored, Some(u64::from(frame.raw)))?;
    drop(stored);
    let mut reader = replay_columnar::leaf::Reader::new(&raw);
    let count = reader.count()?;
    if count > raw.len() {
        return Err(invalid("A block's strings count more than they hold"));
    }
    let mut places = Vec::with_capacity(count);
    for _ in 0..count {
        let length = reader.count()?;
        let at = raw.len() - reader.rest().len();
        reader.raw(length)?;
        places.push((at as u32, length as u32));
    }
    reader.finish()?;
    Ok((raw, places))
}
