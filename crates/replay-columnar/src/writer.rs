//! The writer's side of the change grammar: one table's held columns, an hour's candidate
//! rows compared once with them, and the section that hour writes. Each changed cell is
//! written as the held cell's ADD or PATCH when that takes no more bytes than its SET; a
//! column whose changed cells all take one value writes it once, as a FILL when they are at
//! least half its rows. [`TableWriter::commit`] applies the section through the reader's own
//! [`section::apply`], so after the same hours the writer holds exactly the table, status,
//! sample hour and view a walker holds.
//!
//! An hour's rows come one at a time ([`Hour::row`]) or together as a [`Batch`]
//! ([`TableWriter::prepare`]), which compares each column with the held one in one pass and
//! writes the same section.
//!
//! An hour clears and finishes only the columns it or the hour before staged a cell in, so
//! an hour that changes nothing costs its table no work per column. An hour after one that
//! did not finish (refused, dropped or unwound) clears every column.

use crate::{
    invalid_static,
    leaf::varint_len,
    manifest::{self, Status},
    restart::{self, Acquisition, Restored},
    section::{self, Changes, Op, PatchItems, Patches, Run, Scratch, TableHour},
    state::{Bits, Cell, Column, Kind, Table, Values},
};
use std::{
    cmp::Reverse,
    collections::{BinaryHeap, HashMap},
    io,
    sync::atomic::{AtomicU64, Ordering},
};

#[cfg(test)]
mod tests;

type Hasher = foldhash::fast::RandomState;

/// Names each writer, so a preparation commits only to the writer that made it.
static WRITERS: AtomicU64 = AtomicU64::new(1);

/// The fewest changed cells of one value a FILL writes; fewer take one SET run.
const FILL_MINIMUM: usize = 16;

/// A batch row the table does not hold. No slot is `u32::MAX`: an hour keeps the table's
/// extent within 32 bits.
const NEW: u32 = u32::MAX;

/// A run's strings: exact bytes with append-only ids, each appended before its first use.
#[derive(Default)]
pub struct Dictionary {
    ids: HashMap<Box<[u8]>, u32, Hasher>,
    len: u32,
    appended: Vec<Box<[u8]>>,
    /// Every string by id: string `id` ends at `ends[id]` in `bytes`.
    bytes: Vec<u8>,
    ends: Vec<usize>,
}

impl Dictionary {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.len as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// String `id`'s bytes.
    pub fn get(&self, id: u32) -> Option<&[u8]> {
        let id = id as usize;
        let end = *self.ends.get(id)?;
        let start = id.checked_sub(1).map_or(0, |previous| self.ends[previous]);
        Some(&self.bytes[start..end])
    }

    /// The id of `bytes`, appending it when the run has not used it.
    pub fn intern(&mut self, bytes: &[u8]) -> io::Result<u32> {
        if let Some(&id) = self.ids.get(bytes) {
            return Ok(id);
        }
        let id = self.len;
        let next = id
            .checked_add(1)
            .ok_or_else(|| invalid_static("Dictionary ids exceed 32 bits"))?;
        let mut owned = engine_allocation::vec(bytes.len())?;
        owned.extend_from_slice(bytes);
        let owned = engine_allocation::boxed_slice(owned)?;
        let mut copy = engine_allocation::vec(bytes.len())?;
        copy.extend_from_slice(bytes);
        let copy = engine_allocation::boxed_slice(copy)?;
        engine_allocation::reserve_map(&mut self.ids, 1)?;
        engine_allocation::reserve(&mut self.appended, 1)?;
        engine_allocation::reserve(&mut self.bytes, bytes.len())?;
        engine_allocation::reserve(&mut self.ends, 1)?;
        self.ids.insert(owned, id);
        self.appended.push(copy);
        self.bytes.extend_from_slice(bytes);
        self.ends.push(self.bytes.len());
        self.len = next;
        Ok(id)
    }

    /// The strings appended since the last take, in id order: the hour's manifest appends.
    pub fn take_appended(&mut self) -> Vec<Box<[u8]>> {
        std::mem::take(&mut self.appended)
    }
}

/// The run's dictionary as one table's hour uses it. Tables prepared side by side share it
/// [`Strings::Reading`], where a string the run has not used fails the hour: that table is
/// prepared again interning, in table order, so every id is the one a single thread assigns.
pub enum Strings<'a> {
    Interning(&'a mut Dictionary),
    Reading(&'a Dictionary),
}

impl Strings<'_> {
    /// The id of `bytes`; reading, a string the run has not used fails.
    pub fn intern(&mut self, bytes: &[u8]) -> io::Result<u32> {
        match self {
            Self::Interning(dictionary) => dictionary.intern(bytes),
            Self::Reading(dictionary) => dictionary
                .ids
                .get(bytes)
                .copied()
                .ok_or_else(|| invalid_static("A new string is interned in table order")),
        }
    }

    pub fn dictionary(&self) -> &Dictionary {
        match self {
            Self::Interning(dictionary) => dictionary,
            Self::Reading(dictionary) => dictionary,
        }
    }
}

impl<'a> From<&'a mut Dictionary> for Strings<'a> {
    fn from(dictionary: &'a mut Dictionary) -> Self {
        Self::Interning(dictionary)
    }
}

/// Which held rows an hour's visits cover.
#[derive(Clone, Copy, Debug)]
pub enum Coverage<'a> {
    /// A complete census: a held row the hour does not visit is deleted.
    Complete,
    /// A partial read: the held rows visited are the rows read, and nothing is deleted.
    Partial,
    /// Proven sparse changes on a complete table: only the visited rows change, and
    /// `removed` rows are deleted.
    Changes { removed: &'a [u128] },
}

/// One candidate cell.
#[derive(Clone, Copy, Debug)]
pub enum Input<'a> {
    Value(Cell<'a>),
    /// A Bytes column's string, interned in the run's dictionary.
    Text(&'a [u8]),
    /// The recording did not record this cell.
    Unrecorded,
    /// The cell the held row already holds: neither compared nor written. Only a held row's.
    Kept,
}

/// The sample hour a table's view holds after an hour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sample {
    /// The read bit decides: a read table is sampled at the hour, another keeps its last.
    FromReadBit,
    /// Not known: an untimed snapshot.
    Reset,
    /// A carried acquisition's hour.
    Carried(i32),
}

/// A table's evidence for one hour, beside its rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Evidence {
    pub status: Status,
    /// The manifest's read bit.
    pub read: bool,
    pub sample: Sample,
}

/// What an hour's runs carry: their values by op, and its FILLs with the slots they set and
/// the live slots they leave.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Census {
    pub set: u64,
    pub add: u64,
    pub patch: u64,
    pub fills: u64,
    pub fill_slots: u64,
    pub fill_exceptions: u64,
}

/// Which columns a writer's hours clear and finish.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sweep {
    /// Those the hour or the hour before staged a cell in.
    #[default]
    Touched,
    /// Every column, every hour, and an hour of no rows staged as any other: the reference,
    /// which writes the same sections.
    Every,
}

/// What a writer's hours have cost it so far.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Work {
    /// Hours begun.
    pub hours: u64,
    /// Columns an hour cleared or finished.
    pub columns: u64,
}

/// One column of a [`Batch`]: every row's cell, in the order the rows came.
#[derive(Default)]
struct Cells {
    /// A scalar cell's bits as [`word`] takes them; a text, list or opaque cell's end in its
    /// items.
    words: Vec<u64>,
    /// An optional cell's presence, a bit per row.
    present: Vec<u64>,
    u32s: Vec<u32>,
    u64s: Vec<u64>,
    bytes: Vec<u8>,
    /// The rows whose cell the recording did not record, ascending.
    unrecorded: Vec<u32>,
    /// The rows whose cell is the one their held row holds, ascending.
    kept: Vec<u32>,
}

fn optional(kind: Kind) -> bool {
    matches!(
        kind,
        Kind::OptionalI32 | Kind::OptionalU32 | Kind::OptionalI64 | Kind::OptionalU64
    )
}

impl Cells {
    fn clear(&mut self) {
        self.words.clear();
        self.present.clear();
        self.u32s.clear();
        self.u64s.clear();
        self.bytes.clear();
        self.unrecorded.clear();
        self.kept.clear();
    }

    /// The end of the items a `kind` column's cells take so far.
    fn end(&self, kind: Kind) -> u64 {
        (match kind {
            Kind::U32List => self.u32s.len(),
            Kind::U64List => self.u64s.len(),
            Kind::Bytes | Kind::Opaque => self.bytes.len(),
            _ => 0,
        }) as u64
    }

    /// The items of row `row`'s text, list or opaque cell.
    fn span(&self, row: usize) -> std::ops::Range<usize> {
        let start = row
            .checked_sub(1)
            .map_or(0, |previous| self.words[previous] as usize);
        start..self.words[row] as usize
    }

    fn present(&self, row: usize) -> bool {
        self.present
            .get(row / 64)
            .is_some_and(|bits| bits >> (row % 64) & 1 != 0)
    }

    /// Row `row`'s cell of a `kind` column other than text, none when it is unrecorded.
    fn cell(&self, kind: Kind, row: u32) -> Option<Cell<'_>> {
        if self.unrecorded.binary_search(&row).is_ok() {
            return None;
        }
        let at = row as usize;
        let word = self.words[at];
        let present = self.present(at);
        Some(match kind {
            Kind::U8 => Cell::U8(word as u8),
            Kind::I8 => Cell::I8(word as i8),
            Kind::U16 => Cell::U16(word as u16),
            Kind::I16 => Cell::I16(word as i16),
            Kind::U32 => Cell::U32(word as u32),
            Kind::I32 => Cell::I32(word as i32),
            Kind::U64 => Cell::U64(word),
            Kind::I64 => Cell::I64(word as i64),
            Kind::Bool => Cell::Bool(word != 0),
            Kind::OptionalI32 => Cell::OptionalI32(present.then_some(word as i32)),
            Kind::OptionalU32 => Cell::OptionalU32(present.then_some(word as u32)),
            Kind::OptionalI64 => Cell::OptionalI64(present.then_some(word as i64)),
            Kind::OptionalU64 => Cell::OptionalU64(present.then_some(word)),
            Kind::Bytes => return None,
            Kind::U32List => Cell::U32List(&self.u32s[self.span(at)]),
            Kind::U64List => Cell::U64List(&self.u64s[self.span(at)]),
            Kind::Opaque => Cell::Opaque(&self.bytes[self.span(at)]),
        })
    }
}

/// An hour's rows by column, a row at a time: each row's key, then one cell per column in
/// any order. Kept between hours, so a warm batch allocates nothing.
#[derive(Default)]
pub struct Batch {
    kinds: Vec<Kind>,
    keys: Vec<u128>,
    /// One per column of `kinds`, and those wider tables left.
    columns: Vec<Cells>,
}

impl Batch {
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts an hour of about `rows` rows of a table with these columns.
    pub fn begin(&mut self, kinds: &[Kind], rows: usize) -> io::Result<()> {
        // A cell follows its row's key, so only a batch with rows holds cells, and only in
        // its own columns.
        if !self.keys.is_empty() {
            for cells in self.columns.iter_mut().take(self.kinds.len()) {
                cells.clear();
            }
        }
        self.keys.clear();
        if self.columns.len() < kinds.len() {
            engine_allocation::resize_default(&mut self.columns, kinds.len())?;
        }
        self.kinds.clear();
        extend(&mut self.kinds, kinds)?;
        engine_allocation::reserve(&mut self.keys, rows)?;
        if rows > 0 {
            for (cells, &kind) in self.columns.iter_mut().zip(kinds) {
                engine_allocation::reserve(&mut cells.words, rows)?;
                if optional(kind) {
                    engine_allocation::reserve(&mut cells.present, rows.div_ceil(64))?;
                }
            }
        }
        Ok(())
    }

