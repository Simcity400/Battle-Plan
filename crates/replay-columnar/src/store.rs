//! The recorder's live state: one copy of each table's current rows, as plain arrays by slot at
//! the narrowest width each column's values need. Every change goes in through one entry point
//! ([`TableChanges`]), which reads the held cell, writes the new one and notes it in the hour's
//! log ([`HourLog`]); nothing else holds the run's values.
//!
//! A row keeps its slot from insert to delete. A number column starts at width 0 (every cell
//! zero) and widens in place the first time a value does not fit. Optional columns keep a
//! presence bit per slot; a column's unrecorded bits are made the first time one of its cells is
//! unrecorded. List and opaque cells are bytes, a page of [`PAGE`] slots in one buffer. Keys are
//! two narrow arrays (their high and low 64 bits); the key index is built the first time a table
//! is looked up by key.

use crate::{
    invalid,
    manifest::Status,
    state::{Cell, Kind},
};
use std::{
    collections::{BTreeMap, HashMap},
    io,
};

pub mod journal;
pub mod lent;
#[cfg(test)]
mod tests;

/// Slots a list page holds.
pub const PAGE: usize = 1024;

/// A table's rows at most: its slot count is recorded in 32 bits, so its highest slot is one less.
pub const SLOTS: u32 = u32::MAX;

/// A run's strings at most: their count is recorded in 32 bits, as a table's slots are.
pub const STRINGS: u32 = u32::MAX;

/// The refusal of a row past [`SLOTS`] in `table`.
pub fn past_slots(table: impl std::fmt::Display) -> io::Error {
    invalid(format!("{table}: more than 4,294,967,295 rows, the format's slot width"))
}

/// The slot of bit `bit` of live word `word`; none past [`SLOTS`].
fn slot_at(word: usize, bit: u32) -> Option<u32> {
    word.checked_mul(64).and_then(|first| first.checked_add(bit as usize)).and_then(|slot| u32::try_from(slot).ok()).filter(|&slot| slot < SLOTS)
}

/// Bits by slot.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Flags(Vec<u64>);

impl Flags {
    #[inline]
    pub fn get(&self, slot: u32) -> bool {
        self.0.get(slot as usize / 64).is_some_and(|word| word >> (slot % 64) & 1 == 1)
    }

    #[inline]
    pub(crate) fn set(&mut self, slot: u32) {
        let word = slot as usize / 64;
        if word >= self.0.len() {
            reserve_tight(&mut self.0, word + 1, "store: bits grown");
            self.0.resize(word + 1, 0);
        }
        self.0[word] |= 1 << (slot % 64);
    }

    #[inline]
    pub(crate) fn clear(&mut self, slot: u32) {
        if let Some(word) = self.0.get_mut(slot as usize / 64) {
            *word &= !(1 << (slot % 64));
        }
    }

    #[inline]
    pub(crate) fn put(&mut self, slot: u32, value: bool) {
        if value { self.set(slot) } else { self.clear(slot) }
    }

    pub fn words(&self) -> &[u64] {
        &self.0
    }

    /// Whether every slot of `from..to` is set.
    fn all(&self, from: u32, to: u32) -> bool {
        (from..to).step_by(64).all(|start| {
            let index = start as usize / 64;
            let base = index as u32 * 64;
            let mut wanted = u64::MAX << (start - base);
            if to - base < 64 {
                wanted &= (1_u64 << (to - base)) - 1;
            }
            self.0.get(index).is_some_and(|word| word & wanted == wanted)
        })
    }

    /// Clears slots `from..to`.
    fn clear_range(&mut self, from: u32, to: u32) {
        let last = (to as usize).div_ceil(64).min(self.0.len());
        for index in from as usize / 64..last {
            let base = index as u32 * 64;
            let mut cleared = u64::MAX;
            if base < from {
                cleared &= u64::MAX << (from - base);
            }
            if to - base < 64 {
                cleared &= (1_u64 << (to - base)) - 1;
            }
            self.0[index] &= !cleared;
        }
    }

    /// Sets every bit `other` sets in slots `from..to`.
    pub(crate) fn include(&mut self, other: &Self, from: u32, to: u32) {
        let last = (to as usize).div_ceil(64).min(other.0.len());
        if last > self.0.len() {
            reserve_tight(&mut self.0, last, "store: bits grown");
            self.0.resize(last, 0);
        }
        for index in from as usize / 64..last {
            let base = index as u32 * 64;
            let mut word = other.0[index];
            if base < from {
                word &= u64::MAX << (from - base);
            }
            if to - base < 64 {
                word &= (1_u64 << (to - base)) - 1;
            }
            self.0[index] |= word;
        }
    }

    pub fn ones(&self) -> impl Iterator<Item = u32> + '_ {
        self.0.iter().enumerate().flat_map(|(index, &word)| {
            let mut rest = word;
            std::iter::from_fn(move || {
                (rest != 0).then(|| {
                    let bit = rest.trailing_zeros();
                    rest &= rest - 1;
                    index as u32 * 64 + bit
                })
            })
        })
    }

    pub fn count(&self) -> usize {
        self.0.iter().map(|word| word.count_ones() as usize).sum()
    }

    fn bytes(&self) -> usize {
        self.0.capacity() * 8
    }

    /// Lets go of bits from slot `slots` on. Bits are held only through the highest one set, so
    /// the boundary word is masked only when it is held.
    fn truncate(&mut self, slots: usize) {
        let words = slots.div_ceil(64);
        if self.0.len() >= words {
            self.0.truncate(words);
            shrink_loose(&mut self.0);
            if slots % 64 != 0
                && let Some(last) = self.0.last_mut()
            {
                *last &= (1 << (slots % 64)) - 1;
            }
        }
    }

    /// Held only through the highest bit set, as bits set one at a time are.
    fn trim_zeros(&mut self) {
        let words = self.0.iter().rposition(|&word| word != 0).map_or(0, |last| last + 1);
        self.0.truncate(words);
        shrink_loose(&mut self.0);
    }
}

/// A log's rows the game still lists: a bit per ordinal from `base` (a multiple of 64) on, the
/// words before the lowest one set and after the highest let go of, `base` 0 when none is listed.
/// So two hold the same ordinals exactly when they are equal, however each got there.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Listed {
    base: u64,
    words: std::collections::VecDeque<u64>,
}

impl Listed {
    #[inline]
    pub fn get(&self, ordinal: u64) -> bool {
        ordinal >= self.base && usize::try_from((ordinal - self.base) / 64).ok().and_then(|word| self.words.get(word)).is_some_and(|word| word >> (ordinal % 64) & 1 == 1)
    }

    /// Listed from `ordinal` on, which is past every one held.
    pub fn append(&mut self, ordinal: u64) {
        if self.words.is_empty() {
            self.base = ordinal / 64 * 64;
        }
        assert!(ordinal >= self.base, "a log's rows are listed in ordinal order");
        let word = ((ordinal - self.base) / 64) as usize;
        if word >= self.words.len() {
            self.words.resize(word + 1, 0);
        }
        self.words[word] |= 1 << (ordinal % 64);
    }

    /// One past the last ordinal its words hold: `u64::MAX` past it, which no ordinal reaches.
    fn end(&self) -> u64 {
        self.base.saturating_add(self.words.len() as u64 * 64)
    }

    pub fn clear(&mut self, ordinal: u64) {
        self.clear_range(ordinal, ordinal.saturating_add(1));
    }

    /// Ordinals `from..to` no longer listed.
    pub fn clear_range(&mut self, from: u64, to: u64) {
        let (from, to) = (from.max(self.base), to.min(self.end()));
        let mut at = from;
        while at < to {
            let word = ((at - self.base) / 64) as usize;
            let start = at % 64;
            let end = (to - (at - start)).min(64);
            let mask = if end == 64 { u64::MAX << start } else { (u64::MAX << start) & ((1 << end) - 1) };
            self.words[word] &= !mask;
            at += end - start;
        }
        while self.words.front() == Some(&0) {
            self.words.pop_front();
            self.base = self.base.saturating_add(64);
        }
        while self.words.back() == Some(&0) {
            self.words.pop_back();
        }
        if self.words.is_empty() {
            self.base = 0;
        }
        if self.words.len() * 4 < self.words.capacity() && self.words.capacity() > 64 {
            self.words.shrink_to_fit();
        }
    }

    /// Whether every ordinal of `from..to` is listed: an empty range is, wherever it lies.
    pub fn all(&self, from: u64, to: u64) -> bool {
        from >= to || from >= self.base && to <= self.end() && {
            let mut at = from;
            let mut every = true;
            while at < to && every {
                let word = self.words[((at - self.base) / 64) as usize];
                let start = at % 64;
                let end = (to - (at - start)).min(64);
                let mask = if end == 64 { u64::MAX << start } else { (u64::MAX << start) & ((1 << end) - 1) };
                every = word & mask == mask;
                at += end - start;
            }
            every
        }
    }

    /// The lowest ordinal listed.
    pub fn lowest(&self) -> Option<u64> {
        self.words.front().map(|word| self.base + u64::from(word.trailing_zeros()))
    }

    pub fn ones(&self) -> impl Iterator<Item = u64> + '_ {
        self.words.iter().enumerate().flat_map(|(index, &word)| {
            let base = self.base + index as u64 * 64;
            let mut rest = word;
            std::iter::from_fn(move || {
                (rest != 0).then(|| {
                    let bit = rest.trailing_zeros();
                    rest &= rest - 1;
                    base + u64::from(bit)
                })
            })
        })
    }

    pub fn len(&self) -> usize {
        self.words.iter().map(|word| word.count_ones() as usize).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }

    pub fn bytes(&self) -> usize {
        self.words.capacity() * 8
    }
}

/// The keys a log was given in its run, or in an hour, as ranges `start..=end`: a log's keys are
/// mostly sequences, which a few ranges hold. A key given again is a row changed after it was
/// written, which a log's producer refuses by name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Given(BTreeMap<u128, u128>);

impl Given {
    pub fn holds(&self, key: u128) -> bool {
        self.0.range(..=key).next_back().is_some_and(|(_, &end)| key <= end)
    }

    /// `key`, which it does not hold.
    pub fn add(&mut self, key: u128) {
        let before = self.0.range(..=key).next_back().map(|(&start, &end)| (start, end)).filter(|&(_, end)| end.checked_add(1) == Some(key));
        let after = key.checked_add(1).and_then(|next| self.0.get(&next).map(|&end| (next, end)));
        match (before, after) {
            (Some((start, _)), Some((next, end))) => {
                self.0.remove(&next);
                self.0.insert(start, end);
            }
            (Some((start, _)), None) => {
                self.0.insert(start, key);
            }
            (None, Some((next, end))) => {
                self.0.remove(&next);
                self.0.insert(key, end);
            }
            (None, None) => {
                self.0.insert(key, key);
            }
        }
    }

    /// The ranges held, `(start, end)` inclusive, ascending.
    pub fn ranges(&self) -> impl ExactSizeIterator<Item = (u128, u128)> + '_ {
        self.0.iter().map(|(&start, &end)| (start, end))
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Range `start..=end` as [`Self::ranges`] listed it, apart from every range held: a resume
    /// state read back.
    pub fn hold_range(&mut self, start: u128, end: u128) {
        self.0.insert(start, end);
    }
}

/// Labels this thread's allocations as the store's part `name` until the scope drops, for a build
/// counting memory; nothing otherwise.
pub(crate) fn label(name: &str) -> engine_allocation::labels::Scope {
    use engine_allocation::labels;
    labels::enter(labels::owner(name), labels::Kind::Operator)
}

/// The memory label of the cells a kept table's hour log keeps ([`Store::keep_previous`]), apart
/// from the hour log's own.
pub const KEPT_LABEL: &str = "store: kept cells";

/// `value` pushed onto the hour log's `held`, labelled when it grows.
#[inline]
fn log_push<T>(held: &mut Vec<T>, value: T) {
    if held.len() == held.capacity() {
        let _label = label("hour log");
        held.push(value);
    } else {
        held.push(value);
    }
}

/// Room for `wanted` items with a sixteenth more, at least 4 KiB: arrays grown a slot at a time
/// hold that much slack, not the doubling `Vec` would.
fn reserve_tight<T>(held: &mut Vec<T>, wanted: usize, what: &str) {
    if held.capacity() < wanted {
        let _label = label(what);
        let room = wanted + (wanted / 16).max(4096 / size_of::<T>().max(1));
        held.reserve_exact(room - held.len());
    }
}

/// Gives back what passes a sixteenth more than `held` holds.
fn shrink_loose<T>(held: &mut Vec<T>) {
    let keep = held.len() + held.len() / 16;
    if held.capacity() > keep + 4096 / size_of::<T>().max(1) {
        held.shrink_to(keep);
    }
}

/// A number per slot at 0, 1, 2, 4 or 8 bytes: the narrowest every value held fits. Values are
/// their bits, sign-extended when the column is signed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Narrow {
    width: u8,
    signed: bool,
    bytes: Vec<u8>,
}

/// The narrowest width holding `bits`.
#[inline]
fn width_of(bits: u64, signed: bool) -> u8 {
    if signed {
        let value = bits as i64;
        if value == 0 {
            0
        } else if i64::from(value as i8) == value {
            1
        } else if i64::from(value as i16) == value {
            2
        } else if i64::from(value as i32) == value {
            4
        } else {
            8
        }
    } else if bits == 0 {
        0
    } else if bits <= u64::from(u8::MAX) {
        1
    } else if bits <= u64::from(u16::MAX) {
        2
    } else if bits <= u64::from(u32::MAX) {
        4
    } else {
        8
    }
}

impl Narrow {
    pub fn new(signed: bool) -> Self {
        Self { width: 0, signed, bytes: Vec::new() }
    }

    pub fn width(&self) -> u8 {
        self.width
    }

    #[inline]
    pub fn get(&self, slot: u32) -> u64 {
        let at = slot as usize;
        let bytes = &self.bytes;
        match self.width {
            0 => 0,
            1 => {
                let byte = bytes.get(at).copied().unwrap_or(0);
                if self.signed { byte as i8 as i64 as u64 } else { u64::from(byte) }
            }
            2 => {
                let Some(held) = bytes.get(at * 2..at * 2 + 2) else { return 0 };
                let value = u16::from_le_bytes([held[0], held[1]]);
                if self.signed { value as i16 as i64 as u64 } else { u64::from(value) }
            }
            4 => {
                let Some(held) = bytes.get(at * 4..at * 4 + 4) else { return 0 };
                let value = u32::from_le_bytes(held.try_into().expect("four bytes"));
                if self.signed { value as i32 as i64 as u64 } else { u64::from(value) }
            }
            _ => {
                let Some(held) = bytes.get(at * 8..at * 8 + 8) else { return 0 };
                u64::from_le_bytes(held.try_into().expect("eight bytes"))
            }
        }
    }

    /// Holds `slots` slots: new ones zero.
    pub fn grow(&mut self, slots: usize) {
        let wanted = slots * self.width as usize;
        if self.bytes.len() < wanted {
            reserve_tight(&mut self.bytes, wanted, "store: cells grown");
            self.bytes.resize(wanted, 0);
        }
    }

