
use crate::{
    layers::{List, Num, Number, Text},
    playhead_host::mirror::{Cell, Mirror, Mirrors, not_recorded, reason_words},
};
use playhead::changes::Changes;
use replay_columnar::{manifest::Status, store::Value};
use replay_data::{
    Hour,
    rows::{Field, NativeValue, Opaque, Recorded, TableId},
    scope::CaptureMode,
};
use std::collections::HashMap;

pub struct Shape {
    pub table: TableId,
    numbers: Vec<usize>,
    lists: Vec<(usize, bool)>,
    texts: Vec<usize>,
}

impl Shape {
    pub fn new(table: TableId, numbers: &[usize], lists: &[(usize, bool)]) -> Self {
        Self { table, numbers: numbers.to_vec(), lists: lists.to_vec(), texts: Vec::new() }
    }

    pub fn with_texts(mut self, texts: &[usize]) -> Self {
        self.texts = texts.to_vec();
        self
    }
}

const VALUE: u8 = 0;
const MISSING: u8 = 1;
const UNRECORDED: u8 = 2;
const UNNAMED: u8 = 3;

fn split(cell: Cell<u64>) -> (u64, u8) {
    match cell {
        Cell::Value(raw) => (raw, VALUE),
        Cell::Missing => (0, MISSING),
        Cell::Unrecorded => (0, UNRECORDED),
    }
}

fn join<T>(value: T, state: u8) -> Cell<T> {
    match state {
        VALUE => Cell::Value(value),
        MISSING => Cell::Missing,
        _ => Cell::Unrecorded,
    }
}

fn kept(cell: Cell<&[u8]>, first: bool) -> (Box<[u8]>, u8) {
    match cell {
        Cell::Value(bytes) => (if first { bytes[..bytes.len().min(4)].into() } else { bytes.into() }, VALUE),
        Cell::Missing => (Box::default(), MISSING),
        Cell::Unrecorded => (Box::default(), UNRECORDED),
    }
}