    pub fn rows(&self) -> usize {
        self.keys.len()
    }

    /// Starts the next row.
    #[inline]
    pub fn key(&mut self, key: u128) -> io::Result<()> {
        push(&mut self.keys, key)
    }

    /// The current row's cells of `column`, which holds `kind`, and the row.
    #[inline]
    fn next(&mut self, column: usize, kind: Kind) -> io::Result<(&mut Cells, usize)> {
        let row = self.keys.len().wrapping_sub(1);
        let held = *self
            .kinds
            .get(column)
            .ok_or_else(|| invalid_static("A batch column is past the schema"))?;
        if held != kind {
            return Err(invalid_static("A batch cell's kind is not its column's"));
        }
        let cells = &mut self.columns[column];
        if cells.words.len() != row {
            return Err(invalid_static(
                "A batch row carries other than one cell per column",
            ));
        }
        Ok((cells, row))
    }

    /// The current row's cell of `column`: any but a text cell, which is its bytes.
    #[inline]
    pub fn value(&mut self, column: usize, cell: Cell<'_>) -> io::Result<()> {
        let (cells, row) = self.next(column, cell_kind(&cell))?;
        match cell {
            Cell::Bytes(_) => Err(invalid_static("A batch takes text as its bytes")),
            Cell::U32List(items) => {
                extend(&mut cells.u32s, items)?;
                let end = cells.u32s.len() as u64;
                push(&mut cells.words, end)
            }
            Cell::U64List(items) => {
                extend(&mut cells.u64s, items)?;
                let end = cells.u64s.len() as u64;
                push(&mut cells.words, end)
            }
            Cell::Opaque(items) => {
                extend(&mut cells.bytes, items)?;
                let end = cells.bytes.len() as u64;
                push(&mut cells.words, end)
            }
            Cell::OptionalI32(_)
            | Cell::OptionalU32(_)
            | Cell::OptionalI64(_)
            | Cell::OptionalU64(_) => {
                let (word, present) = word(cell);
                presence(cells, row, present)?;
                push(&mut cells.words, word)
            }
            cell => push(&mut cells.words, word(cell).0),
        }
    }

    /// The current row's string in a Bytes column.
    #[inline]
    pub fn text(&mut self, column: usize, text: &[u8]) -> io::Result<()> {
        let (cells, _) = self.next(column, Kind::Bytes)?;
        extend(&mut cells.bytes, text)?;
        let end = cells.bytes.len() as u64;
        push(&mut cells.words, end)
    }

    /// The current row's opaque cell, as `write` appends it.
    #[inline]
    pub fn opaque_with(
        &mut self,
        column: usize,
        write: impl FnOnce(&mut Vec<u8>) -> io::Result<()>,
    ) -> io::Result<()> {
        let (cells, _) = self.next(column, Kind::Opaque)?;
        write(&mut cells.bytes)?;
        let end = cells.bytes.len() as u64;
        push(&mut cells.words, end)
    }

    /// The current row's U32List cell, as `write` appends its items.
    #[inline]
    pub fn u32s_with(
        &mut self,
        column: usize,
        write: impl FnOnce(&mut Vec<u32>) -> io::Result<()>,
    ) -> io::Result<()> {
        let (cells, _) = self.next(column, Kind::U32List)?;
        write(&mut cells.u32s)?;
        let end = cells.u32s.len() as u64;
        push(&mut cells.words, end)
    }

    /// The current row's U64List cell, as `write` appends its items.
    #[inline]
    pub fn u64s_with(
        &mut self,
        column: usize,
        write: impl FnOnce(&mut Vec<u64>) -> io::Result<()>,
    ) -> io::Result<()> {
        let (cells, _) = self.next(column, Kind::U64List)?;
        write(&mut cells.u64s)?;
        let end = cells.u64s.len() as u64;
        push(&mut cells.words, end)
    }

    /// The current row's cell of `column`, which its held row already holds: the hour leaves
    /// it as it is, without comparing it. Only a held row's cell can be kept.
    #[inline]
    pub fn kept(&mut self, column: usize) -> io::Result<()> {
        let kind = *self
            .kinds
            .get(column)
            .ok_or_else(|| invalid_static("A batch column is past the schema"))?;
        let (cells, row) = self.next(column, kind)?;
        push(&mut cells.kept, row as u32)?;
        if optional(kind) {
            presence(cells, row, false)?;
        }
        let end = cells.end(kind);
        push(&mut cells.words, end)
    }

    /// The current row's cell of `column`, which the recording did not record.
    #[inline]
    pub fn unrecorded(&mut self, column: usize) -> io::Result<()> {
        let kind = *self
            .kinds
            .get(column)
            .ok_or_else(|| invalid_static("A batch column is past the schema"))?;
        let (cells, row) = self.next(column, kind)?;
        push(&mut cells.unrecorded, row as u32)?;
        if optional(kind) {
            presence(cells, row, false)?;
        }
        let end = cells.end(kind);
        push(&mut cells.words, end)
    }
}

/// Row `row`'s presence bit, set when `present`.
fn presence(cells: &mut Cells, row: usize, present: bool) -> io::Result<()> {
    if row % 64 == 0 {
        push(&mut cells.present, 0)?;
    }
    if present {
        cells.present[row / 64] |= 1 << (row % 64);
    }
    Ok(())
}

/// A held scalar column's value type and the cells it makes.
trait Word: Copy {
    /// The bits [`word`] takes of its cell.
    fn word(self) -> u64;
    /// The cell of `word`'s bits.
    fn plain(word: u64) -> Cell<'static>;
}

macro_rules! words {
    ($($kind:ty => $variant:ident),+) => {
        $(impl Word for $kind {
            fn word(self) -> u64 {
                self as i64 as u64
            }
            fn plain(word: u64) -> Cell<'static> {
                Cell::$variant(word as $kind)
            }
        })+
    };
}
words!(u8 => U8, i8 => I8, u16 => U16, i16 => I16, u32 => U32, i32 => I32, u64 => U64, i64 => I64);

impl Word for bool {
    fn word(self) -> u64 {
        u64::from(self)
    }
    fn plain(word: u64) -> Cell<'static> {
        Cell::Bool(word != 0)
    }
}

/// An optional column's value type.
trait Present: Word {
    fn optional(value: Option<u64>) -> Cell<'static>;
}

macro_rules! present {
    ($($kind:ty => $variant:ident),+) => {
        $(impl Present for $kind {
            fn optional(value: Option<u64>) -> Cell<'static> {
                Cell::$variant(value.map(|word| word as $kind))
            }
        })+
    };
}
present!(i32 => OptionalI32, u32 => OptionalU32, i64 => OptionalI64, u64 => OptionalU64);

/// A list or opaque column's item type.
trait Items: Copy + PartialEq {
    /// The items one [`Matcher`] window holds: eight bytes of them.
    const PROBE: usize;
    fn cell(items: &[Self]) -> Cell<'_>;
    /// A window's [`Self::PROBE`] items as one word.
    fn probe(items: &[Self]) -> u64;
}

impl Items for u32 {
    const PROBE: usize = 2;
    fn cell(items: &[Self]) -> Cell<'_> {
        Cell::U32List(items)
    }
    fn probe(items: &[Self]) -> u64 {
        u64::from(items[0]) | u64::from(items[1]) << 32
    }
}

impl Items for u64 {
    const PROBE: usize = 1;
    fn cell(items: &[Self]) -> Cell<'_> {
        Cell::U64List(items)
    }
    fn probe(items: &[Self]) -> u64 {
        items[0]
    }
}

impl Items for u8 {
    const PROBE: usize = 8;
    fn cell(items: &[Self]) -> Cell<'_> {
        Cell::Opaque(items)
    }
    fn probe(items: &[Self]) -> u64 {
        u64::from_le_bytes(items[..8].try_into().expect("a window of eight bytes"))
    }
}

/// One batch column beside its held column: the cells it changes go to `pending`, those it
/// no longer records to `unrecord`.
struct Staged<'s> {
    /// Each batch row's held slot, or [`NEW`].
    slots: &'s [u32],
    cells: &'s Cells,
    /// The held column's unrecorded slots, when it has any.
    marked: Option<&'s Bits>,
    pending: &'s mut Pending,
    unrecord: &'s mut Vec<u32>,
    /// A cell was staged in `pending` or `unrecord`.
    dirty: bool,
}

impl Staged<'_> {
    /// Each held row the batch records, with whether the table records it: `change` stages
    /// what differs, saying whether it did. A cell the batch leaves unrecorded is staged here.
    #[inline]
    fn each(
        &mut self,
        mut change: impl FnMut(&mut Pending, usize, u32, bool) -> io::Result<bool>,
    ) -> io::Result<()> {
        let mut skipped = self.cells.unrecorded.iter().copied().peekable();
        let mut kept = self.cells.kept.iter().copied().peekable();
        for (row, &slot) in (0u32..).zip(self.slots) {
            let recorded = skipped.next_if_eq(&row).is_none();
            if kept.next_if_eq(&row).is_some() || slot == NEW {
                continue;
            }
            let was = !self.marked.is_some_and(|bits| bits.get(slot));
            if !recorded {
                if was {
                    self.dirty = true;
                    push(&mut *self.unrecord, slot)?;
                }
                continue;
            }
            self.dirty |= change(&mut *self.pending, row as usize, slot, was)?;
        }
        Ok(())
    }

    fn scalars<T: Word>(&mut self, held: &[T]) -> io::Result<()> {
        let cells = self.cells;
        self.each(|pending, row, slot, was| {
            let value = *held
                .get(slot as usize)
                .ok_or_else(|| invalid_static("A held slot is past its column"))?;
            let new = cells.words[row];
            if was && value.word() == new {
                return Ok(false);
            }
            let held = was.then(|| T::plain(value.word()));
            pending.change(slot, T::plain(new), held).map(|()| true)
        })
    }

    fn optionals<T: Present>(&mut self, held: &[T], presence: &Bits) -> io::Result<()> {
        let cells = self.cells;
        self.each(|pending, row, slot, was| {
            let value = *held
                .get(slot as usize)
                .ok_or_else(|| invalid_static("A held slot is past its column"))?;
            let old = presence.get(slot).then(|| value.word());
            let new = cells.present(row).then(|| cells.words[row]);
            if was && old == new {
                return Ok(false);
            }
            let held = was.then(|| T::optional(old));
            pending.change(slot, T::optional(new), held).map(|()| true)
        })
    }

    fn lists<T: Items>(&mut self, held: &[Box<[T]>], items: &[T]) -> io::Result<()> {
        let cells = self.cells;
        self.each(|pending, row, slot, was| {
            let value = held
                .get(slot as usize)
                .ok_or_else(|| invalid_static("A held slot is past its column"))?;
            let new = &items[cells.span(row)];
            if was && **value == *new {
                return Ok(false);
            }
            let held = was.then(|| T::cell(value));
            pending.change(slot, T::cell(new), held).map(|()| true)
        })
    }
}

/// An empty vector keeping `values`' allocation (std collects in place), for items of
/// another lifetime.
fn recycle<T, U>(values: Vec<T>) -> Vec<U> {
    values.into_iter().filter_map(|_| None).collect()
}