    fn widen(&mut self, width: u8, slots: usize) {
        let _label = label("store: cells widened");
        let old = std::mem::replace(self, Self { width, signed: self.signed, bytes: vec![0; slots * width as usize] });
        let count = if old.width == 0 { 0 } else { old.bytes.len() / old.width as usize };
        for slot in 0..count as u32 {
            let bits = old.get(slot);
            if bits != 0 {
                self.store(slot, bits);
            }
        }
    }

    #[inline]
    fn store(&mut self, slot: u32, bits: u64) {
        let at = slot as usize;
        match self.width {
            0 => {}
            1 => self.bytes[at] = bits as u8,
            2 => self.bytes[at * 2..at * 2 + 2].copy_from_slice(&(bits as u16).to_le_bytes()),
            4 => self.bytes[at * 4..at * 4 + 4].copy_from_slice(&(bits as u32).to_le_bytes()),
            _ => self.bytes[at * 8..at * 8 + 8].copy_from_slice(&bits.to_le_bytes()),
        }
    }

    /// Sets slot `slot` (below `slots`) to `bits`, widening the column when they do not fit.
    #[inline]
    pub fn set(&mut self, slot: u32, bits: u64, slots: usize) {
        let needed = width_of(bits, self.signed);
        if needed > self.width {
            self.widen(needed, slots);
        } else {
            self.grow(slots);
        }
        self.store(slot, bits);
    }

    /// The narrowest width its first `slots` values need, when it is narrower than it is.
    pub fn narrow(&mut self, slots: usize) {
        let needed = (0..slots as u32).map(|slot| width_of(self.get(slot), self.signed)).max().unwrap_or(0);
        if needed < self.width {
            let mut narrowed = Self { width: needed, signed: self.signed, bytes: vec![0; slots * needed as usize] };
            for slot in 0..slots as u32 {
                narrowed.store(slot, self.get(slot));
            }
            *self = narrowed;
        }
    }

    pub fn signed(&self) -> bool {
        self.signed
    }

    /// Slots `from..to` zero, as far as it holds them.
    fn zero_range(&mut self, from: u32, to: u32) {
        let width = self.width as usize;
        let end = (to as usize * width).min(self.bytes.len());
        let start = (from as usize * width).min(end);
        self.bytes[start..end].fill(0);
    }

    /// Its first `slots` cells as bytes at its width, for a reader's loop to read and write in
    /// place; [`Self::hold`] widens it when a value does not fit.
    pub(crate) fn cells_mut(&mut self, slots: usize) -> &mut [u8] {
        self.grow(slots);
        &mut self.bytes[..slots * self.width as usize]
    }

    /// Widened, when it must be, so that `bits` fits at any of its first `slots` slots.
    pub(crate) fn hold(&mut self, bits: u64, slots: usize) {
        let needed = width_of(bits, self.signed);
        if needed > self.width {
            self.widen(needed, slots);
        }
    }

    pub fn bytes(&self) -> usize {
        self.bytes.capacity()
    }

    /// The bytes its slots use, slack apart.
    pub fn used(&self) -> usize {
        self.bytes.len()
    }

    fn truncate(&mut self, slots: usize) {
        self.bytes.truncate(slots * self.width as usize);
        shrink_loose(&mut self.bytes);
    }
}

/// How a table's list pages hold their cells: the room a cell rewritten longer takes, and the
/// slack a compaction keeps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ListPolicy {
    /// A cell rewritten longer takes this many eighths of its length more as room.
    pub room_eighths: u32,
    /// A compaction keeps this many sixteenths of the cells' bytes as slack.
    pub slack_sixteenths: u32,
}

impl ListPolicy {
    /// A reader's cells: events applied in place, so a cell rewritten longer keeps room to grow.
    pub const ROOMY: Self = Self { room_eighths: 1, slack_sixteenths: 4 };
    /// The recorder's store: one copy of the world, held at its length.
    pub const TIGHT: Self = Self { room_eighths: 0, slack_sixteenths: 1 };
}

/// A list column's cells of [`PAGE`] slots: each slot's cell is `length` bytes at `start` of
/// one of the page's segments, in `room` bytes held for it. A cell rewritten within its room
/// stays where it is; a longer one goes to the end of the last segment, or a new one past
/// [`SEGMENT`] bytes, with the room its table's [`ListPolicy`] gives when it was rewritten
/// before. A segment grows and is compacted on its own, so the page never holds two copies of
/// more than a segment: compacted in place, once the page's abandoned bytes pass a quarter of it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ListPage {
    spans: Vec<Span>,
    segments: Vec<Vec<u8>>,
    abandoned: u32,
    held: usize,
}

/// The bytes a page's segment fills before cells go to a new one; a cell longer has one alone.
const SEGMENT: usize = 1 << 20;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Span {
    segment: u32,
    start: u32,
    length: u32,
    room: u32,
}

impl ListPage {
    #[inline]
    fn get(&self, offset: usize) -> &[u8] {
        match self.spans.get(offset) {
            Some(span) if span.length > 0 => &self.segments[span.segment as usize][span.start as usize..(span.start + span.length) as usize],
            _ => &[],
        }
    }

    /// The cell at `offset` to change in place: its length stays.
    #[inline]
    fn get_mut(&mut self, offset: usize) -> &mut [u8] {
        match self.spans.get(offset) {
            Some(span) if span.length > 0 => &mut self.segments[span.segment as usize][span.start as usize..(span.start + span.length) as usize],
            _ => &mut [],
        }
    }

    fn set(&mut self, offset: usize, cell: &[u8], policy: ListPolicy) -> io::Result<()> {
        if self.spans.len() <= offset {
            self.spans.resize(offset + 1, Span::default());
        }
        let span = self.spans[offset];
        if cell.len() <= span.room as usize {
            if !cell.is_empty() {
                self.segments[span.segment as usize][span.start as usize..span.start as usize + cell.len()].copy_from_slice(cell);
            }
            self.spans[offset].length = cell.len() as u32;
            return Ok(());
        }
        let room = if span.room == 0 { cell.len() } else { cell.len() + cell.len() * policy.room_eighths as usize / 8 };
        self.abandoned += span.room;
        let fits = self.segments.last().is_some_and(|last| last.len() + room <= SEGMENT);
        if !fits {
            self.segments.push(Vec::new());
        }
        let segment = self.segments.len() - 1;
        let held = &mut self.segments[segment];
        let start = held.len();
        let end = u32::try_from(start + room).map_err(|_| invalid("A list segment passes 4 GiB"))?;
        self.spans[offset] = Span { segment: segment as u32, start: start as u32, length: cell.len() as u32, room: end - start as u32 };
        reserve_tight(held, end as usize, "store: list items grown");
        held.extend_from_slice(cell);
        held.resize(end as usize, 0);
        self.held += room;
        if self.abandoned as usize * 4 > self.held && self.held > 4096 {
            self.compact(policy);
        }
        Ok(())
    }

    /// Cells of slots that hold none yet, in slot order, each `(offset, start, length)` of `bytes`:
    /// laid one after another in segments sized to them, each cell's room its length.
    fn fill(&mut self, cells: &[(u16, u32, u32)], bytes: &[u8]) -> io::Result<()> {
        let Some(&(last, ..)) = cells.last() else { return Ok(()) };
        if self.spans.len() <= usize::from(last) {
            self.spans.resize(usize::from(last) + 1, Span::default());
        }
        let mut at = 0;
        while at < cells.len() {
            let fits = |used: usize, length: u32| used == 0 || used + length as usize <= SEGMENT;
            let mut taken = 0_usize;
            let mut end = at;
            while end < cells.len() && fits(taken, cells[end].2) {
                taken += cells[end].2 as usize;
                end += 1;
            }
            let appends = self.segments.last().is_some_and(|held| held.len() + taken <= SEGMENT);
            if !appends {
                self.segments.push(Vec::new());
            }
            let segment = self.segments.len() - 1;
            let held = &mut self.segments[segment];
            reserve_tight(held, held.len() + taken, "store: list items grown");
            for &(offset, start, length) in &cells[at..end] {
                let span = &mut self.spans[usize::from(offset)];
                if span.room != 0 || span.length != 0 {
                    return Err(invalid("A list cell is filled twice"));
                }
                let begin = u32::try_from(held.len()).map_err(|_| invalid("A list segment passes 4 GiB"))?;
                let cell = bytes.get(start as usize..(start + length) as usize).ok_or_else(|| invalid("A list cell passes its bytes"))?;
                held.extend_from_slice(cell);
                *span = Span { segment: segment as u32, start: begin, length, room: length };
            }
            self.held += taken;
            at = end;
        }
        Ok(())
    }

    /// Moves every cell down over the bytes abandoned before it in its own segment, each
    /// keeping its room (only its length under a policy giving none); gives back what passes the
    /// policy's slack in each segment, and the segments left empty.
    fn compact(&mut self, policy: ListPolicy) {
        if policy.room_eighths == 0 {
            for span in &mut self.spans {
                span.room = span.length;
            }
        }
        let mut order: Vec<u32> = (0..self.spans.len() as u32).filter(|&at| self.spans[at as usize].room != 0).collect();
        order.sort_unstable_by_key(|&at| (self.spans[at as usize].segment, self.spans[at as usize].start));
        let mut kept = vec![0_u32; self.segments.len()];
        for at in order {
            let span = self.spans[at as usize];
            let segment = span.segment as usize;
            let place = kept[segment];
            self.segments[segment].copy_within(span.start as usize..(span.start + span.length) as usize, place as usize);
            self.spans[at as usize].start = place;
            kept[segment] = place + span.room;
        }
        // Segments left empty go, the rest are renumbered in order.
        let mut renumbered = vec![u32::MAX; self.segments.len()];
        let mut next = 0_u32;
        for (segment, &used) in kept.iter().enumerate() {
            if used > 0 {
                renumbered[segment] = next;
                next += 1;
            }
        }
        for span in &mut self.spans {
            if span.room == 0 {
                *span = Span { length: span.length, ..Span::default() };
            } else {
                span.segment = renumbered[span.segment as usize];
            }
        }
        let segments = std::mem::take(&mut self.segments);
        self.held = 0;
        for (mut held, used) in segments.into_iter().zip(kept) {
            if used == 0 {
                continue;
            }
            held.truncate(used as usize);
            let keep = held.len() + held.len() * policy.slack_sixteenths as usize / 16;
            if held.capacity() > keep + 4096 {
                let _label = label("store: list items compacted");
                held.shrink_to(keep);
            }
            self.held += held.len();
            self.segments.push(held);
        }
        self.abandoned = 0;
    }

    /// Lets go of the slots from `offset` on.
    fn truncate(&mut self, offset: usize, policy: ListPolicy) {
        self.spans.truncate(offset);
        self.compact(policy);
    }

    fn holds(&self, offset: usize) -> bool {
        self.spans.len() > offset
    }

    fn bytes(&self) -> (usize, usize) {
        (
            self.spans.capacity() * size_of::<Span>() + self.segments.capacity() * size_of::<Vec<u8>>(),
            self.segments.iter().map(Vec::capacity).sum(),
        )
    }

    fn used(&self) -> usize {
        self.spans.iter().map(|span| span.length as usize).sum()
    }
}

/// One column's cells.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ColumnState {
    Number { kind: Kind, cells: Narrow, present: Option<Flags>, unrecorded: Option<Flags> },
    Bool { cells: Flags, unrecorded: Option<Flags> },
    Text { ids: Narrow, unrecorded: Option<Flags> },
    List { kind: Kind, pages: Vec<ListPage>, unrecorded: Option<Flags> },
}

/// A cell as the store holds it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Value<'a> {
    Unrecorded,
    /// An optional number without a value.
    Missing,
    /// A number's bits, sign-extended when it is signed; a bool as 0 or 1.
    Number(u64),
    Text(u32),
    /// A list's items as their little-endian bytes, or an opaque cell's bytes.
    List(&'a [u8]),
}

impl Value<'_> {
    /// A value that is no list, free of the cell it was read from.
    fn owned_scalar(self) -> Value<'static> {
        match self {
            Value::Unrecorded => Value::Unrecorded,
            Value::Missing => Value::Missing,
            Value::Number(bits) => Value::Number(bits),
            Value::Text(id) => Value::Text(id),
            Value::List(_) => Value::Unrecorded,
        }
    }
}

fn signed(kind: Kind) -> bool {
    matches!(kind, Kind::I8 | Kind::I16 | Kind::I32 | Kind::I64 | Kind::OptionalI32 | Kind::OptionalI64)
}

fn optional(kind: Kind) -> bool {
    matches!(kind, Kind::OptionalI32 | Kind::OptionalU32 | Kind::OptionalI64 | Kind::OptionalU64)
}

/// The bytes of an item of a list kind.
pub fn item_width(kind: Kind) -> usize {
    match kind {
        Kind::U32List => 4,
        Kind::U64List => 8,
        _ => 1,
    }
}

/// `value` cut to `kind`'s width and extended back: what the column holds after an ADD.
#[inline]
fn at_width(kind: Kind, bits: u64) -> u64 {
    match kind {
        Kind::U8 => bits as u8 as u64,
        Kind::I8 => bits as i8 as i64 as u64,
        Kind::U16 => bits as u16 as u64,
        Kind::I16 => bits as i16 as i64 as u64,
        Kind::U32 | Kind::OptionalU32 => bits as u32 as u64,
        Kind::I32 | Kind::OptionalI32 => bits as i32 as i64 as u64,
        _ => bits,
    }
}

/// A [`Cell`]'s value as the store holds it, refused unless it is `kind`'s.
fn value_of<'a>(kind: Kind, cell: Cell<'a>, scratch: &'a mut Vec<u8>) -> io::Result<Value<'a>> {
    let number = |bits: u64| Ok(Value::Number(bits));
    match (kind, cell) {
        (Kind::U8, Cell::U8(value)) => number(value.into()),
        (Kind::I8, Cell::I8(value)) => number(value as i64 as u64),
        (Kind::U16, Cell::U16(value)) => number(value.into()),
        (Kind::I16, Cell::I16(value)) => number(value as i64 as u64),
        (Kind::U32, Cell::U32(value)) => number(value.into()),
        (Kind::I32, Cell::I32(value)) => number(value as i64 as u64),
        (Kind::U64, Cell::U64(value)) => number(value),
        (Kind::I64, Cell::I64(value)) => number(value as u64),
        (Kind::Bool, Cell::Bool(value)) => number(value.into()),
        (Kind::OptionalI32, Cell::OptionalI32(value)) => Ok(value.map_or(Value::Missing, |value| Value::Number(value as i64 as u64))),
        (Kind::OptionalU32, Cell::OptionalU32(value)) => Ok(value.map_or(Value::Missing, |value| Value::Number(value.into()))),
        (Kind::OptionalI64, Cell::OptionalI64(value)) => Ok(value.map_or(Value::Missing, |value| Value::Number(value as u64))),
        (Kind::OptionalU64, Cell::OptionalU64(value)) => Ok(value.map_or(Value::Missing, Value::Number)),
        (Kind::Bytes, Cell::Bytes(id)) => Ok(Value::Text(id)),
        (Kind::U32List, Cell::U32List(items)) => {
            scratch.clear();
            items.iter().for_each(|item| scratch.extend_from_slice(&item.to_le_bytes()));
            Ok(Value::List(scratch))
        }
        (Kind::U64List, Cell::U64List(items)) => {
            scratch.clear();
            items.iter().for_each(|item| scratch.extend_from_slice(&item.to_le_bytes()));
            Ok(Value::List(scratch))
        }
        (Kind::Opaque, Cell::Opaque(items)) => Ok(Value::List(items)),
        (kind, cell) => Err(invalid(format!("A {cell:?} cell for a {kind:?} column"))),
    }
}

