
use crate::framework::{Columns, Hour, Rows, Value};
use replay_columnar::store::TableView;
use replay_data::rows::TableId;
use std::{
    collections::{HashMap, HashSet},
    io,
};

pub fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

#[derive(Default)]
pub struct Owners {
    links: HashMap<u32, u32>,
    countries: HashSet<u32>,
}

impl Owners {
    pub fn read_links(&mut self, hour: &Hour<'_>) {
        self.links.clear();
        let links = hour.table(TableId::CountryLinks);
        for slot in links.rows() {
            let (Some(key), Some(canonical)) = (links.key(slot), links.number(0, slot)) else { continue };
            self.links.insert(key as u32, canonical as u32);
        }
    }

    pub fn read_countries(&mut self, hour: &Hour<'_>) {
        self.countries.clear();
        let countries = hour.table(TableId::Countries);
        self.countries.extend(countries.rows().filter_map(|slot| countries.number(0, slot)).map(|canonical| canonical as u32));
    }

    pub fn slot(&self, raw: u32) -> Option<u32> {
        (raw != 0).then(|| self.links.get(&raw).copied()).flatten()
    }

    pub fn countries(&self) -> &HashSet<u32> {
        &self.countries
    }
}

pub fn links_moved(hour: &Hour<'_>) -> bool {
    hour.wrote(TableId::CountryLinks, Columns::These(&[0]))
}

pub fn countries_moved(hour: &Hour<'_>) -> bool {
    hour.wrote(TableId::Countries, Columns::These(&[0]))
}

pub fn exact(value: i128) -> Value {
    i64::try_from(value).map_or(Value::Unrecorded, Value::I64)
}

pub fn held(out: &Rows<'_>, table: TableId) -> Vec<u128> {
    let view = out.table(table);
    view.rows().filter_map(|slot| view.key(slot)).collect()
}

pub fn forget_listed(hour: &Hour<'_>, out: &mut Rows<'_>, table: TableId) {
    let Some((_, listed)) = hour.listed(table) else { return };
    let mut run: Option<(u64, u64)> = None;
    for ordinal in listed.ones() {
        match &mut run {
            Some((_, end)) if *end == ordinal => *end += 1,
            _ => {
                if let Some((first, end)) = run.replace((ordinal, ordinal + 1)) {
                    out.forget_run(table, first, end - first);
                }
            }
        }
    }
    if let Some((first, end)) = run {
        out.forget_run(table, first, end - first);
    }
}

pub fn write(out: &mut Rows<'_>, table: TableId, keys: impl IntoIterator<Item = u128>, all: bool, mut row: impl FnMut(u128) -> Option<Vec<Value>>) {
    let mut keys: Vec<u128> = keys.into_iter().collect();
    if all {
        keys.extend(held(out, table));
    }
    keys.sort_unstable();
    keys.dedup();
    for key in keys {
        match row(key) {
            Some(cells) => out.set(table, key, cells),
            None if out.table(table).slot(key).is_some() => out.delete(table, key),
            None => {}
        }
    }
}

pub fn written(hour: &Hour<'_>, table: TableId, columns: &'static [usize]) -> Vec<u32> {
    let mut slots = hour.written_rows(table, Columns::These(columns));
    slots.sort_unstable();
    slots.dedup();
    slots
}

pub fn number(view: &TableView<'_>, table: TableId, column: usize, slot: u32) -> io::Result<i64> {
    view.number(column, slot)
        .ok_or_else(|| invalid(format!("{table:?}.{} is not recorded and its row cannot say so", table.field_name(column).unwrap_or("?"))))
}

#[cfg(any(test, feature = "test-support"))]
pub mod testing {
    use replay_columnar::{
        manifest::Status,
        state::{Cell, Kind as Column},
        store::Store,
    };
    use replay_data::rows::{Field, Kind, Opaque, RowCodec, TABLE_COUNT, TableId};

    fn column(kind: Kind) -> Column {
        match kind {
            Kind::U8 => Column::U8,
            Kind::I8 => Column::I8,
            Kind::U16 => Column::U16,
            Kind::I16 => Column::I16,
            Kind::U32 => Column::U32,
            Kind::I32 => Column::I32,
            Kind::U64 => Column::U64,
            Kind::I64 => Column::I64,
            Kind::Bool => Column::Bool,
            Kind::OptionalI32 => Column::OptionalI32,
            Kind::OptionalU32 => Column::OptionalU32,
            Kind::OptionalI64 => Column::OptionalI64,
            Kind::OptionalU64 => Column::OptionalU64,
            Kind::Bytes => Column::Bytes,
            Kind::U32List => Column::U32List,
            Kind::U64List => Column::U64List,
            Kind::Opaque => Column::Opaque,
        }
    }

