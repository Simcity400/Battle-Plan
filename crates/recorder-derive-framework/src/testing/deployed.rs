use super::store;
use crate::{
    deployed::row,
    framework::{Definitions, Writer, Writers},
};
use replay_columnar::{manifest::Status, state::Cell, store::Store, unit::Discard};
use replay_data::rows::{
    CountryCounterInputRow, CountryLinkRow, CountryRow, DivisionRow, Field, List, ManpowerReferences, Opaque, Recorded, RowCodec, SentVolunteerRow, TableId,
};

enum Owned {
    Plain(Cell<'static>),
    Text(Vec<u8>),
    Opaque(Vec<u8>),
    U32s(Vec<u32>),
    U64s(Vec<u64>),
    Unrecorded,
}

fn owned(field: Field<'_>) -> Owned {
    match field {
        Field::NotRecorded => Owned::Unrecorded,
        Field::U8(value) => Owned::Plain(Cell::U8(value)),
        Field::I8(value) => Owned::Plain(Cell::I8(value)),
        Field::U16(value) => Owned::Plain(Cell::U16(value)),
        Field::I16(value) => Owned::Plain(Cell::I16(value)),
        Field::U32(value) => Owned::Plain(Cell::U32(value)),
        Field::I32(value) => Owned::Plain(Cell::I32(value)),
        Field::U64(value) => Owned::Plain(Cell::U64(value)),
        Field::I64(value) => Owned::Plain(Cell::I64(value)),
        Field::Bool(value) => Owned::Plain(Cell::Bool(value)),
        Field::OptionalI32(value) => Owned::Plain(Cell::OptionalI32(value)),
        Field::OptionalU32(value) => Owned::Plain(Cell::OptionalU32(value)),
        Field::OptionalI64(value) => Owned::Plain(Cell::OptionalI64(value)),
        Field::OptionalU64(value) => Owned::Plain(Cell::OptionalU64(value)),
        Field::Bytes(text) => Owned::Text(text.to_vec()),
        Field::U32List(List::Native(values)) => Owned::U32s(values.to_vec()),
        Field::U32List(List::Encoded(bytes)) => Owned::U32s(bytes.chunks_exact(4).map(|item| u32::from_le_bytes(item.try_into().unwrap())).collect()),
        Field::U64List(List::Native(values)) => Owned::U64s(values.to_vec()),
        Field::U64List(List::Encoded(bytes)) => Owned::U64s(bytes.chunks_exact(8).map(|item| u64::from_le_bytes(item.try_into().unwrap())).collect()),
        Field::Opaque(Opaque::Encoded(bytes, _)) => Owned::Opaque(bytes.to_vec()),
        Field::Opaque(Opaque::Native(value)) => {
            let mut bytes = Vec::new();
            value.encode_value(&mut bytes).unwrap();
            Owned::Opaque(bytes)
        }
        Field::Empty => panic!("an empty field"),
    }
}

#[derive(Default)]
pub struct Edits {
    rows: Vec<(TableId, u128, Vec<Owned>)>,
    deletes: Vec<(TableId, u128)>,
    statuses: Vec<(TableId, Result<Status, Vec<u8>>)>,
    unstamped: Vec<TableId>,
}

impl Edits {
    pub fn put<R: RowCodec>(&mut self, table: TableId, row: R) -> &mut Self {
        let mut cells = Vec::new();
        row.visit_wire(&mut |_, field| {
            cells.push(owned(field));
            Ok(())
        })
        .unwrap();
        self.rows.push((table, row.key(), cells));
        self
    }

    pub fn delete(&mut self, table: TableId, key: u128) -> &mut Self {
        self.deletes.push((table, key));
        self
    }

    pub fn status(&mut self, table: TableId, status: Status) -> &mut Self {
        assert!(!matches!(status, Status::Unavailable { .. }), "an unavailable status names its reason with Edits::unavailable");
        self.statuses.push((table, Ok(status)));
        self
    }

    pub fn unavailable(&mut self, table: TableId, reason: &[u8]) -> &mut Self {
        self.statuses.push((table, Err(reason.to_vec())));
        self
    }

    pub fn unstamped(&mut self, table: TableId) -> &mut Self {
        self.unstamped.push(table);
        self
    }
}

pub struct Run {
    pub store: Store,
    writers: Writers,
    make: fn() -> Vec<Box<dyn Writer>>,
    inputs: &'static [TableId],
    next: i32,
}

impl Run {
    pub fn new(make: fn() -> Vec<Box<dyn Writer>>, inputs: &'static [TableId]) -> Self {
        let mut writers = Writers::new(make());
        writers.start_run();
        Self { store: store(), writers, make, inputs, next: 0 }
    }