impl ColumnState {
    fn new(kind: Kind) -> Self {
        match kind {
            Kind::Bool => Self::Bool { cells: Flags::default(), unrecorded: None },
            Kind::Bytes => Self::Text { ids: Narrow::new(false), unrecorded: None },
            Kind::U32List | Kind::U64List | Kind::Opaque => Self::List { kind, pages: Vec::new(), unrecorded: None },
            _ => Self::Number { kind, cells: Narrow::new(signed(kind)), present: optional(kind).then(Flags::default), unrecorded: None },
        }
    }

    pub fn kind(&self) -> Kind {
        match self {
            Self::Number { kind, .. } | Self::List { kind, .. } => *kind,
            Self::Bool { .. } => Kind::Bool,
            Self::Text { .. } => Kind::Bytes,
        }
    }

    fn unrecorded(&self) -> Option<&Flags> {
        match self {
            Self::Number { unrecorded, .. } | Self::Bool { unrecorded, .. } | Self::Text { unrecorded, .. } | Self::List { unrecorded, .. } => {
                unrecorded.as_ref()
            }
        }
    }

    fn unrecorded_mut(&mut self) -> &mut Option<Flags> {
        match self {
            Self::Number { unrecorded, .. } | Self::Bool { unrecorded, .. } | Self::Text { unrecorded, .. } | Self::List { unrecorded, .. } => {
                unrecorded
            }
        }
    }

    pub fn recorded(&self, slot: u32) -> bool {
        self.unrecorded().is_none_or(|bits| !bits.get(slot))
    }

    /// The cell at `slot` as held, recorded or not.
    #[inline]
    pub fn held(&self, slot: u32) -> Value<'_> {
        match self {
            Self::Number { cells, present, .. } => match present {
                Some(present) if !present.get(slot) => Value::Missing,
                _ => Value::Number(cells.get(slot)),
            },
            Self::Bool { cells, .. } => Value::Number(u64::from(cells.get(slot))),
            Self::Text { ids, .. } => Value::Text(ids.get(slot) as u32),
            Self::List { pages, .. } => Value::List(pages.get(slot as usize / PAGE).map_or(&[][..], |page| page.get(slot as usize % PAGE))),
        }
    }

    /// The cell at `slot`, or [`Value::Unrecorded`].
    #[inline]
    pub fn value(&self, slot: u32) -> Value<'_> {
        if self.recorded(slot) { self.held(slot) } else { Value::Unrecorded }
    }

    /// Puts `value` (of this column's kind, never unrecorded) at `slot`, recorded.
    fn put(&mut self, slot: u32, value: Value<'_>, slots: usize, policy: ListPolicy) -> io::Result<()> {
        match (&mut *self, value) {
            (Self::Number { cells, present, .. }, Value::Number(bits)) => {
                cells.set(slot, bits, slots);
                if let Some(present) = present {
                    present.set(slot);
                }
            }
            (Self::Number { cells, present: Some(present), .. }, Value::Missing) => {
                cells.set(slot, 0, slots);
                present.clear(slot);
            }
            (Self::Bool { cells, .. }, Value::Number(bits)) => cells.put(slot, bits != 0),
            (Self::Text { ids, .. }, Value::Text(id)) => ids.set(slot, u64::from(id), slots),
            (Self::List { pages, .. }, Value::List(bytes)) => {
                let page = slot as usize / PAGE;
                if pages.len() <= page {
                    pages.resize_with(page + 1, ListPage::default);
                }
                pages[page].set(slot as usize % PAGE, bytes, policy)?;
            }
            (_, value) => return Err(invalid(format!("A {value:?} value for a {:?} column", self.kind()))),
        }
        if let Some(bits) = self.unrecorded_mut() {
            bits.clear(slot);
        }
        Ok(())
    }

    /// The cell at a slot a row left: zero, missing or empty, recorded.
    fn release(&mut self, slot: u32, policy: ListPolicy) -> io::Result<()> {
        match self {
            Self::Number { cells, present, .. } => {
                if cells.width > 0 && (slot as usize + 1) * cells.width as usize <= cells.bytes.len() {
                    cells.store(slot, 0);
                }
                if let Some(present) = present {
                    present.clear(slot);
                }
            }
            Self::Bool { cells, .. } => cells.clear(slot),
            Self::Text { ids, .. } => {
                if ids.width > 0 && (slot as usize + 1) * ids.width as usize <= ids.bytes.len() {
                    ids.store(slot, 0);
                }
            }
            Self::List { pages, .. } => {
                if let Some(page) = pages.get_mut(slot as usize / PAGE)
                    && page.holds(slot as usize % PAGE)
                {
                    page.set(slot as usize % PAGE, &[], policy)?;
                }
            }
        }
        if let Some(bits) = self.unrecorded_mut() {
            bits.clear(slot);
        }
        Ok(())
    }

    /// The cells at slots `from..to`, rows left: zero, missing or empty, recorded.
    fn release_range(&mut self, from: u32, to: u32, policy: ListPolicy) -> io::Result<()> {
        match self {
            Self::Number { cells, present, .. } => {
                cells.zero_range(from, to);
                if let Some(present) = present {
                    present.clear_range(from, to);
                }
            }
            Self::Bool { cells, .. } => cells.clear_range(from, to),
            Self::Text { ids, .. } => ids.zero_range(from, to),
            Self::List { .. } => {
                for slot in from..to {
                    self.release(slot, policy)?;
                }
            }
        }
        if let Some(bits) = self.unrecorded_mut() {
            bits.clear_range(from, to);
        }
        Ok(())
    }

    fn unrecord(&mut self, slot: u32) {
        self.unrecorded_mut().get_or_insert_with(Flags::default).set(slot);
    }

    /// Moves the cell at `old` to `new`, a lower slot holding no row: recorded or not, and
    /// `old` left released. A list cell is copied through `scratch`, one cell at a time.
    fn move_cell(&mut self, old: u32, new: u32, slots: usize, policy: ListPolicy, scratch: &mut Vec<u8>) -> io::Result<()> {
        let recorded = self.recorded(old);
        match self {
            Self::Number { cells, present, .. } => {
                cells.store(new, cells.get(old));
                if let Some(present) = present {
                    present.put(new, present.get(old));
                }
            }
            Self::Bool { cells, .. } => cells.put(new, cells.get(old)),
            Self::Text { ids, .. } => ids.store(new, ids.get(old)),
            Self::List { pages, .. } => {
                scratch.clear();
                scratch.extend_from_slice(pages.get(old as usize / PAGE).map_or(&[][..], |page| page.get(old as usize % PAGE)));
                let page = new as usize / PAGE;
                if pages.len() <= page {
                    pages.resize_with(page + 1, ListPage::default);
                }
                pages[page].set(new as usize % PAGE, scratch, policy)?;
            }
        }
        if let Some(bits) = self.unrecorded_mut() {
            bits.put(new, !recorded);
        }
        let _ = slots;
        self.release(old, policy)
    }

    fn grow(&mut self, slots: usize) {
        match self {
            Self::Number { cells, .. } => cells.grow(slots),
            Self::Text { ids, .. } => ids.grow(slots),
            Self::Bool { .. } | Self::List { .. } => {}
        }
    }

    pub fn bytes(&self) -> ColumnBytes {
        let unrecorded = self.unrecorded().map_or(0, Flags::bytes);
        match self {
            Self::Number { cells, present, .. } => {
                ColumnBytes { numbers: cells.bytes(), numbers_used: cells.used(), bits: unrecorded + present.as_ref().map_or(0, Flags::bytes), ..ColumnBytes::default() }
            }
            Self::Bool { cells, .. } => ColumnBytes { bits: unrecorded + cells.bytes(), ..ColumnBytes::default() },
            Self::Text { ids, .. } => ColumnBytes { numbers: ids.bytes(), numbers_used: ids.used(), bits: unrecorded, ..ColumnBytes::default() },
            Self::List { pages, .. } => {
                let mut bytes = ColumnBytes { bits: unrecorded, ..ColumnBytes::default() };
                for page in pages {
                    let (spans, items) = page.bytes();
                    bytes.list_spans += spans;
                    bytes.list_items += items;
                    bytes.list_items_used += page.used();
                    bytes.list_abandoned += page.abandoned as usize;
                }
                bytes.list_spans += pages.capacity() * size_of::<ListPage>();
                bytes
            }
        }
    }
}

/// A number, bool or text column's cells borrowed whole, for a reader's loop to read and write
/// in place, with the table's live bits beside them.
pub(crate) struct Numbers<'a> {
    pub(crate) live: &'a Flags,
    pub(crate) slots: usize,
    pub(crate) cells: NumberCells<'a>,
    pub(crate) present: Option<&'a mut Flags>,
    pub(crate) unrecorded: &'a mut Option<Flags>,
}

pub(crate) enum NumberCells<'a> {
    /// A number column's cells, or a text column's ids (`text`), which hold 32 bits.
    Narrow { cells: &'a mut Narrow, text: bool },
    Bool(&'a mut Flags),
}

/// A list column's cells borrowed whole, for a reader's loop to change in place.
pub(crate) struct Lists<'a> {
    pub(crate) live: &'a Flags,
    pages: &'a mut Vec<ListPage>,
    unrecorded: &'a mut Option<Flags>,
    policy: ListPolicy,
}

impl Lists<'_> {
    #[inline]
    pub(crate) fn get(&self, slot: u32) -> &[u8] {
        self.pages.get(slot as usize / PAGE).map_or(&[][..], |page| page.get(slot as usize % PAGE))
    }

    /// The cell at `slot` to change in place, at its length.
    #[inline]
    pub(crate) fn get_mut(&mut self, slot: u32) -> &mut [u8] {
        match self.pages.get_mut(slot as usize / PAGE) {
            Some(page) => page.get_mut(slot as usize % PAGE),
            None => &mut [],
        }
    }

    /// The cell at `slot` rewritten: in place at its length, appended to its page at another.
    pub(crate) fn set(&mut self, slot: u32, cell: &[u8]) -> io::Result<()> {
        let page = slot as usize / PAGE;
        if self.pages.len() <= page {
            self.pages.resize_with(page + 1, ListPage::default);
        }
        self.pages[page].set(slot as usize % PAGE, cell, self.policy)
    }

    /// Cells of page `page`'s slots that hold none yet, in slot order, each `(offset in the page,
    /// start, length)` of `bytes`, laid in at once.
    pub(crate) fn fill(&mut self, page: usize, cells: &[(u16, u32, u32)], bytes: &[u8]) -> io::Result<()> {
        if self.pages.len() <= page {
            self.pages.resize_with(page + 1, ListPage::default);
        }
        self.pages[page].fill(cells, bytes)
    }

    #[inline]
    pub(crate) fn unrecorded(&self, slot: u32) -> bool {
        self.unrecorded.as_ref().is_some_and(|bits| bits.get(slot))
    }

    /// The cell at `slot` recorded or not; its bytes stay.
    pub(crate) fn record(&mut self, slot: u32, recorded: bool) {
        if recorded {
            if let Some(bits) = self.unrecorded.as_mut() {
                bits.clear(slot);
            }
        } else {
            self.unrecorded.get_or_insert_with(Flags::default).set(slot);
        }
    }
}

/// What a store's parts hold, in bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ColumnBytes {
    pub numbers: usize,
    pub bits: usize,
    pub list_spans: usize,
    pub list_items: usize,
    /// Of `list_items`, bytes of cells rewritten since and not yet compacted away.
    pub list_abandoned: usize,
    /// Of `numbers` and `list_items`, the bytes the cells use: the rest is slack.
    pub numbers_used: usize,
    pub list_items_used: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StoreBytes {
    pub tables: usize,
    pub live_rows: usize,
    pub slots: usize,
    pub keys: usize,
    /// Of `keys`, the bytes the live and free slots use.
    pub keys_used: usize,
    pub index: usize,
    pub live: usize,
    pub columns: ColumnBytes,
    pub views: usize,
    pub last_complete: usize,
    pub strings: usize,
}

impl StoreBytes {
    pub fn total(&self) -> usize {
        self.keys + self.index + self.live + self.columns.numbers + self.columns.bits + self.columns.list_spans + self.columns.list_items + self.views + self.last_complete + self.strings
    }

    fn add(&mut self, other: &Self) {
        self.tables += other.tables;
        self.live_rows += other.live_rows;
        self.slots += other.slots;
        self.keys += other.keys;
        self.index += other.index;
        self.live += other.live;
        self.columns.numbers += other.columns.numbers;
        self.columns.bits += other.columns.bits;
        self.columns.list_spans += other.columns.list_spans;
        self.columns.list_items += other.columns.list_items;
        self.columns.list_abandoned += other.columns.list_abandoned;
        self.columns.numbers_used += other.columns.numbers_used;
        self.columns.list_items_used += other.columns.list_items_used;
        self.keys_used += other.keys_used;
        self.views += other.views;
        self.last_complete += other.last_complete;
        self.strings += other.strings;
    }
}

/// Live slots by key: open addressing over slots, at most three quarters full, the key read
/// from the table's keys.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct KeyIndex {
    /// Each bucket's slot plus one, which fits as slots stay below [`SLOTS`]; zero for none.
    buckets: Vec<u32>,
    held: usize,
    /// Lookups since the last block's end: an index looked up fewer times than its table has
    /// rows is let go of there.
    lookups: u64,
}

#[inline]
fn bucket_of(key: u128, mask: usize) -> usize {
    let mixed = (key as u64 ^ (key >> 64) as u64 ^ (key >> 64) as u64 >> 29).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    (mixed >> 32) as usize & mask
}

impl KeyIndex {
    fn find(&self, key: u128, keys: &Keys) -> Option<u32> {
        if self.buckets.is_empty() {
            return None;
        }
        let mask = self.buckets.len() - 1;
        let mut at = bucket_of(key, mask);
        loop {
            match self.buckets[at] {
                0 => return None,
                held if keys.get(held - 1) == key => return Some(held - 1),
                _ => at = (at + 1) & mask,
            }
        }
    }

    fn insert(&mut self, key: u128, slot: u32, keys: &Keys) {
        if (self.held + 1) * 4 > self.buckets.len() * 3 {
            let _label = label("store: index grown");
            let size = ((self.held + 1) * 4 / 3 + 1).next_power_of_two().max(16);
            let old = std::mem::replace(&mut self.buckets, vec![0; size]);
            self.held = 0;
            for held in old.into_iter().filter(|&held| held != 0) {
                self.place(keys.get(held - 1), held);
            }
        }
        self.place(key, slot + 1);
    }