/// One cell's value kept by its own type: integers by their bits, lists and opaque
/// cells as spans of item arrays. A changed cell also keeps how its run writes it.
#[derive(Default)]
struct Pending {
    /// The held slot, or the insert's index, of each entry.
    at: Vec<u32>,
    words: Vec<u64>,
    present: Vec<bool>,
    spans: Vec<(u32, u32)>,
    u32s: Vec<u32>,
    u64s: Vec<u64>,
    bytes: Vec<u8>,
    /// Each changed entry's op: SET, ADD or PATCH.
    ops: Vec<Op>,
    /// An integer column's entries' ADD deltas at the column's width; zero for a SET.
    deltas: Vec<u64>,
    /// A list or opaque column's entries' PATCH edits in `edits`; none for a SET.
    hunks: Vec<(u32, u32)>,
    edits: Vec<(u32, u32, u32)>,
    /// Every entry holds the first entry's value.
    uniform: bool,
}

fn span(start: usize, len: usize) -> io::Result<(u32, u32)> {
    let end = start
        .checked_add(len)
        .filter(|&end| u32::try_from(end).is_ok())
        .ok_or_else(|| invalid_static("Cell items exceed 32 bits"))?;
    Ok((start as u32, (end - start) as u32))
}

fn push<T>(values: &mut Vec<T>, value: T) -> io::Result<()> {
    engine_allocation::reserve(values, 1)?;
    values.push(value);
    Ok(())
}

fn extend<T: Copy>(values: &mut Vec<T>, items: &[T]) -> io::Result<()> {
    engine_allocation::reserve(values, items.len())?;
    values.extend_from_slice(items);
    Ok(())
}

/// A cell's integer bits and presence; a list or opaque cell keeps its items apart.
fn word(cell: Cell<'_>) -> (u64, bool) {
    match cell {
        Cell::U8(value) => (u64::from(value), true),
        Cell::I8(value) => (value as i64 as u64, true),
        Cell::U16(value) => (u64::from(value), true),
        Cell::I16(value) => (value as i64 as u64, true),
        Cell::U32(value) | Cell::Bytes(value) => (u64::from(value), true),
        Cell::I32(value) => (value as i64 as u64, true),
        Cell::U64(value) => (value, true),
        Cell::I64(value) => (value as u64, true),
        Cell::Bool(value) => (u64::from(value), true),
        Cell::OptionalI32(value) => (value.map_or(0, |v| v as i64 as u64), value.is_some()),
        Cell::OptionalU32(value) => (value.map_or(0, u64::from), value.is_some()),
        Cell::OptionalI64(value) => (value.map_or(0, |v| v as u64), value.is_some()),
        Cell::OptionalU64(value) => (value.unwrap_or(0), value.is_some()),
        Cell::U32List(_) | Cell::U64List(_) | Cell::Opaque(_) => (0, true),
    }
}

/// The bits and signedness of a column an ADD can shorten: byte-wide values are raw either
/// way, and other kinds take no ADD.
fn integer(kind: Kind) -> Option<(u32, bool)> {
    Some(match kind {
        Kind::U16 => (16, false),
        Kind::I16 => (16, true),
        Kind::U32 | Kind::OptionalU32 => (32, false),
        Kind::I32 | Kind::OptionalI32 => (32, true),
        Kind::U64 | Kind::OptionalU64 => (64, false),
        Kind::I64 | Kind::OptionalI64 => (64, true),
        _ => return None,
    })
}

/// The varint a `bits`-wide integer takes in a run: its low bits, zigzagged when signed.
fn wire(word: u64, bits: u32, signed: bool) -> u64 {
    let shift = 64 - bits;
    if signed {
        let value = ((word << shift) as i64) >> shift;
        ((value << 1) ^ (value >> 63)) as u64
    } else {
        word << shift >> shift
    }
}

fn count(items: usize) -> io::Result<u32> {
    u32::try_from(items).map_err(|_| invalid_static("Cell items exceed 32 bits"))
}

/// The bytes `hunks` take as a PATCH's edits.
fn edit_bytes(hunks: &[(u32, u32, u32)]) -> usize {
    varint_len(hunks.len() as u64)
        + hunks
            .iter()
            .map(|&(skip, removed, added)| {
                varint_len(skip.into()) + varint_len(removed.into()) + varint_len(added.into())
            })
            .sum::<usize>()
}

/// Appends to `edits` the hunks turning `held` into `new`: equal lengths edit their
/// differing runs, merged across fewer than `gap` equal items; otherwise one hunk replaces
/// what lies between the common prefix and suffix. When that leaves more than
/// [`MATCH_FLOOR`] items to insert, [`Matcher`] also looks for the runs of `held` that
/// `new` keeps elsewhere, such as behind a changed count, and its hunks are kept when
/// cheaper and at most half `new` whole. Whether the kept hunks and the items they
/// insert, `bytes` each, are shorter than `new` whole.
fn hunks<T: Items>(
    held: &[T],
    new: &[T],
    gap: usize,
    bytes: impl Fn(T) -> usize,
    edits: &mut Vec<(u32, u32, u32)>,
) -> io::Result<bool> {
    let items = |range: &[T]| range.iter().map(|&item| bytes(item)).sum::<usize>();
    let prefix = held.iter().zip(new).take_while(|(a, b)| a == b).count();
    let limit = held.len().min(new.len()) - prefix;
    let suffix = held
        .iter()
        .rev()
        .zip(new.iter().rev())
        .take(limit)
        .take_while(|(a, b)| a == b)
        .count();
    let start = edits.len();
    let (mut inserted, mut added) = (0, 0);
    if held.len() != new.len() {
        let removed = held.len() - prefix - suffix;
        added = new.len() - prefix - suffix;
        push(edits, (count(prefix)?, count(removed)?, count(added)?))?;
        inserted = items(&new[prefix..prefix + added]);
    } else {
        let end = held.len() - suffix;
        let (mut at, mut kept) = (prefix, 0);
        while at < end {
            if held[at] == new[at] {
                at += 1;
                continue;
            }
            let (first, mut stop, mut probe, mut equal) = (at, at + 1, at + 1, 0);
            while probe < end && equal < gap {
                if held[probe] == new[probe] {
                    equal += 1;
                } else {
                    equal = 0;
                    stop = probe + 1;
                }
                probe += 1;
            }
            let edited = count(stop - first)?;
            push(edits, (count(first - kept)?, edited, edited))?;
            inserted += items(&new[first..stop]);
            added += stop - first;
            (kept, at) = (stop, stop);
        }
    }
    let mut cost = edit_bytes(&edits[start..]) + inserted;
    let whole = varint_len(new.len() as u64) + items(new);
    if added > MATCH_FLOOR {
        MATCHER.with(|matcher| {
            let mut matcher = matcher.borrow_mut();
            let found = matcher.find(held, new, (prefix, suffix), &items)?;
            // Scattered hunks keeping under half the cell compress worse than the whole cell.
            if found < cost && found * 2 <= whole {
                edits.truncate(start);
                extend(edits, &matcher.edits)?;
                cost = found;
            }
            Ok::<_, io::Error>(())
        })?;
    }
    Ok(cost < whole)
}

/// Items the cheap hunks may leave to insert before [`Matcher`] looks for moved runs.
const MATCH_FLOOR: usize = 32;

/// Chain positions a [`Matcher`] compares at one item of `new`.
const MATCH_PROBES: usize = 8;

const NO_POSITION: u32 = u32::MAX;

thread_local! {
    static MATCHER: std::cell::RefCell<Matcher> = std::cell::RefCell::default();
}

/// A greedy linear diff: each window of `new` copies the longest run among the nearest
/// equal windows of `held` at or past the last copy, as PATCH hunks only move forward.
#[derive(Default)]
struct Matcher {
    heads: Vec<u32>,
    next: Vec<u32>,
    edits: Vec<(u32, u32, u32)>,
}

impl Matcher {
    /// Leaves in `edits` the hunks turning `held` into `new` around their common `prefix`
    /// and `suffix` items, and returns the bytes they and their inserted items take.
    fn find<T: Items>(
        &mut self,
        held: &[T],
        new: &[T],
        (prefix, suffix): (usize, usize),
        items: &impl Fn(&[T]) -> usize,
    ) -> io::Result<usize> {
        let probe = T::PROBE;
        let (start, held_end, new_end) = (prefix, held.len() - suffix, new.len() - suffix);
        self.edits.clear();
        let windows = (held_end - start + 1).saturating_sub(probe);
        let bits = windows.next_power_of_two().trailing_zeros().max(4);
        let bucket = |key: u64| (key.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> (64 - bits)) as usize;
        self.heads.clear();
        engine_allocation::resize(&mut self.heads, 1 << bits, NO_POSITION)?;
        self.next.clear();
        engine_allocation::resize(&mut self.next, windows, NO_POSITION)?;
        for at in (start..start + windows).rev() {
            let head = &mut self.heads[bucket(T::probe(&held[at..at + probe]))];
            self.next[at - start] = *head;
            *head = at as u32;
        }
        let (mut cursor, mut at, mut unmatched, mut kept, mut inserted) =
            (start, start, start, prefix, 0);
        while at + probe <= new_end {
            let head = &mut self.heads[bucket(T::probe(&new[at..at + probe]))];
            // Copies only move forward: a position before the last copy never matches again.
            let mut position = *head;
            while position != NO_POSITION && (position as usize) < cursor {
                position = self.next[position as usize - start];
            }
            *head = position;
            let (mut from, mut length) = (0, 0);
            for _ in 0..MATCH_PROBES {
                if position == NO_POSITION {
                    break;
                }
                let candidate = position as usize;
                let run = held[candidate..held_end]
                    .iter()
                    .zip(&new[at..new_end])
                    .take_while(|(a, b)| a == b)
                    .count();
                if run > length {
                    (from, length) = (candidate, run);
                }
                position = self.next[candidate - start];
            }
            if length < probe {
                at += 1;
                continue;
            }
            if from != cursor || at != unmatched {
                push(
                    &mut self.edits,
                    (count(kept)?, count(from - cursor)?, count(at - unmatched)?),
                )?;
                inserted += items(&new[unmatched..at]);
                kept = 0;
            }
            kept += length;
            (cursor, at, unmatched) = (from + length, at + length, at + length);
        }
        if cursor != held_end || unmatched != new_end {
            push(
                &mut self.edits,
                (
                    count(kept)?,
                    count(held_end - cursor)?,
                    count(new_end - unmatched)?,
                ),
            )?;
            inserted += items(&new[unmatched..new_end]);
        }
        Ok(edit_bytes(&self.edits) + inserted)
    }
}

impl Pending {
    fn clear(&mut self) {
        self.at.clear();
        self.words.clear();
        self.present.clear();
        self.spans.clear();
        self.u32s.clear();
        self.u64s.clear();
        self.bytes.clear();
        self.ops.clear();
        self.deltas.clear();
        self.hunks.clear();
        self.edits.clear();
        self.uniform = false;
    }

    fn len(&self) -> usize {
        self.at.len()
    }

    fn items<T: Copy>(
        spans: &mut Vec<(u32, u32)>,
        into: &mut Vec<T>,
        items: &[T],
    ) -> io::Result<()> {
        push(spans, span(into.len(), items.len())?)?;
        extend(into, items)
    }

