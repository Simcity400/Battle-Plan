
#[cfg(test)]
use crate::section::Changes;
use crate::{
    invalid_static,
    leaf::varint_len,
    manifest::{self, Status},
    restart::{self, Acquisition, Last, Restored},
    section::{self, Op, PatchItems, Patches, Run, TableHour},
    state::{Bits, Bools, Cell, Column, Kind, Lists, Paged, PagedBits, Table, Values},
};
use std::{
    cmp::Reverse,
    collections::BinaryHeap,
    hash::BuildHasher,
    io,
    sync::atomic::{AtomicU64, Ordering},
};

#[cfg(test)]
mod tests;

type Hasher = foldhash::fast::RandomState;

static WRITERS: AtomicU64 = AtomicU64::new(1);

const FILL_MINIMUM: usize = 16;

const NEW: u32 = u32::MAX;
const EMPTY: u32 = u32::MAX;
const INSERT_SCRATCH_LIMIT: usize = 16 * 1024;
const HOUR_SCRATCH_LIMIT: usize = 32 * 1024;
const SECTION_SCRATCH_LIMIT: usize = 16 * 1024;
const BATCH_SCRATCH_LIMIT: usize = 256 * 1024;

fn drop_over<T>(values: &mut Vec<T>, limit_bytes: usize) {
    values.clear();
    if values.capacity().saturating_mul(size_of::<T>()) > limit_bytes {
        *values = Vec::new();
    }
}

#[derive(Default)]
pub struct Dictionary {
    table: Vec<u32>,
    hasher: Hasher,
    len: u32,
    appended_from: u32,
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

    pub fn get(&self, id: u32) -> Option<&[u8]> {
        let id = id as usize;
        let end = *self.ends.get(id)?;
        let start = id.checked_sub(1).map_or(0, |previous| self.ends[previous]);
        Some(&self.bytes[start..end])
    }

    fn find(&self, bytes: &[u8]) -> Option<u32> {
        if self.table.is_empty() {
            return None;
        }
        let mask = self.table.len() - 1;
        let mut slot = self.hasher.hash_one(bytes) as usize & mask;
        loop {
            let id = self.table[slot];
            if id == EMPTY {
                return None;
            }
            if self.get(id) == Some(bytes) {
                return Some(id);
            }
            slot = (slot + 1) & mask;
        }
    }

    fn place(table: &mut [u32], hasher: &Hasher, bytes: &[u8], id: u32) {
        let mask = table.len() - 1;
        let mut slot = hasher.hash_one(bytes) as usize & mask;
        while table[slot] != EMPTY {
            slot = (slot + 1) & mask;
        }
        table[slot] = id;
    }

    pub fn intern(&mut self, bytes: &[u8]) -> io::Result<u32> {
        if let Some(id) = self.find(bytes) {
            return Ok(id);
        }
        let id = self.len;
        let next = id
            .checked_add(1)
            .ok_or_else(|| invalid_static("Dictionary ids exceed 32 bits"))?;
        let mut grown = if self.table.is_empty() || (next as usize).saturating_mul(2) > self.table.len()
        {
            let capacity = self.table.len().saturating_mul(2).max(16);
            let mut table = engine_allocation::filled(capacity, EMPTY)?;
            for existing in 0..self.len {
                if let Some(entry) = self.get(existing) {
                    Self::place(&mut table, &self.hasher, entry, existing);
                }
            }
            Some(table)
        } else {
            None
        };
        engine_allocation::reserve(&mut self.bytes, bytes.len())?;
        engine_allocation::reserve(&mut self.ends, 1)?;
        if let Some(table) = grown.take() {
            self.table = table;
        }
        Self::place(&mut self.table, &self.hasher, bytes, id);
        self.bytes.extend_from_slice(bytes);
        self.ends.push(self.bytes.len());
        self.len = next;
        Ok(id)
    }

    pub fn take_appended_range(&mut self) -> std::ops::Range<u32> {
        let from = self.appended_from;
        self.appended_from = self.len;
        from..self.len
    }

    pub fn slice_from(&self, from: u32) -> io::Result<Vec<Box<[u8]>>> {
        let count = self.len.saturating_sub(from) as usize;
        let mut out = engine_allocation::vec(count)?;
        for id in from..self.len {
            let bytes = self.get(id).unwrap_or(&[]);
            let copy = engine_allocation::copy_vec(bytes)?;
            out.push(engine_allocation::boxed_slice(copy)?);
        }
        Ok(out)
    }

    pub fn take_appended(&mut self) -> Vec<Box<[u8]>> {
        let range = self.take_appended_range();
        let mut out = Vec::with_capacity(range.len());
        for id in range {
            if let Some(bytes) = self.get(id) {
                out.push(bytes.to_vec().into_boxed_slice());
            }
        }
        out
    }
}