    pub fn store() -> Store {
        Store::new(TableId::ALL.iter().map(|table| table.schema().iter().map(|&kind| column(kind)).collect()).collect(), vec![false; TABLE_COUNT])
    }

    pub fn put<R: RowCodec>(store: &mut Store, hour: i32, table: TableId, rows: &[R], status: Status) {
        let index = table.index();
        store.status(index, status, true, Some(hour), true).unwrap();
        let keys: Vec<u128> = rows.iter().map(RowCodec::key).collect();
        let texts: Vec<Vec<(usize, u32)>> = rows
            .iter()
            .map(|row| {
                let mut ids = Vec::new();
                row.visit_wire(&mut |column, field| {
                    if let Field::Bytes(text) = field {
                        ids.push((column, store.intern(text).unwrap()));
                    }
                    Ok(())
                })
                .unwrap();
                ids
            })
            .collect();
        let live: Vec<(u32, u128)> = {
            let view = store.view(index);
            view.rows().map(|slot| (slot, view.key(slot).unwrap())).collect()
        };
        let mut changes = store.table(index).unwrap();
        for (slot, key) in live {
            if !keys.contains(&key) {
                changes.delete(slot).unwrap();
            }
        }
        for (row, ids) in rows.iter().zip(&texts) {
            let (slot, fresh) = match changes.slot(row.key()) {
                Some(slot) => (slot, false),
                None => (changes.insert(row.key()).unwrap(), true),
            };
            row.visit_wire(&mut |column, field| {
                let bytes;
                let cell = match field {
                    Field::U8(value) => Cell::U8(value),
                    Field::I8(value) => Cell::I8(value),
                    Field::U16(value) => Cell::U16(value),
                    Field::I16(value) => Cell::I16(value),
                    Field::U32(value) => Cell::U32(value),
                    Field::I32(value) => Cell::I32(value),
                    Field::U64(value) => Cell::U64(value),
                    Field::I64(value) => Cell::I64(value),
                    Field::Bool(value) => Cell::Bool(value),
                    Field::OptionalI32(value) => Cell::OptionalI32(value),
                    Field::OptionalU32(value) => Cell::OptionalU32(value),
                    Field::OptionalI64(value) => Cell::OptionalI64(value),
                    Field::OptionalU64(value) => Cell::OptionalU64(value),
                    Field::Opaque(Opaque::Native(value)) => {
                        let mut encoded = Vec::new();
                        value.encode_value(&mut encoded)?;
                        bytes = encoded;
                        Cell::Opaque(&bytes)
                    }
                    Field::Bytes(_) => match ids.iter().find(|(at, _)| *at == column) {
                        Some(&(_, id)) => Cell::Bytes(id),
                        None => return Ok(()),
                    },
                    Field::NotRecorded => {
                        if fresh || changes.held().columns()[column].recorded(slot) {
                            changes.unrecord(slot, column)?;
                        }
                        return Ok(());
                    }
                    _ => return Ok(()),
                };
                match fresh {
                    true => changes.write(slot, column, cell)?,
                    false => {
                        changes.offer(slot, column, cell)?;
                    }
                }
                Ok(())
            })
            .unwrap();
        }
    }

    pub fn rows(store: &Store, table: TableId) -> Vec<(u128, Vec<String>)> {
        let view = store.view(table.index());
        let mut rows: Vec<(u128, Vec<String>)> = view
            .rows()
            .map(|slot| (view.key(slot).unwrap(), (0..view.kinds().len()).map(|column| format!("{:?}", view.value(column, slot))).collect()))
            .collect();
        rows.sort_unstable();
        rows
    }

    pub fn cells(store: &Store, table: TableId, key: u128) -> Option<Vec<Option<i64>>> {
        let view = store.view(table.index());
        let slot = view.slot(key)?;
        Some((0..view.kinds().len()).map(|column| view.number(column, slot)).collect())
    }
}