    fn push(&mut self, at: u32, cell: Cell<'_>) -> io::Result<()> {
        match cell {
            Cell::U32List(items) => Self::items(&mut self.spans, &mut self.u32s, items)?,
            Cell::U64List(items) => Self::items(&mut self.spans, &mut self.u64s, items)?,
            Cell::Opaque(items) => Self::items(&mut self.spans, &mut self.bytes, items)?,
            _ => {}
        }
        let (word, present) = word(cell);
        engine_allocation::reserve(&mut self.at, 1)?;
        engine_allocation::reserve(&mut self.words, 1)?;
        engine_allocation::reserve(&mut self.present, 1)?;
        self.at.push(at);
        self.words.push(word);
        self.present.push(present);
        Ok(())
    }

    /// Whether entries `a` and `b` of a `kind` column hold one value.
    fn equal(&self, kind: Kind, a: usize, b: usize) -> bool {
        let items = |at: usize| {
            let (start, len) = self.spans[at];
            start as usize..(start + len) as usize
        };
        match kind {
            Kind::U32List => self.u32s[items(a)] == self.u32s[items(b)],
            Kind::U64List => self.u64s[items(a)] == self.u64s[items(b)],
            Kind::Opaque => self.bytes[items(a)] == self.bytes[items(b)],
            _ => self.words[a] == self.words[b] && self.present[a] == self.present[b],
        }
    }

    /// A held row's changed cell, written as an ADD or PATCH of `held`, the value the
    /// recording recorded there, when that takes no more bytes than a SET.
    fn change(&mut self, at: u32, cell: Cell<'_>, held: Option<Cell<'_>>) -> io::Result<()> {
        let entry = self.len();
        self.push(at, cell)?;
        let kind = cell_kind(&cell);
        self.uniform = entry == 0 || (self.uniform && self.equal(kind, 0, entry));
        let op = if let Some((bits, signed)) = integer(kind) {
            let (new, present) = (self.words[entry], self.present[entry]);
            let delta = match held.map(word) {
                Some((old, true)) if present => Some(new.wrapping_sub(old)).filter(|&delta| {
                    varint_len(wire(delta, bits, signed)) <= varint_len(wire(new, bits, signed))
                }),
                _ => None,
            };
            push(&mut self.deltas, delta.unwrap_or(0))?;
            if delta.is_some() { Op::Add } else { Op::Set }
        } else if matches!(kind, Kind::U32List | Kind::U64List | Kind::Opaque) {
            let start = self.edits.len();
            let edits = &mut self.edits;
            let patch = match (cell, held) {
                (Cell::U32List(new), Some(Cell::U32List(old))) => {
                    hunks(old, new, 1, |item| varint_len(item.into()), edits)?
                }
                (Cell::U64List(new), Some(Cell::U64List(old))) => {
                    hunks(old, new, 1, varint_len, edits)?
                }
                (Cell::Opaque(new), Some(Cell::Opaque(old))) => hunks(old, new, 3, |_| 1, edits)?,
                _ => false,
            };
            if !patch {
                self.edits.truncate(start);
            }
            push(&mut self.hunks, span(start, self.edits.len() - start)?)?;
            if patch { Op::Patch } else { Op::Set }
        } else {
            Op::Set
        };
        push(&mut self.ops, op)
    }
}

/// One run's values, laid out as [`Values`] borrows them.
#[derive(Default)]
struct Out {
    u8s: Vec<u8>,
    i8s: Vec<i8>,
    u16s: Vec<u16>,
    i16s: Vec<i16>,
    u32s: Vec<u32>,
    i32s: Vec<i32>,
    u64s: Vec<u64>,
    i64s: Vec<i64>,
    present: Vec<u64>,
    ends: Vec<u32>,
    /// Values pushed: the next one's presence bit.
    count: usize,
}

impl Out {
    fn clear(&mut self) {
        self.u8s.clear();
        self.i8s.clear();
        self.u16s.clear();
        self.i16s.clear();
        self.u32s.clear();
        self.i32s.clear();
        self.u64s.clear();
        self.i64s.clear();
        self.present.clear();
        self.ends.clear();
        self.count = 0;
    }

    /// `pending`'s entry `at` as `kind`'s next value, its integer bits taken from `words`.
    fn push(&mut self, kind: Kind, pending: &Pending, words: &[u64], at: usize) -> io::Result<()> {
        let word = words[at];
        match kind {
            Kind::U8 | Kind::Bool => push(&mut self.u8s, word as u8)?,
            Kind::I8 => push(&mut self.i8s, word as i8)?,
            Kind::U16 => push(&mut self.u16s, word as u16)?,
            Kind::I16 => push(&mut self.i16s, word as i16)?,
            Kind::U32 | Kind::Bytes => push(&mut self.u32s, word as u32)?,
            Kind::I32 => push(&mut self.i32s, word as i32)?,
            Kind::U64 => push(&mut self.u64s, word)?,
            Kind::I64 => push(&mut self.i64s, word as i64)?,
            Kind::OptionalI32 | Kind::OptionalU32 | Kind::OptionalI64 | Kind::OptionalU64 => {
                if self.count % 64 == 0 {
                    push(&mut self.present, 0)?;
                }
                if pending.present[at] {
                    self.present[self.count / 64] |= 1 << (self.count % 64);
                    match kind {
                        Kind::OptionalI32 => push(&mut self.i32s, word as i32)?,
                        Kind::OptionalU32 => push(&mut self.u32s, word as u32)?,
                        Kind::OptionalI64 => push(&mut self.i64s, word as i64)?,
                        _ => push(&mut self.u64s, word)?,
                    }
                }
            }
            Kind::U32List | Kind::U64List | Kind::Opaque => {
                let (start, len) = pending.spans[at];
                let range = start as usize..(start + len) as usize;
                match kind {
                    Kind::U32List => extend(&mut self.u32s, &pending.u32s[range])?,
                    Kind::U64List => extend(&mut self.u64s, &pending.u64s[range])?,
                    _ => extend(&mut self.u8s, &pending.bytes[range])?,
                }
                let end = self
                    .ends
                    .last()
                    .copied()
                    .unwrap_or(0)
                    .checked_add(len)
                    .ok_or_else(|| invalid_static("Cell items exceed 32 bits"))?;
                push(&mut self.ends, end)?;
            }
        }
        self.count += 1;
        Ok(())
    }

    /// `pending`'s entries at `order` (indices into it), as `kind`'s values.
    fn fill(&mut self, kind: Kind, pending: &Pending, order: &[u32]) -> io::Result<()> {
        self.clear();
        for &at in order {
            self.push(kind, pending, &pending.words, at as usize)?;
        }
        Ok(())
    }

    /// The items `edits` insert into `pending`'s entry `at`, a list or opaque cell.
    fn inserted(
        &mut self,
        kind: Kind,
        pending: &Pending,
        at: usize,
        edits: &[(u32, u32, u32)],
    ) -> io::Result<()> {
        let mut from = pending.spans[at].0 as usize;
        for &(skip, _, added) in edits {
            from += skip as usize;
            let range = from..from + added as usize;
            match kind {
                Kind::U32List => extend(&mut self.u32s, &pending.u32s[range])?,
                Kind::U64List => extend(&mut self.u64s, &pending.u64s[range])?,
                _ => extend(&mut self.u8s, &pending.bytes[range])?,
            }
            from += added as usize;
        }
        Ok(())
    }

    fn patch_items(&self, kind: Kind) -> PatchItems<'_> {
        match kind {
            Kind::U32List => PatchItems::U32(&self.u32s),
            Kind::U64List => PatchItems::U64(&self.u64s),
            _ => PatchItems::Bytes(&self.u8s),
        }
    }

    fn values(&self, kind: Kind) -> Values<'_> {
        match kind {
            Kind::U8 => Values::U8(&self.u8s),
            Kind::Bool => Values::Bool(&self.u8s),
            Kind::I8 => Values::I8(&self.i8s),
            Kind::U16 => Values::U16(&self.u16s),
            Kind::I16 => Values::I16(&self.i16s),
            Kind::U32 => Values::U32(&self.u32s),
            Kind::Bytes => Values::Bytes(&self.u32s),
            Kind::I32 => Values::I32(&self.i32s),
            Kind::U64 => Values::U64(&self.u64s),
            Kind::I64 => Values::I64(&self.i64s),
            Kind::OptionalI32 => Values::OptionalI32 {
                present: &self.present,
                values: &self.i32s,
            },
            Kind::OptionalU32 => Values::OptionalU32 {
                present: &self.present,
                values: &self.u32s,
            },
            Kind::OptionalI64 => Values::OptionalI64 {
                present: &self.present,
                values: &self.i64s,
            },
            Kind::OptionalU64 => Values::OptionalU64 {
                present: &self.present,
                values: &self.u64s,
            },
            Kind::U32List => Values::U32List {
                ends: &self.ends,
                items: &self.u32s,
            },
            Kind::U64List => Values::U64List {
                ends: &self.ends,
                items: &self.u64s,
            },
            Kind::Opaque => Values::Opaque {
                ends: &self.ends,
                items: &self.u8s,
            },
        }
    }
}

/// One column's runs for an hour, laid out as [`Run`] borrows them; kept between hours.
#[derive(Default)]
struct Runs {
    /// The column's changed entries, by slot.
    order: Vec<u32>,
    /// A FILL writes every changed cell, and `except` names the other live slots.
    fill: bool,
    except: Vec<u32>,
    set_positions: Vec<u32>,
    add_positions: Vec<u32>,
    patch_positions: Vec<u32>,
    noop_positions: Vec<u32>,
    /// SET values, or a FILL's one value.
    values: Out,
    /// ADD deltas of an integer column, or the items a list or opaque column's PATCH inserts.
    deltas: Out,
    hunks: Vec<u32>,
    edits: Vec<(u32, u32, u32)>,
}

impl Runs {
    fn clear(&mut self) {
        self.order.clear();
        self.fill = false;
        self.except.clear();
        self.set_positions.clear();
        self.add_positions.clear();
        self.patch_positions.clear();
        self.noop_positions.clear();
        self.values.clear();
        self.deltas.clear();
        self.hunks.clear();
        self.edits.clear();
    }
}

/// The slots live after an hour, the held live ones it does not delete and those it
/// inserts, that ascending `covered` does not name.
fn exceptions(
    live: &Bits,
    deleted: &[u32],
    inserted: &[u32],
    covered: impl Iterator<Item = u32>,
    into: &mut Vec<u32>,
) -> io::Result<()> {
    let mut deleted = deleted.iter().copied().peekable();
    let mut held = live
        .ones()
        .filter(|&slot| {
            while deleted.next_if(|&gone| gone < slot).is_some() {}
            deleted.next_if_eq(&slot).is_none()
        })
        .peekable();
    let mut inserted = inserted.iter().copied().peekable();
    let mut covered = covered.peekable();
    loop {
        // An insert takes a slot no held row keeps.
        let slot = match (held.peek().copied(), inserted.peek().copied()) {
            (Some(kept), Some(new)) if new < kept => {
                inserted.next();
                new
            }
            (Some(kept), _) => {
                held.next();
                kept
            }
            (None, Some(new)) => {
                inserted.next();
                new
            }
            (None, None) => return Ok(()),
        };
        while covered.next_if(|&set| set < slot).is_some() {}
        if covered.next_if_eq(&slot).is_none() {
            push(into, slot)?;
        }
    }
}

