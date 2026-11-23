pub mod deployed;
pub mod schedule;


pub fn kinds() -> Vec<Vec<Kind>> {
    TableId::ALL.iter().map(|&table| column_kinds(table)).collect()
}

pub mod noting {
    use crate::framework::{Hour, Output, Read, Rows, Uses, Writer};
    use replay_columnar::leaf::Writer as Bytes;
    use replay_data::rows::TableId;
    use std::{cell::RefCell, io};

    pub type Noted = Vec<(TableId, Vec<u128>, Vec<u128>)>;

    thread_local! {
        static NOTED: RefCell<Vec<Noted>> = const { RefCell::new(Vec::new()) };
    }

    pub fn take() -> Vec<Noted> {
        NOTED.with(|noted| std::mem::take(&mut *noted.borrow_mut()))
    }

    #[derive(Default)]
    pub struct Noting<W>(pub W);

    impl<W: Writer> Writer for Noting<W> {
        fn name(&self) -> &'static str {
            self.0.name()
        }

        fn version(&self) -> u32 {
            self.0.version()
        }

        fn tables(&self) -> &'static [Output] {
            self.0.tables()
        }

        fn reads(&self) -> &'static [Read] {
            self.0.reads()
        }

        fn hourly(&self) -> bool {
            self.0.hourly()
        }

        fn strings(&self) -> &'static [&'static str] {
            self.0.strings()
        }

        fn definitions(&self) -> &'static [Uses] {
            self.0.definitions()
        }

        fn start(&mut self) {
            self.0.start();
        }

        fn hour(&mut self, hour: &Hour<'_>, out: &mut Rows<'_>) -> io::Result<()> {
            self.0.hour(hour, out)?;
            let noted: Noted = self
                .0
                .tables()
                .iter()
                .map(|output| {
                    let table = output.table();
                    let (sets, deletes) = out.keys(table);
                    (table, sets, deletes)
                })
                .filter(|(_, sets, deletes)| !sets.is_empty() || !deletes.is_empty())
                .collect();
            NOTED.with(|held| held.borrow_mut().push(noted));
            Ok(())
        }

        fn save(&self, blob: &mut Bytes) -> io::Result<()> {
            self.0.save(blob)
        }

        fn load(&mut self, blob: &[u8]) -> io::Result<()> {
            self.0.load(blob)
        }

        fn prime(&mut self, hour: &Hour<'_>) {
            self.0.prime(hour);
        }
    }
}

use crate::framework::{Output, Rows};
use replay_columnar::{
    state::{Cell, Kind},
    store::{Store, StringSource},
};
use replay_data::rows::{Field, List, Opaque, RowCodec, TABLE_COUNT, TableId};
use std::io;