    fn place(&mut self, key: u128, held: u32) {
        let mask = self.buckets.len() - 1;
        let mut at = bucket_of(key, mask);
        while self.buckets[at] != 0 {
            at = (at + 1) & mask;
        }
        self.buckets[at] = held;
        self.held += 1;
    }

    /// Removes `slot`, which holds `key`, shifting later buckets of its run back.
    fn remove(&mut self, key: u128, slot: u32, keys: &Keys) {
        if self.buckets.is_empty() {
            return;
        }
        let mask = self.buckets.len() - 1;
        let mut at = bucket_of(key, mask);
        loop {
            match self.buckets[at] {
                0 => return,
                held if held == slot + 1 => break,
                _ => at = (at + 1) & mask,
            }
        }
        self.buckets[at] = 0;
        self.held -= 1;
        let mut next = (at + 1) & mask;
        while self.buckets[next] != 0 {
            let held = self.buckets[next];
            let home = bucket_of(keys.get(held - 1), mask);
            // A bucket moves back when the gap lies between its home and where it is.
            let between = if at <= next { home <= at || home > next } else { home <= at && home > next };
            if between {
                self.buckets[at] = held;
                self.buckets[next] = 0;
                at = next;
            }
            next = (next + 1) & mask;
        }
    }

    fn bytes(&self) -> usize {
        self.buckets.capacity() * 4
    }
}

/// Keys by slot: their high and low 64 bits, each narrow.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Keys {
    high: Narrow,
    low: Narrow,
}

impl Keys {
    fn new() -> Self {
        Self { high: Narrow::new(false), low: Narrow::new(false) }
    }

    #[inline]
    pub fn get(&self, slot: u32) -> u128 {
        u128::from(self.high.get(slot)) << 64 | u128::from(self.low.get(slot))
    }

    fn set(&mut self, slot: u32, key: u128, slots: usize) {
        self.high.set(slot, (key >> 64) as u64, slots);
        self.low.set(slot, key as u64, slots);
    }

    fn bytes(&self) -> usize {
        self.high.bytes() + self.low.bytes()
    }
}

/// A table's last complete read while partial reads since changed its rows.
#[derive(Clone, Debug, PartialEq)]
pub enum Last {
    /// The rows are the read, made at the hour when known.
    Held(Option<i32>),
    /// A partial read since changed the rows: the read's table apart, and its hour.
    Apart(Box<TableState>, Option<i32>),
}

/// One table's rows.
#[derive(Clone, Debug, PartialEq)]
pub struct TableState {
    slots: u32,
    live: Flags,
    /// No free slot lies below this one.
    free_from: u32,
    keys: Keys,
    index: Option<KeyIndex>,
    columns: Vec<ColumnState>,
    status: Status,
    sampled: Option<i32>,
    /// A table read in part: the slots its last hour read.
    view: Vec<u32>,
    last: Option<Last>,
    lists: ListPolicy,
    /// A log: `count` the rows appended, `listed` those the game still lists, and nothing else
    /// is held per row.
    log: bool,
    count: u64,
    listed: Listed,
}

impl TableState {
    pub fn new(kinds: &[Kind]) -> Self {
        Self {
            slots: 0,
            live: Flags::default(),
            free_from: 0,
            keys: Keys::new(),
            index: None,
            columns: kinds.iter().map(|&kind| ColumnState::new(kind)).collect(),
            status: Status::NotRecorded,
            sampled: None,
            view: Vec::new(),
            last: None,
            lists: ListPolicy::ROOMY,
            log: false,
            count: 0,
            listed: Listed::default(),
        }
    }

    /// A log table of `kinds` that has appended `count` rows, `listed` of them still listed: its
    /// count, live set and status, no row.
    pub fn log(kinds: &[Kind], count: u64, listed: Listed) -> Self {
        Self { log: true, count, listed, ..Self::new(kinds) }
    }

    /// A log's rows the game still lists.
    pub fn listed(&self) -> Option<&Listed> {
        self.log.then_some(&self.listed)
    }

    /// A log restore's count and live set.
    pub(crate) fn set_log_rows(&mut self, count: u64, listed: Listed) {
        self.count = count;
        self.listed = listed;
    }

    /// Its rows are appended and never change: it holds a count, not rows.
    pub fn is_log(&self) -> bool {
        self.log
    }

    /// A log's rows appended so far; none for a table holding state.
    pub fn count(&self) -> Option<u64> {
        self.log.then_some(self.count)
    }

    /// Its list pages held by `policy`.
    pub fn holding_lists(mut self, policy: ListPolicy) -> Self {
        self.lists = policy;
        self
    }

    pub fn set_lists(&mut self, policy: ListPolicy) {
        self.lists = policy;
    }

    /// One past the highest slot in use.
    pub fn slots(&self) -> u32 {
        self.slots
    }

    pub fn live(&self) -> &Flags {
        &self.live
    }

    pub fn rows(&self) -> impl Iterator<Item = u32> + '_ {
        self.live.ones()
    }

    pub fn key(&self, slot: u32) -> Option<u128> {
        self.live.get(slot).then(|| self.keys.get(slot))
    }

    pub fn columns(&self) -> &[ColumnState] {
        &self.columns
    }

    pub fn column(&self, column: usize) -> Option<&ColumnState> {
        self.columns.get(column)
    }

    pub fn value(&self, column: usize, slot: u32) -> Value<'_> {
        self.columns.get(column).map_or(Value::Unrecorded, |held| held.value(slot))
    }

    pub fn status(&self) -> Status {
        self.status
    }

    pub fn sampled_at(&self) -> Option<i32> {
        self.sampled
    }

    /// For a table read in part, the slots its last hour read; none otherwise.
    pub fn view(&self) -> Option<&[u32]> {
        matches!(self.status, Status::Truncated { .. }).then_some(self.view.as_slice())
    }

    pub fn last(&self) -> Option<&Last> {
        self.last.as_ref()
    }

    /// The slot of the live row with `key`; builds the key index the first time.
    pub fn slot(&mut self, key: u128) -> Option<u32> {
        if self.index.is_none() {
            let mut index = KeyIndex::default();
            for slot in self.live.ones() {
                index.insert(self.keys.get(slot), slot, &self.keys);
            }
            self.index = Some(index);
        }
        let index = self.index.as_mut().expect("built");
        index.lookups += 1;
        index.find(key, &self.keys)
    }

    /// The slot of the live row with `key`: `hint` when that slot holds it, read without the key
    /// index; else through it ([`Self::slot`]).
    pub fn slot_of(&mut self, key: u128, hint: Option<u32>) -> Option<u32> {
        if let Some(hint) = hint
            && self.live.get(hint)
            && self.keys.get(hint) == key
        {
            return Some(hint);
        }
        self.slot(key)
    }

    /// At a block's end: the key index let go of when the block looked it up fewer times than
    /// the table has rows (the next miss builds it again, one pass over the keys).
    fn let_go_index(&mut self) {
        let rows = self.live.count() as u64;
        if let Some(index) = &mut self.index {
            if index.lookups < rows {
                self.index = None;
            } else {
                index.lookups = 0;
            }
        }
    }

    /// The slot of the live row with `key`: through the key index when one is built, else by a
    /// pass over the live rows.
    pub fn find(&self, key: u128) -> Option<u32> {
        match &self.index {
            Some(index) => index.find(key, &self.keys),
            None => self.live.ones().find(|&slot| self.keys.get(slot) == key),
        }
    }

    /// Builds the key index when it is not built: a table its readers look up by key.
    pub fn index_keys(&mut self) {
        if self.index.is_none() {
            self.slot(0);
        }
    }

    /// The lowest slot holding no row; none when every slot below [`SLOTS`] holds one.
    fn lowest_free(&mut self) -> Option<u32> {
        let mut word = self.free_from as usize / 64;
        while self.live.0.get(word) == Some(&u64::MAX) {
            word += 1;
        }
        let bits = self.live.0.get(word).copied().unwrap_or(0);
        let slot = slot_at(word, bits.trailing_ones())?;
        self.free_from = slot;
        Some(slot)
    }

    fn insert_at(&mut self, slot: u32, key: u128) -> io::Result<()> {
        if slot >= SLOTS {
            return Err(past_slots("A table"));
        }
        if self.live.get(slot) {
            return Err(invalid(format!("Insert names slot {slot}, which holds a row")));
        }
        if slot >= self.slots {
            self.slots = slot + 1;
        }
        let slots = self.slots as usize;
        self.keys.set(slot, key, slots);
        self.live.set(slot);
        if slot == self.free_from {
            self.free_from += 1;
        }
        if let Some(index) = &mut self.index {
            if index.find(key, &self.keys).is_some_and(|held| held != slot) {
                return Err(invalid(format!("Row key {key:#x} at slot {slot} is already held")));
            }
            index.insert(key, slot, &self.keys);
        }
        for column in &mut self.columns {
            column.grow(slots);
        }
        Ok(())
    }

    fn delete(&mut self, slot: u32) -> io::Result<u128> {
        if !self.live.get(slot) {
            return Err(invalid(format!("Delete names slot {slot}, which holds no row")));
        }
        let key = self.keys.get(slot);
        if let Some(index) = &mut self.index {
            index.remove(key, slot, &self.keys);
        }
        self.live.clear(slot);
        self.free_from = self.free_from.min(slot);
        let slots = self.slots as usize;
        self.keys.set(slot, 0, slots);
        for column in &mut self.columns {
            column.release(slot, self.lists)?;
        }
        Ok(key)
    }

    /// A restore's slot count: its arrays reach it.
    pub(crate) fn set_slots(&mut self, slots: u32) {
        if slots > self.slots {
            self.slots = slots;
            let held = slots as usize;
            for column in &mut self.columns {
                column.grow(held);
            }
        }
    }

    /// A table's status and sample hour as the status runs give them at a block's end; a complete
    /// table keeping its last complete read holds it at that sample hour.
    pub(crate) fn set_status(&mut self, status: Status, sampled: Option<i32>) {
        self.status = status;
        self.sampled = sampled;
        if status == Status::Complete && matches!(self.last, Some(Last::Held(_))) {
            self.last = Some(Last::Held(sampled));
        }
    }

    /// A restore's status, sample hour, rows read and last complete read.
    pub(crate) fn set_meta(&mut self, status: Status, sampled: Option<i32>, view: Option<Vec<u32>>, last: Option<Last>) {
        self.status = status;
        self.sampled = sampled;
        self.view = view.unwrap_or_default();
        self.last = last;
    }

    /// A restore's rows of slots `from..to`, below its slot count and holding none yet: `bits` their
    /// live bits from slot `from`, `keys` the live ones' keys in slot order. Keys are held at the
    /// width the widest needs, widened once.
    pub(crate) fn put_rows(&mut self, from: u32, to: u32, bits: &[u8], keys: &[u128]) -> io::Result<()> {
        let Some(bits) = to.checked_sub(from).filter(|_| to <= self.slots).and_then(|rows| bits.get(..rows.div_ceil(8) as usize)) else {
            return Err(invalid("A restore's rows pass its slots"));
        };
        let (mut high, mut low) = (0_u64, 0_u64);
        for &key in keys {
            high |= (key >> 64) as u64;
            low |= key as u64;
        }
        let slots = self.slots as usize;
        self.keys.high.hold(high, slots);
        self.keys.low.hold(low, slots);
        self.keys.high.grow(slots);
        self.keys.low.grow(slots);
        let words = (to as usize).div_ceil(64);
        if self.live.0.len() < words {
            reserve_tight(&mut self.live.0, words, "store: bits grown");
            self.live.0.resize(words, 0);
        }
        let mut count = 0;
        for (index, &byte) in bits.iter().enumerate() {
            let first = from + index as u32 * 8;
            if byte == 0 || first >= to {
                continue;
            }
            let byte = if to - first < 8 { byte & ((1_u16 << (to - first)) - 1) as u8 } else { byte };
            let (word, shift) = (first as usize / 64, first % 64);
            let wide = u128::from(byte) << shift;
            let (low, high) = (wide as u64, (wide >> 64) as u64);
            if self.live.0[word] & low != 0 || (high != 0 && self.live.0[word + 1] & high != 0) {
                return Err(invalid(format!("A restore names a slot of {first}..{} twice", u64::from(first) + 8)));
            }
            self.live.0[word] |= low;
            if high != 0 {
                self.live.0[word + 1] |= high;
            }
            count += byte.count_ones() as usize;
        }
        if count != keys.len() {
            return Err(invalid("A restore's rows hold another count of keys than live slots"));
        }
        let mut keys = keys.iter();
        let (high, low) = (self.keys.high.width(), self.keys.low.width());
        for (index, &byte) in bits.iter().enumerate() {
            let mut rest = byte;
            while rest != 0 {
                let slot = from + index as u32 * 8 + rest.trailing_zeros();
                rest &= rest - 1;
                if slot >= to {
                    break;
                }
                let key = *keys.next().expect("counted");
                if high > 0 {
                    self.keys.high.store(slot, (key >> 64) as u64);
                }
                if low > 0 {
                    self.keys.low.store(slot, key as u64);
                }
            }
        }
        self.free_from = 0;
        Ok(())
    }

    /// Every live row's cell of `column` not recorded. A column past the table's, which a recording
    /// of an earlier format never held, already reads not recorded.
    pub(crate) fn unrecord_rows(&mut self, column: usize) -> io::Result<()> {
        let Some(held) = self.columns.get_mut(column) else { return Ok(()) };
        let held = held.unrecorded_mut();
        match held {
            None => *held = Some(self.live.clone()),
            Some(bits) => self.live.ones().for_each(|slot| bits.set(slot)),
        }
        Ok(())
    }

    /// A reader's new row at `slot`: its cells empty until put.
    pub(crate) fn insert_row(&mut self, slot: u32, key: u128) -> io::Result<()> {
        self.insert_at(slot, key)
    }

    /// A reader's row gone from `slot`.
    pub(crate) fn delete_row(&mut self, slot: u32) -> io::Result<()> {
        self.delete(slot).map(drop)
    }

    /// Row `at` of `rows` put at the lowest free slot, its key and its cells of `columns` (the
    /// others not recorded): a reader holding a log's listed rows as a table's cells. Its slot.
    pub fn put_row_from(&mut self, key: u128, rows: &TableState, at: u32, columns: &[usize]) -> io::Result<u32> {
        let slot = self.lowest_free().ok_or_else(|| past_slots("A table"))?;
        self.insert_at(slot, key)?;
        for column in 0..self.columns.len() {
            match columns.contains(&column).then(|| rows.value(column, at)) {
                Some(Value::Unrecorded) | None => self.unrecord_cell(column, slot)?,
                Some(value) => self.put_cell(column, slot, value)?,
            }
        }
        Ok(slot)
    }

    /// The row at `slot` taken out: its key.
    pub fn remove_row(&mut self, slot: u32) -> io::Result<u128> {
        self.delete(slot)
    }

    /// A reader's rows gone from slots `from..to`, every one holding a row: each one's slot and
    /// key given to `each`, then all cleared at once.
    pub(crate) fn delete_rows(&mut self, from: u32, to: u32, mut each: impl FnMut(u32, u128)) -> io::Result<()> {
        if !self.live.all(from, to) {
            return Err(invalid(format!("Delete names a slot of {from}..{to}, which holds no row")));
        }
        for slot in from..to {
            let key = self.keys.get(slot);
            if let Some(index) = &mut self.index {
                index.remove(key, slot, &self.keys);
            }
            each(slot, key);
        }
        self.live.clear_range(from, to);
        self.free_from = self.free_from.min(from);
        self.keys.high.zero_range(from, to);
        self.keys.low.zero_range(from, to);
        for column in &mut self.columns {
            column.release_range(from, to, self.lists)?;
        }
        Ok(())
    }

    /// Column `column`'s cells for a reader's loop, when it is a number, bool or text column.
    pub(crate) fn numbers(&mut self, column: usize) -> Option<Numbers<'_>> {
        let slots = self.slots as usize;
        let live = &self.live;
        Some(match self.columns.get_mut(column)? {
            ColumnState::Number { cells, present, unrecorded, .. } => {
                Numbers { live, slots, cells: NumberCells::Narrow { cells, text: false }, present: present.as_mut(), unrecorded }
            }
            ColumnState::Text { ids, unrecorded } => Numbers { live, slots, cells: NumberCells::Narrow { cells: ids, text: true }, present: None, unrecorded },
            ColumnState::Bool { cells, unrecorded } => Numbers { live, slots, cells: NumberCells::Bool(cells), present: None, unrecorded },
            ColumnState::List { .. } => return None,
        })
    }

    /// Column `column`'s cells for a reader's loop, when it is a list or opaque column.
    pub(crate) fn lists(&mut self, column: usize) -> Option<Lists<'_>> {
        let live = &self.live;
        match self.columns.get_mut(column)? {
            ColumnState::List { pages, unrecorded, .. } => Some(Lists { live, pages, unrecorded, policy: self.lists }),
            _ => None,
        }
    }

    /// A reader's cell put at `slot`, recorded.
    pub(crate) fn put_cell(&mut self, column: usize, slot: u32, value: Value<'_>) -> io::Result<()> {
        let slots = self.slots as usize;
        let policy = self.lists;
        self.columns.get_mut(column).ok_or_else(|| invalid(format!("No column {column}")))?.put(slot, value, slots, policy)
    }

    /// A reader's cell at `slot` unrecorded; a column past the table's already reads so.
    pub(crate) fn unrecord_cell(&mut self, column: usize, slot: u32) -> io::Result<()> {
        if let Some(held) = self.columns.get_mut(column) {
            held.unrecord(slot);
        }
        Ok(())
    }

    /// Moves the live rows to slots `0..live` in slot order; each old slot's new one, or none for a
    /// slot holding no row.
    pub fn pack(&mut self) -> io::Result<Vec<Option<u32>>> {
        if self.log {
            return Err(invalid("A log holds no slots to pack"));
        }
        let _label = label("store: packed");
        let mut moved = vec![None; self.slots as usize];
        // In place: each live row moves down to the next packed slot, in slot order, so a row
        // never lands on one not yet moved.
        let mut new = 0_u32;
        let live: Vec<u32> = self.live.ones().collect();
        let policy = self.lists;
        let slots = self.slots as usize;
        let mut scratch = Vec::new();
        for &old in &live {
            moved[old as usize] = Some(new);
            if new != old {
                let key = self.keys.get(old);
                self.keys.set(new, key, slots);
                self.live.set(new);
                self.live.clear(old);
                for column in &mut self.columns {
                    column.move_cell(old, new, slots, policy, &mut scratch)?;
                }
            }
            new += 1;
        }
        self.free_from = new;
        for slot in &mut self.view {
            *slot = moved.get(*slot as usize).copied().flatten().unwrap_or(u32::MAX);
        }
        self.view.retain(|&slot| slot != u32::MAX);
        // The next lookup that misses its hint builds the index again over the packed slots.
        self.index = None;
        self.trim();
        Ok(moved)
    }

    /// Each number and text column at the narrowest width its live cells need.
    pub fn narrow(&mut self) {
        let slots = self.slots as usize;
        for column in &mut self.columns {
            match column {
                ColumnState::Number { cells, .. } => cells.narrow(slots),
                ColumnState::Text { ids, .. } => ids.narrow(slots),
                _ => {}
            }
        }
        self.keys.high.narrow(slots);
        self.keys.low.narrow(slots);
    }

    /// Lets go of slots past the last live row.
    fn trim(&mut self) {
        if self.log {
            return;
        }
        let end = self.live.ones().last().map_or(0, |slot| slot + 1);
        self.cut(end);
    }

    /// Lets go of slots from `end` on, none of them live.
    fn cut(&mut self, end: u32) {
        if end >= self.slots {
            return;
        }
        self.slots = end;
        let slots = end as usize;
        let policy = self.lists;
        self.live.truncate(slots);
        self.keys.high.truncate(slots);
        self.keys.low.truncate(slots);
        self.free_from = self.free_from.min(end);
        for column in &mut self.columns {
            match column {
                ColumnState::Number { cells, present, unrecorded, .. } => {
                    cells.truncate(slots);
                    if let Some(bits) = present {
                        bits.truncate(slots);
                    }
                    if let Some(bits) = unrecorded {
                        bits.truncate(slots);
                    }
                }
                ColumnState::Text { ids, unrecorded } => {
                    ids.truncate(slots);
                    if let Some(bits) = unrecorded {
                        bits.truncate(slots);
                    }
                }
                ColumnState::Bool { cells, unrecorded } => {
                    cells.truncate(slots);
                    if let Some(bits) = unrecorded {
                        bits.truncate(slots);
                    }
                }
                ColumnState::List { pages, unrecorded, .. } => {
                    // A page no cell was ever written to was never made: the last one held may
                    // end before the slot trimmed to.
                    pages.truncate(slots.div_ceil(PAGE));
                    if let Some(page) = pages.get_mut(slots / PAGE)
                        && slots % PAGE != 0
                    {
                        page.truncate(slots % PAGE, policy);
                    }
                    if let Some(bits) = unrecorded {
                        bits.truncate(slots);
                    }
                }
            }
        }
    }

    pub fn bytes(&self) -> StoreBytes {
        let mut bytes = StoreBytes {
            tables: 1,
            live_rows: self.live.count(),
            slots: self.slots as usize,
            keys: self.keys.bytes(),
            index: self.index.as_ref().map_or(0, KeyIndex::bytes),
            live: self.live.bytes() + self.listed.bytes(),
            views: self.view.capacity() * 4,
            ..StoreBytes::default()
        };
        for column in &self.columns {
            let held = column.bytes();
            bytes.columns.numbers += held.numbers;
            bytes.columns.bits += held.bits;
            bytes.columns.list_spans += held.list_spans;
            bytes.columns.list_items += held.list_items;
            bytes.columns.list_abandoned += held.list_abandoned;
            bytes.columns.numbers_used += held.numbers_used;
            bytes.columns.list_items_used += held.list_items_used;
        }
        bytes.keys_used = self.keys.high.used() + self.keys.low.used();
        if let Some(Last::Apart(table, _)) = &self.last {
            bytes.last_complete = table.bytes().total();
        }
        bytes
    }
}