fn cell_kind(cell: &Cell<'_>) -> Kind {
    match cell {
        Cell::U8(_) => Kind::U8,
        Cell::I8(_) => Kind::I8,
        Cell::U16(_) => Kind::U16,
        Cell::I16(_) => Kind::I16,
        Cell::U32(_) => Kind::U32,
        Cell::I32(_) => Kind::I32,
        Cell::U64(_) => Kind::U64,
        Cell::I64(_) => Kind::I64,
        Cell::Bool(_) => Kind::Bool,
        Cell::OptionalI32(_) => Kind::OptionalI32,
        Cell::OptionalU32(_) => Kind::OptionalU32,
        Cell::OptionalI64(_) => Kind::OptionalI64,
        Cell::OptionalU64(_) => Kind::OptionalU64,
        Cell::Bytes(_) => Kind::Bytes,
        Cell::U32List(_) => Kind::U32List,
        Cell::U64List(_) => Kind::U64List,
        Cell::Opaque(_) => Kind::Opaque,
    }
}

/// A kept cell names no held row to keep.
fn kept_insert() -> io::Error {
    invalid_static("A kept cell's row is not held")
}

/// The value an unrecorded inserted cell holds: never read, since the cell is unrecorded.
fn placeholder(kind: Kind) -> Cell<'static> {
    match kind {
        Kind::U8 => Cell::U8(0),
        Kind::I8 => Cell::I8(0),
        Kind::U16 => Cell::U16(0),
        Kind::I16 => Cell::I16(0),
        Kind::U32 => Cell::U32(0),
        Kind::I32 => Cell::I32(0),
        Kind::U64 => Cell::U64(0),
        Kind::I64 => Cell::I64(0),
        Kind::Bool => Cell::Bool(false),
        Kind::OptionalI32 => Cell::OptionalI32(None),
        Kind::OptionalU32 => Cell::OptionalU32(None),
        Kind::OptionalI64 => Cell::OptionalI64(None),
        Kind::OptionalU64 => Cell::OptionalU64(None),
        Kind::Bytes => Cell::Bytes(0),
        Kind::U32List => Cell::U32List(&[]),
        Kind::U64List => Cell::U64List(&[]),
        Kind::Opaque => Cell::Opaque(&[]),
    }
}

/// Marks `column` as holding the hour's cells.
fn mark_dirty(dirty: &mut [u64], column: usize) {
    dirty[column / 64] |= 1 << (column % 64);
}

/// Marks the first `columns` columns.
fn mark_all_dirty(dirty: &mut [u64], columns: usize) {
    for (index, word) in dirty.iter_mut().enumerate() {
        *word = match columns.saturating_sub(index * 64) {
            left @ 0..64 => (1u64 << left) - 1,
            _ => u64::MAX,
        };
    }
}

/// The marked columns, ascending.
fn dirty_columns(dirty: &[u64]) -> impl Iterator<Item = usize> + '_ {
    dirty.iter().enumerate().flat_map(|(index, &word)| {
        let mut rest = word;
        std::iter::from_fn(move || {
            (rest != 0).then(|| {
                let bit = rest.trailing_zeros() as usize;
                rest &= rest - 1;
                index * 64 + bit
            })
        })
    })
}

fn dirty_count(dirty: &[u64]) -> u64 {
    dirty.iter().map(|word| u64::from(word.count_ones())).sum()
}

/// Marks a held slot visited, refusing a second visit.
fn see(seen: &mut [u64], visited: &mut Vec<u32>, slot: u32) -> io::Result<()> {
    let word = &mut seen[slot as usize / 64];
    let bit = 1u64 << (slot % 64);
    if *word & bit != 0 {
        return Err(invalid_static("An hour visits a row twice"));
    }
    engine_allocation::reserve(visited, 1)?;
    *word |= bit;
    visited.push(slot);
    Ok(())
}

/// One table's held state and the buffers its hours reuse.
pub struct TableWriter {
    kinds: Box<[Kind]>,
    table: Table,
    status: Status,
    acquisition: Acquisition,
    /// A truncated table's view: its last hour's read and inserted slots, ascending.
    view: Vec<u32>,
    /// Slots below the table's extent that hold no row, lowest first.
    free: BinaryHeap<Reverse<u32>>,
    /// This writer's name and the hours begun, so only its latest preparation commits.
    identity: u64,
    generation: u64,
    /// The last hour that finished. One begun and not finished may have staged cells in
    /// columns it did not mark.
    finished: u64,
    /// A commit failed part way: the held table is not one a walker holds.
    poisoned: bool,
    // Per hour, reused. `seen` holds one bit per visited held slot and is zero between
    // hours: the hour clears only the bits `visited` names.
    seen: Vec<u64>,
    visited: Vec<u32>,
    updates: Vec<Pending>,
    unrecord: Vec<Vec<u32>>,
    noops: Vec<Vec<u32>>,
    touched: Vec<u32>,
    /// A changes hour's removed keys, ascending.
    removed: Vec<u128>,
    /// The hour's new keys, in visit order: an insert's index is its position.
    new_keys: Vec<u128>,
    inserts: Vec<Pending>,
    insert_unrecorded: Vec<Vec<u32>>,
    runs: Vec<Runs>,
    insert_outs: Vec<Out>,
    scratch: Scratch,
    changes: Changes,
    /// The Bytes columns, which a batch stages row by row as their strings intern.
    texts: Box<[usize]>,
    /// A batch's held slot of each row, or [`NEW`], and its new rows.
    slots: Vec<u32>,
    insert_rows: Vec<u32>,
    /// Each text column's next unrecorded and next kept row while a batch is staged.
    cursors: Vec<(usize, usize)>,
    order: Order,
    parts: Parts,
    /// A bit per column whose per-hour buffers (staged cells, runs, insert values and
    /// unrecorded slots) may hold anything; after a finished hour, every other column's are
    /// empty.
    dirty: Vec<u64>,
    sweep: Sweep,
    work: Work,
}

/// The rows the last complete batch visited, in its order, with their slots.
#[derive(Default)]
struct Order {
    keys: Vec<u128>,
    slots: Vec<u32>,
    /// The table holds exactly these rows at these slots.
    held: bool,
}

/// An hour's section in the making, kept between hours.
#[derive(Default)]
struct Parts {
    deleted: Vec<u32>,
    read: Vec<u32>,
    /// The hour's new keys by key, with their insert index.
    sorted_keys: Vec<(u128, u32)>,
    /// Each insert's slot, by its index.
    by_index: Vec<u32>,
    by_slot: Vec<(u32, u128, u32)>,
    popped: Vec<u32>,
    inserted: Vec<u32>,
    keys: Vec<u128>,
    insert_order: Vec<u32>,
    changed: Vec<u32>,
    /// Per column, its changed and inserted slots left unrecorded.
    unrecorded: Vec<Vec<u32>>,
    // Empty between hours; only their allocations are kept.
    runs: Vec<Run<'static>>,
    inserts: Vec<Values<'static>>,
    unrecorded_runs: Vec<(usize, &'static [u32])>,
    /// The last committed hour's column bits and section, written over by the next.
    columns: Vec<u64>,
    section: Vec<u8>,
}

impl TableWriter {
    pub fn new(kinds: &[Kind]) -> io::Result<Self> {
        Self::with_table(kinds, Table::new(kinds)?)
    }

    fn with_table(kinds: &[Kind], table: Table) -> io::Result<Self> {
        let columns = kinds.len();
        let many = |count: usize| engine_allocation::collect_vec((0..count).map(|_| Vec::new()));
        let texts = engine_allocation::collect_vec(
            (0..columns).filter(|&column| kinds[column] == Kind::Bytes),
        )?;
        Ok(Self {
            kinds: engine_allocation::boxed_slice(engine_allocation::collect_vec(
                kinds.iter().copied(),
            )?)?,
            table,
            status: Status::NotRecorded,
            acquisition: Acquisition::default(),
            view: Vec::new(),
            free: BinaryHeap::new(),
            identity: WRITERS.fetch_add(1, Ordering::Relaxed),
            generation: 0,
            finished: 0,
            poisoned: false,
            seen: Vec::new(),
            visited: Vec::new(),
            updates: engine_allocation::collect_vec((0..columns).map(|_| Pending::default()))?,
            unrecord: many(columns)?,
            noops: many(columns)?,
            touched: Vec::new(),
            removed: Vec::new(),
            new_keys: Vec::new(),
            inserts: engine_allocation::collect_vec((0..columns).map(|_| Pending::default()))?,
            insert_unrecorded: many(columns)?,
            runs: engine_allocation::collect_vec((0..columns).map(|_| Runs::default()))?,
            insert_outs: engine_allocation::collect_vec((0..columns).map(|_| Out::default()))?,
            scratch: Scratch::default(),
            changes: Changes::default(),
            texts: engine_allocation::boxed_slice(texts)?,
            slots: Vec::new(),
            insert_rows: Vec::new(),
            cursors: Vec::new(),
            order: Order::default(),
            parts: Parts {
                unrecorded: many(columns)?,
                ..Parts::default()
            },
            dirty: engine_allocation::filled(columns.div_ceil(64), 0_u64)?,
            sweep: Sweep::Touched,
            work: Work::default(),
        })
    }

    /// A writer continuing from a restart: the next hour writes what it would have written
    /// had the writer never stopped.
    pub fn restore(kinds: &[Kind], restored: Restored) -> io::Result<Self> {
        if !restored.table.kinds().eq(kinds.iter().copied()) {
            return Err(invalid_static(
                "Restart table's columns differ from its schema",
            ));
        }
        let mut free = Vec::new();
        for slot in 0..restored.table.slots() as u32 {
            if !restored.table.live().get(slot) {
                engine_allocation::reserve(&mut free, 1)?;
                free.push(Reverse(slot));
            }
        }
        let mut writer = Self::with_table(kinds, restored.table)?;
        writer.status = restored.status;
        writer.acquisition = restored.acquisition;
        writer.view = restored.view;
        writer.free = BinaryHeap::from(free);
        Ok(writer)
    }

    /// The restart a block beginning now carries.
    pub fn restart(&self) -> io::Result<Vec<u8>> {
        self.usable()?;
        restart::write(&self.table, self.status, &self.acquisition, &self.view)
    }

    pub fn table(&self) -> &Table {
        &self.table
    }

    pub fn status(&self) -> Status {
        self.status
    }

    pub fn acquisition(&self) -> &Acquisition {
        &self.acquisition
    }

    /// The slots of the rows a truncated table read, ascending.
    pub fn view(&self) -> &[u32] {
        &self.view
    }

    /// The table's column kinds, in schema order.
    pub fn kinds(&self) -> &[Kind] {
        &self.kinds
    }

    pub fn sweep(&self) -> Sweep {
        self.sweep
    }

    /// The columns the next hours clear and finish; the sections are the same either way.
    pub fn set_sweep(&mut self, sweep: Sweep) {
        self.sweep = sweep;
    }

    pub fn work(&self) -> Work {
        self.work
    }

    fn usable(&self) -> io::Result<()> {
        if self.poisoned {
            return Err(invalid_static(
                "Column writer failed part way through a commit; restore it from a restart",
            ));
        }
        Ok(())
    }