pub enum Strings<'a> {
    Interning(&'a mut Dictionary),
    Reading(&'a Dictionary),
}

impl Strings<'_> {
    pub fn intern(&mut self, bytes: &[u8]) -> io::Result<u32> {
        match self {
            Self::Interning(dictionary) => dictionary.intern(bytes),
            Self::Reading(dictionary) => dictionary
                .find(bytes)
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

#[derive(Clone, Copy, Debug)]
pub enum Coverage<'a> {
    Complete,
    Partial,
    Changes { removed: &'a [u128] },
}

#[derive(Clone, Copy, Debug)]
pub enum Input<'a> {
    Value(Cell<'a>),
    Text(&'a [u8]),
    Unrecorded,
    Kept,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sample {
    FromReadBit,
    Reset,
    Carried(i32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Evidence {
    pub status: Status,
    pub read: bool,
    pub sample: Sample,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Census {
    pub set: u64,
    pub add: u64,
    pub patch: u64,
    pub fills: u64,
    pub fill_slots: u64,
    pub fill_exceptions: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sweep {
    #[default]
    Touched,
    Every,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Work {
    pub hours: u64,
    pub columns: u64,
}

#[derive(Default)]
struct Cells {
    rows: Vec<u32>,
    words: Vec<u64>,
    present: Vec<u64>,
    u32s: Vec<u32>,
    u64s: Vec<u64>,
    bytes: Vec<u8>,
    unrecorded: Vec<u32>,
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
        self.rows.clear();
        self.words.clear();
        self.present.clear();
        self.u32s.clear();
        self.u64s.clear();
        self.bytes.clear();
        self.unrecorded.clear();
        self.kept.clear();
    }

    fn release_scratch(&mut self, limit_bytes: usize) {
        drop_over(&mut self.rows, limit_bytes);
        drop_over(&mut self.words, limit_bytes);
        drop_over(&mut self.present, limit_bytes);
        drop_over(&mut self.u32s, limit_bytes);
        drop_over(&mut self.u64s, limit_bytes);
        drop_over(&mut self.bytes, limit_bytes);
        drop_over(&mut self.unrecorded, limit_bytes);
        drop_over(&mut self.kept, limit_bytes);
    }

    fn end(&self, kind: Kind) -> u64 {
        (match kind {
            Kind::U32List => self.u32s.len(),
            Kind::U64List => self.u64s.len(),
            Kind::Bytes | Kind::Opaque => self.bytes.len(),
            _ => 0,
        }) as u64
    }

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

#[derive(Default)]
pub struct Batch {
    kinds: Vec<Kind>,
    keys: Vec<u128>,
    columns: Vec<Cells>,
    sparse: bool,
    uniform_optional_i32: Option<(usize, Option<i32>)>,
}

impl Batch {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn release_scratch(&mut self) {
        drop_over(&mut self.keys, BATCH_SCRATCH_LIMIT);
        for cells in &mut self.columns {
            cells.release_scratch(BATCH_SCRATCH_LIMIT);
        }
        self.sparse = false;
        self.uniform_optional_i32 = None;
    }

    pub fn begin(&mut self, kinds: &[Kind], rows: usize) -> io::Result<()> {
        if !self.keys.is_empty() {
            for cells in self.columns.iter_mut().take(self.kinds.len()) {
                cells.clear();
            }
        }
        self.sparse = false;
        self.uniform_optional_i32 = None;
        self.keys.clear();
        if self.columns.len() < kinds.len() {
            engine_allocation::resize_default(&mut self.columns, kinds.len())?;
        }
        self.kinds.clear();
        extend(&mut self.kinds, kinds)?;
        if self.keys.capacity() < rows {
            engine_allocation::reserve(&mut self.keys, rows)?;
        }
        if rows > 0 {
            let present_words = rows.div_ceil(64);
            for (cells, &kind) in self.columns.iter_mut().zip(kinds) {
                if cells.words.capacity() < rows {
                    engine_allocation::reserve(&mut cells.words, rows)?;
                }
                if optional(kind) && cells.present.capacity() < present_words {
                    engine_allocation::reserve(&mut cells.present, present_words)?;
                }
            }
        }
        Ok(())
    }

    pub fn begin_sparse(&mut self, kinds: &[Kind], rows: usize) -> io::Result<()> {
        if !self.keys.is_empty() {
            for cells in self.columns.iter_mut().take(self.kinds.len()) {
                cells.clear();
            }
        }
        self.sparse = true;
        self.uniform_optional_i32 = None;
        self.keys.clear();
        if self.columns.len() < kinds.len() {
            engine_allocation::resize_default(&mut self.columns, kinds.len())?;
        }
        self.kinds.clear();
        extend(&mut self.kinds, kinds)?;
        if self.keys.capacity() < rows {
            engine_allocation::reserve(&mut self.keys, rows)?;
        }
        Ok(())
    }

    pub fn fill_optional_i32(&mut self, column: usize, value: Option<i32>) -> io::Result<()> {
        if self.kinds.get(column).copied() != Some(Kind::OptionalI32) {
            return Err(invalid_static("A uniform fill targets another column kind"));
        }
        self.uniform_optional_i32 = Some((column, value));
        Ok(())
    }

    pub fn rows(&self) -> usize {
        self.keys.len()
    }

    #[inline]
    pub fn key(&mut self, key: u128) -> io::Result<()> {
        push(&mut self.keys, key)
    }

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
        if self.sparse {
            let entry = cells.words.len();
            if cells
                .rows
                .last()
                .copied()
                .is_some_and(|last| last as usize >= row)
            {
                return Err(invalid_static(
                    "A batch row carries other than one cell per column",
                ));
            }
            push(&mut cells.rows, row as u32)?;
            return Ok((cells, entry));
        }
        if cells.words.len() != row {
            return Err(invalid_static(
                "A batch row carries other than one cell per column",
            ));
        }
        Ok((cells, row))
    }

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

    #[inline]
    pub fn text(&mut self, column: usize, text: &[u8]) -> io::Result<()> {
        let (cells, _) = self.next(column, Kind::Bytes)?;
        extend(&mut cells.bytes, text)?;
        let end = cells.bytes.len() as u64;
        push(&mut cells.words, end)
    }

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

fn presence(cells: &mut Cells, row: usize, present: bool) -> io::Result<()> {
    if row % 64 == 0 {
        push(&mut cells.present, 0)?;
    }
    if present {
        cells.present[row / 64] |= 1 << (row % 64);
    }
    Ok(())
}

trait Word: Copy {
    fn word(self) -> u64;
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

trait Items: Copy + PartialEq {
    const PROBE: usize;
    fn cell(items: &[Self]) -> Cell<'_>;
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

struct Staged<'s> {
    slots: &'s [u32],
    cells: &'s Cells,
    marked: Option<&'s Bits>,
    pending: &'s mut Pending,
    unrecord: &'s mut Vec<u32>,
    dirty: bool,
    has_new: bool,
}

impl Staged<'_> {
    #[inline]
    fn all_recorded(&self) -> bool {
        self.cells.unrecorded.is_empty()
            && self.cells.kept.is_empty()
            && self.marked.is_none()
            && !self.has_new
    }

    #[inline]
    fn each(
        &mut self,
        mut change: impl FnMut(&mut Pending, usize, u32, bool) -> io::Result<bool>,
    ) -> io::Result<()> {
        let unrecorded = self.cells.unrecorded.as_slice();
        let kept = self.cells.kept.as_slice();
        let mut unrecorded_at = 0usize;
        let mut kept_at = 0usize;
        let mut next_unrecorded = unrecorded.first().copied().unwrap_or(u32::MAX);
        let mut next_kept = kept.first().copied().unwrap_or(u32::MAX);
        for (row, &slot) in (0u32..).zip(self.slots) {
            let recorded = if next_unrecorded == row {
                unrecorded_at += 1;
                next_unrecorded = unrecorded.get(unrecorded_at).copied().unwrap_or(u32::MAX);
                false
            } else {
                true
            };
            let is_kept = if next_kept == row {
                kept_at += 1;
                next_kept = kept.get(kept_at).copied().unwrap_or(u32::MAX);
                true
            } else {
                false
            };
            if is_kept || slot == NEW {
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

    fn scalars<T: Word>(&mut self, held: &Paged<T>) -> io::Result<()> {
        if self.all_recorded() {
            for (&slot, &new) in self.slots.iter().zip(&self.cells.words) {
                let value = *held
                    .get(slot as usize)
                    .ok_or_else(|| invalid_static("A held slot is past its column"))?;
                if value.word() == new {
                    continue;
                }
                self.dirty = true;
                self.pending
                    .change(slot, T::plain(new), Some(T::plain(value.word())))?;
            }
            return Ok(());
        }
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

    fn bools(&mut self, held: &Bools) -> io::Result<()> {
        if self.all_recorded() {
            for (&slot, &new) in self.slots.iter().zip(&self.cells.words) {
                let value = *held
                    .get(slot as usize)
                    .ok_or_else(|| invalid_static("A held slot is past its column"))?;
                if value.word() == new {
                    continue;
                }
                self.dirty = true;
                self.pending
                    .change(slot, bool::plain(new), Some(bool::plain(value.word())))?;
            }
            return Ok(());
        }
        let cells = self.cells;
        self.each(|pending, row, slot, was| {
            let value = *held
                .get(slot as usize)
                .ok_or_else(|| invalid_static("A held slot is past its column"))?;
            let new = cells.words[row];
            if was && value.word() == new {
                return Ok(false);
            }
            let held = was.then(|| bool::plain(value.word()));
            pending.change(slot, bool::plain(new), held).map(|()| true)
        })
    }

    fn optionals<T: Present>(&mut self, held: &Paged<T>, presence: &PagedBits) -> io::Result<()> {
        if self.all_recorded() {
            let cells = self.cells;
            for (row, (&slot, &word)) in self.slots.iter().zip(&cells.words).enumerate() {
                let value = *held
                    .get(slot as usize)
                    .ok_or_else(|| invalid_static("A held slot is past its column"))?;
                let old = presence.get(slot).then(|| value.word());
                let new = cells.present(row).then_some(word);
                if old == new {
                    continue;
                }
                self.dirty = true;
                self.pending
                    .change(slot, T::optional(new), Some(T::optional(old)))?;
            }
            return Ok(());
        }
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

    fn lists<T: Items>(&mut self, held: &Lists<T>, items: &[T]) -> io::Result<()> {
        if self.all_recorded() {
            let mut start = 0usize;
            for (&slot, &end) in self.slots.iter().zip(&self.cells.words) {
                let end = end as usize;
                let value = held
                    .get(slot as usize)
                    .ok_or_else(|| invalid_static("A held slot is past its column"))?;
                let new = &items[start..end];
                start = end;
                if *value == *new {
                    continue;
                }
                self.dirty = true;
                self.pending
                    .change(slot, T::cell(new), Some(T::cell(value)))?;
            }
            return Ok(());
        }
        let cells = self.cells;
        self.each(|pending, row, slot, was| {
            let value = held
                .get(slot as usize)
                .ok_or_else(|| invalid_static("A held slot is past its column"))?;
            let new = &items[cells.span(row)];
            if was && *value == *new {
                return Ok(false);
            }
            let held = was.then(|| T::cell(value));
            pending.change(slot, T::cell(new), held).map(|()| true)
        })
    }

    #[inline]
    fn sparse_all_recorded(&self) -> bool {
        self.cells.unrecorded.is_empty() && self.marked.is_none() && !self.has_new
    }

    #[inline]
    fn each_sparse(
        &mut self,
        mut change: impl FnMut(&mut Pending, usize, u32, bool) -> io::Result<bool>,
    ) -> io::Result<()> {
        let unrecorded = self.cells.unrecorded.as_slice();
        let mut unrecorded_at = 0usize;
        let mut next_unrecorded = unrecorded.first().copied().unwrap_or(u32::MAX);
        for (entry, &row) in (0u32..).zip(&self.cells.rows) {
            let recorded = if next_unrecorded == entry {
                unrecorded_at += 1;
                next_unrecorded = unrecorded.get(unrecorded_at).copied().unwrap_or(u32::MAX);
                false
            } else {
                true
            };
            let slot = self.slots[row as usize];
            if slot == NEW {
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
            self.dirty |= change(&mut *self.pending, entry as usize, slot, was)?;
        }
        Ok(())
    }

    fn sparse_scalars<T: Word>(&mut self, held: &Paged<T>) -> io::Result<()> {
        if self.sparse_all_recorded() {
            for (&row, &new) in self.cells.rows.iter().zip(&self.cells.words) {
                let slot = self.slots[row as usize];
                let value = *held
                    .get(slot as usize)
                    .ok_or_else(|| invalid_static("A held slot is past its column"))?;
                if value.word() == new {
                    continue;
                }
                self.dirty = true;
                self.pending
                    .change(slot, T::plain(new), Some(T::plain(value.word())))?;
            }
            return Ok(());
        }
        let cells = self.cells;
        self.each_sparse(|pending, entry, slot, was| {
            let value = *held
                .get(slot as usize)
                .ok_or_else(|| invalid_static("A held slot is past its column"))?;
            let new = cells.words[entry];
            if was && value.word() == new {
                return Ok(false);
            }
            let held = was.then(|| T::plain(value.word()));
            pending.change(slot, T::plain(new), held).map(|()| true)
        })
    }

    fn sparse_bools(&mut self, held: &Bools) -> io::Result<()> {
        if self.sparse_all_recorded() {
            for (&row, &new) in self.cells.rows.iter().zip(&self.cells.words) {
                let slot = self.slots[row as usize];
                let value = *held
                    .get(slot as usize)
                    .ok_or_else(|| invalid_static("A held slot is past its column"))?;
                if value.word() == new {
                    continue;
                }
                self.dirty = true;
                self.pending
                    .change(slot, bool::plain(new), Some(bool::plain(value.word())))?;
            }
            return Ok(());
        }
        let cells = self.cells;
        self.each_sparse(|pending, entry, slot, was| {
            let value = *held
                .get(slot as usize)
                .ok_or_else(|| invalid_static("A held slot is past its column"))?;
            let new = cells.words[entry];
            if was && value.word() == new {
                return Ok(false);
            }
            let held = was.then(|| bool::plain(value.word()));
            pending.change(slot, bool::plain(new), held).map(|()| true)
        })
    }

    fn sparse_optionals<T: Present>(
        &mut self,
        held: &Paged<T>,
        presence: &PagedBits,
    ) -> io::Result<()> {
        if self.sparse_all_recorded() {
            let cells = self.cells;
            for (entry, (&row, &word)) in cells.rows.iter().zip(&cells.words).enumerate() {
                let slot = self.slots[row as usize];
                let value = *held
                    .get(slot as usize)
                    .ok_or_else(|| invalid_static("A held slot is past its column"))?;
                let old = presence.get(slot).then(|| value.word());
                let new = cells.present(entry).then_some(word);
                if old == new {
                    continue;
                }
                self.dirty = true;
                self.pending
                    .change(slot, T::optional(new), Some(T::optional(old)))?;
            }
            return Ok(());
        }
        let cells = self.cells;
        self.each_sparse(|pending, entry, slot, was| {
            let value = *held
                .get(slot as usize)
                .ok_or_else(|| invalid_static("A held slot is past its column"))?;
            let old = presence.get(slot).then(|| value.word());
            let new = cells.present(entry).then(|| cells.words[entry]);
            if was && old == new {
                return Ok(false);
            }
            let held = was.then(|| T::optional(old));
            pending.change(slot, T::optional(new), held).map(|()| true)
        })
    }

    fn sparse_lists<T: Items>(&mut self, held: &Lists<T>, items: &[T]) -> io::Result<()> {
        if self.sparse_all_recorded() {
            let mut start = 0usize;
            for (&row, &end) in self.cells.rows.iter().zip(&self.cells.words) {
                let slot = self.slots[row as usize];
                let end = end as usize;
                let value = held
                    .get(slot as usize)
                    .ok_or_else(|| invalid_static("A held slot is past its column"))?;
                let new = &items[start..end];
                start = end;
                if *value == *new {
                    continue;
                }
                self.dirty = true;
                self.pending
                    .change(slot, T::cell(new), Some(T::cell(value)))?;
            }
            return Ok(());
        }
        let cells = self.cells;
        self.each_sparse(|pending, entry, slot, was| {
            let value = held
                .get(slot as usize)
                .ok_or_else(|| invalid_static("A held slot is past its column"))?;
            let new = &items[cells.span(entry)];
            if was && *value == *new {
                return Ok(false);
            }
            let held = was.then(|| T::cell(value));
            pending.change(slot, T::cell(new), held).map(|()| true)
        })
    }
}

fn recycle<T, U>(values: Vec<T>) -> Vec<U> {
    values.into_iter().filter_map(|_| None).collect()
}

#[derive(Default)]
struct Pending {
    at: Vec<u32>,
    words: Vec<u64>,
    present: Vec<bool>,
    spans: Vec<(u32, u32)>,
    u32s: Vec<u32>,
    u64s: Vec<u64>,
    bytes: Vec<u8>,
    ops: Vec<Op>,
    deltas: Vec<u64>,
    hunks: Vec<(u32, u32)>,
    edits: Vec<(u32, u32, u32)>,
    uniform: bool,
}

fn span(start: usize, len: usize) -> io::Result<(u32, u32)> {
    let end = start
        .checked_add(len)
        .filter(|&end| u32::try_from(end).is_ok())
        .ok_or_else(|| invalid_static("Cell items exceed 32 bits"))?;
    Ok((start as u32, (end - start) as u32))
}

#[inline]
fn reserve<T>(values: &mut Vec<T>, additional: usize) -> io::Result<()> {
    if values.capacity() - values.len() < additional {
        engine_allocation::reserve(values, additional)?;
    }
    Ok(())
}

#[inline]
fn push<T>(values: &mut Vec<T>, value: T) -> io::Result<()> {
    if values.len() == values.capacity() {
        engine_allocation::reserve(values, 1)?;
    }
    values.push(value);
    Ok(())
}

#[inline]
fn extend<T: Copy>(values: &mut Vec<T>, items: &[T]) -> io::Result<()> {
    if values.capacity() - values.len() < items.len() {
        engine_allocation::reserve(values, items.len())?;
    }
    values.extend_from_slice(items);
    Ok(())
}

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

fn edit_bytes(hunks: &[(u32, u32, u32)]) -> usize {
    varint_len(hunks.len() as u64)
        + hunks
            .iter()
            .map(|&(skip, removed, added)| {
                varint_len(skip.into()) + varint_len(removed.into()) + varint_len(added.into())
            })
            .sum::<usize>()
}

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
            let result = (|| {
                let found = matcher.find(held, new, (prefix, suffix), &items)?;
                if found < cost && found * 2 <= whole {
                    edits.truncate(start);
                    extend(edits, &matcher.edits)?;
                    cost = found;
                }
                Ok::<_, io::Error>(())
            })();
            drop_over(&mut matcher.heads, 64 << 10);
            drop_over(&mut matcher.next, 64 << 10);
            drop_over(&mut matcher.edits, 64 << 10);
            result
        })?;
    }
    Ok(cost < whole)
}

const MATCH_FLOOR: usize = 32;

const MATCH_PROBES: usize = 8;

const NO_POSITION: u32 = u32::MAX;

thread_local! {
    static MATCHER: std::cell::RefCell<Matcher> = std::cell::RefCell::default();
}

#[derive(Default)]
struct Matcher {
    heads: Vec<(u32, u32)>,
    generation: u32,
    next: Vec<u32>,
    edits: Vec<(u32, u32, u32)>,
}

impl Matcher {
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
        let table_len = 1_usize << bits;
        self.generation = self.generation.wrapping_add(1);
        if self.generation == 0 {
            self.heads.fill((0, NO_POSITION));
            self.generation = 1;
        }
        if self.heads.len() < table_len {
            engine_allocation::resize(&mut self.heads, table_len, (0, NO_POSITION))?;
        }
        let generation = self.generation;
        if self.next.len() < windows {
            engine_allocation::resize(&mut self.next, windows, NO_POSITION)?;
        }
        for at in (start..start + windows).rev() {
            let head = &mut self.heads[bucket(T::probe(&held[at..at + probe]))];
            let previous = if head.0 == generation {
                head.1
            } else {
                NO_POSITION
            };
            self.next[at - start] = previous;
            *head = (generation, at as u32);
        }
        let (mut cursor, mut at, mut unmatched, mut kept, mut inserted) =
            (start, start, start, prefix, 0);
        while at + probe <= new_end {
            let head = &mut self.heads[bucket(T::probe(&new[at..at + probe]))];
            let mut position = if head.0 == generation {
                head.1
            } else {
                NO_POSITION
            };
            while position != NO_POSITION && (position as usize) < cursor {
                position = self.next[position as usize - start];
            }
            if head.0 == generation {
                head.1 = position;
            }
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

    fn release_scratch(&mut self, limit_bytes: usize) {
        drop_over(&mut self.at, limit_bytes);
        drop_over(&mut self.words, limit_bytes);
        drop_over(&mut self.present, limit_bytes);
        drop_over(&mut self.spans, limit_bytes);
        drop_over(&mut self.u32s, limit_bytes);
        drop_over(&mut self.u64s, limit_bytes);
        drop_over(&mut self.bytes, limit_bytes);
        drop_over(&mut self.ops, limit_bytes);
        drop_over(&mut self.deltas, limit_bytes);
        drop_over(&mut self.hunks, limit_bytes);
        drop_over(&mut self.edits, limit_bytes);
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

    fn extend_scalar_words(&mut self, words: &[u64]) -> io::Result<()> {
        let base = u32::try_from(self.at.len())
            .ok()
            .filter(|&base| base.checked_add(u32::try_from(words.len()).unwrap_or(u32::MAX)).is_some())
            .ok_or_else(|| invalid_static("An hour inserts more than 2^32 rows"))?;
        reserve(&mut self.at, words.len())?;
        reserve(&mut self.words, words.len())?;
        reserve(&mut self.present, words.len())?;
        self.at.extend(base..base + words.len() as u32);
        self.words.extend_from_slice(words);
        self.present.resize(self.present.len() + words.len(), true);
        Ok(())
    }

    fn push(&mut self, at: u32, cell: Cell<'_>) -> io::Result<()> {
        match cell {
            Cell::U32List(items) => Self::items(&mut self.spans, &mut self.u32s, items)?,
            Cell::U64List(items) => Self::items(&mut self.spans, &mut self.u64s, items)?,
            Cell::Opaque(items) => Self::items(&mut self.spans, &mut self.bytes, items)?,
            _ => {}
        }
        let (word, present) = word(cell);
        reserve(&mut self.at, 1)?;
        reserve(&mut self.words, 1)?;
        reserve(&mut self.present, 1)?;
        self.at.push(at);
        self.words.push(word);
        self.present.push(present);
        Ok(())
    }

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

    fn release_scratch(&mut self, limit_bytes: usize) {
        drop_over(&mut self.u8s, limit_bytes);
        drop_over(&mut self.i8s, limit_bytes);
        drop_over(&mut self.u16s, limit_bytes);
        drop_over(&mut self.i16s, limit_bytes);
        drop_over(&mut self.u32s, limit_bytes);
        drop_over(&mut self.i32s, limit_bytes);
        drop_over(&mut self.u64s, limit_bytes);
        drop_over(&mut self.i64s, limit_bytes);
        drop_over(&mut self.present, limit_bytes);
        drop_over(&mut self.ends, limit_bytes);
        self.count = 0;
    }

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

    fn push_optional_i32(&mut self, value: Option<i32>) -> io::Result<()> {
        if self.count % 64 == 0 {
            push(&mut self.present, 0)?;
        }
        if let Some(value) = value {
            self.present[self.count / 64] |= 1 << (self.count % 64);
            push(&mut self.i32s, value)?;
        }
        self.count += 1;
        Ok(())
    }

    fn fill(&mut self, kind: Kind, pending: &Pending, order: &[u32]) -> io::Result<()> {
        self.clear();
        let count = order.len();
        if count == 0 {
            return Ok(());
        }
        let words = &pending.words;
        match kind {
            Kind::U8 | Kind::Bool => {
                reserve(&mut self.u8s, count)?;
                self.u8s.extend(order.iter().map(|&at| words[at as usize] as u8));
                self.count = count;
            }
            Kind::I8 => {
                reserve(&mut self.i8s, count)?;
                self.i8s.extend(order.iter().map(|&at| words[at as usize] as i8));
                self.count = count;
            }
            Kind::U16 => {
                reserve(&mut self.u16s, count)?;
                self.u16s.extend(order.iter().map(|&at| words[at as usize] as u16));
                self.count = count;
            }
            Kind::I16 => {
                reserve(&mut self.i16s, count)?;
                self.i16s.extend(order.iter().map(|&at| words[at as usize] as i16));
                self.count = count;
            }
            Kind::U32 | Kind::Bytes => {
                reserve(&mut self.u32s, count)?;
                self.u32s.extend(order.iter().map(|&at| words[at as usize] as u32));
                self.count = count;
            }
            Kind::I32 => {
                reserve(&mut self.i32s, count)?;
                self.i32s.extend(order.iter().map(|&at| words[at as usize] as i32));
                self.count = count;
            }
            Kind::U64 => {
                reserve(&mut self.u64s, count)?;
                self.u64s.extend(order.iter().map(|&at| words[at as usize]));
                self.count = count;
            }
            Kind::I64 => {
                reserve(&mut self.i64s, count)?;
                self.i64s.extend(order.iter().map(|&at| words[at as usize] as i64));
                self.count = count;
            }
            _ => {
                for &at in order {
                    self.push(kind, pending, words, at as usize)?;
                }
            }
        }
        Ok(())
    }

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

#[derive(Default)]
struct Runs {
    order: Vec<u32>,
    fill: bool,
    except: Vec<u32>,
    set_positions: Vec<u32>,
    add_positions: Vec<u32>,
    patch_positions: Vec<u32>,
    noop_positions: Vec<u32>,
    values: Out,
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

    fn release_scratch(&mut self, limit_bytes: usize) {
        drop_over(&mut self.order, limit_bytes);
        self.fill = false;
        drop_over(&mut self.except, limit_bytes);
        drop_over(&mut self.set_positions, limit_bytes);
        drop_over(&mut self.add_positions, limit_bytes);
        drop_over(&mut self.patch_positions, limit_bytes);
        drop_over(&mut self.noop_positions, limit_bytes);
        self.values.release_scratch(limit_bytes);
        self.deltas.release_scratch(limit_bytes);
        drop_over(&mut self.hunks, limit_bytes);
        drop_over(&mut self.edits, limit_bytes);
    }
}

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

fn kept_insert() -> io::Error {
    invalid_static("A kept cell's row is not held")
}

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

fn mark_dirty(dirty: &mut [u64], column: usize) {
    dirty[column / 64] |= 1 << (column % 64);
}

fn mark_all_dirty(dirty: &mut [u64], columns: usize) {
    for (index, word) in dirty.iter_mut().enumerate() {
        *word = match columns.saturating_sub(index * 64) {
            left @ 0..64 => (1u64 << left) - 1,
            _ => u64::MAX,
        };
    }
}

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

fn see(seen: &mut [u64], visited: &mut Vec<u32>, slot: u32) -> io::Result<()> {
    let word = &mut seen[slot as usize / 64];
    let bit = 1u64 << (slot % 64);
    if *word & bit != 0 {
        return Err(invalid_static("An hour visits a row twice"));
    }
    reserve(visited, 1)?;
    *word |= bit;
    visited.push(slot);
    Ok(())
}

struct ColumnScratch {
    updates: Vec<Pending>,
    unrecord: Vec<Vec<u32>>,
    noops: Vec<Vec<u32>>,
    inserts: Vec<Pending>,
    insert_unrecorded: Vec<Vec<u32>>,
    runs: Vec<Runs>,
    insert_outs: Vec<Out>,
    unrecorded: Vec<Vec<u32>>,
}

impl ColumnScratch {
    fn new(columns: usize) -> io::Result<Box<Self>> {
        let many = |count: usize| engine_allocation::collect_vec((0..count).map(|_| Vec::new()));
        engine_allocation::boxed(Self {
            updates: engine_allocation::collect_vec((0..columns).map(|_| Pending::default()))?,
            unrecord: many(columns)?,
            noops: many(columns)?,
            inserts: engine_allocation::collect_vec((0..columns).map(|_| Pending::default()))?,
            insert_unrecorded: many(columns)?,
            runs: engine_allocation::collect_vec((0..columns).map(|_| Runs::default()))?,
            insert_outs: engine_allocation::collect_vec((0..columns).map(|_| Out::default()))?,
            unrecorded: many(columns)?,
        })
    }
}

pub struct TableWriter {
    kinds: Box<[Kind]>,
    table: Table,
    status: Status,
    acquisition: Acquisition,
    view: Vec<u32>,
    keeps: bool,
    last: Option<Last>,
    free: BinaryHeap<Reverse<u32>>,
    identity: u64,
    generation: u64,
    finished: u64,
    poisoned: bool,
    seen: Vec<u64>,
    visited: Vec<u32>,
    columns: Option<Box<ColumnScratch>>,
    touched: Vec<u32>,
    removed: Vec<u128>,
    new_keys: Vec<u128>,
    #[cfg(test)]
    changes: Changes,
    #[cfg(test)]
    fill_merge_slots: Vec<u32>,
    #[cfg(test)]
    fill_merge_masks: Vec<u64>,
    texts: Box<[usize]>,
    slots: Vec<u32>,
    insert_rows: Vec<u32>,
    cursors: Vec<(usize, usize)>,
    order: Order,
    parts: Parts,
    dirty: Vec<u64>,
    sweep: Sweep,
    work: Work,
    uniform_optional_i32: Option<(usize, Option<i32>)>,
}

#[derive(Default)]
struct Order {
    slots: Vec<u32>,
    len: usize,
    identity: bool,
    held: bool,
}

#[derive(Default)]
struct Parts {
    deleted: Vec<u32>,
    read: Vec<u32>,
    sorted_keys: Vec<(u128, u32)>,
    by_index: Vec<u32>,
    by_slot: Vec<(u32, u128, u32)>,
    popped: Vec<u32>,
    inserted: Vec<u32>,
    keys: Vec<u128>,
    insert_order: Vec<u32>,
    changed: Vec<u32>,
    apply_slots: Vec<u32>,
    runs: Vec<Run<'static>>,
    inserts: Vec<Values<'static>>,
    unrecorded_runs: Vec<(usize, &'static [u32])>,
    columns: Vec<u64>,
    section: Vec<u8>,
}

impl Parts {
    fn release_scratch(&mut self) {
        drop_over(&mut self.deleted, HOUR_SCRATCH_LIMIT);
        drop_over(&mut self.read, HOUR_SCRATCH_LIMIT);
        drop_over(&mut self.sorted_keys, INSERT_SCRATCH_LIMIT);
        drop_over(&mut self.by_index, INSERT_SCRATCH_LIMIT);
        drop_over(&mut self.by_slot, INSERT_SCRATCH_LIMIT);
        drop_over(&mut self.popped, INSERT_SCRATCH_LIMIT);
        drop_over(&mut self.inserted, INSERT_SCRATCH_LIMIT);
        drop_over(&mut self.keys, INSERT_SCRATCH_LIMIT);
        drop_over(&mut self.insert_order, INSERT_SCRATCH_LIMIT);
        drop_over(&mut self.changed, HOUR_SCRATCH_LIMIT);
        drop_over(&mut self.apply_slots, HOUR_SCRATCH_LIMIT);
        drop_over(&mut self.runs, INSERT_SCRATCH_LIMIT);
        drop_over(&mut self.inserts, INSERT_SCRATCH_LIMIT);
        drop_over(&mut self.unrecorded_runs, INSERT_SCRATCH_LIMIT);
        drop_over(&mut self.columns, INSERT_SCRATCH_LIMIT);
        drop_over(&mut self.section, SECTION_SCRATCH_LIMIT);
    }

    fn drop_all(&mut self) {
        *self = Self::default();
    }
}

impl TableWriter {
    pub fn new(kinds: &[Kind]) -> io::Result<Self> {
        Self::with_table(kinds, Table::new(kinds)?)
    }

    fn with_table(kinds: &[Kind], table: Table) -> io::Result<Self> {
        let columns = kinds.len();
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
            keeps: false,
            last: None,
            free: BinaryHeap::new(),
            identity: WRITERS.fetch_add(1, Ordering::Relaxed),
            generation: 0,
            finished: 0,
            poisoned: false,
            seen: Vec::new(),
            visited: Vec::new(),
            columns: None,
            touched: Vec::new(),
            removed: Vec::new(),
            new_keys: Vec::new(),
            #[cfg(test)]
            changes: Changes::default(),
            #[cfg(test)]
            fill_merge_slots: Vec::new(),
            #[cfg(test)]
            fill_merge_masks: Vec::new(),
            texts: engine_allocation::boxed_slice(texts)?,
            slots: Vec::new(),
            insert_rows: Vec::new(),
            cursors: Vec::new(),
            order: Order::default(),
            parts: Parts::default(),
            dirty: engine_allocation::filled(columns.div_ceil(64), 0_u64)?,
            sweep: Sweep::Touched,
            work: Work::default(),
            uniform_optional_i32: None,
        })
    }

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
        writer.last = restored.last;
        writer.free = BinaryHeap::from(free);
        Ok(writer)
    }

    pub fn keep_last(&mut self) {
        self.keeps = true;
    }

    pub fn last(&self) -> Option<&Last> {
        self.last.as_ref().filter(|_| self.keeps)
    }

    pub fn restart(&self) -> io::Result<Vec<u8>> {
        self.usable()?;
        restart::write_with(
            &self.table,
            self.status,
            &self.acquisition,
            &self.view,
            self.last(),
        )
    }

    pub fn table(&self) -> &Table {
        &self.table
    }

    pub fn has_texts(&self) -> bool {
        !self.texts.is_empty()
    }

    pub fn status(&self) -> Status {
        self.status
    }

    pub fn acquisition(&self) -> &Acquisition {
        &self.acquisition
    }

    pub fn view(&self) -> &[u32] {
        &self.view
    }

    pub fn kinds(&self) -> &[Kind] {
        &self.kinds
    }

    pub fn sweep(&self) -> Sweep {
        self.sweep
    }

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
        if self.columns.is_none() {
            self.columns = Some(ColumnScratch::new(columns)?);
            self.dirty.fill(0);
        }
        if unfinished || self.sweep == Sweep::Every {
            mark_all_dirty(&mut self.dirty, columns);
        }
        let Self {
            dirty,
            columns: scratch,
            work,
            ..
        } = &mut *self;
        let scratch = scratch.as_mut().expect("column scratch initialized");
        work.columns += dirty_count(dirty.as_slice());
        for column in dirty_columns(dirty.as_slice()) {
            scratch.updates[column].clear();
            scratch.inserts[column].clear();
            scratch.unrecord[column].clear();
            scratch.noops[column].clear();
            scratch.insert_unrecorded[column].clear();
            scratch.runs[column].clear();
            scratch.insert_outs[column].clear();
            scratch.unrecorded[column].clear();
        }
        dirty.fill(0);
        if self.sweep == Sweep::Every {
            mark_all_dirty(&mut self.dirty, columns);
        }
        self.touched.clear();
        self.new_keys.clear();
        self.removed.clear();
        self.uniform_optional_i32 = None;
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

    fn hold_order(&mut self) -> io::Result<()> {
        let Self {
            table,
            slots,
            parts,
            order,
            ..
        } = self;
        order.held = false;
        order.identity = false;
        order.len = 0;
        let len = slots.len();
        let mut identity = true;
        {
            let mut inserted = parts.by_index.iter().copied();
            for (expected, &slot) in (0u32..).zip(slots.iter()) {
                let resolved = match slot {
                    NEW => inserted
                        .next()
                        .ok_or_else(|| invalid_static("A batch's new row has no slot"))?,
                    slot => slot,
                };
                if table.key(resolved).is_none() {
                    return Err(invalid_static("A batch's row is not held"));
                }
                if resolved != expected {
                    identity = false;
                }
            }
        }
        if identity {
            order.slots = Vec::new();
            order.identity = true;
        } else {
            order.slots.clear();
            if order.slots.capacity() < len {
                engine_allocation::reserve_exact(&mut order.slots, len)?;
            }
            let mut inserted = parts.by_index.iter().copied();
            for &slot in slots.iter() {
                order.slots.push(match slot {
                    NEW => inserted
                        .next()
                        .ok_or_else(|| invalid_static("A batch's new row has no slot"))?,
                    slot => slot,
                });
            }
        }
        order.len = len;
        order.held = true;
        Ok(())
    }

    fn apply_prepared(&mut self, truncated: bool) -> io::Result<()> {
        let Self {
            kinds,
            table,
            free,
            columns,
            parts,
            dirty,
            #[cfg(test)]
            changes,
            #[cfg(test)]
            fill_merge_slots,
            #[cfg(test)]
            fill_merge_masks,
            ..
        } = self;
        let ColumnScratch {
            runs,
            insert_outs,
            unrecorded,
            ..
        } = &**columns.as_ref().expect("column scratch prepared");
        #[cfg(test)]
        {
            changes.fill_slots.clear();
            changes.fill_masks.clear();
        }
        let extent = table.slots();
        table.delete(&parts.deleted)?;
        if !parts.inserted.is_empty() {
            table.insert_keys(&parts.inserted, &parts.keys)?;
            for (column, &kind) in kinds.iter().enumerate() {
                table.set(column, &parts.inserted, insert_outs[column].values(kind))?;
            }
        }
        let apply_slots = &mut parts.apply_slots;
        let changed = parts.changed.as_slice();
        for column in dirty_columns(dirty.as_slice()) {
            let kind = kinds[column];
            let column_runs = &runs[column];
            if column_runs.fill {
                table.fill(
                    column,
                    &column_runs.except,
                    column_runs.values.values(kind),
                    apply_slots,
                )?;
                if truncated
                    && apply_slots.iter().any(|slot| {
                        parts.read.binary_search(slot).is_err()
                            && parts.inserted.binary_search(slot).is_err()
                    })
                {
                    return Err(invalid_static(
                        "A partly read table fills a slot it did not read",
                    ));
                }
                #[cfg(test)]
                section::union(
                    &mut changes.fill_slots,
                    &mut changes.fill_masks,
                    apply_slots,
                    section::column_bit(column),
                    fill_merge_slots,
                    fill_merge_masks,
                )?;
            }
            if !column_runs.set_positions.is_empty() {
                apply_slots.clear();
                reserve(apply_slots, column_runs.set_positions.len())?;
                apply_slots.extend(
                    column_runs
                        .set_positions
                        .iter()
                        .map(|&position| changed[position as usize]),
                );
                table.set(column, apply_slots, column_runs.values.values(kind))?;
            }
            if !column_runs.patch_positions.is_empty() {
                apply_slots.clear();
                reserve(apply_slots, column_runs.patch_positions.len())?;
                apply_slots.extend(
                    column_runs
                        .patch_positions
                        .iter()
                        .map(|&position| changed[position as usize]),
                );
                table.patch(
                    column,
                    apply_slots,
                    Patches {
                        hunks: &column_runs.hunks,
                        edits: &column_runs.edits,
                        items: column_runs.deltas.patch_items(kind),
                    },
                )?;
            }
            if !column_runs.add_positions.is_empty() {
                apply_slots.clear();
                reserve(apply_slots, column_runs.add_positions.len())?;
                apply_slots.extend(
                    column_runs
                        .add_positions
                        .iter()
                        .map(|&position| changed[position as usize]),
                );
                table.add(
                    column,
                    apply_slots,
                    column_runs.deltas.values(section::base(kind)?),
                )?;
            }
        }
        for column in dirty_columns(dirty.as_slice()) {
            if !unrecorded[column].is_empty() {
                table.unrecord(column, &unrecorded[column])?;
            }
        }
        for &slot in &parts.inserted {
            if slot as usize >= extent || parts.deleted.binary_search(&slot).is_ok() {
                continue;
            }
            if free.pop() != Some(Reverse(slot)) {
                return Err(invalid_static(
                    "Inserted slots are not the lowest free ones",
                ));
            }
        }
        let freed = parts.deleted.len();
        if freed > free.capacity() - free.len() {
            engine_allocation::allocation_attempt()?;
            free.try_reserve(freed)
                .map_err(|_| engine_allocation::oom())?;
        }
        for &slot in &parts.deleted {
            if parts.inserted.binary_search(&slot).is_err() {
                free.push(Reverse(slot));
            }
        }
        Ok(())
    }

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
        let sectioned = written.is_some();
        if self.keeps {
            Last::before(&mut self.last, &self.table, evidence.status)?;
        }
        if written.is_some()
            && let Err(error) = self.apply_prepared(truncated)
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
        if self.keeps {
            Last::after(
                &mut self.last,
                evidence.status,
                sectioned,
                self.acquisition.last_read,
            );
        }
        self.status = evidence.status;
        self.view.clear();
        if truncated {
            let live = self.table.live();
            let reserved = engine_allocation::reserve(
                &mut self.view,
                self.parts.read.len() + self.parts.inserted.len(),
            );
            if let Err(error) = reserved {
                self.poisoned = true;
                return Err(error);
            }
            self.view.extend(
                self.parts
                    .read
                    .iter()
                    .copied()
                    .filter(|&slot| live.get(slot)),
            );
            self.view.extend_from_slice(&self.parts.inserted);
            self.view.sort_unstable();
            self.view.dedup();
        }
        if order {
            if let Err(error) = self.hold_order() {
                self.poisoned = true;
                return Err(error);
            }
        } else if written.is_some()
            && !(self.parts.inserted.is_empty() && self.parts.deleted.is_empty())
        {
            self.order.held = false;
        }
        if let Some(bytes) = written {
            self.parts.section = bytes;
        }
        if sectioned
            || !self.visited.is_empty()
            || !self.slots.is_empty()
            || !self.new_keys.is_empty()
        {
            self.release_scratch();
        }
        Ok(())
    }

    pub fn drop_scratch(&mut self) {
        if self.finished != self.generation {
            return;
        }
        self.columns = None;
        self.dirty.fill(0);
        self.seen = Vec::new();
        self.visited = Vec::new();
        self.touched = Vec::new();
        self.removed = Vec::new();
        self.new_keys = Vec::new();
        self.slots = Vec::new();
        self.insert_rows = Vec::new();
        self.cursors = Vec::new();
        self.parts.drop_all();
    }

    fn release_scratch(&mut self) {
        if self.visited.capacity().saturating_mul(size_of::<u32>()) > HOUR_SCRATCH_LIMIT {
            for &slot in &self.visited {
                self.seen[slot as usize / 64] &= !(1 << (slot % 64));
            }
            self.visited = Vec::new();
        }
        drop_over(&mut self.touched, HOUR_SCRATCH_LIMIT);
        drop_over(&mut self.removed, INSERT_SCRATCH_LIMIT);
        drop_over(&mut self.new_keys, INSERT_SCRATCH_LIMIT);
        drop_over(&mut self.slots, HOUR_SCRATCH_LIMIT);
        drop_over(&mut self.insert_rows, INSERT_SCRATCH_LIMIT);
        drop_over(&mut self.cursors, INSERT_SCRATCH_LIMIT);
        if let Some(scratch) = self.columns.as_deref_mut() {
            if !self.parts.inserted.is_empty() {
                for column in 0..self.kinds.len() {
                    scratch.inserts[column].release_scratch(INSERT_SCRATCH_LIMIT);
                    scratch.insert_outs[column].release_scratch(INSERT_SCRATCH_LIMIT);
                    drop_over(&mut scratch.insert_unrecorded[column], INSERT_SCRATCH_LIMIT);
                }
            }
            for column in dirty_columns(self.dirty.as_slice()) {
                scratch.updates[column].release_scratch(HOUR_SCRATCH_LIMIT);
                scratch.runs[column].release_scratch(HOUR_SCRATCH_LIMIT);
                drop_over(&mut scratch.unrecord[column], HOUR_SCRATCH_LIMIT);
                drop_over(&mut scratch.noops[column], HOUR_SCRATCH_LIMIT);
                drop_over(&mut scratch.unrecorded[column], HOUR_SCRATCH_LIMIT);
            }
        }
        self.parts.release_scratch();
    }
}

pub struct Hour<'w> {
    writer: &'w mut TableWriter,
    dictionary: Strings<'w>,
    coverage: Coverage<'w>,
    inserted: u32,
    failed: bool,
    ordered: bool,
}

#[derive(Debug)]
pub struct Prepared {
    identity: u64,
    generation: u64,
    partial: bool,
    order: bool,
    pub section: Option<Vec<u8>>,
    pub columns: Vec<u64>,
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
                            let scratch = self
                                .writer
                                .columns
                                .as_deref_mut()
                                .expect("column scratch prepared");
                            push(&mut scratch.unrecord[column], slot)?;
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
                                let scratch = self
                                    .writer
                                    .columns
                                    .as_deref_mut()
                                    .expect("column scratch prepared");
                                scratch.updates[column].change(slot, cell, held)?;
                            }
                        }
                    }
                }
            }
            None => {
                let index = self.inserted;
                push(&mut self.writer.new_keys, key)?;
                mark_all_dirty(&mut self.writer.dirty, cells.len());
                for (column, &input) in cells.iter().enumerate() {
                    let resolved = self.resolve(column, input)?;
                    let kind = self.writer.kinds[column];
                    let scratch = self
                        .writer
                        .columns
                        .as_deref_mut()
                        .expect("column scratch prepared");
                    let cell = match resolved {
                        Some(cell) => cell,
                        None => {
                            push(&mut scratch.insert_unrecorded[column], index)?;
                            placeholder(kind)
                        }
                    };
                    scratch.inserts[column].push(index, cell)?;
                }
                self.inserted = index
                    .checked_add(1)
                    .ok_or_else(|| invalid_static("An hour inserts more than 2^32 rows"))?;
            }
        }
        Ok(())
    }

    fn stage(&mut self, batch: &Batch) -> io::Result<()> {
        let complete = matches!(self.coverage, Coverage::Complete);
        let dictionary = &mut self.dictionary;
        let TableWriter {
            kinds,
            table,
            seen,
            visited,
            columns: scratch,
            removed,
            new_keys,
            texts,
            slots,
            insert_rows,
            cursors,
            order,
            dirty,
            uniform_optional_i32,
            ..
        } = &mut *self.writer;
        let ColumnScratch {
            updates,
            unrecord,
            inserts,
            insert_unrecorded,
            ..
        } = scratch.as_deref_mut().expect("column scratch prepared");
        let rows = batch.keys.len();
        if batch.kinds.as_slice() != &**kinds {
            return Err(invalid_static("A batch's columns differ from its table's"));
        }
        if let Some((column, value)) = batch.uniform_optional_i32 {
            if matches!(self.coverage, Coverage::Partial) {
                return Err(invalid_static("A partly read table cannot take a uniform fill"));
            }
            if kinds.get(column).copied() != Some(Kind::OptionalI32) {
                return Err(invalid_static("A uniform fill targets another column kind"));
            }
            *uniform_optional_i32 = Some((column, value));
            mark_dirty(dirty, column);
        }
        if u32::try_from(rows).is_err() {
            return Err(invalid_static("A batch holds more than 2^32 rows"));
        }
        let columns = &batch.columns[..kinds.len()];
        if batch.sparse {
            if rows > 0 && columns.iter().any(|cells| cells.words.len() != cells.rows.len()) {
                return Err(invalid_static(
                    "A batch row carries other than one cell per column",
                ));
            }
        } else if rows > 0 && columns.iter().any(|cells| cells.words.len() != rows) {
            return Err(invalid_static(
                "A batch row carries other than one cell per column",
            ));
        }
        slots.clear();
        insert_rows.clear();
        if complete
            && order.held
            && order.len == rows
            && if order.identity {
                table.keys_match_prefix(&batch.keys)
            } else {
                order
                    .slots
                    .iter()
                    .zip(&batch.keys)
                    .all(|(&slot, &key)| table.key(slot) == Some(key))
            }
        {
            if order.identity {
                reserve(slots, rows)?;
                slots.extend(0..rows as u32);
            } else {
                extend(slots, &order.slots)?;
            }
            self.ordered = true;
        } else if table.slots() == 0 && new_keys.is_empty() {
            if !removed.is_empty() {
                for &key in &batch.keys {
                    if removed.binary_search(&key).is_ok() {
                        return Err(invalid_static("A row key is both changed and removed"));
                    }
                }
            }
            extend(new_keys, &batch.keys)?;
            reserve(insert_rows, rows)?;
            insert_rows.extend(0..rows as u32);
            reserve(slots, rows)?;
            slots.resize(rows, NEW);
            self.inserted = insert_rows.len() as u32;
        } else {
            reserve(slots, rows)?;
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
        let has_new = !insert_rows.is_empty();
        if has_new {
            mark_all_dirty(dirty, kinds.len());
        }
        let slots = slots.as_slice();
        if batch.sparse {
            for (column, (&kind, cells)) in kinds.iter().zip(columns).enumerate() {
                if !has_new && cells.rows.is_empty() {
                    continue;
                }
                if has_new && cells.rows.len() < insert_rows.len() {
                    return Err(kept_insert());
                }
                if kind == Kind::Bytes {
                    let mut next_unrecorded = 0usize;
                    let mut next_insert = 0usize;
                    for (entry, &row) in (0u32..).zip(&cells.rows) {
                        let recorded = cells.unrecorded.get(next_unrecorded) != Some(&entry);
                        if !recorded {
                            next_unrecorded += 1;
                        }
                        let slot = slots[row as usize];
                        let text = &cells.bytes[cells.span(entry as usize)];
                        if slot == NEW {
                            if insert_rows.get(next_insert) != Some(&row) {
                                return Err(kept_insert());
                            }
                            let index = next_insert as u32;
                            next_insert += 1;
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
                    if next_insert != insert_rows.len() {
                        return Err(kept_insert());
                    }
                    continue;
                }
                let skip_existing = batch
                    .uniform_optional_i32
                    .is_some_and(|(col, _)| col == column);
                if !skip_existing {
                    let held = table
                        .column(column)
                        .ok_or_else(|| invalid_static("A batch names a column past the table"))?;
                    let marked = table.unrecorded_bits(column).filter(|bits| {
                        let words = bits.words();
                        words.len() > cells.rows.len() || words.iter().any(|&word| word != 0)
                    });
                    let mut staged = Staged {
                        slots,
                        cells,
                        marked,
                        pending: &mut updates[column],
                        unrecord: &mut unrecord[column],
                        dirty: false,
                        has_new,
                    };
                    let compared = match held {
                        Column::U8(values) => staged.sparse_scalars(values),
                        Column::I8(values) => staged.sparse_scalars(values),
                        Column::U16(values) => staged.sparse_scalars(values),
                        Column::I16(values) => staged.sparse_scalars(values),
                        Column::U32(values) => staged.sparse_scalars(values),
                        Column::I32(values) => staged.sparse_scalars(values),
                        Column::U64(values) => staged.sparse_scalars(values),
                        Column::I64(values) => staged.sparse_scalars(values),
                        Column::Bool(values) => staged.sparse_bools(values),
                        Column::OptionalI32(values, presence) => {
                            staged.sparse_optionals(values, presence)
                        }
                        Column::OptionalU32(values, presence) => {
                            staged.sparse_optionals(values, presence)
                        }
                        Column::OptionalI64(values, presence) => {
                            staged.sparse_optionals(values, presence)
                        }
                        Column::OptionalU64(values, presence) => {
                            staged.sparse_optionals(values, presence)
                        }
                        Column::U32List(values) => staged.sparse_lists(values, &cells.u32s),
                        Column::U64List(values) => staged.sparse_lists(values, &cells.u64s),
                        Column::Opaque(values) => staged.sparse_lists(values, &cells.bytes),
                        Column::Bytes(_) => Ok(()),
                    };
                    if staged.dirty {
                        mark_dirty(dirty, column);
                    }
                    compared?;
                }
                if has_new {
                    let mut entry = 0usize;
                    for (index, &ins_row) in (0u32..).zip(insert_rows.iter()) {
                        while cells.rows.get(entry).is_some_and(|&r| r < ins_row) {
                            entry += 1;
                        }
                        if cells.rows.get(entry) != Some(&ins_row) {
                            return Err(kept_insert());
                        }
                        let cell = match cells.cell(kind, entry as u32) {
                            Some(cell) => cell,
                            None => {
                                push(&mut insert_unrecorded[column], index)?;
                                placeholder(kind)
                            }
                        };
                        inserts[column].push(index, cell)?;
                        entry += 1;
                    }
                }
            }
            return Ok(());
        }
        let all_new = insert_rows.len() == rows;
        for (column, (&kind, cells)) in kinds.iter().zip(columns).enumerate() {
            if kind == Kind::Bytes || (!has_new && cells.kept.len() == rows) {
                continue;
            }
            if all_new {
                if !cells.kept.is_empty() {
                    return Err(kept_insert());
                }
                if cells.unrecorded.is_empty()
                    && matches!(
                        kind,
                        Kind::U8
                            | Kind::I8
                            | Kind::U16
                            | Kind::I16
                            | Kind::U32
                            | Kind::I32
                            | Kind::U64
                            | Kind::I64
                            | Kind::Bool
                    )
                {
                    inserts[column].extend_scalar_words(&cells.words)?;
                    continue;
                }
                let mut next_unrecorded = 0usize;
                for row in 0..rows as u32 {
                    let recorded = cells.unrecorded.get(next_unrecorded) != Some(&row);
                    if !recorded {
                        next_unrecorded += 1;
                    }
                    let cell = if recorded {
                        cells.cell(kind, row).unwrap_or_else(|| placeholder(kind))
                    } else {
                        push(&mut insert_unrecorded[column], row)?;
                        placeholder(kind)
                    };
                    inserts[column].push(row, cell)?;
                }
                continue;
            }
            let held = table
                .column(column)
                .ok_or_else(|| invalid_static("A batch names a column past the table"))?;
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
                has_new,
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
                Column::Bool(values) => staged.bools(values),
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
        if texts.is_empty()
            || (!has_new && texts.iter().all(|&column| columns[column].kept.len() == rows))
        {
            return Ok(());
        }
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
        let scratch = self
            .writer
            .columns
            .as_deref_mut()
            .expect("column scratch prepared");
        push(&mut scratch.noops[column], slot)
    }

    pub fn touch(&mut self, key: u128) -> io::Result<()> {
        self.staged(|hour| hour.stage_touch(key))
    }

    fn stage_touch(&mut self, key: u128) -> io::Result<()> {
        let slot = self
            .held_slot(key)
            .ok_or_else(|| invalid_static("Touch names a row key that is not held"))?;
        push(&mut self.writer.touched, slot)
    }

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
        let is_seen =
            |seen: &[u64], slot: u32| ordered || seen[slot as usize / 64] >> (slot % 64) & 1 != 0;
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
            runs: run_list,
            inserts: insert_list,
            unrecorded_runs,
            columns: spare_columns,
            section: spare_section,
            ..
        } = &mut writer.parts;
        let scratch = writer
            .columns
            .as_deref_mut()
            .expect("column scratch prepared");
        deleted.clear();
        match coverage {
            Coverage::Complete if !ordered => {
                for slot in writer.table.live().ones() {
                    if !is_seen(&writer.seen, slot) {
                        push(deleted, slot)?;
                    }
                }
            }
            Coverage::Complete | Coverage::Partial => {}
            Coverage::Changes { .. } => {
                reserve(deleted, writer.removed.len())?;
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
        read.clear();
        if partial {
            extend(read, &writer.visited)?;
            read.sort_unstable();
        }
        let extent = u32::try_from(writer.table.slots())
            .map_err(|_| invalid_static("Row slots exceed 32 bits"))?;
        let new_len = writer.new_keys.len();
        let ascending_new = writer.new_keys.windows(2).all(|pair| pair[0] < pair[1]);
        if ascending_new
            && new_len == inserted as usize
            && writer.free.is_empty()
            && deleted.is_empty()
        {
            if extent
                .checked_add(u32::try_from(new_len).unwrap_or(u32::MAX))
                .is_none()
            {
                return Err(invalid_static("Row slots exceed 32 bits"));
            }
            by_index.clear();
            reserve(by_index, new_len)?;
            by_index.extend(extent..extent + new_len as u32);
            inserted_slots.clear();
            reserve(inserted_slots, new_len)?;
            inserted_slots.extend(extent..extent + new_len as u32);
            keys.clear();
            extend(keys, &writer.new_keys)?;
            insert_order.clear();
            reserve(insert_order, new_len)?;
            insert_order.extend(0..new_len as u32);
        } else {
            sorted_keys.clear();
            reserve(sorted_keys, new_len)?;
            sorted_keys.extend(
                writer
                    .new_keys
                    .iter()
                    .enumerate()
                    .map(|(index, &key)| (key, index as u32)),
            );
            if !ascending_new {
                sorted_keys.sort_unstable();
                if sorted_keys.windows(2).any(|pair| pair[0].0 == pair[1].0) {
                    return Err(invalid_static("An hour inserts a row key twice"));
                }
            }
            let fresh = sorted_keys
                .len()
                .saturating_sub(writer.free.len() + deleted.len());
            if extent
                .checked_add(u32::try_from(fresh).unwrap_or(u32::MAX))
                .is_none()
            {
                return Err(invalid_static("Row slots exceed 32 bits"));
            }
            by_index.clear();
            reserve(by_index, inserted as usize)?;
            by_index.resize(inserted as usize, 0);
            by_slot.clear();
            reserve(by_slot, sorted_keys.len())?;
            popped.clear();
            reserve(popped, sorted_keys.len())?;
            {
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
            reserve(inserted_slots, by_slot.len())?;
            inserted_slots.extend(by_slot.iter().map(|entry| entry.0));
            keys.clear();
            reserve(keys, by_slot.len())?;
            keys.extend(by_slot.iter().map(|entry| entry.1));
            insert_order.clear();
            reserve(insert_order, by_slot.len())?;
            insert_order.extend(by_slot.iter().map(|entry| entry.2));
        }
        let mut live_after = None;
        let mut direct_fill = None;
        if let Some((col, value)) = writer.uniform_optional_i32 {
            let live = *live_after.get_or_insert_with(|| {
                writer.table.live().count() - deleted.len() + inserted_slots.len()
            });
            let runs = &mut scratch.runs[col];
            runs.clear();
            if live > 0 {
                let Some(Column::OptionalI32(held_vals, held_presence)) = writer.table.column(col)
                else {
                    return Err(invalid_static("A uniform fill targets another column kind"));
                };
                let marked = writer.table.unrecorded_bits(col).filter(|bits| {
                    bits.words().iter().any(|&word| word != 0)
                });
                let mut del = deleted.iter().copied().peekable();
                let mut held = writer
                    .table
                    .live()
                    .ones()
                    .filter(|&slot| {
                        while del.next_if(|&gone| gone < slot).is_some() {}
                        del.next_if_eq(&slot).is_none()
                    })
                    .peekable();
                let mut ins = inserted_slots.iter().copied().peekable();
                let mut changing = 0usize;
                loop {
                    match (held.peek().copied(), ins.peek().copied()) {
                        (Some(kept), Some(new)) if new < kept => {
                            ins.next();
                            push(&mut runs.except, new)?;
                        }
                        (Some(kept), _) => {
                            held.next();
                            let was = !marked.is_some_and(|bits| bits.get(kept));
                            let old = held_presence.get(kept).then(|| {
                                *held_vals
                                    .get(kept as usize)
                                    .expect("live slot is within column")
                            });
                            if was && old == value {
                                push(&mut runs.except, kept)?;
                            } else {
                                changing += 1;
                            }
                        }
                        (None, Some(new)) => {
                            ins.next();
                            push(&mut runs.except, new)?;
                        }
                        (None, None) => break,
                    }
                }
                if changing == 0 {
                    runs.except.clear();
                } else if changing >= FILL_MINIMUM && changing * 2 >= live {
                    runs.fill = true;
                    direct_fill = Some((col, value, changing));
                } else {
                    runs.except.clear();
                    let mut del = deleted.iter().copied().peekable();
                    for kept in writer.table.live().ones() {
                        while del.next_if(|&gone| gone < kept).is_some() {}
                        if del.next_if_eq(&kept).is_some() {
                            continue;
                        }
                        let was = !marked.is_some_and(|bits| bits.get(kept));
                        let old = held_presence.get(kept).then(|| {
                            *held_vals
                                .get(kept as usize)
                                .expect("live slot is within column")
                        });
                        if was && old == value {
                            continue;
                        }
                        if writer.seen[kept as usize / 64] >> (kept % 64) & 1 == 0 {
                            see(&mut writer.seen, &mut writer.visited, kept)?;
                        }
                        scratch.updates[col].change(
                            kept,
                            Cell::OptionalI32(value),
                            was.then_some(Cell::OptionalI32(old)),
                        )?;
                    }
                }
            }
        }
        for column in dirty_columns(&writer.dirty) {
            if direct_fill.is_some_and(|(col, _, _)| col == column) {
                continue;
            }
            let (pending, runs) = (&scratch.updates[column], &mut scratch.runs[column]);
            let noops = &mut scratch.noops[column];
            runs.clear();
            if pending.len() == 0 && noops.is_empty() {
                continue;
            }
            reserve(&mut runs.order, pending.len())?;
            runs.order.extend(0..pending.len() as u32);
            if !pending.at.windows(2).all(|pair| pair[0] < pair[1]) {
                runs.order
                    .sort_unstable_by_key(|&entry| pending.at[entry as usize]);
            }
            let slot_of = |entry: &u32| pending.at[*entry as usize];
            if !noops.is_empty() {
                noops.sort_unstable();
                noops.dedup();
                noops.retain(|slot| runs.order.binary_search_by_key(slot, slot_of).is_err());
            }
            if pending.uniform && pending.len() >= FILL_MINIMUM {
                let live = *live_after.get_or_insert_with(|| {
                    writer.table.live().count() - deleted.len() + inserted_slots.len()
                });
                if pending.len() * 2 >= live {
                    runs.fill = true;
                    reserve(&mut runs.except, live - pending.len())?;
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
        changed.clear();
        for column in dirty_columns(&writer.dirty) {
            let set = if scratch.runs[column].fill {
                &[][..]
            } else {
                &scratch.updates[column].at[..]
            };
            reserve(
                changed,
                set.len() + scratch.unrecord[column].len() + scratch.noops[column].len(),
            )?;
            changed.extend_from_slice(set);
            changed.extend_from_slice(&scratch.unrecord[column]);
            changed.extend_from_slice(&scratch.noops[column]);
        }
        if !changed.windows(2).all(|pair| pair[0] < pair[1]) {
            changed.sort_unstable();
            changed.dedup();
        }
        if changed.iter().any(|&slot| !is_seen(&writer.seen, slot)) {
            return Err(invalid_static(
                "A row has a NOOP but was not visited this hour",
            ));
        }
        if writer
            .touched
            .iter()
            .any(|&slot| !is_seen(&writer.seen, slot))
        {
            return Err(invalid_static(
                "A row is touched but was not visited this hour",
            ));
        }
        if partial && !deleted.is_empty() {
            return Err(invalid_static("A partial read deletes no row"));
        }
        if !writer.touched.is_empty() {
            writer.touched.sort_unstable();
            writer.touched.dedup();
        }
        let changed = changed.as_slice();
        let next_position = |pos: &mut usize, slot: u32| -> u32 {
            if changed.get(*pos) == Some(&slot) {
                let found = *pos as u32;
                *pos += 1;
                return found;
            }
            let mut step = 1usize;
            while *pos + step < changed.len() && changed[*pos + step] < slot {
                *pos += step;
                step <<= 1;
            }
            let end = (*pos + step + 1).min(changed.len());
            *pos += changed[*pos..end]
                .binary_search(&slot)
                .expect("changed slot");
            let found = *pos as u32;
            *pos += 1;
            found
        };
        let mut census = Census::default();
        for column in dirty_columns(&writer.dirty) {
            let kind = writer.kinds[column];
            let (pending, runs) = (&scratch.updates[column], &mut scratch.runs[column]);
            if runs.fill {
                if let Some((_, value, changing)) =
                    direct_fill.filter(|&(col, _, _)| col == column)
                {
                    runs.values.push_optional_i32(value)?;
                    census.fills += 1;
                    census.fill_slots += changing as u64;
                    census.fill_exceptions += runs.except.len() as u64;
                } else {
                    runs.values.push(kind, pending, &pending.words, 0)?;
                    census.fills += 1;
                    census.fill_slots += pending.len() as u64;
                    census.fill_exceptions += runs.except.len() as u64;
                }
            } else {
                let uniform = pending.uniform && pending.len() > 1;
                let mut pos = 0usize;
                for &entry in &runs.order {
                    let (entry, at) = (
                        entry as usize,
                        next_position(&mut pos, pending.at[entry as usize]),
                    );
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
            let noops = &scratch.noops[column];
            if !noops.is_empty() {
                reserve(&mut runs.noop_positions, noops.len())?;
                let mut noop_pos = 0usize;
                runs.noop_positions
                    .extend(noops.iter().map(|&slot| next_position(&mut noop_pos, slot)));
            }
            let order = insert_order.as_slice();
            scratch.insert_outs[column].fill(kind, &scratch.inserts[column], order)?;
        }
        let mut runs: Vec<Run<'_>> = recycle(std::mem::take(run_list));
        for column in dirty_columns(&writer.dirty) {
            let kind = writer.kinds[column];
            let column_runs = &scratch.runs[column];
            reserve(&mut runs, 5)?;
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
            reserve(&mut inserts, columns)?;
            inserts.extend(
                (0..columns).map(|column| scratch.insert_outs[column].values(writer.kinds[column])),
            );
        }
        for column in dirty_columns(&writer.dirty) {
            let slots = &mut scratch.unrecorded[column];
            slots.clear();
            let needed = scratch.unrecord[column].len() + scratch.insert_unrecorded[column].len();
            if needed == 0 {
                continue;
            }
            reserve(slots, needed)?;
            slots.extend_from_slice(&scratch.unrecord[column]);
            slots.extend(
                scratch.insert_unrecorded[column]
                    .iter()
                    .map(|&index| by_index[index as usize]),
            );
            slots.sort_unstable();
        }
        let unrecorded_slots = &scratch.unrecorded;
        let mut unrecorded: Vec<(usize, &[u32])> = recycle(std::mem::take(unrecorded_runs));
        let marked = dirty_columns(&writer.dirty)
            .filter(|&column| !unrecorded_slots[column].is_empty())
            .count();
        reserve(&mut unrecorded, marked)?;
        for column in dirty_columns(&writer.dirty) {
            if !unrecorded_slots[column].is_empty() {
                unrecorded.push((column, unrecorded_slots[column].as_slice()));
            }
        }
        let mut column_bits = std::mem::take(spare_columns);
        column_bits.clear();
        reserve(&mut column_bits, columns.div_ceil(64))?;
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
