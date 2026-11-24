
use crate::{invalid, lane::log::Ordinal};
use replay_columnar::{
    manifest::Status,
    store::{ColumnState, Flags, TableState, Value},
};
use replay_data::Hour;
use std::{io, ops::Range};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Form {
    #[default]
    Number,
    Text,
    List,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Raw<'a> {
    Numbers { width: u8, signed: bool, bytes: &'a [u8] },
    Texts { width: u8, bytes: &'a [u8] },
    Lists { ends: &'a [u32], bytes: &'a [u8] },
}

#[derive(Clone, Debug, Default)]
pub struct Written {
    pub(crate) column: usize,
    pub(crate) form: Form,
    pub(crate) slots: Vec<u32>,
    pub(crate) width: u8,
    pub(crate) signed: bool,
    pub(crate) numbers: Vec<u8>,
    pub(crate) ends: Vec<u32>,
    pub(crate) bytes: Vec<u8>,
    pub(crate) missing: Vec<u32>,
    pub(crate) unrecorded: Vec<u32>,
}

impl Written {
    pub fn column(&self) -> usize {
        self.column
    }

    pub fn list(&self) -> bool {
        self.form == Form::List
    }

    pub fn slots(&self) -> &[u32] {
        &self.slots
    }

    pub fn len(&self) -> usize {
        self.slots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    pub fn raw(&self) -> Raw<'_> {
        match self.form {
            Form::Number => Raw::Numbers { width: self.width, signed: self.signed, bytes: &self.numbers },
            Form::Text => Raw::Texts { width: self.width, bytes: &self.numbers },
            Form::List => Raw::Lists { ends: &self.ends, bytes: &self.bytes },
        }
    }

    pub fn missing(&self) -> &[u32] {
        &self.missing
    }

    pub fn unrecorded(&self) -> &[u32] {
        &self.unrecorded
    }

    pub fn value(&self, index: usize) -> Value<'_> {
        let at = index as u32;
        if self.unrecorded.binary_search(&at).is_ok() {
            Value::Unrecorded
        } else if self.missing.binary_search(&at).is_ok() {
            Value::Missing
        } else {
            self.held(index)
        }
    }

    pub fn values(&self) -> impl Iterator<Item = (u32, Value<'_>)> + '_ {
        let (mut missing, mut unrecorded) = (0, 0);
        self.slots.iter().enumerate().map(move |(index, &slot)| {
            let at = index as u32;
            if self.unrecorded.get(unrecorded) == Some(&at) {
                unrecorded += 1;
                return (slot, Value::Unrecorded);
            }
            if self.missing.get(missing) == Some(&at) {
                missing += 1;
                return (slot, Value::Missing);
            }
            (slot, self.held(index))
        })
    }

    pub fn each(&self, mut each: impl FnMut(u32, Value<'_>)) {
        if !self.missing.is_empty() || !self.unrecorded.is_empty() {
            self.values().for_each(|(slot, value)| each(slot, value));
            return;
        }
        let slots = &self.slots;
        let number = |bits: u64| if self.form == Form::Text { Value::Text(bits as u32) } else { Value::Number(bits) };
        match (self.form, self.width, self.signed) {
            (Form::List, ..) => {
                let mut start = 0;
                for (&slot, &end) in slots.iter().zip(&self.ends) {
                    each(slot, Value::List(&self.bytes[start..end as usize]));
                    start = end as usize;
                }
            }
            (_, 0, _) => slots.iter().for_each(|&slot| each(slot, number(0))),
            (_, 1, false) => slots.iter().zip(&self.numbers).for_each(|(&slot, &byte)| each(slot, number(u64::from(byte)))),
            (_, 1, true) => slots.iter().zip(&self.numbers).for_each(|(&slot, &byte)| each(slot, number(byte as i8 as i64 as u64))),
            (_, 2, signed) => slots.iter().zip(self.numbers.chunks_exact(2)).for_each(|(&slot, held)| {
                let bits = u16::from_le_bytes([held[0], held[1]]);
                each(slot, number(if signed { bits as i16 as i64 as u64 } else { u64::from(bits) }));
            }),
            (_, 4, signed) => slots.iter().zip(self.numbers.chunks_exact(4)).for_each(|(&slot, held)| {
                let bits = u32::from_le_bytes(held.try_into().expect("four bytes"));
                each(slot, number(if signed { bits as i32 as i64 as u64 } else { u64::from(bits) }));
            }),
            _ => slots.iter().zip(self.numbers.chunks_exact(8)).for_each(|(&slot, held)| each(slot, number(u64::from_le_bytes(held.try_into().expect("eight bytes"))))),
        }
    }

    #[inline]
    fn held(&self, index: usize) -> Value<'_> {
        match self.form {
            Form::List => {
                let start = if index == 0 { 0 } else { self.ends[index - 1] as usize };
                Value::List(&self.bytes[start..self.ends[index] as usize])
            }
            Form::Text => Value::Text(self.number(index) as u32),
            Form::Number => Value::Number(self.number(index)),
        }
    }

    #[inline]
    fn number(&self, index: usize) -> u64 {
        let width = self.width as usize;
        let bytes = &self.numbers[index * width..index * width + width];
        match width {
            0 => 0,
            1 if self.signed => bytes[0] as i8 as i64 as u64,
            1 => u64::from(bytes[0]),
            2 if self.signed => i16::from_le_bytes([bytes[0], bytes[1]]) as i64 as u64,
            2 => u64::from(u16::from_le_bytes([bytes[0], bytes[1]])),
            4 if self.signed => i32::from_le_bytes(bytes.try_into().expect("four bytes")) as i64 as u64,
            4 => u64::from(u32::from_le_bytes(bytes.try_into().expect("four bytes"))),
            _ => u64::from_le_bytes(bytes.try_into().expect("eight bytes")),
        }
    }

    pub(crate) fn start(&mut self, column: usize, held: &ColumnState, staged: bool) {
        self.column = column;
        self.clear();
        (self.form, self.width, self.signed) = match held {
            ColumnState::Number { cells, .. } => (Form::Number, cells.width(), cells.signed()),
            ColumnState::Bool { .. } => (Form::Number, 1, false),
            ColumnState::Text { ids, .. } => (Form::Text, ids.width(), false),
            ColumnState::List { .. } => (Form::List, 0, false),
        };
        if staged && self.form != Form::List {
            (self.width, self.signed) = (8, false);
        }
    }

    pub(crate) fn fit(&mut self, bits: u64) {
        if self.form == Form::List {
            return;
        }
        let needed = match self.signed {
            false if bits <= 0xff => 1,
            false if bits <= 0xffff => 2,
            false if bits <= 0xffff_ffff => 4,
            false => 8,
            true => match bits as i64 {
                value if i8::try_from(value).is_ok() => 1,
                value if i16::try_from(value).is_ok() => 2,
                value if i32::try_from(value).is_ok() => 4,
                _ => 8,
            },
        };
        self.width = self.width.max(needed);
    }

    pub(crate) fn narrow(&mut self, held: &ColumnState) {
        if self.form == Form::List || self.width != 8 {
            return;
        }
        let cells = self.slots.len();
        let staged = |numbers: &[u8], index: usize| u64::from_le_bytes(numbers[index * 8..index * 8 + 8].try_into().expect("eight bytes"));
        (self.width, self.signed) = (1, matches!(held, ColumnState::Number { cells, .. } if cells.signed()));
        for index in 0..cells {
            self.fit(staged(&self.numbers, index));
        }
        let width = self.width as usize;
        for index in 0..cells {
            let bits = staged(&self.numbers, index).to_le_bytes();
            self.numbers[index * width..index * width + width].copy_from_slice(&bits[..width]);
        }
        self.numbers.truncate(cells * width);
    }

    pub(crate) fn push(&mut self, slot: u32, value: Value<'_>) -> io::Result<()> {
        let index = self.slots.len() as u32;
        self.slots.push(slot);
        let bits = match value {
            Value::Unrecorded => {
                self.unrecorded.push(index);
                0
            }
            Value::Missing => {
                self.missing.push(index);
                0
            }
            Value::Number(bits) => bits,
            Value::Text(id) => u64::from(id),
            Value::List(bytes) => {
                self.bytes.extend_from_slice(bytes);
                0
            }
        };
        if self.form == Form::List {
            self.ends.push(u32::try_from(self.bytes.len()).map_err(|_| invalid("A step's list cells of one column pass 4 GiB"))?);
        } else {
            self.numbers.extend_from_slice(&bits.to_le_bytes()[..self.width as usize]);
        }
        Ok(())
    }

    fn clear(&mut self) {
        self.slots.clear();
        self.numbers.clear();
        self.ends.clear();
        self.bytes.clear();
        self.missing.clear();
        self.unrecorded.clear();
    }

    pub(crate) fn fill(&mut self, held: &ColumnState) -> io::Result<()> {
        self.numbers.clear();
        self.ends.clear();
        self.bytes.clear();
        self.missing.clear();
        self.unrecorded.clear();
        let Self { slots, numbers, ends, bytes, missing, unrecorded, .. } = self;
        let marked = |bits: &Option<Flags>, out: &mut Vec<u32>| {
            if let Some(bits) = bits {
                out.extend(slots.iter().enumerate().filter(|&(_, &slot)| bits.get(slot)).map(|(index, _)| index as u32));
            }
        };
        match held {
            ColumnState::Number { cells, present, unrecorded: bits, .. } => {
                self.form = Form::Number;
                (self.width, self.signed) = (cells.width(), cells.signed());
                if bits.is_some() || present.is_some() {
                    for (index, &slot) in slots.iter().enumerate() {
                        if bits.as_ref().is_some_and(|bits| bits.get(slot)) {
                            unrecorded.push(index as u32);
                        } else if present.as_ref().is_some_and(|present| !present.get(slot)) {
                            missing.push(index as u32);
                        }
                    }
                }
                put_numbers(numbers, self.width, slots, |slot| cells.get(slot));
            }
            ColumnState::Bool { cells, unrecorded: bits } => {
                self.form = Form::Number;
                (self.width, self.signed) = (1, false);
                marked(bits, unrecorded);
                numbers.extend(slots.iter().map(|&slot| u8::from(cells.get(slot))));
            }
            ColumnState::Text { ids, unrecorded: bits } => {
                self.form = Form::Text;
                (self.width, self.signed) = (ids.width(), false);
                marked(bits, unrecorded);
                put_numbers(numbers, self.width, slots, |slot| ids.get(slot));
            }
            ColumnState::List { unrecorded: bits, .. } => {
                self.form = Form::List;
                marked(bits, unrecorded);
                ends.reserve(slots.len());
                let mut next = 0;
                for (index, &slot) in slots.iter().enumerate() {
                    if unrecorded.get(next) == Some(&(index as u32)) {
                        next += 1;
                    } else if let Value::List(cell) = held.held(slot) {
                        bytes.extend_from_slice(cell);
                    }
                    ends.push(u32::try_from(bytes.len()).map_err(|_| invalid("A step's list cells of one column pass 4 GiB"))?);
                }
            }
        }
        Ok(())
    }

    fn bytes_held(&self) -> usize {
        (self.slots.capacity() + self.ends.capacity() + self.missing.capacity() + self.unrecorded.capacity()) * 4 + self.numbers.capacity() + self.bytes.capacity()
    }

    fn payload(&self) -> usize {
        (self.slots.len() + self.ends.len() + self.missing.len() + self.unrecorded.len()) * 4 + self.numbers.len() + self.bytes.len()
    }
}

fn put_numbers(out: &mut Vec<u8>, width: u8, slots: &[u32], get: impl Fn(u32) -> u64) {
    out.reserve(slots.len() * width as usize);
    match width {
        0 => {}
        1 => out.extend(slots.iter().map(|&slot| get(slot) as u8)),
        2 => slots.iter().for_each(|&slot| out.extend_from_slice(&(get(slot) as u16).to_le_bytes())),
        4 => slots.iter().for_each(|&slot| out.extend_from_slice(&(get(slot) as u32).to_le_bytes())),
        _ => slots.iter().for_each(|&slot| out.extend_from_slice(&get(slot).to_le_bytes())),
    }
}

#[derive(Clone, Debug, Default)]
pub struct Lives {
    pub(crate) hours: Vec<Hour>,
    pub(crate) slots: Vec<u32>,
    pub(crate) keys: Vec<u128>,
    pub(crate) keyed: bool,
}

impl Lives {
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    pub fn slots(&self) -> &[u32] {
        &self.slots
    }

    pub fn hours(&self) -> &[Hour] {
        &self.hours
    }

    pub fn keys(&self) -> Option<&[u128]> {
        self.keyed.then_some(self.keys.as_slice())
    }

    pub(crate) fn push(&mut self, hour: Hour, slot: u32, key: u128) {
        self.hours.push(hour);
        self.slots.push(slot);
        if self.keyed {
            self.keys.push(key);
        }
    }

    pub(crate) fn reverse(&mut self) {
        self.hours.reverse();
        self.slots.reverse();
        self.keys.reverse();
    }

    fn clear(&mut self, keyed: bool) {
        self.hours.clear();
        self.slots.clear();
        self.keys.clear();
        self.keyed = keyed;
    }

    fn bytes_held(&self) -> usize {
        self.hours.capacity() * 4 + self.slots.capacity() * 4 + self.keys.capacity() * 16
    }
}

#[derive(Clone, Debug, Default)]
pub struct Texts {
    pub(crate) ids: Vec<u32>,
    pub(crate) ends: Vec<u32>,
    pub(crate) bytes: Vec<u8>,
}

impl Texts {
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (u32, &[u8])> + '_ {
        self.ids.iter().enumerate().map(|(at, &id)| {
            let start = if at == 0 { 0 } else { self.ends[at - 1] as usize };
            (id, &self.bytes[start..self.ends[at] as usize])
        })
    }

    pub(crate) fn push(&mut self, id: u32, text: &[u8]) {
        self.bytes.extend_from_slice(text);
        self.ids.push(id);
        self.ends.push(self.bytes.len() as u32);
    }

    pub(crate) fn clear(&mut self) {
        self.ids.clear();
        self.ends.clear();
        self.bytes.clear();
    }

    fn bytes_held(&self) -> usize {
        self.ids.capacity() * 4 + self.ends.capacity() * 4 + self.bytes.capacity()
    }
}

#[derive(Clone, Debug, Default)]
pub struct Passing {
    pub(crate) born: Vec<Hour>,
    pub(crate) last: Vec<Hour>,
    pub(crate) keys: Vec<u128>,
    pub(crate) keyed: bool,
    pub(crate) columns: Vec<Written>,
}

impl Passing {
    pub fn len(&self) -> usize {
        self.born.len()
    }

    pub fn is_empty(&self) -> bool {
        self.born.is_empty()
    }

    pub fn born(&self) -> &[Hour] {
        &self.born
    }

    pub fn last(&self) -> &[Hour] {
        &self.last
    }

    pub fn keys(&self) -> Option<&[u128]> {
        self.keyed.then_some(self.keys.as_slice())
    }

    pub fn columns(&self) -> &[Written] {
        &self.columns
    }

    fn clear(&mut self) {
        self.born.clear();
        self.last.clear();
        self.keys.clear();
        self.columns.clear();
    }

    fn bytes_held(&self) -> usize {
        (self.born.capacity() + self.last.capacity()) * 4 + self.keys.capacity() * 16 + self.columns.iter().map(Written::bytes_held).sum::<usize>() + self.columns.capacity() * size_of::<Written>()
    }

    fn payload(&self) -> usize {
        (self.born.len() + self.last.len()) * 4 + self.keys.len() * 16 + self.columns.iter().map(Written::payload).sum::<usize>()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Forgotten {
    pub hour: Hour,
    pub from: Ordinal,
    pub to: Ordinal,
}

#[derive(Clone, Debug, Default)]
pub struct Logged {
    pub(crate) run: usize,
    pub(crate) first: Ordinal,
    pub(crate) hours: Vec<Hour>,
    pub(crate) keys: Vec<u128>,
    pub(crate) keyed: bool,
    pub(crate) columns: Vec<Written>,
    pub(crate) forgotten: Vec<Forgotten>,
    pub(crate) count: Ordinal,
    pub(crate) listed: u64,
}

impl Logged {
    pub fn run(&self) -> usize {
        self.run
    }

    pub fn appended(&self) -> Range<Ordinal> {
        self.first..self.first + self.hours.len() as Ordinal
    }

    pub fn len(&self) -> usize {
        self.hours.len()
    }

    pub fn is_empty(&self) -> bool {
        self.hours.is_empty()
    }

    pub fn hours(&self) -> &[Hour] {
        &self.hours
    }

    pub fn keys(&self) -> Option<&[u128]> {
        self.keyed.then_some(self.keys.as_slice())
    }

    pub fn columns(&self) -> &[Written] {
        &self.columns
    }

    pub fn column(&self, column: usize) -> Option<&Written> {
        self.columns.iter().find(|written| written.column == column)
    }

    pub fn rows(&self) -> impl Iterator<Item = (u32, Ordinal, Hour)> + '_ {
        self.hours.iter().enumerate().map(|(place, &hour)| (place as u32, self.first + place as Ordinal, hour))
    }

    pub fn forgotten(&self) -> &[Forgotten] {
        &self.forgotten
    }

    pub fn count(&self) -> Ordinal {
        self.count
    }

    pub fn listed(&self) -> u64 {
        self.listed
    }

    pub(crate) fn begin(&mut self, run: usize, count: Ordinal, keyed: bool, columns: &[usize], template: &TableState) {
        (self.run, self.first, self.count, self.keyed) = (run, count, count, keyed);
        self.hours.clear();
        self.keys.clear();
        self.forgotten.clear();
        self.columns.truncate(columns.len());
        self.columns.resize_with(columns.len(), Written::default);
        for (written, &column) in self.columns.iter_mut().zip(columns) {
            written.start(column, template.column(column).expect("a selected column"), true);
        }
    }

    pub(crate) fn push(&mut self, hour: Hour, key: u128, rows: &TableState, at: u32) -> io::Result<()> {
        let place = self.hours.len() as u32;
        self.hours.push(hour);
        if self.keyed {
            self.keys.push(key);
        }
        for written in &mut self.columns {
            written.push(place, rows.value(written.column, at))?;
        }
        Ok(())
    }

    pub(crate) fn finish(&mut self, count: Ordinal, listed: u64, template: &TableState) {
        (self.count, self.listed) = (count, listed);
        for written in &mut self.columns {
            written.narrow(template.column(written.column).expect("a selected column"));
        }
    }

    fn clear(&mut self) {
        self.hours.clear();
        self.keys.clear();
        self.columns.clear();
        self.forgotten.clear();
    }

    fn bytes_held(&self) -> usize {
        self.hours.capacity() * 4 + self.keys.capacity() * 16 + self.forgotten.capacity() * size_of::<Forgotten>() + self.columns.iter().map(Written::bytes_held).sum::<usize>() + self.columns.capacity() * size_of::<Written>()
    }

    fn payload(&self) -> usize {
        self.hours.len() * 4 + self.keys.len() * 16 + self.forgotten.len() * size_of::<Forgotten>() + self.columns.iter().map(Written::payload).sum::<usize>()
    }
}

#[derive(Clone, Debug, Default)]
pub struct Changes {
    pub(crate) table: usize,
    pub(crate) reset: bool,
    pub(crate) head: Option<Hour>,
    pub(crate) short: u32,
    pub(crate) stalled: bool,
    pub(crate) columns: Vec<Written>,
    pub(crate) deleted: Lives,
    pub(crate) inserted: Lives,
    pub(crate) status: Option<(Status, Option<Hour>)>,
    pub(crate) status_left: Option<Hour>,
    pub(crate) read_rows_changed: bool,
    pub(crate) renumbered: bool,
    pub(crate) renumber: Vec<u32>,
    pub(crate) texts: Texts,
    pub(crate) passing: Passing,
    pub(crate) is_log: bool,
    pub(crate) log: Logged,
}

impl Changes {
    pub fn table(&self) -> usize {
        self.table
    }

    pub fn reset(&self) -> bool {
        self.reset
    }

    pub fn head(&self) -> Option<Hour> {
        self.head
    }

    pub fn short(&self) -> u32 {
        self.short
    }

    pub fn stalled(&self) -> bool {
        self.stalled
    }

    pub fn columns(&self) -> &[Written] {
        &self.columns
    }

    pub fn deleted(&self) -> &Lives {
        &self.deleted
    }

    pub fn inserted(&self) -> &Lives {
        &self.inserted
    }

    pub fn status(&self) -> Option<(Status, Option<Hour>)> {
        self.status
    }

    pub fn status_left(&self) -> Option<Hour> {
        self.status_left
    }

    pub fn read_rows_changed(&self) -> bool {
        self.read_rows_changed
    }

    pub fn renumber(&self) -> Option<&[u32]> {
        self.renumbered.then_some(self.renumber.as_slice())
    }

    pub fn texts(&self) -> &Texts {
        &self.texts
    }

    pub fn passing(&self) -> &Passing {
        &self.passing
    }

    pub fn log(&self) -> Option<&Logged> {
        self.is_log.then_some(&self.log)
    }

    pub fn cells(&self) -> usize {
        self.columns.iter().map(Written::len).sum::<usize>() + self.log.columns.iter().map(Written::len).sum::<usize>()
    }

    pub fn rows(&self) -> usize {
        self.deleted.len() + self.inserted.len() + self.log.hours.len() + self.log.forgotten.iter().map(|run| (run.to - run.from) as usize).sum::<usize>()
    }

    pub fn payload(&self) -> usize {
        self.columns.iter().map(Written::payload).sum::<usize>() + (self.deleted.len() + self.inserted.len()) * 8 + self.renumber.len() * 4 + self.texts.bytes.len() + self.texts.ids.len() * 8 + self.passing.payload() + self.log.payload()
    }

    pub fn bytes_held(&self) -> usize {
        self.columns.iter().map(Written::bytes_held).sum::<usize>()
            + self.columns.capacity() * size_of::<Written>()
            + self.deleted.bytes_held()
            + self.inserted.bytes_held()
            + self.renumber.capacity() * 4
            + self.texts.bytes_held()
            + self.passing.bytes_held()
            + self.log.bytes_held()
    }

    pub(crate) fn begin(&mut self, table: usize, columns: &[usize], keyed: bool, reset: bool) {
        self.table = table;
        self.reset = reset;
        self.head = None;
        self.short = 0;
        self.stalled = false;
        self.columns.truncate(columns.len());
        self.columns.resize_with(columns.len(), Written::default);
        for (written, &column) in self.columns.iter_mut().zip(columns) {
            written.column = column;
            written.clear();
        }
        self.deleted.clear(keyed);
        self.inserted.clear(keyed);
        self.status = None;
        self.status_left = None;
        self.read_rows_changed = false;
        self.renumbered = false;
        self.renumber.clear();
        self.texts.clear();
        self.passing.clear();
        self.is_log = false;
        self.log.clear();
    }

    pub(crate) fn set_head(&mut self, head: Hour, short: u32, stalled: bool) {
        self.head = Some(head);
        self.short = short;
        self.stalled = stalled;
    }

    pub(crate) fn set_renumber(&mut self, old: impl Iterator<Item = u32>) {
        self.renumber.clear();
        self.renumber.extend(old);
        self.renumbered = true;
    }
}