    /// Starts an hour's rows. Nothing held changes until its preparation commits.
    pub fn begin<'w>(
        &'w mut self,
        dictionary: impl Into<Strings<'w>>,
        coverage: Coverage<'w>,
    ) -> io::Result<Hour<'w>> {
        let dictionary = dictionary.into();
        self.usable()?;
        let unfinished = self.finished != self.generation;
        self.generation += 1;
        self.work.hours += 1;
        for &slot in &self.visited {
            self.seen[slot as usize / 64] &= !(1 << (slot % 64));
        }
        self.visited.clear();
        engine_allocation::resize_default(&mut self.seen, self.table.slots().div_ceil(64))?;
        let columns = self.kinds.len();
        if unfinished || self.sweep == Sweep::Every {
            mark_all_dirty(&mut self.dirty, columns);
        }
        let Self {
            dirty,
            updates,
            inserts,
            unrecord,
            noops,
            insert_unrecorded,
            runs,
            insert_outs,
            parts,
            work,
            ..
        } = &mut *self;
        work.columns += dirty_count(dirty.as_slice());
        for column in dirty_columns(dirty.as_slice()) {
            updates[column].clear();
            inserts[column].clear();
            unrecord[column].clear();
            noops[column].clear();
            insert_unrecorded[column].clear();
            runs[column].clear();
            insert_outs[column].clear();
            parts.unrecorded[column].clear();
        }
        dirty.fill(0);
        if self.sweep == Sweep::Every {
            mark_all_dirty(&mut self.dirty, columns);
        }
        self.touched.clear();
        self.new_keys.clear();
        self.removed.clear();
        if let Coverage::Changes { removed } = coverage {
            engine_allocation::reserve(&mut self.removed, removed.len())?;
            self.removed.extend_from_slice(removed);
            self.removed.sort_unstable();
            if self.removed.windows(2).any(|pair| pair[0] == pair[1]) {
                return Err(invalid_static("A removed row key is named twice"));
            }
        }
        Ok(Hour {
            writer: self,
            dictionary,
            coverage,
            inserted: 0,
            failed: false,
            ordered: false,
        })
    }

    /// An hour's rows as one batch, each column compared with the held column in one pass:
    /// the section [`Hour::row`] writes of the same rows in the same order. A complete hour
    /// visiting the rows the last complete batch did, in its order, finds their slots
    /// without looking up a key. Nothing held changes until the preparation commits.
    pub fn prepare<'w>(
        &'w mut self,
        dictionary: impl Into<Strings<'w>>,
        coverage: Coverage<'w>,
        batch: &Batch,
    ) -> io::Result<Prepared> {
        let mut hour = self.begin(dictionary, coverage)?;
        hour.staged(|hour| hour.stage(batch))?;
        let ordered = hour.ordered;
        let mut prepared = hour.finish()?;
        prepared.order = matches!(coverage, Coverage::Complete) && !ordered;
        Ok(prepared)
    }

    /// Holds the order the committed complete batch visited its rows in, and their slots.
    fn hold_order(&mut self) -> io::Result<()> {
        let Self {
            table,
            slots,
            parts,
            order,
            ..
        } = self;
        order.held = false;
        order.slots.clear();
        engine_allocation::reserve(&mut order.slots, slots.len())?;
        // Inserts took their indices in the order the batch visited them.
        let mut inserted = parts.by_index.iter().copied();
        for &slot in slots.iter() {
            order.slots.push(match slot {
                NEW => inserted
                    .next()
                    .ok_or_else(|| invalid_static("A batch's new row has no slot"))?,
                slot => slot,
            });
        }
        order.keys.clear();
        engine_allocation::reserve(&mut order.keys, order.slots.len())?;
        for &slot in &order.slots {
            let key = table
                .key(slot)
                .ok_or_else(|| invalid_static("A batch's row is not held"))?;
            order.keys.push(key);
        }
        order.held = true;
        Ok(())
    }

    /// Applies a section to the held table and keeps the free slots it leaves.
    fn apply(&mut self, bytes: &[u8], truncated: bool) -> io::Result<()> {
        let extent = self.table.slots();
        section::apply(
            bytes,
            &self.kinds,
            truncated,
            &mut self.table,
            &mut self.scratch,
            &mut self.changes,
        )?;
        let changes = &self.changes;
        // Inserts below the old extent the hour did not free itself took the lowest free
        // slots, in order, as the preparation did.
        for &slot in &changes.inserted {
            if slot as usize >= extent || changes.deleted.binary_search(&slot).is_ok() {
                continue;
            }
            if self.free.pop() != Some(Reverse(slot)) {
                return Err(invalid_static(
                    "Inserted slots are not the lowest free ones",
                ));
            }
        }
        let freed = changes.deleted.len();
        if freed > self.free.capacity() - self.free.len() {
            engine_allocation::allocation_attempt()?;
            self.free
                .try_reserve(freed)
                .map_err(|_| engine_allocation::oom())?;
        }
        for &slot in &changes.deleted {
            if changes.inserted.binary_search(&slot).is_err() {
                self.free.push(Reverse(slot));
            }
        }
        Ok(())
    }

    /// Applies an hour: its section (none for an hour that changed no row, or one that
    /// read none) and its evidence, as a walker applies its manifest and section.
    pub fn commit(
        &mut self,
        prepared: Option<Prepared>,
        evidence: Evidence,
        hour: i32,
    ) -> io::Result<()> {
        self.usable()?;
        let truncated = matches!(evidence.status, Status::Truncated { .. });
        let readable = matches!(evidence.status, Status::Complete | Status::Truncated { .. });
        let (written, order) = match prepared {
            Some(prepared) => {
                if (prepared.identity, prepared.generation) != (self.identity, self.generation) {
                    return Err(invalid_static(
                        "Column preparation is not the latest hour's",
                    ));
                }
                if !readable {
                    return Err(invalid_static(
                        "A table with no rows this hour carries a section",
                    ));
                }
                if prepared.partial != truncated {
                    return Err(invalid_static(
                        "A partial read and a truncated status come together",
                    ));
                }
                self.parts.columns = prepared.columns;
                (prepared.section, prepared.order)
            }
            None => (None, false),
        };
        if truncated && written.is_none() {
            return Err(invalid_static("A partly read table names no rows it read"));
        }
        if let Some(bytes) = &written
            && let Err(error) = self.apply(bytes, truncated)
        {
            self.poisoned = true;
            return Err(error);
        }
        let available =
            |status: Status| matches!(status, Status::Complete | Status::Truncated { .. });
        if available(evidence.status) != available(self.status)
            || self.acquisition.available_from.is_none()
        {
            self.acquisition.available_from = Some(hour);
        }
        self.acquisition.refusal = match evidence.status {
            Status::Unavailable { reason } => Some(reason),
            _ => None,
        };
        if evidence.read {
            self.acquisition.last_read = Some(hour);
        }
        match evidence.sample {
            Sample::FromReadBit => {}
            Sample::Reset => self.acquisition.last_read = None,
            Sample::Carried(carried) => self.acquisition.last_read = Some(carried),
        }
        self.status = evidence.status;
        self.view.clear();
        if truncated {
            let changes = &self.changes;
            let live = self.table.live();
            let reserved = engine_allocation::reserve(
                &mut self.view,
                changes.read.len() + changes.inserted.len(),
            );
            if let Err(error) = reserved {
                self.poisoned = true;
                return Err(error);
            }
            // A read row the hour deleted is gone; a slot it deleted and filled again holds
            // the inserted row, once.
            self.view
                .extend(changes.read.iter().copied().filter(|&slot| live.get(slot)));
            self.view.extend_from_slice(&changes.inserted);
            self.view.sort_unstable();
            self.view.dedup();
        }
        if order {
            if let Err(error) = self.hold_order() {
                self.poisoned = true;
                return Err(error);
            }
        } else if written.is_some()
            && !(self.changes.inserted.is_empty() && self.changes.deleted.is_empty())
        {
            self.order.held = false;
        }
        if let Some(bytes) = written {
            self.parts.section = bytes;
        }
        Ok(())
    }
}

/// An hour's rows while they are visited.
pub struct Hour<'w> {
    writer: &'w mut TableWriter,
    dictionary: Strings<'w>,
    coverage: Coverage<'w>,
    inserted: u32,
    /// A row, NOOP or touch was refused: the hour is partly staged and never finishes.
    failed: bool,
    /// A batch visited the held rows in the last complete batch's order: every live row,
    /// each at the slot that order gives it.
    ordered: bool,
}

/// An hour's section and what its manifest says of it.
#[derive(Debug)]
pub struct Prepared {
    identity: u64,
    generation: u64,
    partial: bool,
    /// A complete batch's order, which its commit holds.
    order: bool,
    /// None: the hour changed no row of a fully read table, so it writes no section.
    pub section: Option<Vec<u8>>,
    /// One bit per column a run or unrecorded cell names.
    pub columns: Vec<u64>,
    /// [`manifest::INSERTS`], [`manifest::DELETES`], [`manifest::FILLS`] and
    /// [`manifest::TOUCHES`].
    pub ops: u8,
    pub census: Census,
}