fn cell(value: Value<'_>) -> Cell<Value<'_>> {
    match value {
        Value::Missing => Cell::Missing,
        Value::Unrecorded => Cell::Unrecorded,
        value => Cell::Value(value),
    }
}

#[derive(Default)]
pub struct Standing {
    table: Option<TableId>,
    status: Option<(Status, Option<Hour>)>,
    head: Option<Hour>,
    gap: bool,
    reason: Option<(u32, Box<str>)>,
    capture: Option<CaptureMode>,
    format: u16,
    strings: Option<HashMap<u32, Box<[u8]>>>,
}

impl Standing {
    pub fn of(mirror: &Mirror) -> Self {
        let (status, gap, reason) = mirror.standing();
        Self {
            table: Some(mirror.table()),
            status: Some(status),
            head: mirror.head(),
            gap,
            reason: reason.map(|(id, text)| (id, text.into())),
            capture: mirror.capture(),
            format: mirror.format(),
            strings: mirror.strings().map(|strings| strings.map(|(id, text)| (id, Box::from(text))).collect()),
        }
    }

    pub fn step(&mut self, changes: &Changes) {
        self.gap = false;
        if let Some(strings) = &mut self.strings {
            for (id, text) in changes.texts().iter() {
                strings.insert(id, text.into());
            }
        }
        if let Some(status) = changes.status() {
            self.status = Some(status);
            match status.0 {
                Status::Unavailable { reason } if self.reason.as_ref().is_none_or(|(id, _)| *id != reason) => {
                    let held = self.strings.as_ref().and_then(|strings| strings.get(&reason)).map(|text| &text[..]);
                    self.reason = Some((reason, reason_words(reason, changes, held)));
                }
                Status::Unavailable { .. } => {}
                _ => self.reason = None,
            }
        }
        if changes.head().is_some() {
            self.head = changes.head();
        }
    }

    pub fn gap(&mut self) {
        self.gap = true;
    }

    pub fn status(&self) -> Option<Status> {
        if self.gap { None } else { self.status.map(|(status, _)| status) }
    }

    pub fn head(&self) -> Option<Hour> {
        self.head
    }

    pub fn not_recorded(&self) -> Option<String> {
        not_recorded(self.table?, self.status(), self.capture, self.reason.as_ref().map(|(_, text)| &**text))
    }

    fn string(&self, id: u32) -> Option<&[u8]> {
        self.strings.as_ref()?.get(&id).map(|text| &text[..])
    }

    fn bytes(&self) -> usize {
        self.strings.as_ref().map_or(0, |strings| strings.values().map(|text| text.len() + 40).sum())
    }
}

#[derive(Default)]
pub struct Table {
    standing: Standing,
    live: Vec<bool>,
    keys: Vec<u128>,
    index: HashMap<u128, u32>,
    numbers: Vec<(usize, Vec<u64>, Vec<u8>)>,
    lists: Vec<(usize, bool, Vec<Box<[u8]>>, Vec<u8>)>,
    texts: Vec<(usize, Vec<Box<[u8]>>, Vec<u8>)>,
}

fn named(string: Option<&[u8]>, id: u32) -> (Box<[u8]>, u8) {
    match string {
        Some(bytes) => (bytes.into(), VALUE),
        None => (id.to_le_bytes().into(), UNNAMED),
    }
}

fn text_of(mirror: &Mirror, column: usize, slot: u32) -> (Box<[u8]>, u8) {
    match mirror.cell(column, slot) {
        Cell::Value(id) => named(mirror.string(id as u32), id as u32),
        Cell::Missing => (Box::default(), MISSING),
        Cell::Unrecorded => (Box::default(), UNRECORDED),
    }
}

impl Table {
    pub fn format(&self) -> u16 {
        self.standing.format
    }

    pub fn from_mirror(mirror: &Mirror, shape: &Shape) -> Self {
        let mut table = Self {
            standing: Standing::of(mirror),
            numbers: shape.numbers.iter().map(|&column| (column, Vec::new(), Vec::new())).collect(),
            lists: shape.lists.iter().map(|&(column, first)| (column, first, Vec::new(), Vec::new())).collect(),
            texts: shape.texts.iter().map(|&column| (column, Vec::new(), Vec::new())).collect(),
            ..Self::default()
        };
        for slot in mirror.rows() {
            table.put(mirror, slot, mirror.key(slot).unwrap_or(0));
        }
        table
    }

    fn grow(&mut self, slot: usize) {
        if self.live.len() <= slot {
            let slots = slot + 1;
            self.live.resize(slots, false);
            self.keys.resize(slots, 0);
            for (_, values, states) in &mut self.numbers {
                values.resize(slots, 0);
                states.resize(slots, UNRECORDED);
            }
            for (_, _, values, states) in &mut self.lists {
                values.resize_with(slots, Box::default);
                states.resize(slots, UNRECORDED);
            }
            for (_, values, states) in &mut self.texts {
                values.resize_with(slots, Box::default);
                states.resize(slots, UNRECORDED);
            }
        }
    }

    fn put(&mut self, mirror: &Mirror, slot: u32, key: u128) {
        let at = slot as usize;
        self.grow(at);
        self.live[at] = true;
        self.keys[at] = key;
        self.index.insert(key, slot);
        for (column, values, states) in &mut self.numbers {
            (values[at], states[at]) = split(mirror.cell(*column, slot));
        }
        for (column, first, values, states) in &mut self.lists {
            (values[at], states[at]) = kept(mirror.list_cell(*column, slot), *first);
        }
        for (column, values, states) in &mut self.texts {
            (values[at], states[at]) = text_of(mirror, *column, slot);
        }
    }

    pub fn step(&mut self, changes: &Changes) -> Option<Delta> {
        self.standing.step(changes);
        let inserted = changes.inserted();
        let mut delta = Delta {
            deleted: Lives { slots: changes.deleted().slots().to_vec(), keys: changes.deleted().keys().map_or_else(Vec::new, <[u128]>::to_vec) },
            inserted: Lives { slots: inserted.slots().to_vec(), keys: inserted.keys().map_or_else(|| vec![0; inserted.slots().len()], <[u128]>::to_vec) },
            columns: Vec::new(),
            numbers: Vec::new(),
            lists: Vec::new(),
            texts: Vec::new(),
        };
        let mut seen = vec![false; self.numbers.len() + self.lists.len() + self.texts.len()];
        for written in changes.columns() {
            let column = written.column();
            if written.is_empty() {
                continue;
            }
            if let Some(at) = self.numbers.iter().position(|(kept, _, _)| *kept == column) {
                delta.numbers.extend(written.values().map(|(slot, value)| {
                    let (bits, state) = match cell(value) {
                        Cell::Value(Value::Number(bits)) => (bits, VALUE),
                        Cell::Value(Value::Text(id)) => (u64::from(id), VALUE),
                        Cell::Value(_) => (0, VALUE),
                        Cell::Missing => (0, MISSING),
                        Cell::Unrecorded => (0, UNRECORDED),
                    };
                    (slot, at, bits, state)
                }));
                seen[at] = true;
            } else if let Some(at) = self.lists.iter().position(|(kept, _, _, _)| *kept == column) {
                let first = self.lists[at].1;
                delta.lists.extend(written.values().map(|(slot, value)| {
                    let (bytes, state) = match cell(value) {
                        Cell::Value(Value::List(bytes)) => kept(Cell::Value(bytes), first),
                        Cell::Value(_) => kept(Cell::Value(&[]), first),
                        Cell::Missing => kept(Cell::Missing, first),
                        Cell::Unrecorded => kept(Cell::Unrecorded, first),
                    };
                    (slot, at, bytes, state)
                }));
                seen[self.numbers.len() + at] = true;
            } else if let Some(at) = self.texts.iter().position(|(kept, _, _)| *kept == column) {
                let standing = &self.standing;
                delta.texts.extend(written.values().map(|(slot, value)| {
                    let (bytes, state) = match cell(value) {
                        Cell::Value(Value::Text(id)) => named(standing.string(id), id),
                        Cell::Value(_) => (Box::default(), VALUE),
                        Cell::Missing => (Box::default(), MISSING),
                        Cell::Unrecorded => (Box::default(), UNRECORDED),
                    };
                    (slot, at, bytes, state)
                }));
                seen[self.numbers.len() + self.lists.len() + at] = true;
            } else {
                continue;
            }
            let mut slots = written.slots().to_vec();
            slots.extend_from_slice(inserted.slots());
            delta.columns.push(Written { column, slots });
        }
        if !inserted.is_empty() {
            let columns = self.numbers.iter().map(|(column, _, _)| *column).chain(self.lists.iter().map(|(column, _, _, _)| *column)).chain(self.texts.iter().map(|(column, _, _)| *column));
            for (at, column) in columns.enumerate() {
                if !seen[at] {
                    delta.columns.push(Written { column, slots: inserted.slots().to_vec() });
                }
            }
        }
        if delta.deleted.is_empty() && delta.inserted.is_empty() && delta.columns.is_empty() {
            return None;
        }
        self.apply(&delta);
        Some(delta)
    }

    pub fn gap(&mut self) {
        self.standing.gap();
    }

    pub fn not_recorded(&self) -> Option<String> {
        self.standing.not_recorded()
    }

    fn apply(&mut self, delta: &Delta) {
        for &slot in delta.deleted.slots() {
            let at = slot as usize;
            if at < self.live.len() && self.live[at] {
                self.live[at] = false;
                if self.index.get(&self.keys[at]) == Some(&slot) {
                    self.index.remove(&self.keys[at]);
                }
            }
        }
        for (&slot, &key) in delta.inserted.slots.iter().zip(&delta.inserted.keys) {
            let at = slot as usize;
            self.grow(at);
            self.live[at] = true;
            self.keys[at] = key;
            self.index.insert(key, slot);
        }
        for &(slot, at, value, state) in &delta.numbers {
            self.grow(slot as usize);
            let (_, values, states) = &mut self.numbers[at];
            (values[slot as usize], states[slot as usize]) = (value, state);
        }
        for (slot, at, value, state) in &delta.lists {
            self.grow(*slot as usize);
            let (_, _, values, states) = &mut self.lists[*at];
            (values[*slot as usize], states[*slot as usize]) = (value.clone(), *state);
        }
        for (slot, at, value, state) in &delta.texts {
            self.grow(*slot as usize);
            let (_, values, states) = &mut self.texts[*at];
            (values[*slot as usize], states[*slot as usize]) = (value.clone(), *state);
        }
    }

    pub fn rows(&self) -> impl Iterator<Item = u32> + '_ {
        self.live.iter().enumerate().filter(|&(_, &live)| live).map(|(slot, _)| slot as u32)
    }

    pub fn live(&self, slot: u32) -> bool {
        self.live.get(slot as usize).copied().unwrap_or(false)
    }

    pub fn key(&self, slot: u32) -> Option<u128> {
        self.keys.get(slot as usize).copied()
    }

    pub fn slot(&self, key: u128) -> Option<u32> {
        self.index.get(&key).copied()
    }

    pub fn number<T: Number>(&self, column: Num<T>, slot: u32) -> Cell<T> {
        let Some((_, values, states)) = self.numbers.iter().find(|(held, _, _)| *held == column.column) else { return Cell::Unrecorded };
        let at = slot as usize;
        if at >= values.len() {
            return Cell::Unrecorded;
        }
        join(T::from_raw(values[at]), states[at])
    }

    pub fn list(&self, column: List, slot: u32) -> Cell<&[u8]> {
        let Some((_, _, values, states)) = self.lists.iter().find(|(held, _, _, _)| *held == column.column) else { return Cell::Unrecorded };
        let at = slot as usize;
        if at >= values.len() {
            return Cell::Unrecorded;
        }
        join(&values[at][..], states[at])
    }

    pub fn named_text(&self, column: Text, slot: u32) -> Result<Cell<&[u8]>, u32> {
        let unnamed = self.texts.iter().find(|(held, _, _)| *held == column.column).and_then(|(_, values, states)| (states.get(slot as usize) == Some(&UNNAMED)).then(|| &values[slot as usize]));
        match unnamed {
            Some(id) => Err(u32::from_le_bytes(id[..4].try_into().expect("an unnamed cell holds its id"))),
            None => Ok(self.text(column, slot)),
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn unname(&mut self, column: Text, slot: u32, id: u32) {
        let (_, values, states) = self.texts.iter_mut().find(|(held, _, _)| *held == column.column).expect("a kept text column");
        (values[slot as usize], states[slot as usize]) = (id.to_le_bytes().into(), UNNAMED);
    }

    pub fn text(&self, column: Text, slot: u32) -> Cell<&[u8]> {
        let Some((_, values, states)) = self.texts.iter().find(|(held, _, _)| *held == column.column) else { return Cell::Unrecorded };
        let at = slot as usize;
        if at >= values.len() {
            return Cell::Unrecorded;
        }
        join(&values[at][..], states[at])
    }

    pub fn complete(&self) -> bool {
        self.status() == Some(Status::Complete)
    }

    pub fn status(&self) -> Option<Status> {
        self.standing.status()
    }

    pub fn head(&self) -> Option<Hour> {
        self.standing.head()
    }

    pub fn bytes(&self) -> usize {
        self.standing.bytes() + self.live.capacity() + self.keys.capacity() * 16 + self.index.capacity() * 24
            + self.numbers.iter().map(|(_, values, states)| values.capacity() * 8 + states.capacity()).sum::<usize>()
            + self.lists.iter().map(|(_, _, values, states)| values.capacity() * 16 + values.iter().map(|value| value.len()).sum::<usize>() + states.capacity()).sum::<usize>()
            + self.texts.iter().map(|(_, values, states)| values.capacity() * 16 + values.iter().map(|value| value.len()).sum::<usize>() + states.capacity()).sum::<usize>()
    }
}

#[derive(Default)]
pub struct Tables {
    tables: Vec<Option<Table>>,
}

impl Tables {
    pub fn from_mirrors(mirrors: &Mirrors, shapes: &[Shape]) -> Self {
        let mut tables = Self::default();
        for shape in shapes {
            if let Some(mirror) = mirrors.get(shape.table) {
                let at = shape.table.index();
                if tables.tables.len() <= at {
                    tables.tables.resize_with(at + 1, || None);
                }
                tables.tables[at] = Some(Table::from_mirror(mirror, shape));
            }
        }
        tables
    }

    pub fn get(&self, table: TableId) -> Option<&Table> {
        self.tables.get(table.index()).and_then(Option::as_ref)
    }

    pub fn get_mut(&mut self, table: TableId) -> Option<&mut Table> {
        self.tables.get_mut(table.index()).and_then(Option::as_mut)
    }

    pub fn gap(&mut self) {
        self.tables.iter_mut().flatten().for_each(Table::gap);
    }

    pub fn bytes(&self) -> usize {
        self.tables.iter().flatten().map(Table::bytes).sum()
    }
}

pub fn complete(tables: &Tables, table: TableId) -> Option<&Table> {
    tables.get(table).filter(|table| table.complete())
}

pub trait ListCells {
    fn list_cell(&self, column: List, slot: u32) -> Cell<&[u8]>;
    fn format(&self) -> u16;
}

impl ListCells for Mirror {
    fn list_cell(&self, column: List, slot: u32) -> Cell<&[u8]> {
        self.list(column, slot)
    }

    fn format(&self) -> u16 {
        Mirror::format(self)
    }
}

impl ListCells for Table {
    fn list_cell(&self, column: List, slot: u32) -> Cell<&[u8]> {
        self.list(column, slot)
    }

    fn format(&self) -> u16 {
        Table::format(self)
    }
}

pub fn opaque<T: NativeValue>(rows: &impl ListCells, column: List, slot: u32) -> Result<Recorded<T>, String> {
    let field = match rows.list_cell(column, slot) {
        Cell::Value(bytes) => Field::Opaque(Opaque::Encoded(bytes, rows.format())),
        _ => Field::NotRecorded,
    };
    <Recorded<T> as NativeValue>::from_field(field).map_err(|error| error.to_string())
}

pub fn u32s(table: &Table, column: List, slot: u32) -> impl Iterator<Item = u32> + '_ {
    let bytes = match table.list(column, slot) {
        Cell::Value(bytes) => bytes,
        _ => &[],
    };
    bytes.chunks_exact(4).map(|word| u32::from_le_bytes(word.try_into().expect("four bytes")))
}

#[derive(Default)]
pub struct Lives {
    slots: Vec<u32>,
    keys: Vec<u128>,
}

impl Lives {
    pub fn slots(&self) -> &[u32] {
        &self.slots
    }

    pub fn keys(&self) -> Option<&[u128]> {
        Some(&self.keys)
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }
}

pub struct Written {
    column: usize,
    slots: Vec<u32>,
}

impl Written {
    pub fn column(&self) -> usize {
        self.column
    }

    pub fn slots(&self) -> &[u32] {
        &self.slots
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }
}

pub struct Delta {
    deleted: Lives,
    inserted: Lives,
    columns: Vec<Written>,
    numbers: Vec<(u32, usize, u64, u8)>,
    lists: Vec<(u32, usize, Box<[u8]>, u8)>,
    texts: Vec<(u32, usize, Box<[u8]>, u8)>,
}

impl Delta {
    pub fn deleted(&self) -> &Lives {
        &self.deleted
    }

    pub fn inserted(&self) -> &Lives {
        &self.inserted
    }

    pub fn columns(&self) -> &[Written] {
        &self.columns
    }
}

pub trait Keyed {
    fn keyed(&self) -> Vec<(u128, u32)>;
}

impl Keyed for Mirror {
    fn keyed(&self) -> Vec<(u128, u32)> {
        self.rows().filter_map(|slot| self.key(slot).map(|key| (key, slot))).collect()
    }
}

impl Keyed for Table {
    fn keyed(&self) -> Vec<(u128, u32)> {
        self.rows().filter_map(|slot| self.key(slot).map(|key| (key, slot))).collect()
    }
}

pub fn owner_rows(rows: &impl Keyed, raw: u32) -> Vec<(u32, u32)> {
    let mut held: Vec<(u32, u32)> = rows.keyed().into_iter().filter(|(key, _)| (key >> 32) as u32 == raw).map(|(key, slot)| (key as u32, slot)).collect();
    held.sort_unstable();
    held
}
