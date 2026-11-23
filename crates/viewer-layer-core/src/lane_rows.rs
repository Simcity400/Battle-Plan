
pub mod rosters;

use playhead::{changes::Changes, lane::Lane, pages::Pages};
use replay_columnar::{
    manifest::Status,
    state::Kind,
    store::Value,
    unit::{BLOCK, LIFETIMES, STRINGS, column_stream, reader},
};
use replay_data::{
    Hour,
    rows::{Field, List, Opaque, RowCodec, TableRows, TableSchema},
};
use std::{collections::HashMap, io, sync::Arc};

fn invalid(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

pub fn field<'a>(kind: Kind, value: Option<Value<'a>>, texts: &'a HashMap<u32, Box<[u8]>>, format: u16) -> Field<'a> {
    let Some(value) = value else {
        return match kind {
            Kind::U8 => Field::U8(0),
            Kind::I8 => Field::I8(0),
            Kind::U16 => Field::U16(0),
            Kind::I16 => Field::I16(0),
            Kind::U32 => Field::U32(0),
            Kind::I32 => Field::I32(0),
            Kind::U64 => Field::U64(0),
            Kind::I64 => Field::I64(0),
            Kind::Bool => Field::Bool(false),
            Kind::OptionalI32 => Field::OptionalI32(None),
            Kind::OptionalU32 => Field::OptionalU32(None),
            Kind::OptionalI64 => Field::OptionalI64(None),
            Kind::OptionalU64 => Field::OptionalU64(None),
            Kind::Bytes => Field::Bytes(&[]),
            Kind::U32List => Field::U32List(List::Encoded(&[])),
            Kind::U64List => Field::U64List(List::Encoded(&[])),
            Kind::Opaque => Field::NotRecorded,
        };
    };
    match (kind, value) {
        (_, Value::Unrecorded) => Field::NotRecorded,
        (Kind::OptionalI32, Value::Missing) => Field::OptionalI32(None),
        (Kind::OptionalU32, Value::Missing) => Field::OptionalU32(None),
        (Kind::OptionalI64, Value::Missing) => Field::OptionalI64(None),
        (Kind::OptionalU64, Value::Missing) => Field::OptionalU64(None),
        (_, Value::Missing) => Field::NotRecorded,
        (Kind::U8, Value::Number(bits)) => Field::U8(bits as u8),
        (Kind::I8, Value::Number(bits)) => Field::I8(bits as i64 as i8),
        (Kind::U16, Value::Number(bits)) => Field::U16(bits as u16),
        (Kind::I16, Value::Number(bits)) => Field::I16(bits as i64 as i16),
        (Kind::U32, Value::Number(bits)) => Field::U32(bits as u32),
        (Kind::I32, Value::Number(bits)) => Field::I32(bits as i64 as i32),
        (Kind::U64, Value::Number(bits)) => Field::U64(bits),
        (Kind::I64, Value::Number(bits)) => Field::I64(bits as i64),
        (Kind::Bool, Value::Number(bits)) => Field::Bool(bits != 0),
        (Kind::OptionalI32, Value::Number(bits)) => Field::OptionalI32(Some(bits as i64 as i32)),
        (Kind::OptionalU32, Value::Number(bits)) => Field::OptionalU32(Some(bits as u32)),
        (Kind::OptionalI64, Value::Number(bits)) => Field::OptionalI64(Some(bits as i64)),
        (Kind::OptionalU64, Value::Number(bits)) => Field::OptionalU64(Some(bits)),
        (Kind::Bytes, Value::Text(id)) => texts.get(&id).map_or(Field::NotRecorded, |text| Field::Bytes(text)),
        (Kind::U32List, Value::List(bytes)) => Field::U32List(List::Encoded(bytes)),
        (Kind::U64List, Value::List(bytes)) => Field::U64List(List::Encoded(bytes)),
        (Kind::Opaque, Value::List(bytes)) => Field::Opaque(Opaque::Encoded(bytes, format)),
        _ => Field::NotRecorded,
    }
}

pub trait Kept: Send {
    fn seek(&mut self, hour: Hour) -> io::Result<()>;
    fn step(&mut self, hour: Hour) -> io::Result<()>;
    fn status(&self) -> (Status, Option<Hour>);
    fn reason(&self) -> &str;
    fn bytes(&self) -> (usize, playhead::lane::Held);
    fn rows(&self) -> &TableRows;
    fn touched(&self) -> bool;
    fn steps(&self) -> (u64, u64);
}

pub struct Table<T: TableSchema> {
    lane: Lane,
    kinds: Vec<Kind>,
    mask: u64,
    rows: TableRows,
    kind: std::marker::PhantomData<fn() -> T>,
    texts: HashMap<u32, Box<[u8]>>,
    changes: Changes,
    reason: String,
    touched: bool,
    pages: Arc<dyn Pages>,
    run: usize,
    streams: Vec<u32>,
    block: Option<u32>,
    busy: Vec<u64>,
    stepped: u64,
    skipped: u64,
}

impl<T: TableSchema + 'static> Table<T>
where
    T::Row: Send + Default,
{
    pub fn open(pages: &Arc<dyn Pages>, run: usize, fields: Option<&[&str]>, reader: &str) -> io::Result<Self> {
        let table = T::ID;
        let kinds = pages.kinds(table.index()).ok_or_else(|| invalid(format!("{reader} reads {table:?}, which the recording's schema does not hold")))?.to_vec();
        let columns: Vec<usize> = match fields {
            None => (0..kinds.len()).collect(),
            Some(fields) => fields
                .iter()
                .map(|name| (0..kinds.len()).find(|&column| table.field_name(column) == Some(name)).ok_or_else(|| invalid(format!("{reader} reads {table:?}.{name}, which the recording's schema does not hold"))))
                .collect::<io::Result<_>>()?,
        };
        let streams = [LIFETIMES].into_iter().chain(columns.iter().map(|&column| column_stream(column))).chain(columns.iter().any(|&column| kinds[column] == Kind::Bytes).then_some(STRINGS)).collect();
        Ok(Self {
            lane: Lane::new(Arc::clone(pages), run, table.index(), &columns, true)?,
            kinds,
            rows: T::wrap(Vec::new()),
            kind: std::marker::PhantomData,
            texts: HashMap::new(),
            changes: Changes::default(),
            reason: String::new(),
            touched: true,
            streams,
            mask: columns.iter().fold(0, |mask, &column| mask | 1 << column),
            pages: Arc::clone(pages),
            run,
            block: None,
            busy: Vec::new(),
            stepped: 0,
            skipped: 0,
        })
    }

    fn busy(&mut self, at: u32) -> io::Result<bool> {
        let block = at / BLOCK;
        if self.block != Some(block) {
            self.block = Some(block);
            let run = &self.pages.runs()[self.run];
            let Some(page) = run.pages.get(block as usize) else { return Ok(true) };
            let entry = &page.tables[T::ID.index()];
            let mut busy = vec![0u64; usize::from(page.hours).div_ceil(64)];
            let mut set = |hour: u16| busy[usize::from(hour) / 64] |= 1 << (hour % 64);
            for (at, part) in entry.parts.iter().enumerate() {
                if part.frames.iter().any(|frame| frame.streams.iter().any(|stream| self.streams.contains(stream))) {
                    let end = entry.parts.get(at + 1).map_or(page.hours, |next| next.first);
                    (part.first..end).for_each(&mut set);
                }
            }
            let first = run.first_hour + (block * BLOCK) as Hour;
            let mut hour = 0;
            while let (_, _, _, Some(change)) = reader::status(entry, hour, first)? {
                if change <= hour || change >= page.hours {
                    break;
                }
                set(change);
                hour = change;
            }
            self.busy = busy;
        }
        let hour = at % BLOCK;
        Ok(self.busy.get(hour as usize / 64).is_none_or(|word| word & (1 << (hour % 64)) != 0))
    }

    fn list(&mut self) -> &mut Vec<T::Row> {
        T::borrow_mut(&mut self.rows).expect("a table's own rows")
    }

    fn row(&self, slot: u32) -> io::Result<Option<T::Row>> {
        let cells = self.lane.cells();
        let Some(key) = cells.key(slot).filter(|_| cells.live().get(slot)) else { return Ok(None) };
        let (kinds, texts, format) = (&self.kinds, &self.texts, self.lane.format());
        T::Row::from_wire_fields(key, self.mask, &mut |index| Ok(field(kinds[index], Some(cells.value(index, slot)), texts, format))).map(Some)
    }

    fn rebuild(&mut self) -> io::Result<()> {
        let slots: Vec<u32> = self.lane.cells().rows().collect();
        let mut rows = Vec::with_capacity(slots.len());
        for slot in slots {
            rows.extend(self.row(slot)?);
        }
        rows.sort_unstable_by_key(|row| row.key());
        self.rows = T::wrap(rows);
        self.read_reason();
        Ok(())
    }

    fn read_reason(&mut self) {
        if let Status::Unavailable { reason } = self.lane.status().0 {
            self.reason = self.texts.get(&reason).map_or_else(|| crate::playhead_host::mirror::UNREAD_REASON.to_owned(), |text| String::from_utf8_lossy(text).into_owned());
        }
    }

    fn upsert(&mut self, row: T::Row) {
        let rows = self.list();
        match rows.binary_search_by_key(&row.key(), RowCodec::key) {
            Ok(at) => rows[at] = row,
            Err(at) => rows.insert(at, row),
        }
    }
}

impl<T: TableSchema + 'static> Kept for Table<T>
where
    T::Row: Send + Default,
{
    fn seek(&mut self, hour: Hour) -> io::Result<()> {
        self.lane.seek(hour)?;
        self.touched = true;
        self.texts.clear();
        for (id, text) in self.lane.texts().iter() {
            self.texts.insert(id, text.into());
        }
        self.rebuild()
    }

    fn step(&mut self, hour: Hour) -> io::Result<()> {
        let at = (hour - self.pages.runs()[self.run].first_hour) as u32;
        if !self.busy(at)? {
            self.touched = false;
            self.skipped += 1;
            return Ok(());
        }
        self.stepped += 1;
        self.lane.fetch_through(hour)?;
        let mut changes = std::mem::take(&mut self.changes);
        let status = self.lane.status().0;
        self.lane.step(hour, &mut changes)?;
        self.touched = status != self.lane.status().0 || !changes.deleted().is_empty() || !changes.inserted().is_empty() || changes.columns().iter().any(|written| !written.is_empty()) || changes.renumber().is_some();
        for (id, text) in changes.texts().iter() {
            self.texts.insert(id, text.into());
        }
        if changes.renumber().is_some() {
            self.changes = changes;
            return self.rebuild();
        }
        for &key in changes.deleted().keys().unwrap_or_default() {
            let rows = self.list();
            if let Ok(at) = rows.binary_search_by_key(&key, RowCodec::key) {
                rows.remove(at);
            }
        }
        let mut slots: Vec<u32> = changes.inserted().slots().to_vec();
        slots.extend(changes.columns().iter().flat_map(|written| written.slots().iter().copied()));
        slots.sort_unstable();
        slots.dedup();
        for slot in slots {
            if let Some(row) = self.row(slot)? {
                self.upsert(row);
            }
        }
        self.changes = changes;
        self.read_reason();
        Ok(())
    }

    fn status(&self) -> (Status, Option<Hour>) {
        self.lane.status()
    }

    fn reason(&self) -> &str {
        &self.reason
    }

    fn bytes(&self) -> (usize, playhead::lane::Held) {
        let held = T::borrow(&self.rows).unwrap_or_default();
        let rows = held.len() * size_of::<T::Row>() + held.iter().map(RowCodec::heap_bytes).sum::<usize>() + self.texts.values().map(|text| text.len() + 40).sum::<usize>();
        (rows, self.lane.held())
    }

    fn rows(&self) -> &TableRows {
        &self.rows
    }

    fn touched(&self) -> bool {
        self.touched
    }

    fn steps(&self) -> (u64, u64) {
        (self.stepped, self.skipped)
    }
}