impl Hour<'_> {
    fn held_slot(&self, key: u128) -> Option<u32> {
        self.writer.table.slot(key)
    }

    fn mark_seen(&mut self, slot: u32) -> io::Result<()> {
        see(&mut self.writer.seen, &mut self.writer.visited, slot)
    }

    /// Runs one staging step; a refusal fails the whole hour.
    fn staged(&mut self, step: impl FnOnce(&mut Self) -> io::Result<()>) -> io::Result<()> {
        if self.failed {
            return Err(invalid_static("An earlier row of this hour was refused"));
        }
        let result = step(self);
        self.failed = result.is_err();
        result
    }

    fn resolve<'c>(&mut self, column: usize, input: Input<'c>) -> io::Result<Option<Cell<'c>>> {
        let kind = self.writer.kinds[column];
        let cell = match input {
            Input::Unrecorded => return Ok(None),
            Input::Kept => return Err(kept_insert()),
            Input::Text(bytes) if kind == Kind::Bytes => {
                Cell::Bytes(self.dictionary.intern(bytes)?)
            }
            Input::Text(_) => {
                return Err(invalid_static("Text is given for a column not of Bytes"));
            }
            Input::Value(cell) => cell,
        };
        if cell_kind(&cell) != kind {
            return Err(invalid_static("A cell's kind is not its column's"));
        }
        if let Cell::Bytes(id) = cell
            && id as usize >= self.dictionary.dictionary().len()
        {
            return Err(invalid_static(
                "A text cell's id is not in the run's dictionary",
            ));
        }
        Ok(Some(cell))
    }

    /// One candidate row, with one input per column in schema order. A held row's cells
    /// are compared once with what it holds; a new key becomes an insert.
    pub fn row(&mut self, key: u128, cells: &[Input<'_>]) -> io::Result<()> {
        self.staged(|hour| hour.stage_row(key, cells))
    }

    fn stage_row(&mut self, key: u128, cells: &[Input<'_>]) -> io::Result<()> {
        if cells.len() != self.writer.kinds.len() {
            return Err(invalid_static(
                "A row carries other than one cell per column",
            ));
        }
        if self.writer.removed.binary_search(&key).is_ok() {
            return Err(invalid_static("A row key is both changed and removed"));
        }
        match self.held_slot(key) {
            Some(slot) => {
                self.mark_seen(slot)?;
                for (column, &input) in cells.iter().enumerate() {
                    if matches!(input, Input::Kept) {
                        continue;
                    }
                    let cell = self.resolve(column, input)?;
                    let recorded = self.writer.table.recorded(column, slot);
                    match cell {
                        None if recorded => {
                            mark_dirty(&mut self.writer.dirty, column);
                            engine_allocation::reserve(&mut self.writer.unrecord[column], 1)?;
                            self.writer.unrecord[column].push(slot);
                        }
                        None => {}
                        Some(cell) => {
                            let held = self
                                .writer
                                .table
                                .column(column)
                                .and_then(|values| values.cell(slot));
                            if !recorded || held != Some(cell) {
                                let held = held.filter(|_| recorded);
                                mark_dirty(&mut self.writer.dirty, column);
                                self.writer.updates[column].change(slot, cell, held)?;
                            }
                        }
                    }
                }
            }
            None => {
                let index = self.inserted;
                engine_allocation::reserve(&mut self.writer.new_keys, 1)?;
                self.writer.new_keys.push(key);
                mark_all_dirty(&mut self.writer.dirty, cells.len());
                for (column, &input) in cells.iter().enumerate() {
                    let cell = match self.resolve(column, input)? {
                        Some(cell) => cell,
                        None => {
                            engine_allocation::reserve(
                                &mut self.writer.insert_unrecorded[column],
                                1,
                            )?;
                            self.writer.insert_unrecorded[column].push(index);
                            placeholder(self.writer.kinds[column])
                        }
                    };
                    self.writer.inserts[column].push(index, cell)?;
                }
                self.inserted = index
                    .checked_add(1)
                    .ok_or_else(|| invalid_static("An hour inserts more than 2^32 rows"))?;
            }
        }
        Ok(())
    }

    /// A batch's rows, a column at a time: what [`Hour::row`] stages of each row in turn.
    fn stage(&mut self, batch: &Batch) -> io::Result<()> {
        let complete = matches!(self.coverage, Coverage::Complete);
        let dictionary = &mut self.dictionary;
        let TableWriter {
            kinds,
            table,
            seen,
            visited,
            updates,
            unrecord,
            removed,
            new_keys,
            inserts,
            insert_unrecorded,
            texts,
            slots,
            insert_rows,
            cursors,
            order,
            dirty,
            ..
        } = &mut *self.writer;
        let rows = batch.keys.len();
        if batch.kinds.as_slice() != &**kinds {
            return Err(invalid_static("A batch's columns differ from its table's"));
        }
        if u32::try_from(rows).is_err() {
            return Err(invalid_static("A batch holds more than 2^32 rows"));
        }
        let columns = &batch.columns[..kinds.len()];
        // A batch of no rows holds no cells.
        if rows > 0 && columns.iter().any(|cells| cells.words.len() != rows) {
            return Err(invalid_static(
                "A batch row carries other than one cell per column",
            ));
        }
        slots.clear();
        insert_rows.clear();
        if complete && order.held && order.keys == batch.keys {
            extend(slots, &order.slots)?;
            self.ordered = true;
        } else {
            engine_allocation::reserve(slots, rows)?;
            for (row, &key) in (0u32..).zip(&batch.keys) {
                if !removed.is_empty() && removed.binary_search(&key).is_ok() {
                    return Err(invalid_static("A row key is both changed and removed"));
                }
                match table.slot(key) {
                    Some(NEW) => return Err(invalid_static("Row slots exceed 32 bits")),
                    Some(slot) => {
                        see(seen, visited, slot)?;
                        slots.push(slot);
                    }
                    None => {
                        push(new_keys, key)?;
                        push(insert_rows, row)?;
                        slots.push(NEW);
                    }
                }
            }
            self.inserted = insert_rows.len() as u32;
        }
        if rows == 0 {
            return Ok(());
        }
        // Every column takes a cell of each inserted row.
        if !insert_rows.is_empty() {
            mark_all_dirty(dirty, kinds.len());
        }
        let slots = slots.as_slice();
        for (column, (&kind, cells)) in kinds.iter().zip(columns).enumerate() {
            if kind == Kind::Bytes {
                continue;
            }
            let held = table
                .column(column)
                .ok_or_else(|| invalid_static("A batch names a column past the table"))?;
            // Fewer rows than bit words test their own bits rather than scan them all.
            let marked = table.unrecorded_bits(column).filter(|bits| {
                let words = bits.words();
                words.len() > rows || words.iter().any(|&word| word != 0)
            });
            let mut staged = Staged {
                slots,
                cells,
                marked,
                pending: &mut updates[column],
                unrecord: &mut unrecord[column],
                dirty: false,
            };
            let compared = match held {
                Column::U8(values) => staged.scalars(values),
                Column::I8(values) => staged.scalars(values),
                Column::U16(values) => staged.scalars(values),
                Column::I16(values) => staged.scalars(values),
                Column::U32(values) => staged.scalars(values),
                Column::I32(values) => staged.scalars(values),
                Column::U64(values) => staged.scalars(values),
                Column::I64(values) => staged.scalars(values),
                Column::Bool(values) => staged.scalars(values),
                Column::OptionalI32(values, presence) => staged.optionals(values, presence),
                Column::OptionalU32(values, presence) => staged.optionals(values, presence),
                Column::OptionalI64(values, presence) => staged.optionals(values, presence),
                Column::OptionalU64(values, presence) => staged.optionals(values, presence),
                Column::U32List(values) => staged.lists(values, &cells.u32s),
                Column::U64List(values) => staged.lists(values, &cells.u64s),
                Column::Opaque(values) => staged.lists(values, &cells.bytes),
                Column::Bytes(_) => Ok(()),
            };
            if staged.dirty {
                mark_dirty(dirty, column);
            }
            compared?;
            let mut kept = cells.kept.iter().peekable();
            for (index, &row) in (0u32..).zip(insert_rows.iter()) {
                while kept.next_if(|&&kept| kept < row).is_some() {}
                if kept.peek() == Some(&&row) {
                    return Err(kept_insert());
                }
                let cell = match cells.cell(kind, row) {
                    Some(cell) => cell,
                    None => {
                        push(&mut insert_unrecorded[column], index)?;
                        placeholder(kind)
                    }
                };
                inserts[column].push(index, cell)?;
            }
        }
        if texts.is_empty() {
            return Ok(());
        }
        // Strings intern in row order, then column order, as rows visited one by one intern
        // them; a held cell whose bytes are its string's needs none.
        cursors.clear();
        engine_allocation::resize_default(cursors, texts.len())?;
        let mut index = 0u32;
        for (row, &slot) in (0u32..).zip(slots) {
            for (&column, (next, next_kept)) in texts.iter().zip(cursors.iter_mut()) {
                let cells = &columns[column];
                let recorded = cells.unrecorded.get(*next) != Some(&row);
                if !recorded {
                    *next += 1;
                }
                if cells.kept.get(*next_kept) == Some(&row) {
                    *next_kept += 1;
                    if slot == NEW {
                        return Err(kept_insert());
                    }
                    continue;
                }
                let text = &cells.bytes[cells.span(row as usize)];
                if slot == NEW {
                    let cell = if recorded {
                        Cell::Bytes(dictionary.intern(text)?)
                    } else {
                        push(&mut insert_unrecorded[column], index)?;
                        placeholder(Kind::Bytes)
                    };
                    inserts[column].push(index, cell)?;
                    continue;
                }
                let held = match table.column(column) {
                    Some(Column::Bytes(ids)) => ids.get(slot as usize).copied(),
                    _ => None,
                }
                .ok_or_else(|| invalid_static("A held slot is past its column"))?;
                let was = table.recorded(column, slot);
                if !recorded {
                    if was {
                        mark_dirty(dirty, column);
                        push(&mut unrecord[column], slot)?;
                    }
                } else if !was || dictionary.dictionary().get(held) != Some(text) {
                    let id = dictionary.intern(text)?;
                    let held = was.then_some(Cell::Bytes(held));
                    mark_dirty(dirty, column);
                    updates[column].change(slot, Cell::Bytes(id), held)?;
                }
            }
            if slot == NEW {
                index += 1;
            }
        }
        Ok(())
    }

    /// The producer's explicit write of a value the held row already holds, preserved as
    /// a NOOP. The row must be visited this hour.
    pub fn noop(&mut self, key: u128, column: usize) -> io::Result<()> {
        self.staged(|hour| hour.stage_noop(key, column))
    }

    fn stage_noop(&mut self, key: u128, column: usize) -> io::Result<()> {
        let slot = self
            .held_slot(key)
            .ok_or_else(|| invalid_static("NOOP names a row key that is not held"))?;
        if column >= self.writer.kinds.len() {
            return Err(invalid_static("NOOP names a column past the schema"));
        }
        mark_dirty(&mut self.writer.dirty, column);
        engine_allocation::reserve(&mut self.writer.noops[column], 1)?;
        self.writer.noops[column].push(slot);
        Ok(())
    }

    /// The producer's touch of a held row that changes no cell.
    pub fn touch(&mut self, key: u128) -> io::Result<()> {
        self.staged(|hour| hour.stage_touch(key))
    }

    fn stage_touch(&mut self, key: u128) -> io::Result<()> {
        let slot = self
            .held_slot(key)
            .ok_or_else(|| invalid_static("Touch names a row key that is not held"))?;
        engine_allocation::reserve(&mut self.writer.touched, 1)?;
        self.writer.touched.push(slot);
        Ok(())
    }

    /// The hour's section. Nothing held changes: [`TableWriter::commit`] applies it.
    pub fn finish(self) -> io::Result<Prepared> {
        let Self {
            writer,
            coverage,
            inserted,
            failed,
            ordered,
            ..
        } = self;
        if failed {
            return Err(invalid_static(
                "An hour with a refused row writes no section",
            ));
        }
        let columns = writer.kinds.len();
        writer.work.columns += dirty_count(&writer.dirty);
        let seen =
            |slot: u32| ordered || writer.seen[slot as usize / 64] >> (slot % 64) & 1 != 0;
        let partial = matches!(coverage, Coverage::Partial);
        let Parts {
            deleted,
            read,
            sorted_keys,
            by_index,
            by_slot,
            popped,
            inserted: inserted_slots,
            keys,
            insert_order,
            changed,
            unrecorded: unrecorded_slots,
            runs: run_list,
            inserts: insert_list,
            unrecorded_runs,
            columns: spare_columns,
            section: spare_section,
        } = &mut writer.parts;
        // Deletes, ascending.
        deleted.clear();
        match coverage {
            // An ordered batch visited every live row.
            Coverage::Complete if !ordered => {
                for slot in writer.table.live().ones() {
                    if !seen(slot) {
                        push(deleted, slot)?;
                    }
                }
            }
            Coverage::Complete | Coverage::Partial => {}
            Coverage::Changes { .. } => {
                engine_allocation::reserve(deleted, writer.removed.len())?;
                for &key in &writer.removed {
                    let slot = writer
                        .table
                        .slot(key)
                        .ok_or_else(|| invalid_static("A removed row key is not held"))?;
                    deleted.push(slot);
                }
                deleted.sort_unstable();
            }
        }
        // The read held slots of a partial read, ascending.
        read.clear();
        if partial {
            extend(read, &writer.visited)?;
            read.sort_unstable();
        }
        // New rows by key, at the lowest free slots (deletes free theirs first), then at
        // new slots past the table's extent.
        sorted_keys.clear();
        engine_allocation::reserve(sorted_keys, writer.new_keys.len())?;
        sorted_keys.extend(
            writer
                .new_keys
                .iter()
                .enumerate()
                .map(|(index, &key)| (key, index as u32)),
        );
        sorted_keys.sort_unstable();
        if sorted_keys.windows(2).any(|pair| pair[0].0 == pair[1].0) {
            return Err(invalid_static("An hour inserts a row key twice"));
        }
        let extent = u32::try_from(writer.table.slots())
            .map_err(|_| invalid_static("Row slots exceed 32 bits"))?;
        let fresh = sorted_keys.len().saturating_sub(writer.free.len() + deleted.len());
        if extent.checked_add(u32::try_from(fresh).unwrap_or(u32::MAX)).is_none() {
            return Err(invalid_static("Row slots exceed 32 bits"));
        }
        by_index.clear();
        engine_allocation::reserve(by_index, inserted as usize)?;
        by_index.resize(inserted as usize, 0);
        by_slot.clear();
        engine_allocation::reserve(by_slot, sorted_keys.len())?;
        popped.clear();
        engine_allocation::reserve(popped, sorted_keys.len())?;
        {
            // The lowest free slots come off the heap here and go back below, so the held
            // heap is unchanged until the hour commits; nothing in between can fail.
            let mut reused = deleted.iter().copied().peekable();
            let mut next = extent;
            for &(key, index) in sorted_keys.iter() {
                let freed = writer.free.peek().map(|lowest| lowest.0);
                let slot = match (freed, reused.peek().copied()) {
                    (Some(a), Some(b)) if a > b => {
                        reused.next();
                        b
                    }
                    (Some(a), _) => {
                        writer.free.pop();
                        popped.push(a);
                        a
                    }
                    (None, Some(b)) => {
                        reused.next();
                        b
                    }
                    (None, None) => {
                        next += 1;
                        next - 1
                    }
                };
                by_index[index as usize] = slot;
                by_slot.push((slot, key, index));
            }
            for &slot in popped.iter() {
                writer.free.push(Reverse(slot));
            }
        }
        by_slot.sort_unstable_by_key(|entry| entry.0);
        inserted_slots.clear();
        engine_allocation::reserve(inserted_slots, by_slot.len())?;
        inserted_slots.extend(by_slot.iter().map(|entry| entry.0));
        keys.clear();
        engine_allocation::reserve(keys, by_slot.len())?;
        keys.extend(by_slot.iter().map(|entry| entry.1));
        insert_order.clear();
        engine_allocation::reserve(insert_order, by_slot.len())?;
        insert_order.extend(by_slot.iter().map(|entry| entry.2));
        // Each column's changed cells by slot, and its NOOPs of cells it did not change. A
        // column whose changed cells all take one value, covering at least half the rows live
        // after the hour, FILLs them. A column the hour did not mark holds no cell.
        let mut live_after = None;
        for column in dirty_columns(&writer.dirty) {
            let (pending, runs) = (&writer.updates[column], &mut writer.runs[column]);
            let noops = &mut writer.noops[column];
            runs.clear();
            if pending.len() == 0 && noops.is_empty() {
                continue;
            }
            engine_allocation::reserve(&mut runs.order, pending.len())?;
            runs.order.extend(0..pending.len() as u32);
            runs.order
                .sort_unstable_by_key(|&entry| pending.at[entry as usize]);
            let slot_of = |entry: &u32| pending.at[*entry as usize];
            noops.sort_unstable();
            noops.dedup();
            noops.retain(|slot| runs.order.binary_search_by_key(slot, slot_of).is_err());
            if pending.uniform && pending.len() >= FILL_MINIMUM {
                let live = *live_after.get_or_insert_with(|| {
                    writer.table.live().count() - deleted.len() + inserted_slots.len()
                });
                if pending.len() * 2 >= live {
                    runs.fill = true;
                    engine_allocation::reserve(&mut runs.except, live - pending.len())?;
                    exceptions(
                        writer.table.live(),
                        deleted.as_slice(),
                        inserted_slots.as_slice(),
                        runs.order.iter().map(slot_of),
                        &mut runs.except,
                    )?;
                }
            }
        }
        // Changed held slots: those with a SET, ADD, PATCH or NOOP, or a cell no longer
        // recorded.
        changed.clear();
        for column in dirty_columns(&writer.dirty) {
            let set = if writer.runs[column].fill {
                &[][..]
            } else {
                &writer.updates[column].at[..]
            };
            engine_allocation::reserve(
                changed,
                set.len() + writer.unrecord[column].len() + writer.noops[column].len(),
            )?;
            changed.extend_from_slice(set);
            changed.extend_from_slice(&writer.unrecord[column]);
            changed.extend_from_slice(&writer.noops[column]);
        }
        changed.sort_unstable();
        changed.dedup();
        if changed.iter().any(|&slot| !seen(slot)) {
            return Err(invalid_static(
                "A row has a NOOP but was not visited this hour",
            ));
        }
        if writer.touched.iter().any(|&slot| !seen(slot)) {
            return Err(invalid_static(
                "A row is touched but was not visited this hour",
            ));
        }
        if partial && !deleted.is_empty() {
            return Err(invalid_static("A partial read deletes no row"));
        }
        writer.touched.sort_unstable();
        writer.touched.dedup();
        let changed = changed.as_slice();
        let position = |slot: u32| changed.binary_search(&slot).expect("changed slot") as u32;
        // Each changed cell in its op's run, by slot; a column of one value SETs it.
        let mut census = Census::default();
        for column in dirty_columns(&writer.dirty) {
            let kind = writer.kinds[column];
            let (pending, runs) = (&writer.updates[column], &mut writer.runs[column]);
            if runs.fill {
                runs.values.push(kind, pending, &pending.words, 0)?;
                census.fills += 1;
                census.fill_slots += pending.len() as u64;
                census.fill_exceptions += runs.except.len() as u64;
            } else {
                let uniform = pending.uniform && pending.len() > 1;
                for &entry in &runs.order {
                    let (entry, at) = (entry as usize, position(pending.at[entry as usize]));
                    let op = if uniform { Op::Set } else { pending.ops[entry] };
                    match op {
                        Op::Add => {
                            push(&mut runs.add_positions, at)?;
                            let base = section::base(kind)?;
                            runs.deltas.push(base, pending, &pending.deltas, entry)?;
                        }
                        Op::Patch => {
                            push(&mut runs.patch_positions, at)?;
                            let (start, count) = pending.hunks[entry];
                            let edits = &pending.edits[start as usize..(start + count) as usize];
                            push(&mut runs.hunks, count)?;
                            extend(&mut runs.edits, edits)?;
                            runs.deltas.inserted(kind, pending, entry, edits)?;
                        }
                        _ => {
                            push(&mut runs.set_positions, at)?;
                            runs.values.push(kind, pending, &pending.words, entry)?;
                        }
                    }
                }
                census.set += runs.set_positions.len() as u64;
                census.add += runs.add_positions.len() as u64;
                census.patch += runs.patch_positions.len() as u64;
            }
            engine_allocation::reserve(&mut runs.noop_positions, writer.noops[column].len())?;
            runs.noop_positions
                .extend(writer.noops[column].iter().map(|&slot| position(slot)));
            let order = insert_order.as_slice();
            writer.insert_outs[column].fill(kind, &writer.inserts[column], order)?;
        }
        let mut runs: Vec<Run<'_>> = recycle(std::mem::take(run_list));
        for column in dirty_columns(&writer.dirty) {
            let kind = writer.kinds[column];
            let column_runs = &writer.runs[column];
            engine_allocation::reserve(&mut runs, 5)?;
            if column_runs.fill {
                runs.push(Run {
                    column,
                    op: Op::Fill,
                    positions: &[],
                    values: Some(column_runs.values.values(kind)),
                    except: &column_runs.except,
                    patches: None,
                });
            }
            if !column_runs.set_positions.is_empty() {
                runs.push(Run {
                    column,
                    op: Op::Set,
                    positions: &column_runs.set_positions,
                    values: Some(column_runs.values.values(kind)),
                    except: &[],
                    patches: None,
                });
            }
            if !column_runs.patch_positions.is_empty() {
                runs.push(Run {
                    column,
                    op: Op::Patch,
                    positions: &column_runs.patch_positions,
                    values: None,
                    except: &[],
                    patches: Some(Patches {
                        hunks: &column_runs.hunks,
                        edits: &column_runs.edits,
                        items: column_runs.deltas.patch_items(kind),
                    }),
                });
            }
            if !column_runs.add_positions.is_empty() {
                runs.push(Run {
                    column,
                    op: Op::Add,
                    positions: &column_runs.add_positions,
                    values: Some(column_runs.deltas.values(section::base(kind)?)),
                    except: &[],
                    patches: None,
                });
            }
            if !column_runs.noop_positions.is_empty() {
                runs.push(Run {
                    column,
                    op: Op::Noop,
                    positions: &column_runs.noop_positions,
                    values: None,
                    except: &[],
                    patches: None,
                });
            }
        }
        let mut inserts: Vec<Values<'_>> = recycle(std::mem::take(insert_list));
        if !inserted_slots.is_empty() {
            engine_allocation::reserve(&mut inserts, columns)?;
            inserts.extend(
                (0..columns).map(|column| writer.insert_outs[column].values(writer.kinds[column])),
            );
        }
        // Unrecorded cells per column: held slots no longer recorded and inserted ones.
        for column in dirty_columns(&writer.dirty) {
            let slots = &mut unrecorded_slots[column];
            slots.clear();
            engine_allocation::reserve(
                slots,
                writer.unrecord[column].len() + writer.insert_unrecorded[column].len(),
            )?;
            slots.extend_from_slice(&writer.unrecord[column]);
            slots.extend(
                writer.insert_unrecorded[column]
                    .iter()
                    .map(|&index| by_index[index as usize]),
            );
            slots.sort_unstable();
        }
        let unrecorded_slots = &*unrecorded_slots;
        let mut unrecorded: Vec<(usize, &[u32])> = recycle(std::mem::take(unrecorded_runs));
        let marked = dirty_columns(&writer.dirty)
            .filter(|&column| !unrecorded_slots[column].is_empty())
            .count();
        engine_allocation::reserve(&mut unrecorded, marked)?;
        for column in dirty_columns(&writer.dirty) {
            if !unrecorded_slots[column].is_empty() {
                unrecorded.push((column, unrecorded_slots[column].as_slice()));
            }
        }
        let mut column_bits = std::mem::take(spare_columns);
        column_bits.clear();
        engine_allocation::reserve(&mut column_bits, columns.div_ceil(64))?;
        column_bits.resize(columns.div_ceil(64), 0);
        for run in &runs {
            column_bits[run.column / 64] |= 1 << (run.column % 64);
        }
        for &(column, _) in &unrecorded {
            column_bits[column / 64] |= 1 << (column % 64);
        }
        let mut ops = 0;
        if !inserted_slots.is_empty() {
            ops |= manifest::INSERTS;
        }
        if !deleted.is_empty() {
            ops |= manifest::DELETES;
        }
        if census.fills > 0 {
            ops |= manifest::FILLS;
        }
        if !writer.touched.is_empty() {
            ops |= manifest::TOUCHES;
        }
        let quiet = runs.is_empty()
            && unrecorded.is_empty()
            && deleted.is_empty()
            && inserted_slots.is_empty()
            && writer.touched.is_empty();
        let bytes = if quiet && !partial {
            None
        } else {
            Some(section::write_with(
                &writer.kinds,
                &TableHour {
                    read: partial.then_some(read.as_slice()),
                    changed,
                    deleted: deleted.as_slice(),
                    inserted: inserted_slots.as_slice(),
                    keys: keys.as_slice(),
                    inserts: &inserts,
                    runs: &runs,
                    unrecorded: &unrecorded,
                    touched: &writer.touched,
                },
                std::mem::take(spare_section),
            )?)
        };
        *run_list = recycle(runs);
        *insert_list = recycle(inserts);
        *unrecorded_runs = recycle(unrecorded);
        writer.finished = writer.generation;
        Ok(Prepared {
            identity: writer.identity,
            generation: writer.generation,
            partial,
            order: false,
            section: bytes,
            columns: column_bits,
            ops,
            census,
        })
    }
}