/// Where a string the store let go of is read again: the file it wrote it to.
pub trait StringSource {
    fn text(&mut self, id: u32) -> io::Result<Option<Vec<u8>>>;

    /// Lets go of what it read for the hour that ended.
    fn release(&mut self) {}
}

/// The text of every string a live cell or refusal names, by id, and of those added since the
/// last block's end; the ids follow the run's appends. A string let go of is read again from the
/// file when a cell names it again. Strings the recorder's own writers name have ids past every
/// append and are held for the run ([`Self::pin`]).
#[derive(Default)]
pub struct Strings {
    held: HashMap<u32, Box<[u8]>, foldhash::fast::FixedState>,
    pinned: HashMap<u32, Box<[u8]>, foldhash::fast::FixedState>,
    next: u32,
    /// The lowest writer's string: appends stop below it.
    pinned_from: Option<u32>,
    bytes: usize,
    /// The held strings by the digest of their text, made the first time a text is interned and
    /// kept in step with what is held; for texts of one digest, the lowest id.
    by_text: Option<HashMap<u64, u32, foldhash::fast::FixedState>>,
}

fn text_digest(text: &[u8]) -> u64 {
    std::hash::BuildHasher::hash_one(&foldhash::fast::FixedState::default(), text)
}

impl Strings {
    pub fn get(&self, id: u32) -> Option<&[u8]> {
        self.held.get(&id).or_else(|| self.pinned.get(&id)).map(|text| &**text)
    }

    /// Holds `text` as `id` for the rest of the run: a string a writer names, past every append.
    pub fn pin(&mut self, id: u32, text: &[u8]) -> io::Result<()> {
        if id < self.next {
            return Err(invalid(format!("A writer's string {id} is among the appends")));
        }
        match self.pinned.get(&id) {
            Some(held) if **held != *text => Err(invalid(format!("A writer's string {id} is held as another text"))),
            Some(_) => Ok(()),
            None => {
                self.bytes += text.len();
                self.pinned.insert(id, text.into());
                self.pinned_from = Some(self.pinned_from.map_or(id, |lowest| lowest.min(id)));
                Ok(())
            }
        }
    }

    /// Ids appends may still take: up to the lowest writer's string, or [`STRINGS`].
    fn room(&self) -> u32 {
        self.pinned_from.unwrap_or(STRINGS).saturating_sub(self.next)
    }

    /// The refusal of an append past [`Self::room`].
    fn full(&self) -> io::Error {
        match self.pinned_from {
            Some(id) if id < STRINGS => invalid(format!("The run's strings reach {id}, a writer's string")),
            _ => invalid("The run's strings pass 4,294,967,295, the format's string id width"),
        }
    }

    /// The next id's text, within [`Self::room`].
    fn append(&mut self, text: &[u8]) {
        self.bytes += text.len();
        self.held.insert(self.next, text.into());
        if let Some(by_text) = &mut self.by_text {
            by_text.entry(text_digest(text)).or_insert(self.next);
        }
        self.next += 1;
    }

    /// The id of `text` among the held strings, appended when none holds it.
    fn intern(&mut self, text: &[u8]) -> io::Result<u32> {
        if self.by_text.is_none() {
            self.index_texts();
        }
        let digest = text_digest(text);
        let found = self.by_text.as_ref().and_then(|by_text| by_text.get(&digest).copied());
        if let Some(id) = found
            && self.held.get(&id).is_some_and(|held| **held == *text)
        {
            return Ok(id);
        }
        if self.room() == 0 {
            return Err(self.full());
        }
        let id = self.next;
        let _label = label("strings");
        // Another text of the same digest keeps the map's entry; this one is appended unmapped.
        self.append(text);
        Ok(id)
    }

    /// The map of texts made again from the held strings.
    fn index_texts(&mut self) {
        let _label = label("strings: by text");
        let mut by_text: HashMap<u64, u32, foldhash::fast::FixedState> = HashMap::default();
        by_text.reserve(self.held.len());
        for (&id, text) in &self.held {
            let entry = by_text.entry(text_digest(text)).or_insert(id);
            *entry = (*entry).min(id);
        }
        self.by_text = Some(by_text);
    }

    pub fn next(&self) -> u32 {
        self.next
    }

    /// Holds `id`'s text again when it was let go of.
    fn named(&mut self, id: u32, source: &mut dyn StringSource) -> io::Result<()> {
        if id >= self.next || self.held.contains_key(&id) {
            return Ok(());
        }
        if self.pinned.contains_key(&id) {
            return Ok(());
        }
        let _label = label("strings");
        let text = source.text(id)?.ok_or_else(|| invalid(format!("String {id} is in neither the store nor its file")))?;
        self.bytes += text.len();
        if let Some(by_text) = &mut self.by_text {
            by_text.entry(text_digest(&text)).or_insert(id);
        }
        self.held.insert(id, text.into_boxed_slice());
        Ok(())
    }

    fn retain(&mut self, named: &Flags) {
        let bytes = &mut self.bytes;
        self.held.retain(|&id, text| {
            let keep = named.get(id);
            if !keep {
                *bytes -= text.len();
            }
            keep
        });
        self.held.shrink_to_fit();
        if self.by_text.is_some() {
            self.index_texts();
        }
    }

    fn held_bytes(&self) -> usize {
        self.bytes
            + (self.held.capacity() + self.pinned.capacity()) * (size_of::<u32>() + size_of::<Box<[u8]>>() + 1)
            + self.by_text.as_ref().map_or(0, |by_text| by_text.capacity() * (size_of::<u64>() + size_of::<u32>() + 1))
    }
}

/// One table of the store as the recorder's readers of an hour see it (the derived tables'
/// writers, the summaries): its rows, cells and strings, read only.
#[derive(Clone, Copy)]
pub struct TableView<'a> {
    table: &'a TableState,
    kinds: &'a [Kind],
    strings: &'a Strings,
    /// A log: its own state (count and status), `table` holding the hour's appended rows.
    log: Option<&'a TableState>,
}

impl<'a> TableView<'a> {
    pub fn new(table: &'a TableState, kinds: &'a [Kind], strings: &'a Strings) -> Self {
        Self { table, kinds, strings, log: None }
    }

    /// A log table: its rows are the hour's appended rows, at slots from 0 in append order.
    pub fn is_log(&self) -> bool {
        self.log.is_some()
    }

    /// A log's ordinal of the row at slot 0: the rows it appended before the hour's first.
    pub fn appended_from(&self) -> Option<u64> {
        self.log.map(|log| log.count - u64::from(self.table.slots))
    }

    pub fn state(&self) -> &'a TableState {
        self.table
    }

    pub fn kinds(&self) -> &'a [Kind] {
        self.kinds
    }

    pub fn status(&self) -> Status {
        self.log.unwrap_or(self.table).status
    }

    pub fn sampled_at(&self) -> Option<i32> {
        self.log.unwrap_or(self.table).sampled
    }

    /// For a table read in part, the slots its last hour read.
    pub fn view(&self) -> Option<&'a [u32]> {
        if self.log.is_some() { None } else { self.table.view() }
    }

    pub fn live(&self) -> &'a Flags {
        &self.table.live
    }

    /// Live slots, ascending.
    pub fn rows(&self) -> impl Iterator<Item = u32> + 'a {
        self.table.live.ones()
    }

    pub fn key(&self, slot: u32) -> Option<u128> {
        self.table.key(slot)
    }

    pub fn slot(&self, key: u128) -> Option<u32> {
        self.table.find(key)
    }

    pub fn recorded(&self, column: usize, slot: u32) -> bool {
        self.table.columns.get(column).is_some_and(|held| held.recorded(slot))
    }

    pub fn value(&self, column: usize, slot: u32) -> Value<'a> {
        self.table.value(column, slot)
    }

    /// A numeric or bool cell as a signed value; none when missing, unrecorded or not a number.
    pub fn number(&self, column: usize, slot: u32) -> Option<i64> {
        match self.table.columns.get(column)? {
            ColumnState::Number { .. } | ColumnState::Bool { .. } => match self.table.value(column, slot) {
                Value::Number(bits) => Some(bits as i64),
                _ => None,
            },
            _ => None,
        }
    }

    pub fn bool(&self, column: usize, slot: u32) -> Option<bool> {
        match (self.table.columns.get(column)?, self.table.value(column, slot)) {
            (ColumnState::Bool { .. }, Value::Number(bits)) => Some(bits != 0),
            _ => None,
        }
    }

    /// A text cell's string id.
    pub fn text(&self, column: usize, slot: u32) -> Option<u32> {
        match self.table.value(column, slot) {
            Value::Text(id) => Some(id),
            _ => None,
        }
    }

    /// A list or opaque cell's bytes.
    pub fn list(&self, column: usize, slot: u32) -> Option<&'a [u8]> {
        match self.table.value(column, slot) {
            Value::List(bytes) => Some(bytes),
            _ => None,
        }
    }

    /// String `id`'s text, when the store holds it.
    pub fn string(&self, id: u32) -> Option<&'a [u8]> {
        self.strings.get(id)
    }
}