    pub fn hour(&mut self, edits: &Edits) {
        let hour = self.next;
        self.next += 1;
        self.store.begin_hour(hour, &[]).unwrap();
        let mut tables: Vec<TableId> = edits.rows.iter().map(|edit| edit.0).chain(edits.deletes.iter().map(|edit| edit.0)).chain(edits.statuses.iter().map(|edit| edit.0)).collect();
        if hour == 0 {
            tables.extend_from_slice(self.inputs);
        }
        tables.sort_unstable();
        tables.dedup();
        for table in tables {
            let index = table.index();
            let status = match edits.statuses.iter().rev().find(|edit| edit.0 == table) {
                None => Status::Complete,
                Some((_, Ok(status))) => *status,
                Some((_, Err(reason))) => Status::Unavailable { reason: self.store.intern(reason).unwrap() },
            };
            let texts: Vec<Vec<Option<u32>>> = edits
                .rows
                .iter()
                .filter(|edit| edit.0 == table)
                .map(|edit| edit.2.iter().map(|cell| if let Owned::Text(text) = cell { Some(self.store.intern(text).unwrap()) } else { None }).collect())
                .collect();
            let edited = edits.rows.iter().any(|edit| edit.0 == table) || edits.deletes.iter().any(|edit| edit.0 == table);
            let stamped = !edits.unstamped.contains(&table);
            let sampled = if stamped { Some(hour) } else { self.store.view(index).sampled_at() };
            self.store.status(index, status, stamped, sampled, edited).unwrap();
            let mut changes = self.store.table(index).unwrap();
            for &(_, key) in edits.deletes.iter().filter(|edit| edit.0 == table) {
                if let Some(slot) = changes.slot(key) {
                    changes.delete(slot).unwrap();
                }
            }
            for ((_, key, cells), ids) in edits.rows.iter().filter(|edit| edit.0 == table).zip(&texts) {
                let (slot, fresh) = match changes.slot(*key) {
                    Some(slot) => (slot, false),
                    None => (changes.insert(*key).unwrap(), true),
                };
                for (column, (cell, id)) in cells.iter().zip(ids).enumerate() {
                    let value = match cell {
                        Owned::Plain(cell) => *cell,
                        Owned::Text(_) => Cell::Bytes(id.unwrap()),
                        Owned::Opaque(bytes) => Cell::Opaque(bytes),
                        Owned::U32s(values) => Cell::U32List(values),
                        Owned::U64s(values) => Cell::U64List(values),
                        Owned::Unrecorded => {
                            if fresh || changes.held().columns()[column].recorded(slot) {
                                changes.unrecord(slot, column).unwrap();
                            }
                            continue;
                        }
                    };
                    if fresh {
                        changes.write(slot, column, value).unwrap();
                    } else {
                        changes.offer(slot, column, value).unwrap();
                    }
                }
            }
        }
        self.store.settle();
        self.writers.hour(1, hour, hour == 0, &mut self.store).unwrap();
        self.store.end_hour(&mut Discard).unwrap();
    }

    pub fn define(&mut self, definitions: Definitions) {
        self.writers.define(definitions);
    }

    pub fn resumed(&self) -> Self {
        let saved = self.writers.save().unwrap();
        let mut store = crate::testing::resumed_holding(&self.store);
        let mut writers = Writers::new((self.make)());
        writers.define(self.writers.definitions().clone());
        writers.load(&saved, &mut store).unwrap();
        writers.prime(&store, 1, self.next - 1);
        Self { store, writers, make: self.make, inputs: self.inputs, next: self.next }
    }

    pub fn rows<R: RowCodec>(&self, table: TableId) -> Vec<R> {
        let view = self.store.view(table.index());
        let every: Vec<usize> = (0..table.field_count()).collect();
        let mut rows: Vec<(u128, R)> = view.rows().map(|slot| (view.key(slot).unwrap(), row(&view, slot, &every).unwrap())).collect();
        rows.sort_unstable_by_key(|(key, _)| *key);
        rows.into_iter().map(|(_, row)| row).collect()
    }

    pub fn status(&self, table: TableId) -> Status {
        self.store.view(table.index()).status()
    }

    pub fn why(&self, table: TableId) -> String {
        let view = self.store.view(table.index());
        match view.status() {
            Status::Unavailable { reason } => format!("unavailable: {}", String::from_utf8_lossy(view.string(reason).unwrap_or_default())),
            status => format!("{status:?}"),
        }
    }
}

pub fn country(id: u32, slot: u32, tag: &[u8]) -> CountryRow {
    CountryRow { id, canonical_slot: slot, tag: tag.to_vec(), ..Default::default() }
}

pub fn link(id: u32, slot: u32) -> CountryLinkRow {
    CountryLinkRow { id, canonical_slot: slot }
}

pub fn division(id: u64, roster: u32, logical: u32, expeditionary: u32, template: u64) -> DivisionRow {
    DivisionRow { id, roster_owner: roster, logical_owner: logical, expeditionary_owner: expeditionary, template, ..Default::default() }
}

pub fn counters(country: u32, sent: Option<&[u64]>) -> CountryCounterInputRow {
    CountryCounterInputRow { country, sent_expeditionaries: Recorded::Value(sent.map(|list| ManpowerReferences::from(list.to_vec()))), ..Default::default() }
}

pub fn volunteer(country: u32, ordinal: u32, division: u64) -> SentVolunteerRow {
    SentVolunteerRow { country, ordinal, division }
}

pub fn world(edits: &mut Edits) -> &mut Edits {
    edits
        .put(TableId::Countries, country(10, 1, b"GER"))
        .put(TableId::Countries, country(20, 2, b"ITA"))
        .put(TableId::Countries, country(30, 3, b"ROM"))
        .put(TableId::CountryLinks, link(10, 1))
        .put(TableId::CountryLinks, link(11, 1))
        .put(TableId::CountryLinks, link(20, 2))
        .put(TableId::CountryLinks, link(30, 3))
}