pub fn logging<T>(
    input: &Store,
    outputs: &[Output],
    write: impl FnOnce(&mut Rows<'_>) -> io::Result<T>,
) -> io::Result<T> {
    let mut scratch = store();
    scratch.resume(input.tables().to_vec(), input.strings().next())?;
    let lent = outputs.iter()
        .filter_map(|output| match *output {
            Output::State(table) => {
                scratch.index(table.index());
                Some(scratch.lend(table.index()))
            }
            Output::Log(..) => None,
        })
        .collect::<io::Result<_>>()?;
    let mut rows = Rows::lending(input, lent);
    let result = write(&mut rows)?;
    rows.check_writes()?;
    Ok(result)
}

pub fn column_kinds(table: TableId) -> Vec<Kind> {
    use replay_data::rows::Kind as Wire;
    table
        .schema()
        .iter()
        .map(|kind| match kind {
            Wire::U8 => Kind::U8,
            Wire::I8 => Kind::I8,
            Wire::U16 => Kind::U16,
            Wire::I16 => Kind::I16,
            Wire::U32 => Kind::U32,
            Wire::I32 => Kind::I32,
            Wire::U64 => Kind::U64,
            Wire::I64 => Kind::I64,
            Wire::Bool => Kind::Bool,
            Wire::OptionalI32 => Kind::OptionalI32,
            Wire::OptionalU32 => Kind::OptionalU32,
            Wire::OptionalI64 => Kind::OptionalI64,
            Wire::OptionalU64 => Kind::OptionalU64,
            Wire::Bytes => Kind::Bytes,
            Wire::U32List => Kind::U32List,
            Wire::U64List => Kind::U64List,
            Wire::Opaque => Kind::Opaque,
        })
        .collect()
}

pub fn store() -> Store {
    let mut store = Store::new(kinds(), vec![false; TABLE_COUNT]);
    store.declare_logs(&replay_data::rows::logs()).expect("logs declared on an empty store");
    store.begin_run(0);
    store
}

fn is_log(store: &Store, table: TableId) -> bool {
    store.logs().get(table.index()).copied().unwrap_or(false)
}

struct Texts(Vec<Option<Vec<u8>>>);

impl StringSource for Texts {
    fn text(&mut self, id: u32) -> io::Result<Option<Vec<u8>>> {
        Ok(self.0.get(id as usize).cloned().flatten())
    }
}

fn take_up(stopped: &Store, texts: Texts) -> Store {
    let mut store = store();
    store.read_strings_from(Box::new(texts));
    store.resume(stopped.tables().to_vec(), stopped.strings().next()).expect("a resumed store");
    store
}

pub fn resumed(stopped: &Store, texts: &[&[u8]]) -> Store {
    take_up(stopped, Texts(texts.iter().map(|text| Some(text.to_vec())).collect()))
}

pub fn resumed_holding(stopped: &Store) -> Store {
    let strings = stopped.strings();
    take_up(stopped, Texts((0..strings.next()).map(|id| strings.get(id).map(<[u8]>::to_vec)).collect()))
}

enum Held {
    Scalar(Cell<'static>),
    Text(u32),
    U32s(Vec<u32>),
    U64s(Vec<u64>),
    Opaque(Vec<u8>),
    Unrecorded,
}

pub fn put<R: RowCodec>(store: &mut Store, table: TableId, row: &R, texts: &[&[u8]]) -> io::Result<()> {
    let mut cells = Vec::new();
    row.visit_wire(&mut |_, field| {
        cells.push(match field {
            Field::Empty => return Err(io::Error::other("a row field is missing")),
            Field::NotRecorded => Held::Unrecorded,
            Field::U8(value) => Held::Scalar(Cell::U8(value)),
            Field::I8(value) => Held::Scalar(Cell::I8(value)),
            Field::U16(value) => Held::Scalar(Cell::U16(value)),
            Field::I16(value) => Held::Scalar(Cell::I16(value)),
            Field::U32(value) => Held::Scalar(Cell::U32(value)),
            Field::I32(value) => Held::Scalar(Cell::I32(value)),
            Field::U64(value) => Held::Scalar(Cell::U64(value)),
            Field::I64(value) => Held::Scalar(Cell::I64(value)),
            Field::Bool(value) => Held::Scalar(Cell::Bool(value)),
            Field::OptionalI32(value) => Held::Scalar(Cell::OptionalI32(value)),
            Field::OptionalU32(value) => Held::Scalar(Cell::OptionalU32(value)),
            Field::OptionalI64(value) => Held::Scalar(Cell::OptionalI64(value)),
            Field::OptionalU64(value) => Held::Scalar(Cell::OptionalU64(value)),
            Field::Bytes(bytes) => {
                let at = texts.iter().position(|text| *text == bytes).ok_or_else(|| io::Error::other("a text the test did not append"))?;
                Held::Text(at as u32)
            }
            Field::U32List(List::Native(values)) => Held::U32s(values.to_vec()),
            Field::U32List(List::Encoded(bytes)) => Held::U32s(bytes.chunks_exact(4).map(|item| u32::from_le_bytes(item.try_into().expect("4 bytes"))).collect()),
            Field::U64List(List::Native(values)) => Held::U64s(values.to_vec()),
            Field::U64List(List::Encoded(bytes)) => Held::U64s(bytes.chunks_exact(8).map(|item| u64::from_le_bytes(item.try_into().expect("8 bytes"))).collect()),
            Field::Opaque(Opaque::Native(value)) => {
                let mut bytes = Vec::new();
                value.encode_value(&mut bytes)?;
                Held::Opaque(bytes)
            }
            Field::Opaque(Opaque::Encoded(bytes, _)) => Held::Opaque(bytes.to_vec()),
        });
        Ok(())
    })?;
    let key = row.key();
    let log = is_log(store, table);
    let mut changes = store.table(table.index())?;
    if log {
        let given: Vec<Option<Cell<'_>>> = cells
            .iter()
            .map(|held| match held {
                Held::Scalar(cell) => Some(*cell),
                Held::Text(id) => Some(Cell::Bytes(*id)),
                Held::U32s(values) => Some(Cell::U32List(values)),
                Held::U64s(values) => Some(Cell::U64List(values)),
                Held::Opaque(bytes) => Some(Cell::Opaque(bytes)),
                Held::Unrecorded => None,
            })
            .collect();
        changes.append(key, &given)?;
        return Ok(());
    }
    let (slot, born) = match changes.slot(key) {
        Some(slot) => (slot, false),
        None => (changes.insert(key)?, true),
    };
    for (column, held) in cells.iter().enumerate() {
        let cell = match held {
            Held::Scalar(cell) => *cell,
            Held::Text(id) => Cell::Bytes(*id),
            Held::U32s(values) => Cell::U32List(values),
            Held::U64s(values) => Cell::U64List(values),
            Held::Opaque(bytes) => Cell::Opaque(bytes),
            Held::Unrecorded => {
                if born || changes.held().columns()[column].recorded(slot) {
                    changes.unrecord(slot, column)?;
                }
                continue;
            }
        };
        if born {
            changes.write(slot, column, cell)?;
        } else {
            changes.offer(slot, column, cell)?;
        }
    }
    Ok(())
}

pub fn remove(store: &mut Store, table: TableId, key: u128) -> io::Result<()> {
    if is_log(store, table) {
        return Err(io::Error::other(format!("{table:?} is a log: a row of it is forgotten by its ordinal")));
    }
    let mut changes = store.table(table.index())?;
    if let Some(slot) = changes.slot(key) {
        changes.delete(slot)?;
    }
    Ok(())
}

pub fn cells(store: &mut Store, table: TableId, key: u128, given: &[(usize, Cell<'_>)]) -> io::Result<()> {
    let columns = column_kinds(table).len();
    let log = is_log(store, table);
    let mut changes = store.table(table.index())?;
    if log {
        let row: Vec<Option<Cell<'_>>> = (0..columns).map(|column| given.iter().find(|(at, _)| *at == column).map(|&(_, cell)| cell)).collect();
        changes.append(key, &row)?;
        return Ok(());
    }
    let (slot, born) = match changes.slot(key) {
        Some(slot) => (slot, false),
        None => (changes.insert(key)?, true),
    };
    for column in 0..columns {
        match given.iter().find(|(at, _)| *at == column) {
            Some(&(_, cell)) if born => changes.write(slot, column, cell)?,
            Some(&(_, cell)) => {
                changes.offer(slot, column, cell)?;
            }
            None if born => changes.unrecord(slot, column)?,
            None => {}
        }
    }
    Ok(())
}

pub fn forget(store: &mut Store, table: TableId, ordinal: u64) -> io::Result<()> {
    store.table(table.index())?.forget(ordinal)
}

pub fn keep_only(store: &mut Store, table: TableId, keep: &[u128]) -> io::Result<()> {
    if is_log(store, table) {
        return match keep {
            [] => Ok(()),
            _ => Err(io::Error::other(format!("{table:?} is a log: it keeps no rows from one hour to the next"))),
        };
    }
    for (key, _) in rows(store, table) {
        if !keep.contains(&key) {
            remove(store, table, key)?;
        }
    }
    Ok(())
}

pub fn listed(store: &Store, table: TableId) -> Vec<u64> {
    store.table_state(table.index()).and_then(|held| held.listed()).map(|listed| listed.ones().collect()).unwrap_or_default()
}

pub fn rows(store: &Store, table: TableId) -> Vec<(u128, Vec<String>)> {
    let view = store.view(table.index());
    let mut rows: Vec<(u128, Vec<String>)> = view
        .rows()
        .filter_map(|slot| Some((view.key(slot)?, (0..view.kinds().len()).map(|column| format!("{:?}", view.value(column, slot))).collect())))
        .collect();
    rows.sort_unstable_by_key(|(key, _)| *key);
    rows
}