/// What one table's hour changed: for the derived tables' writers, the summaries and the digest.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TableHour {
    pub inserted: Vec<u32>,
    /// Each deleted row's slot and the key it held.
    pub deleted: Vec<(u32, u128)>,
    /// Cells an event named on rows older than the hour, (slot, column); in slot order once
    /// the hour ends.
    pub written: Vec<(u32, u16)>,
    pub touched: Vec<u32>,
    pub status_changed: bool,
    /// The table had a section this hour.
    pub present: bool,
    /// A run's first hour: every live row was born in it and the log holds none of them; readers
    /// take the hour from the table's rows.
    pub whole: bool,
    /// A log: the rows the hour appended, and the ordinal of its first. Their cells are the
    /// table's view ([`Store::view`]) while the hour lasts.
    pub appended: u32,
    pub appended_from: u64,
    /// A log: the runs of ordinals the hour forgot, in the order forgotten.
    pub forgotten: Vec<std::ops::Range<u64>>,
    /// Columns written of rows inserted this hour, (slot, column).
    born_written: Vec<(u32, u16)>,
    /// A table kept with [`Store::keep_previous`]: each cell of a row older than the hour as it
    /// was before a write, (slot, column, value), in the order written; a cell's first entry is
    /// its value before the hour.
    pub previous: Vec<(u32, u16, Kept)>,
    /// Such a table, or one kept with [`Store::keep_dropped`]: each row the hour deleted, its key
    /// and its cells as the deletion found them.
    pub dropped: Dropped,
}

/// The rows an hour deleted of a kept table: their keys beside their cells, one row after another
/// (16 bytes a cell and the key a row).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Dropped {
    width: usize,
    keys: Vec<u128>,
    cells: Vec<Kept>,
}

impl Dropped {
    fn push(&mut self, key: u128, cells: impl ExactSizeIterator<Item = Kept>) {
        self.width = cells.len();
        self.keys.push(key);
        self.cells.extend(cells);
    }

    fn clear(&mut self) {
        self.keys.clear();
        self.cells.clear();
    }

    pub fn len(&self) -> usize {
        self.keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// Row `at`'s cells, rows numbered in the order deleted.
    pub fn row(&self, at: usize) -> &[Kept] {
        self.cells.get(at * self.width..(at + 1) * self.width).unwrap_or(&[])
    }

    /// Each row's key and cells, in the order deleted.
    pub fn iter(&self) -> impl Iterator<Item = (&u128, &[Kept])> + '_ {
        self.keys.iter().zip(self.cells.chunks(self.width.max(1)).chain(std::iter::repeat(&[][..])))
    }

    /// The bytes held.
    pub fn bytes(&self) -> usize {
        self.keys.capacity() * 16 + self.cells.capacity() * size_of::<Kept>()
    }
}

impl<'a> IntoIterator for &'a Dropped {
    type Item = (&'a u128, &'a [Kept]);
    type IntoIter = Box<dyn Iterator<Item = (&'a u128, &'a [Kept])> + 'a>;

    fn into_iter(self) -> Self::IntoIter {
        Box::new(self.iter())
    }
}

/// A cell as it was, free of its table: what [`Store::keep_previous`] keeps (16 bytes: a list's
/// bytes behind one pointer).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kept {
    Unrecorded,
    Missing,
    Number(u64),
    Text(u32),
    #[allow(clippy::redundant_allocation, reason = "a thin pointer keeps a kept cell at 16 bytes")]
    List(Box<Box<[u8]>>),
}

impl Kept {
    fn of(value: Value<'_>) -> Self {
        match value {
            Value::Unrecorded => Self::Unrecorded,
            Value::Missing => Self::Missing,
            Value::Number(bits) => Self::Number(bits),
            Value::Text(id) => Self::Text(id),
            Value::List(bytes) => Self::List(Box::new(bytes.into())),
        }
    }

    pub fn value(&self) -> Value<'_> {
        match self {
            Self::Unrecorded => Value::Unrecorded,
            Self::Missing => Value::Missing,
            Self::Number(bits) => Value::Number(*bits),
            Self::Text(id) => Value::Text(*id),
            Self::List(bytes) => Value::List(bytes),
        }
    }
}

/// What one hour changed, by table; cleared at the next hour's start.
#[derive(Default)]
pub struct HourLog {
    pub tables: Vec<TableHour>,
    present: Vec<usize>,
}

/// The run's state: every table, its strings, and the hour's log.
pub struct Store {
    tables: Vec<TableState>,
    kinds: Vec<Vec<Kind>>,
    /// The tables that keep their last complete read.
    keeps: Vec<bool>,
    /// The tables whose hour log keeps what it replaced ([`Self::keep_previous`]).
    previous: Vec<bool>,
    /// The tables whose hour log keeps each deleted row's cells alone ([`Self::keep_dropped`]).
    dropped: Vec<bool>,
    strings: Strings,
    log: HourLog,
    hour: i32,
    /// Rows inserted this hour, by table.
    born: Vec<Flags>,
    /// Of those, rows their producer gives every column of: no cell of theirs is logged.
    given: Vec<Flags>,
    scratch: Vec<u8>,
    list: Vec<u8>,
    /// Where strings let go of are read again.
    source: Option<Box<dyn StringSource + Send>>,
    /// The unit's encoder, when one is written.
    encoder: Option<crate::unit::Encoder>,
    /// Tables the encoder packed at a block's end, each old slot's new one.
    moves: Vec<(usize, Vec<Option<u32>>)>,
    /// Per table, cells of rows inserted over the run that their hour did not give.
    unwritten: Vec<u64>,
    /// The hour the run under way began at.
    run_first: Option<i32>,
    /// The hour applied is the run's first ([`TableHour::whole`]).
    whole: bool,
    /// In a run's first hour, the cells unrecorded on purpose, (table, column, slot): not counted
    /// as cells the hour did not give.
    unrecorded_given: Vec<(usize, u16, u32)>,
    /// The log tables, by table.
    logs: Vec<bool>,
    /// Each log table's rows appended this hour, at slots from 0.
    appended: Vec<Option<Box<TableState>>>,
}

impl Store {
    pub fn new(kinds: Vec<Vec<Kind>>, keeps: Vec<bool>) -> Self {
        let tables = kinds.iter().map(|kinds| TableState::new(kinds).holding_lists(ListPolicy::TIGHT)).collect();
        let count = kinds.len();
        Self {
            tables,
            keeps,
            previous: vec![false; count],
            dropped: vec![false; count],
            strings: Strings::default(),
            log: HourLog { tables: vec![TableHour::default(); count], present: Vec::new() },
            hour: 0,
            born: vec![Flags::default(); count],
            given: vec![Flags::default(); count],
            scratch: Vec::new(),
            list: Vec::new(),
            source: None,
            encoder: None,
            moves: Vec::new(),
            unwritten: vec![0; count],
            run_first: None,
            whole: false,
            unrecorded_given: Vec::new(),
            logs: vec![false; count],
            appended: (0..count).map(|_| None).collect(),
            kinds,
        }
    }

    /// Declares which tables are logs (by table), before any row is in: a log holds a count and
    /// its status, and its rows go to the unit as they are appended.
    pub fn declare_logs(&mut self, logs: &[bool]) -> io::Result<()> {
        if logs.len() != self.tables.len() {
            return Err(invalid(format!("Logs declared for {} tables of a store of {}", logs.len(), self.tables.len())));
        }
        if let Some(table) = self.tables.iter().position(|table| table.slots > 0 || table.count > 0) {
            return Err(invalid(format!("Table {table} is declared a log or state after it holds rows")));
        }
        self.set_logs(logs.to_vec());
        Ok(())
    }

    fn set_logs(&mut self, logs: Vec<bool>) {
        for (table, &log) in logs.iter().enumerate() {
            self.tables[table].log = log;
            self.appended[table] = log.then(|| Box::new(TableState::new(&self.kinds[table]).holding_lists(ListPolicy::TIGHT)));
        }
        if let Some(encoder) = &mut self.encoder {
            encoder.declare_logs(&logs);
        }
        self.logs = logs;
    }

    /// The log tables, by table.
    pub fn logs(&self) -> &[bool] {
        &self.logs
    }

    /// Per table, the cells of rows inserted that their hour did not give, over the store's life.
    pub fn unwritten(&self) -> &[u64] {
        &self.unwritten
    }

    /// The tables packed since last asked, each with every old slot's new one: a producer naming
    /// slots of its own renumbers them.
    pub fn take_moves(&mut self) -> Vec<(usize, Vec<Option<u32>>)> {
        std::mem::take(&mut self.moves)
    }

    /// Writes the unit as changes are applied.
    pub fn attach(&mut self, mut encoder: crate::unit::Encoder) {
        encoder.declare_logs(&self.logs);
        self.encoder = Some(encoder);
    }

    pub fn encoder(&self) -> Option<&crate::unit::Encoder> {
        self.encoder.as_ref()
    }

    pub fn encoder_mut(&mut self) -> Option<&mut crate::unit::Encoder> {
        self.encoder.as_mut()
    }

    /// Where strings let go of are read again: the file the run's strings were written to.
    pub fn read_strings_from(&mut self, source: Box<dyn StringSource + Send>) {
        self.source = Some(source);
    }

    /// Table `table`'s status, whether the hour read it, and its sample hour; `sectioned` when the hour holds its
    /// section. A partial read about to change a table keeping its last complete read keeps that
    /// read apart first.
    pub fn status(&mut self, table: usize, status: Status, read: bool, sampled: Option<i32>, sectioned: bool) -> io::Result<()> {
        if let Status::Unavailable { reason } = status {
            self.hold_again(reason)?;
        }
        if let Some(encoder) = &mut self.encoder {
            encoder.status(table, crate::unit::HourStatus { status, read, sampled });
        }
        let keeps = self.keeps.get(table).copied().unwrap_or(false);
        let state = self.tables.get_mut(table).ok_or_else(|| invalid(format!("No table {table}")))?;
        if keeps && sectioned && matches!(status, Status::Truncated { .. })
            && let Some(Last::Held(read)) = state.last
        {
            let _label = label("store: last read kept apart");
            let mut apart = state.clone();
            apart.last = None;
            apart.index = None;
            state.last = Some(Last::Apart(Box::new(apart), read));
        }
        state.sampled = sampled;
        if !matches!(status, Status::Truncated { .. }) {
            state.view.clear();
        }
        if state.status != status {
            state.status = status;
            if !self.log.tables[table].present {
                self.log.tables[table].present = true;
                self.log.present.push(table);
            }
            self.log.tables[table].status_changed = true;
        }
        Ok(())
    }

    /// After the hour, for a table keeping its last complete read: a complete view is that read,
    /// at its sample hour or, when the hour changed no row, at the hour of the read it holds.
    pub fn after(&mut self, table: usize, sectioned: bool) {
        if !self.keeps.get(table).copied().unwrap_or(false) {
            return;
        }
        let state = &mut self.tables[table];
        if state.status != Status::Complete {
            return;
        }
        let kept = match &state.last {
            Some(Last::Held(read)) if !sectioned => *read,
            _ => None,
        };
        state.last = Some(Last::Held(state.sampled.or(kept)));
    }

    pub fn tables(&self) -> &[TableState] {
        &self.tables
    }

    /// Table `table` as the recorder's readers of an hour see it.
    /// A log's view is the hour's appended rows, with the log's own status.
    pub fn view(&self, table: usize) -> TableView<'_> {
        match &self.appended[table] {
            Some(rows) => TableView { table: rows, kinds: &self.kinds[table], strings: &self.strings, log: Some(&self.tables[table]) },
            None => TableView { table: &self.tables[table], kinds: &self.kinds[table], strings: &self.strings, log: None },
        }
    }

    /// From the next change on, `table`'s hour log keeps each older cell before a write and each
    /// deleted row's cells ([`TableHour::previous`], [`TableHour::dropped`]); kept across runs.
    pub fn keep_previous(&mut self, table: usize) {
        if let Some(held) = self.previous.get_mut(table) {
            *held = true;
        }
    }

    /// From the next change on, `table`'s hour log keeps each deleted row's cells
    /// ([`TableHour::dropped`]) and no older cell of a write; kept across runs.
    pub fn keep_dropped(&mut self, table: usize) {
        if let Some(held) = self.dropped.get_mut(table) {
            *held = true;
        }
    }

    /// Builds `table`'s key index: a table its readers look up by key.
    pub fn index(&mut self, table: usize) {
        if let Some(held) = self.tables.get_mut(table) {
            held.index_keys();
        }
    }

    /// The id of `text` among the strings held now, appended when none holds it: a string let
    /// go of at a block's end and met again is appended again, with a new id.
    pub fn intern(&mut self, text: &[u8]) -> io::Result<u32> {
        self.strings.intern(text)
    }

    /// Holds `text` as string `id` for the run: one the recorder's writers name.
    pub fn pin_string(&mut self, id: u32, text: &[u8]) -> io::Result<()> {
        self.strings.pin(id, text)
    }

    pub fn table_state(&self, table: usize) -> Option<&TableState> {
        self.tables.get(table)
    }

    pub fn kinds(&self, table: usize) -> &[Kind] {
        &self.kinds[table]
    }

    pub fn strings(&self) -> &Strings {
        &self.strings
    }

    pub fn log(&self) -> &HourLog {
        &self.log
    }

    pub fn hour(&self) -> i32 {
        self.hour
    }

    /// Whether the hour begun is its run's first.
    pub fn first_of_run(&self) -> bool {
        self.whole
    }

    /// A new run whose first hour is `first_hour`: every table empty, the strings started again.
    pub fn begin_run(&mut self, first_hour: i32) {
        self.restart();
        self.run_first = Some(first_hour);
        if let Some(encoder) = &mut self.encoder {
            encoder.begin_run(first_hour);
        }
    }

    /// Every table empty, the strings started again; what lasts across runs kept.
    fn restart(&mut self) {
        let kinds = std::mem::take(&mut self.kinds);
        let keeps = std::mem::take(&mut self.keeps);
        let previous = std::mem::take(&mut self.previous);
        let dropped = std::mem::take(&mut self.dropped);
        let source = self.source.take();
        let encoder = self.encoder.take();
        let unwritten = std::mem::take(&mut self.unwritten);
        let logs = std::mem::take(&mut self.logs);
        *self = Self::new(kinds, keeps);
        self.previous = previous;
        self.dropped = dropped;
        self.set_logs(logs);
        self.source = source;
        self.encoder = encoder;
        self.unwritten = unwritten;
    }

    /// The hour's start: the last hour's log cleared, `appends` added to the strings. A run's
    /// first hour keeps no log ([`TableHour::whole`]). Appends past the strings' room are refused
    /// by name before anything changes.
    pub fn begin_hour(&mut self, hour: i32, appends: &[&[u8]]) -> io::Result<()> {
        if u32::try_from(appends.len()).ok().is_none_or(|count| count > self.strings.room()) {
            return Err(self.strings.full());
        }
        self.clear_log();
        let whole = self.run_first == Some(hour);
        if whole || self.whole {
            for held in &mut self.log.tables {
                held.whole = whole;
            }
        }
        self.whole = whole;
        if let Some(source) = &mut self.source {
            source.release();
        }
        {
            let _label = label("strings");
            for text in appends {
                self.strings.append(text);
            }
        }
        self.hour = hour;
        if let Some(encoder) = &mut self.encoder {
            encoder.begin_hour(hour);
        }
        Ok(())
    }

    fn clear_log(&mut self) {
        for &table in &self.log.present {
            let held = &mut self.log.tables[table];
            for &slot in &held.inserted {
                self.born[table].clear(slot);
                self.given[table].clear(slot);
            }
            held.inserted.clear();
            held.deleted.clear();
            held.written.clear();
            held.touched.clear();
            held.status_changed = false;
            held.present = false;
            held.born_written.clear();
            held.previous.clear();
            held.dropped.clear();
            if held.appended > 0 {
                held.appended = 0;
                if let Some(rows) = &mut self.appended[table] {
                    **rows = TableState::new(&self.kinds[table]).holding_lists(ListPolicy::TIGHT);
                }
            }
            held.forgotten.clear();
        }
        self.log.present.clear();
    }

    /// String `id` held, read again from the file when it was let go of: a cell or a refusal
    /// names it.
    fn hold_again(&mut self, id: u32) -> io::Result<()> {
        if self.strings.get(id).is_some() {
            return Ok(());
        }
        let Store { strings, source, .. } = self;
        match source {
            Some(source) => strings.named(id, &mut **source),
            None if id >= strings.next() => Err(invalid(format!("String {id} was never added"))),
            None => Err(invalid(format!("String {id} was let go of and no file is given to read it again"))),
        }
    }

    /// Lets go of the room the hour log and the store's scratch kept for the largest hour so
    /// far: after a run's first hour, which reads every row, and at each block's end. The hour
    /// it is called after is over; its log is gone.
    pub fn shed(&mut self) {
        self.clear_log();
        for held in &mut self.log.tables {
            *held = TableHour::default();
        }
        self.log.present = Vec::new();
        for born in self.born.iter_mut().chain(&mut self.given) {
            *born = Flags::default();
        }
        self.scratch = Vec::new();
        self.list = Vec::new();
    }

    /// One table's changes this hour.
    pub fn table(&mut self, table: usize) -> io::Result<TableChanges<'_>> {
        if table >= self.tables.len() {
            return Err(invalid(format!("No table {table}")));
        }
        if !self.log.tables[table].present {
            self.log.tables[table].present = true;
            self.log.present.push(table);
        }
        Ok(TableChanges { store: self, table, _label: label("store") })
    }

    /// The changes so far settled: each table's log in slot order, and a row inserted this hour
    /// holding no cell of a column it was not given. Readers of the hour (writers, summaries) read
    /// it settled; settling again changes nothing.
    pub fn settle(&mut self) {
        if self.whole {
            return;
        }
        let Self { tables, log, kinds, unwritten: counted, given, .. } = self;
        for &table in &log.present {
            let held = &mut log.tables[table];
            held.written.sort_unstable();
            held.written.dedup();
            held.touched.sort_unstable();
            held.touched.dedup();
            if held.inserted.is_empty() && held.deleted.is_empty() {
                continue;
            }
            held.inserted.sort_unstable();
            held.deleted.sort_unstable_by_key(|&(slot, _)| slot);
            let columns = kinds[table].len();
            held.born_written.sort_unstable();
            held.born_written.dedup();
            let given = &given[table];
            let tracked = held.inserted.iter().filter(|&&slot| !given.get(slot)).count();
            if held.born_written.len() != tracked * columns {
                // The cells found unwritten join the written ones only once every search is done.
                let mut unwritten = Vec::new();
                for &slot in held.inserted.iter().filter(|&&slot| !given.get(slot)) {
                    for column in 0..columns {
                        if held.born_written.binary_search(&(slot, column as u16)).is_err() && tables[table].live.get(slot) {
                            tables[table].columns[column].unrecord(slot);
                            unwritten.push((slot, column as u16));
                        }
                    }
                }
                if let Some(count) = counted.get_mut(table) {
                    *count += unwritten.len() as u64;
                }
                held.born_written.extend(unwritten);
                held.born_written.sort_unstable();
                held.born_written.dedup();
            }
        }
    }

    /// The hour's end, the changes settled ([`Self::settle`]): the unit given the hour's
    /// lifetimes and the inserted rows' cells, and its parts that are due closed into `sink`.
    pub fn end_hour(&mut self, sink: &mut dyn crate::unit::PartSink) -> io::Result<()> {
        self.settle();
        if self.whole {
            self.count_whole_unwritten();
        }
        let Self { tables, log, encoder, kinds, .. } = self;
        for &table in &log.present {
            let held = &log.tables[table];
            if held.inserted.is_empty() && held.deleted.is_empty() {
                continue;
            }
            let columns = kinds[table].len();
            let Some(encoder) = encoder.as_mut().filter(|encoder| encoder.is_on()) else { continue };
            let keys = &tables[table].keys;
            encoder.lives(table, &held.deleted, &held.inserted, |slot| keys.get(slot));
            for column in 0..columns {
                for &slot in &held.inserted {
                    encoder.inserted(table, column, tables[table].value(column, slot));
                }
            }
        }
        let ends_block = encoder.as_ref().is_some_and(crate::unit::Encoder::ends_block);
        if ends_block {
            // Closing the block's parts reads nothing of the hour's log: its room goes first.
            self.shed();
        }
        let Self { tables, encoder, strings, moves, .. } = self;
        let Some(encoder) = encoder else { return Ok(()) };
        encoder.end_hour(tables, strings, sink)?;
        moves.extend(encoder.take_moves());
        Ok(())
    }

    /// A run's first hour ends: its rows were born with every cell not recorded, so a cell still
    /// not recorded and not unrecorded on purpose is one the hour did not give. Columns left with
    /// no such cell hold no bits.
    fn count_whole_unwritten(&mut self) {
        let mut given = std::mem::take(&mut self.unrecorded_given);
        given.sort_unstable();
        given.dedup();
        for (index, table) in self.tables.iter_mut().enumerate() {
            let TableState { live, columns, .. } = table;
            for (column, held) in columns.iter_mut().enumerate() {
                let bits = held.unrecorded_mut();
                let Some(flags) = bits else { continue };
                let key = (index, column as u16);
                let from = given.partition_point(|&(table, at, _)| (table, at) < key);
                let to = given.partition_point(|&(table, at, _)| (table, at) <= key);
                let set = flags.count();
                self.unwritten[index] += (set - given[from..to].iter().filter(|&&(_, _, slot)| flags.get(slot) && live.get(slot)).count()) as u64;
                if set == 0 {
                    *bits = None;
                } else {
                    flags.trim_zeros();
                }
            }
        }
    }

    /// The run ends after the hour applied last: its short last block closes into `sink`.
    pub fn end_run(&mut self, sink: &mut dyn crate::unit::PartSink) -> io::Result<()> {
        match &mut self.encoder {
            Some(encoder) => encoder.end_run(&mut self.tables, &self.strings, sink),
            None => Ok(()),
        }
    }

    /// At a block's end: strings no live cell or refusal names are let go of. Pinned strings,
    /// past every append, are kept.
    pub fn end_block(&mut self) {
        let next = self.strings.next;
        let mut named = Flags::default();
        let mut name = |id: u32| {
            if id < next {
                named.set(id);
            }
        };
        for table in &self.tables {
            if let Status::Unavailable { reason } = table.status {
                name(reason);
            }
            for column in &table.columns {
                if let ColumnState::Text { ids, unrecorded } = column {
                    for slot in table.live.ones() {
                        if unrecorded.as_ref().is_none_or(|bits| !bits.get(slot)) {
                            name(ids.get(slot) as u32);
                        }
                    }
                }
            }
            if let Some(Last::Apart(last, _)) = &table.last {
                for column in &last.columns {
                    if let ColumnState::Text { ids, .. } = column {
                        for slot in last.live.ones() {
                            name(ids.get(slot) as u32);
                        }
                    }
                }
            }
        }
        self.strings.retain(&named);
        for table in &mut self.tables {
            table.trim();
            table.let_go_index();
        }
        if let Some(source) = &mut self.source {
            source.release();
        }
    }

    /// What the store holds, by part, with every table's.
    pub fn bytes(&self) -> (StoreBytes, Vec<StoreBytes>) {
        let mut total = StoreBytes::default();
        let tables: Vec<StoreBytes> = self.tables.iter().map(TableState::bytes).collect();
        for table in &tables {
            total.add(table);
        }
        total.strings = self.strings.held_bytes();
        (total, tables)
    }

    /// Takes a run up at a block's end: every table as read from the file and trimmed as the
    /// block's end trimmed it, the run's string ids counted to `next`, the strings live cells name
    /// held again.
    pub fn resume(&mut self, tables: Vec<TableState>, next: u32) -> io::Result<()> {
        if tables.len() != self.tables.len() {
            return Err(invalid("Resumed tables of another schema"));
        }
        if let Some(table) = (0..tables.len()).find(|&table| tables[table].log != self.logs[table]) {
            let (file, build) = if self.logs[table] { ("state", "a log") } else { ("a log", "state") };
            return Err(invalid(format!("Table {table} is {file} in the file and {build} in this build: a table keeps its class for the run")));
        }
        self.tables = tables;
        for table in &mut self.tables {
            table.set_lists(ListPolicy::TIGHT);
            table.trim();
        }
        self.strings.next = next;
        if let Some(mut source) = self.source.take() {
            let held = (0..self.tables.len()).try_for_each(|table| self.hold_named(table, &mut *source));
            self.source = Some(source);
            held?;
        }
        Ok(())
    }

    /// Makes every string a live text cell or the table's refusal names held: after its table
    /// came from a restore.
    pub fn hold_named(&mut self, table: usize, source: &mut dyn StringSource) -> io::Result<()> {
        let held = &self.tables[table];
        let mut ids = Vec::new();
        if let Status::Unavailable { reason } = held.status {
            ids.push(reason);
        }
        for column in &held.columns {
            if let ColumnState::Text { ids: cells, unrecorded } = column {
                for slot in held.live.ones() {
                    if unrecorded.as_ref().is_none_or(|bits| !bits.get(slot)) {
                        ids.push(cells.get(slot) as u32);
                    }
                }
            }
        }
        for id in ids {
            self.strings.named(id, source)?;
        }
        Ok(())
    }
}

/// The edits of a PATCH to one cell: per hunk the held items it keeps, removes and inserts; the
/// inserted items as bytes.
#[derive(Clone, Copy, Debug)]
pub struct Edits<'a> {
    pub hunks: &'a [(u32, u32, u32)],
    pub items: &'a [u8],
}

/// One table's changes of the hour ([`Store::table`]).
pub struct TableChanges<'s> {
    store: &'s mut Store,
    table: usize,
    /// What the changes allocate is the store's, whoever asks for them.
    _label: engine_allocation::labels::Scope,
}

impl TableChanges<'_> {
    fn state(&mut self) -> &mut TableState {
        &mut self.store.tables[self.table]
    }

    fn log(&mut self) -> &mut TableHour {
        &mut self.store.log.tables[self.table]
    }

    pub fn held(&self) -> &TableState {
        &self.store.tables[self.table]
    }

    /// Refuses a change by row on a log, whose rows are appended and forgotten, never held.
    fn holds_state(&self, what: &str) -> io::Result<()> {
        if self.store.tables[self.table].log {
            return Err(invalid(format!("Table {} is a log: its rows are appended and forgotten, never {what}", self.table)));
        }
        Ok(())
    }

    /// A log's row, every column given (`None` is a column not given: not recorded); its ordinal,
    /// the rows appended before it. Its cells go to the unit and the hour's log, never held.
    pub fn append(&mut self, key: u128, cells: &[Option<Cell<'_>>]) -> io::Result<u64> {
        let table = self.table;
        if !self.store.tables[table].log {
            return Err(invalid(format!("Table {table} holds state: its rows are inserted, never appended")));
        }
        let columns = self.store.kinds[table].len();
        if cells.len() != columns {
            return Err(invalid(format!("A row of {} cells appended to log table {table} of {columns} columns", cells.len())));
        }
        let ordinal = self.store.tables[table].count;
        let next = ordinal.checked_add(1).ok_or_else(|| invalid(format!("Log table {table} passes 2^64 rows")))?;
        let mut scratch = std::mem::take(&mut self.store.scratch);
        let checked = cells.iter().enumerate().try_for_each(|(column, cell)| match cell {
            Some(cell) => value_of(self.store.kinds[table][column], *cell, &mut scratch).map(drop),
            None => Ok(()),
        });
        let held = checked.and_then(|()| {
            cells.iter().try_for_each(|cell| match cell {
                Some(Cell::Bytes(id)) => self.store.hold_again(*id),
                _ => Ok(()),
            })
        });
        if let Err(error) = held {
            self.store.scratch = scratch;
            return Err(error);
        }
        let _label = label("hour log");
        let Store { kinds, appended, encoder, .. } = &mut *self.store;
        let rows = appended[table].as_mut().expect("a log's hour rows");
        let at = rows.slots;
        let result: io::Result<()> = (|| {
            if at >= SLOTS {
                return Err(past_slots(format_args!("Log table {table}'s hour")));
            }
            rows.insert_at(at, key)?;
            if let Some(encoder) = encoder.as_mut() {
                encoder.append_row(table, ordinal, key)?;
            }
            for (column, cell) in cells.iter().enumerate() {
                let value = match cell {
                    Some(cell) => value_of(kinds[table][column], *cell, &mut scratch)?,
                    None => Value::Unrecorded,
                };
                match value {
                    Value::Unrecorded => rows.unrecord_cell(column, at)?,
                    value => rows.put_cell(column, at, value)?,
                }
                if let Some(encoder) = encoder.as_mut() {
                    encoder.inserted(table, column, value);
                }
            }
            Ok(())
        })();
        self.store.scratch = scratch;
        result?;
        let Store { tables, log, .. } = &mut *self.store;
        let state = &mut tables[table];
        state.count = next;
        state.listed.append(ordinal);
        let hour = &mut log.tables[table];
        if hour.appended == 0 {
            hour.appended_from = ordinal;
        }
        hour.appended += 1;
        Ok(ordinal)
    }

    /// The game no longer lists log row `ordinal`, appended this hour or before: its delete is
    /// written at this hour. Its cells stay where it was appended. An hour's forgets may come in any
    /// order, before or after its appends: they are written at the hour's end in ascending order,
    /// after its appends.
    pub fn forget(&mut self, ordinal: u64) -> io::Result<()> {
        self.forget_run(ordinal, 1)
    }

    /// [`Self::forget`] of rows `first..first + count`; a run refused by name for any of its rows
    /// changes nothing.
    pub fn forget_run(&mut self, first: u64, count: u64) -> io::Result<()> {
        let table = self.table;
        let state = &mut self.store.tables[table];
        if !state.log {
            return Err(invalid(format!("Table {table} holds state: its rows are deleted, never forgotten")));
        }
        if count == 0 {
            return Ok(());
        }
        let end = first.checked_add(count).filter(|&end| end <= state.count).ok_or_else(|| {
            let past = first.max(state.count);
            invalid(format!("Log table {table} forgets row {past}, which it never appended"))
        })?;
        if !state.listed.all(first, end) {
            let forgotten = (first..end).find(|&ordinal| !state.listed.get(ordinal)).unwrap_or(first);
            return Err(invalid(format!("Log table {table} forgets row {forgotten}, which it forgot before")));
        }
        self.forgotten(first..end);
        Ok(())
    }

    /// Log rows `run` forgotten: no longer listed, their delete written at this hour.
    fn forgotten(&mut self, run: std::ops::Range<u64>) {
        if run.is_empty() {
            return;
        }
        let table = self.table;
        self.store.tables[table].listed.clear_range(run.start, run.end);
        if let Some(encoder) = &mut self.store.encoder {
            encoder.forget_rows(table, run.start, run.end - run.start);
        }
        let forgotten = &mut self.store.log.tables[table].forgotten;
        match forgotten.last_mut() {
            Some(last) if last.end == run.start => last.end = run.end,
            _ => log_push(forgotten, run),
        }
    }

    /// A table read in part: the rows this hour read, of those live when it began.
    pub fn read_rows(&mut self, slots: &[u32]) -> io::Result<()> {
        self.holds_state("read in part")?;
        let state = self.state();
        if let Some(&slot) = slots.iter().find(|&&slot| !state.live.get(slot)) {
            return Err(invalid(format!("A partly read table reads slot {slot}, which holds no row")));
        }
        state.view.clear();
        state.view.extend_from_slice(slots);
        if let Some(encoder) = &mut self.store.encoder {
            encoder.read_rows(self.table, slots);
        }
        Ok(())
    }

    /// The view of a table read in part once its section applied: the rows read still live, and
    /// those inserted.
    pub fn read_done(&mut self) {
        let table = self.table;
        if self.store.tables[table].log {
            return;
        }
        if self.store.whole {
            let state = &mut self.store.tables[table];
            state.view.clear();
            state.view.extend(state.live.ones());
            return;
        }
        let inserted = std::mem::take(&mut self.store.log.tables[table].inserted);
        let state = &mut self.store.tables[table];
        let live = &state.live;
        state.view.retain(|&slot| live.get(slot));
        state.view.extend_from_slice(&inserted);
        state.view.sort_unstable();
        state.view.dedup();
        self.store.log.tables[table].inserted = inserted;
    }

    /// The live row with `key`; none on a log, which holds no row and keeps no key index.
    pub fn slot(&mut self, key: u128) -> Option<u32> {
        self.slot_of(key, None)
    }

    /// The live row with `key`, `hint` the slot its producer last held for it: read from the
    /// hint when that slot holds the key, without the key index; none on a log.
    pub fn slot_of(&mut self, key: u128, hint: Option<u32>) -> Option<u32> {
        if self.store.tables[self.table].log {
            return None;
        }
        self.state().slot_of(key, hint)
    }

    /// A new row at the lowest free slot.
    pub fn insert(&mut self, key: u128) -> io::Result<u32> {
        self.holds_state("inserted")?;
        let slot = self.lowest_free()?;
        self.insert_at(slot, key)?;
        Ok(slot)
    }

    fn lowest_free(&mut self) -> io::Result<u32> {
        let table = self.table;
        self.state().lowest_free().ok_or_else(|| past_slots(format_args!("Table {table}")))
    }

    /// A new row at `slot`, which holds none. Its cells come through [`Self::write`]; a column
    /// not given is unrecorded.
    pub fn insert_at(&mut self, slot: u32, key: u128) -> io::Result<()> {
        self.inserting(slot, key, false)
    }

    /// [`Self::insert_at`] of a row whose producer gives every column this hour: none of its
    /// cells is logged.
    pub fn insert_given_at(&mut self, slot: u32, key: u128) -> io::Result<()> {
        self.inserting(slot, key, true)
    }

    /// [`Self::insert`] of a row whose producer gives every column this hour.
    pub fn insert_given(&mut self, key: u128) -> io::Result<u32> {
        self.holds_state("inserted")?;
        let slot = self.lowest_free()?;
        self.inserting(slot, key, true)?;
        Ok(slot)
    }

    fn inserting(&mut self, slot: u32, key: u128, given: bool) -> io::Result<()> {
        self.holds_state("inserted")?;
        if slot >= SLOTS {
            return Err(past_slots(format_args!("Table {}", self.table)));
        }
        self.state().insert_at(slot, key)?;
        if self.store.whole && given {
            return Ok(());
        }
        if self.store.whole {
            // A cell the hour does not give stays not recorded: nothing is logged to find it.
            for column in &mut self.store.tables[self.table].columns {
                column.unrecord(slot);
            }
            return Ok(());
        }
        self.store.born[self.table].set(slot);
        if given {
            self.store.given[self.table].set(slot);
        }
        log_push(&mut self.log().inserted, slot);
        Ok(())
    }

    pub fn delete(&mut self, slot: u32) -> io::Result<()> {
        self.holds_state("deleted")?;
        if !self.store.whole && (self.store.previous[self.table] || self.store.dropped[self.table]) && self.store.tables[self.table].live.get(slot) {
            let _label = label(KEPT_LABEL);
            let Store { tables, log, .. } = &mut *self.store;
            let state = &tables[self.table];
            log.tables[self.table].dropped.push(state.keys.get(slot), (0..state.columns.len()).map(|column| Kept::of(state.value(column, slot))));
        }
        let key = self.state().delete(slot)?;
        if self.store.whole {
            let table = self.table;
            self.store.unrecorded_given.retain(|&(held, _, at)| held != table || at != slot);
            return Ok(());
        }
        log_push(&mut self.log().deleted, (slot, key));
        Ok(())
    }

    fn note(&mut self, slot: u32, column: usize) {
        if self.store.whole {
            return;
        }
        if self.store.born[self.table].get(slot) {
            if self.store.given[self.table].get(slot) {
                return;
            }
            log_push(&mut self.store.log.tables[self.table].born_written, (slot, column as u16));
        } else {
            log_push(&mut self.store.log.tables[self.table].written, (slot, column as u16));
        }
    }

    /// The unit's event of an older row's cell about to become `new`.
    fn event(&mut self, slot: u32, column: usize, new: Value<'_>) {
        let table = self.table;
        let Store { tables, encoder, born, previous, log, whole, .. } = &mut *self.store;
        if born[table].get(slot) {
            return;
        }
        let held = &tables[table].columns[column];
        let old = held.value(slot);
        if previous[table] && !*whole {
            let _label = label(KEPT_LABEL);
            log.tables[table].previous.push((slot, column as u16, Kept::of(old)));
        }
        let Some(encoder) = encoder.as_mut().filter(|encoder| encoder.is_on()) else { return };
        if matches!(held, ColumnState::List { .. }) {
            encoder.list(table, column, slot, old, new);
        } else {
            encoder.number(table, column, slot, old, new);
        }
    }

    fn live(&self, slot: u32) -> io::Result<()> {
        if self.store.tables[self.table].live.get(slot) {
            Ok(())
        } else {
            Err(invalid(format!("A change names slot {slot}, which holds no row")))
        }
    }

    fn put(&mut self, slot: u32, column: usize, value: Value<'_>) -> io::Result<()> {
        let state = &mut self.store.tables[self.table];
        let slots = state.slots as usize;
        let held = state.columns.get_mut(column).ok_or_else(|| invalid(format!("No column {column}")))?;
        held.put(slot, value, slots, state.lists)
    }

    /// An event whatever the held cell is: the journal's SET, and its NOOP with the held value.
    pub fn write(&mut self, slot: u32, column: usize, cell: Cell<'_>) -> io::Result<()> {
        self.holds_state("written")?;
        self.live(slot)?;
        let kind = self.store.kinds[self.table].get(column).copied().ok_or_else(|| invalid(format!("No column {column}")))?;
        let mut scratch = std::mem::take(&mut self.store.scratch);
        let result = value_of(kind, cell, &mut scratch).and_then(|value| self.put_value(slot, column, value));
        self.store.scratch = scratch;
        result
    }

    /// [`Self::write`] of a value as the store holds it.
    pub fn put_value(&mut self, slot: u32, column: usize, value: Value<'_>) -> io::Result<()> {
        self.holds_state("written")?;
        if let Value::Text(id) = value {
            self.store.hold_again(id)?;
        }
        if value == Value::Unrecorded {
            return self.unrecord(slot, column);
        }
        if column >= self.store.kinds[self.table].len() {
            return Err(invalid(format!("No column {column}")));
        }
        self.event(slot, column, value);
        self.put(slot, column, value)?;
        self.note(slot, column);
        Ok(())
    }

    /// The held cell written again: an event of no difference (the journal's NOOP).
    pub fn rewrite(&mut self, slot: u32, column: usize) -> io::Result<()> {
        self.holds_state("written")?;
        self.live(slot)?;
        if column >= self.store.kinds[self.table].len() {
            return Err(invalid(format!("No column {column}")));
        }
        let mut list = std::mem::take(&mut self.store.list);
        let held = match self.store.tables[self.table].columns[column].value(slot) {
            Value::List(bytes) => {
                list.clear();
                list.extend_from_slice(bytes);
                None
            }
            other => Some(other.owned_scalar()),
        };
        let value = match held {
            None => Value::List(&list),
            Some(value) => value,
        };
        self.event(slot, column, value);
        self.store.list = list;
        self.note(slot, column);
        Ok(())
    }

    /// An event only when `cell` differs from the held one.
    pub fn offer(&mut self, slot: u32, column: usize, cell: Cell<'_>) -> io::Result<bool> {
        self.holds_state("offered")?;
        self.live(slot)?;
        let kind = self.store.kinds[self.table].get(column).copied().ok_or_else(|| invalid(format!("No column {column}")))?;
        let mut scratch = std::mem::take(&mut self.store.scratch);
        let result = value_of(kind, cell, &mut scratch).and_then(|value| {
            if self.store.tables[self.table].value(column, slot) == value {
                return Ok(false);
            }
            self.put_value(slot, column, value).map(|()| true)
        });
        self.store.scratch = scratch;
        result
    }

    /// The journal's ADD: the held number plus `difference`, at the column's width.
    pub fn add(&mut self, slot: u32, column: usize, difference: u64) -> io::Result<()> {
        self.holds_state("added to")?;
        self.live(slot)?;
        let kind = self.store.kinds[self.table].get(column).copied().ok_or_else(|| invalid(format!("No column {column}")))?;
        let held = self.store.tables[self.table].columns[column].held(slot);
        let Value::Number(bits) = held else {
            return Err(invalid(format!("ADD to a {held:?} cell at slot {slot}")));
        };
        let new = Value::Number(at_width(kind, bits.wrapping_add(difference)));
        self.event(slot, column, new);
        self.put(slot, column, new)?;
        self.note(slot, column);
        Ok(())
    }

    /// The journal's PATCH: `edits` applied to the held list cell.
    pub fn edit(&mut self, slot: u32, column: usize, edits: Edits<'_>) -> io::Result<()> {
        self.holds_state("edited")?;
        self.live(slot)?;
        let kind = self.store.kinds[self.table].get(column).copied().ok_or_else(|| invalid(format!("No column {column}")))?;
        let width = item_width(kind);
        let mut cell = std::mem::take(&mut self.store.list);
        cell.clear();
        let result = (|| {
            let Value::List(held) = self.store.tables[self.table].columns[column].held(slot) else {
                return Err(invalid("PATCH to a cell that is not a list"));
            };
            let (mut from, mut item) = (0_usize, 0_usize);
            for &(skip, removed, inserted) in edits.hunks {
                let kept = from + skip as usize * width;
                cell.extend_from_slice(held.get(from..kept).ok_or_else(|| invalid("PATCH keeps items past its cell"))?);
                from = kept + removed as usize * width;
                if from > held.len() {
                    return Err(invalid("PATCH removes items past its cell"));
                }
                let end = item + inserted as usize * width;
                cell.extend_from_slice(edits.items.get(item..end).ok_or_else(|| invalid("PATCH ends inside its items"))?);
                item = end;
            }
            cell.extend_from_slice(&held[from..]);
            if item != edits.items.len() {
                return Err(invalid("PATCH carries items past its edits"));
            }
            Ok(())
        })();
        let result = result.and_then(|()| {
            self.event(slot, column, Value::List(&cell));
            self.put(slot, column, Value::List(&cell))
        });
        self.store.list = cell;
        result?;
        self.note(slot, column);
        Ok(())
    }

    /// The journal's FILL: one value for every live row but `except` (ascending).
    pub fn fill(&mut self, column: usize, cell: Cell<'_>, except: &[u32]) -> io::Result<Vec<u32>> {
        self.holds_state("filled")?;
        let slots: Vec<u32> = {
            let state = &self.store.tables[self.table];
            let mut except = except.iter().peekable();
            state
                .live
                .ones()
                .filter(|slot| {
                    while except.next_if(|&&held| held < *slot).is_some() {}
                    except.peek() != Some(&slot)
                })
                .collect()
        };
        for &slot in &slots {
            self.write(slot, column, cell)?;
        }
        Ok(slots)
    }

    pub fn unrecord(&mut self, slot: u32, column: usize) -> io::Result<()> {
        self.holds_state("unrecorded")?;
        self.live(slot)?;
        if column >= self.store.kinds[self.table].len() {
            return Err(invalid(format!("No column {column}")));
        }
        self.event(slot, column, Value::Unrecorded);
        self.store.tables[self.table].columns.get_mut(column).ok_or_else(|| invalid(format!("No column {column}")))?.unrecord(slot);
        if self.store.whole {
            self.store.unrecorded_given.push((self.table, column as u16, slot));
        }
        self.note(slot, column);
        Ok(())
    }

    pub fn touch(&mut self, slot: u32) -> io::Result<()> {
        self.holds_state("touched")?;
        self.live(slot)?;
        if !self.store.whole {
            log_push(&mut self.log().touched, slot);
        }
        if let Some(encoder) = &mut self.store.encoder {
            encoder.touch(self.table, slot);
        }
        Ok(())
    }
}
