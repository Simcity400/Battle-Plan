//! A strip page's bytes: the block's extent and its cells at the page's first ordinal, then
//! the length of its changes' headers, the headers, then the changes' values.
//!
//! A column's cells at the first ordinal: whether any is unrecorded and then which, for an
//! optional column which are present, then each present value. Changes come in one group per
//! ordinal, ascending. A group's header: its ordinal's difference from the group before (from
//! the page's first, for the first), then its count of changes shifted over whether a fill
//! leads them; a fill's: its exceptions' count shifted over whether its value is missing, then
//! the exceptions' gaps. A change's header: its offset's gap from the change before shifted
//! over its flags (unrecorded, missing, a difference). The values follow in the same order: a
//! fill's, then each change's: none when missing; for a difference, an integer's zigzag of its
//! wrapping difference from the held cell less that cell's last difference, or a list's or
//! bytes' kept front and back lengths and the items between; any other whole. A change equal
//! to the held cell is not written. A block's rows: which slots are live, then each live key's
//! low and high words; a row change's header is its offset's gap shifted over whether it is
//! live, its value the key.

use crate::{
    invalid,
    leaf::{Int, Reader, Writer, varint_len},
    state::{Bits, Bools, Cell, Column, Kind, Lists, Paged, PagedBits, Table},
};
use std::io;

/// Makes room in `into` for `count` items.
fn scratch<T>(into: &mut Vec<T>, count: usize) -> io::Result<()> {
    if count > into.capacity() {
        into.clear();
        engine_allocation::reserve_exact(into, count)?;
    }
    Ok(())
}

/// A change's flags, below its offset.
const UNRECORDED: u32 = 1;
const MISSING: u32 = 2;
const DELTA: u32 = 4;
const FLAG_BITS: u32 = 3;

fn optional(kind: Kind) -> bool {
    matches!(
        kind,
        Kind::OptionalI32 | Kind::OptionalU32 | Kind::OptionalI64 | Kind::OptionalU64
    )
}

/// An integer cell's bits as the differences between its values are taken.
fn raw(cell: Cell<'_>) -> Option<u64> {
    Some(match cell {
        Cell::U16(value) => u64::from(value),
        Cell::U32(value) | Cell::OptionalU32(Some(value)) => u64::from(value),
        Cell::U64(value) | Cell::OptionalU64(Some(value)) => value,
        Cell::I16(value) => value as i64 as u64,
        Cell::I32(value) | Cell::OptionalI32(Some(value)) => value as i64 as u64,
        Cell::I64(value) | Cell::OptionalI64(Some(value)) => value as u64,
        _ => return None,
    })
}

fn zigzag(value: i64) -> u64 {
    ((value << 1) ^ (value >> 63)) as u64
}

fn unzigzag(value: u64) -> i64 {
    ((value >> 1) as i64) ^ -((value & 1) as i64)
}

/// The bytes `cell`'s value takes written whole.
fn whole_len(cell: Cell<'_>) -> usize {
    match cell {
        Cell::U16(value) => varint_len(value.wire()),
        Cell::U32(value) | Cell::OptionalU32(Some(value)) => varint_len(value.wire()),
        Cell::U64(value) | Cell::OptionalU64(Some(value)) => varint_len(value),
        Cell::I16(value) => varint_len(value.wire()),
        Cell::I32(value) | Cell::OptionalI32(Some(value)) => varint_len(value.wire()),
        Cell::I64(value) | Cell::OptionalI64(Some(value)) => varint_len(value.wire()),
        _ => usize::MAX,
    }
}

/// Writes `cell`'s value whole; nothing for a missing optional.
fn put_value(writer: &mut Writer, cell: Cell<'_>) -> io::Result<()> {
    match cell {
        Cell::U8(value) => writer.u8(value),
        Cell::I8(value) => writer.u8(value as u8),
        Cell::Bool(value) => writer.u8(u8::from(value)),
        Cell::U16(value) => writer.varint(value.wire()),
        Cell::I16(value) => writer.varint(value.wire()),
        Cell::U32(value) | Cell::Bytes(value) => writer.varint(value.wire()),
        Cell::I32(value) => writer.varint(value.wire()),
        Cell::U64(value) => writer.varint(value),
        Cell::I64(value) => writer.varint(value.wire()),
        Cell::OptionalI32(value) => value.map_or(Ok(()), |value| writer.varint(value.wire())),
        Cell::OptionalU32(value) => value.map_or(Ok(()), |value| writer.varint(value.wire())),
        Cell::OptionalI64(value) => value.map_or(Ok(()), |value| writer.varint(value.wire())),
        Cell::OptionalU64(value) => value.map_or(Ok(()), |value| writer.varint(value)),
        Cell::U32List(items) => {
            writer.varint(items.len() as u64)?;
            writer.ints(items)
        }
        Cell::U64List(items) => {
            writer.varint(items.len() as u64)?;
            writer.ints(items)
        }
        Cell::Opaque(bytes) => {
            writer.varint(bytes.len() as u64)?;
            writer.raw(bytes)
        }
    }
}

fn present(cell: Cell<'_>) -> bool {
    !matches!(
        cell,
        Cell::OptionalI32(None)
            | Cell::OptionalU32(None)
            | Cell::OptionalI64(None)
            | Cell::OptionalU64(None)
    )
}

/// `slots` as bits, a byte per eight, lowest first.
fn put_bits(writer: &mut Writer, count: u32, set: impl Fn(u32) -> bool) -> io::Result<()> {
    for byte in 0..count.div_ceil(8) {
        let mut bits = 0_u8;
        for bit in 0..8 {
            let offset = byte * 8 + bit;
            if offset < count && set(offset) {
                bits |= 1 << bit;
            }
        }
        writer.u8(bits)?;
    }
    Ok(())
}

fn bit(bits: &[u8], offset: u32) -> bool {
    bits[(offset / 8) as usize] >> (offset % 8) & 1 != 0
}

fn list(kind: Kind) -> bool {
    matches!(kind, Kind::U32List | Kind::U64List | Kind::Opaque)
}

/// A present scalar cell's bits, as the cells held are compared.
fn scalar(cell: Cell<'_>) -> Option<u64> {
    match cell {
        Cell::U8(value) => Some(u64::from(value)),
        Cell::I8(value) => Some(u64::from(value as u8)),
        Cell::Bool(value) => Some(u64::from(value)),
        Cell::Bytes(value) => Some(u64::from(value)),
        _ => raw(cell),
    }
}

/// A list's or bytes' items, widened, into `into`; false for any other cell.
fn widen(cell: Cell<'_>, into: &mut Vec<u64>) -> io::Result<bool> {
    into.clear();
    match cell {
        Cell::U32List(items) => {
            engine_allocation::reserve(into, items.len())?;
            into.extend(items.iter().map(|&item| u64::from(item)));
        }
        Cell::U64List(items) => {
            engine_allocation::reserve(into, items.len())?;
            into.extend_from_slice(items);
        }
        Cell::Opaque(bytes) => {
            engine_allocation::reserve(into, bytes.len())?;
            into.extend(bytes.iter().map(|&byte| u64::from(byte)));
        }
        _ => return Ok(false),
    }
    Ok(true)
}

/// The bytes widened `items` take written whole: bytes' one each, a list's as varints.
fn items_len(items: &[u64], bytes: bool) -> usize {
    varint_len(items.len() as u64)
        + if bytes {
            items.len()
        } else {
            items.iter().map(|&item| varint_len(item)).sum()
        }
}

/// How many of `held`'s first and last items `new` keeps.
fn kept(held: &[u64], new: &[u64]) -> (usize, usize) {
    let front = held.iter().zip(new).take_while(|(a, b)| a == b).count();
    let room = held.len().min(new.len()) - front;
    let back = held
        .iter()
        .rev()
        .zip(new.iter().rev())
        .take(room)
        .take_while(|(a, b)| a == b)
        .count();
    (front, back)
}

/// Writes the items of a list or bytes `cell` between its first `front` and last `back`.
fn put_middle(writer: &mut Writer, cell: Cell<'_>, front: usize, back: usize) -> io::Result<()> {
    match cell {
        Cell::U32List(items) => {
            let middle = &items[front..items.len() - back];
            writer.varint(middle.len() as u64)?;
            writer.ints(middle)
        }
        Cell::U64List(items) => {
            let middle = &items[front..items.len() - back];
            writer.varint(middle.len() as u64)?;
            writer.ints(middle)
        }
        Cell::Opaque(bytes) => {
            let middle = &bytes[front..bytes.len() - back];
            writer.varint(middle.len() as u64)?;
            writer.raw(middle)
        }
        _ => Err(invalid("A strip patches a cell that is not a list")),
    }
}

/// A list or bytes column's items, every offset's end to end: where each offset's lie, and
/// the items no offset holds any longer, which a cell grown elsewhere left behind.
#[derive(Default)]
struct Spans<T> {
    spans: Vec<(u32, u32)>,
    items: Vec<T>,
    dropped: usize,
}

impl<T: Copy + PartialEq> Spans<T> {
    fn resize(&mut self, slots: usize) -> io::Result<()> {
        engine_allocation::resize(&mut self.spans, slots, (0, 0))
    }

    fn get(&self, at: usize) -> &[T] {
        let (start, count) = self.spans[at];
        &self.items[start as usize..][..count as usize]
    }

    fn set(&mut self, at: usize, new: &[T]) -> io::Result<()> {
        if self.get(at) == new {
            return Ok(());
        }
        let wide = || invalid("A strip's cells pass 2^32 items");
        let (start, count) = self.spans[at];
        let length = u32::try_from(new.len()).map_err(|_| wide())?;
        if length <= count {
            self.items[start as usize..][..new.len()].copy_from_slice(new);
            self.dropped += (count - length) as usize;
            self.spans[at].1 = length;
            return Ok(());
        }
        self.dropped += count as usize;
        self.spans[at] = (0, 0);
        if self.dropped > self.items.len() / 2 {
            self.compact()?;
        }
        let start = u32::try_from(self.items.len())
            .ok()
            .filter(|start| start.checked_add(length).is_some())
            .ok_or_else(wide)?;
        engine_allocation::reserve(&mut self.items, new.len())?;
        self.items.extend_from_slice(new);
        self.spans[at] = (start, length);
        Ok(())
    }

    /// Lets go of the items no offset holds.
    fn compact(&mut self) -> io::Result<()> {
        let mut items = engine_allocation::vec(self.items.len() - self.dropped)?;
        for span in &mut self.spans {
            let (start, count) = *span;
            let at = items.len() as u32;
            items.extend_from_slice(&self.items[start as usize..][..count as usize]);
            *span = (at, count);
        }
        self.items = items;
        self.dropped = 0;
        Ok(())
    }
}

/// A column's cells at its own width, signed ones widened back as [`scalar`] takes them, or
/// a list's or bytes' items.
#[derive(Default)]
enum Lane {
    #[default]
    None,
    W8(Vec<u8>),
    W16(Vec<u16>, bool),
    W32(Vec<u32>, bool),
    W64(Vec<u64>),
    U32s(Spans<u32>),
    U64s(Spans<u64>),
    Bytes(Spans<u8>),
}

impl Lane {
    fn of(cell: Cell<'_>) -> Self {
        match cell {
            Cell::U8(_) | Cell::I8(_) | Cell::Bool(_) => Self::W8(Vec::new()),
            Cell::U16(_) => Self::W16(Vec::new(), false),
            Cell::I16(_) => Self::W16(Vec::new(), true),
            Cell::U32(_) | Cell::Bytes(_) | Cell::OptionalU32(_) => Self::W32(Vec::new(), false),
            Cell::I32(_) | Cell::OptionalI32(_) => Self::W32(Vec::new(), true),
            Cell::U64(_) | Cell::I64(_) | Cell::OptionalU64(_) | Cell::OptionalI64(_) => {
                Self::W64(Vec::new())
            }
            Cell::U32List(_) => Self::U32s(Spans::default()),
            Cell::U64List(_) => Self::U64s(Spans::default()),
            Cell::Opaque(_) => Self::Bytes(Spans::default()),
        }
    }

    fn resize(&mut self, slots: usize) -> io::Result<()> {
        match self {
            Self::None => Ok(()),
            Self::W8(values) => engine_allocation::resize(values, slots, 0),
            Self::W16(values, _) => engine_allocation::resize(values, slots, 0),
            Self::W32(values, _) => engine_allocation::resize(values, slots, 0),
            Self::W64(values) => engine_allocation::resize(values, slots, 0),
            Self::U32s(items) => items.resize(slots),
            Self::U64s(items) => items.resize(slots),
            Self::Bytes(items) => items.resize(slots),
        }
    }

    /// The bits [`scalar`] takes of the cell at `at`; zero for a list.
    fn bits(&self, at: usize) -> u64 {
        match self {
            Self::W8(values) => u64::from(values[at]),
            Self::W16(values, false) => u64::from(values[at]),
            Self::W16(values, true) => values[at] as i16 as i64 as u64,
            Self::W32(values, false) => u64::from(values[at]),
            Self::W32(values, true) => values[at] as i32 as i64 as u64,
            Self::W64(values) => values[at],
            _ => 0,
        }
    }

    /// Keeps `cell` at `at`: nothing for a missing optional, whose bits are never read.
    fn set(&mut self, at: usize, cell: Cell<'_>) -> io::Result<()> {
        match (self, cell) {
            (Self::U32s(held), Cell::U32List(items)) => held.set(at, items),
            (Self::U64s(held), Cell::U64List(items)) => held.set(at, items),
            (Self::Bytes(held), Cell::Opaque(bytes)) => held.set(at, bytes),
            (lane, cell) => {
                let Some(bits) = scalar(cell) else {
                    return Ok(());
                };
                match lane {
                    Self::W8(values) => values[at] = bits as u8,
                    Self::W16(values, _) => values[at] = bits as u16,
                    Self::W32(values, _) => values[at] = bits as u32,
                    Self::W64(values) => values[at] = bits,
                    _ => return Err(invalid("A strip cell differs from its column's kind")),
                }
                Ok(())
            }
        }
    }

    /// Whether the list or bytes at `at` hold `cell`'s items; none for a scalar.
    fn holds_items(&self, at: usize, cell: Cell<'_>) -> Option<bool> {
        Some(match (self, cell) {
            (Self::U32s(held), Cell::U32List(items)) => held.get(at) == items,
            (Self::U64s(held), Cell::U64List(items)) => held.get(at) == items,
            (Self::Bytes(held), Cell::Opaque(bytes)) => held.get(at) == bytes,
            (_, Cell::U32List(items)) => items.is_empty(),
            (_, Cell::U64List(items)) => items.is_empty(),
            (_, Cell::Opaque(bytes)) => bytes.is_empty(),
            _ => return None,
        })
    }

    /// The items held at `at`, widened, into `into`.
    fn widen_items(&self, at: usize, into: &mut Vec<u64>) -> io::Result<()> {
        into.clear();
        match self {
            Self::U32s(held) => {
                let held = held.get(at);
                engine_allocation::reserve(into, held.len())?;
                into.extend(held.iter().map(|&item| u64::from(item)));
            }
            Self::U64s(held) => {
                let held = held.get(at);
                engine_allocation::reserve(into, held.len())?;
                into.extend_from_slice(held);
            }
            Self::Bytes(held) => {
                let held = held.get(at);
                engine_allocation::reserve(into, held.len())?;
                into.extend(held.iter().map(|&byte| u64::from(byte)));
            }
            _ => {}
        }
        Ok(())
    }
}

fn bit_of(words: &[u64], at: usize) -> bool {
    words.get(at / 64).is_some_and(|word| word >> (at % 64) & 1 != 0)
}

fn put_bit(words: &mut [u64], at: usize, set: bool) {
    let (word, bit) = (&mut words[at / 64], 1_u64 << (at % 64));
    if set {
        *word |= bit;
    } else {
        *word &= !bit;
    }
}

/// A last difference past 32 bits, held apart.
const WIDE: i32 = i32::MIN;

/// The cells a column's open page last set, as its next changes are taken against: at each
/// offset whether a present value is known and recorded, a bit each, the cell at its column's
/// width, and an integer's last difference once one is taken.
#[derive(Default)]
pub(super) struct Held {
    slots: usize,
    known: Vec<u64>,
    recorded: Vec<u64>,
    lane: Lane,
    /// Empty while every last difference is zero; one past 32 bits is [`WIDE`] here and in
    /// `wide` by offset.
    velocity: Vec<i32>,
    wide: Vec<(u32, u64)>,
}

impl Held {
    /// Room for offsets through `at`.
    fn grow(&mut self, at: usize, cell: Cell<'_>) -> io::Result<()> {
        if at < self.slots {
            return Ok(());
        }
        if matches!(self.lane, Lane::None) {
            self.lane = Lane::of(cell);
        }
        let slots = at + 1;
        let words = slots.div_ceil(64);
        engine_allocation::resize(&mut self.known, words, 0)?;
        engine_allocation::resize(&mut self.recorded, words, 0)?;
        self.lane.resize(slots)?;
        if !self.velocity.is_empty() {
            engine_allocation::resize(&mut self.velocity, slots, 0)?;
        }
        self.slots = slots;
        Ok(())
    }

    fn keep(&mut self, offset: u32, cell: Cell<'_>, recorded: bool) -> io::Result<()> {
        let at = offset as usize;
        self.grow(at, cell)?;
        put_bit(&mut self.known, at, present(cell));
        put_bit(&mut self.recorded, at, recorded);
        self.lane.set(at, cell)
    }

    fn known(&self, offset: u32) -> bool {
        (offset as usize) < self.slots && bit_of(&self.known, offset as usize)
    }

    fn wide_at(&self, at: usize) -> Result<usize, usize> {
        self.wide.binary_search_by_key(&(at as u32), |&(offset, _)| offset)
    }

    fn velocity(&self, at: usize) -> u64 {
        match self.velocity.get(at) {
            Some(&WIDE) => self.wide_at(at).map_or(0, |index| self.wide[index].1),
            Some(&step) => step as i64 as u64,
            None => 0,
        }
    }

    fn set_velocity(&mut self, at: usize, step: u64) -> io::Result<()> {
        // A whole value clears the difference, at an offset past the held ones too.
        if step == 0 && self.velocity.get(at).is_none_or(|&held| held == 0) {
            return Ok(());
        }
        if at >= self.velocity.len() {
            if at >= self.slots {
                return Err(invalid("A difference is taken at an offset not held"));
            }
            engine_allocation::resize(&mut self.velocity, self.slots, 0)?;
        }
        let was = std::mem::replace(&mut self.velocity[at], 0);
        match i32::try_from(step as i64).ok().filter(|&narrow| narrow != WIDE) {
            Some(narrow) => {
                self.velocity[at] = narrow;
                if was == WIDE
                    && let Ok(index) = self.wide_at(at)
                {
                    self.wide.remove(index);
                }
            }
            None => {
                self.velocity[at] = WIDE;
                match self.wide_at(at) {
                    Ok(index) => self.wide[index].1 = step,
                    Err(index) => {
                        engine_allocation::reserve(&mut self.wide, 1)?;
                        self.wide.insert(index, (at as u32, step));
                    }
                }
            }
        }
        Ok(())
    }

    /// Whether `cell`, recorded or not, is the cell held at `offset`.
    fn holds(&self, offset: u32, cell: Cell<'_>, recorded: bool) -> bool {
        let at = offset as usize;
        if !self.known(offset) || bit_of(&self.recorded, at) != recorded {
            return false;
        }
        match self.lane.holds_items(at, cell) {
            Some(holds) => holds,
            None => scalar(cell) == Some(self.lane.bits(at)),
        }
    }
}

/// Writes the cells of `column` of `table` at slots `start..start + extent`, the page's first,
/// into `held`.
pub(super) fn put_base(
    writer: &mut Writer,
    table: &Table,
    column: usize,
    kind: Kind,
    start: u32,
    extent: u32,
    held: &mut Held,
) -> io::Result<()> {
    let values = table
        .columns
        .get(column)
        .ok_or_else(|| invalid("A strip names a column past its table"))?;
    let cell = |offset: u32| {
        values
            .cell(start + offset)
            .ok_or_else(|| invalid("A strip's slot holds no cell"))
    };
    writer.varint(u64::from(extent))?;
    let recorded = |offset: u32| table.recorded(column, start + offset);
    if (0..extent).all(recorded) {
        writer.u8(0)?;
    } else {
        writer.u8(1)?;
        put_bits(writer, extent, |offset| !recorded(offset))?;
    }
    if optional(kind) {
        let mut cells = Vec::new();
        engine_allocation::reserve(&mut cells, extent as usize)?;
        for offset in 0..extent {
            cells.push(present(cell(offset)?));
        }
        put_bits(writer, extent, |offset| cells[offset as usize])?;
    }
    if let Some(last) = extent.checked_sub(1) {
        held.grow(last as usize, cell(0)?)?;
    }
    for offset in 0..extent {
        let cell = cell(offset)?;
        put_value(writer, cell)?;
        held.keep(offset, cell, recorded(offset))?;
    }
    Ok(())
}

/// Writes the rows of `table` at slots `start..start + extent`, the page's first; the least and
/// the most key among them.
pub(super) fn put_rows(
    writer: &mut Writer,
    table: &Table,
    start: u32,
    extent: u32,
) -> io::Result<Option<(u128, u128)>> {
    writer.varint(u64::from(extent))?;
    put_bits(writer, extent, |offset| table.live().get(start + offset))?;
    let mut keys: Option<(u128, u128)> = None;
    for offset in 0..extent {
        if let Some(key) = table.key(start + offset) {
            writer.varint(key as u64)?;
            writer.varint((key >> 64) as u64)?;
            keys = Some(keys.map_or((key, key), |(low, high)| (low.min(key), high.max(key))));
        }
    }
    Ok(keys)
}

/// One ordinal's changes of a strip as they are written: their headers, gathered until their
/// count is known, and the offset of the change before.
#[derive(Default)]
pub(super) struct Group {
    head: Writer,
    count: u64,
    fill: bool,
    previous: Option<u32>,
    /// A list's or bytes' new and held items, widened.
    scratch: Vec<u64>,
    old: Vec<u64>,
}

impl Group {
    /// Starts the strip's next group.
    pub(super) fn start(&mut self) {
        self.head = Writer::reusing(std::mem::take(&mut self.head).into_bytes());
        self.count = 0;
        self.fill = false;
        self.previous = None;
    }

    /// A change's header: the gap to `offset` from the change before, over `flags` of `width`
    /// bits.
    fn header(&mut self, offset: u32, width: u32, flags: u64) -> io::Result<()> {
        let gap = match self.previous {
            None => offset,
            Some(previous) if offset > previous => offset - previous - 1,
            Some(_) => {
                return Err(invalid(
                    "A strip's changes in an hour are not in slot order",
                ));
            }
        };
        self.head.varint(u64::from(gap) << width | flags)?;
        self.previous = Some(offset);
        self.count += 1;
        Ok(())
    }

    /// A fill of `cell`, leading the group: every slot of the block but the `except` offsets,
    /// ascending, takes it; `held` holds it for those of the first `extent`.
    pub(super) fn fill(
        &mut self,
        values: &mut Writer,
        except: &[u32],
        cell: Cell<'_>,
        extent: u32,
        held: &mut Held,
    ) -> io::Result<()> {
        if self.fill || self.count != 0 {
            return Err(invalid("A fill does not lead its hour's changes"));
        }
        self.fill = true;
        self.head
            .varint((except.len() as u64) << 1 | u64::from(!present(cell)))?;
        for (index, &offset) in except.iter().enumerate() {
            self.head.varint(u64::from(match index {
                0 => offset,
                _ => offset - except[index - 1] - 1,
            }))?;
        }
        put_value(values, cell)?;
        let mut skip = except.iter().copied().peekable();
        for offset in 0..extent {
            if skip.next_if_eq(&offset).is_none() {
                held.keep(offset, cell, true)?;
            }
        }
        Ok(())
    }

    /// The change of `slot`'s cell of `column` of `table`, at `offset`, against the cells
    /// `held` holds; none when it holds that cell.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn change(
        &mut self,
        values: &mut Writer,
        table: &Table,
        column: usize,
        slot: u32,
        offset: u32,
        held: &mut Held,
    ) -> io::Result<()> {
        let cell = table
            .columns
            .get(column)
            .and_then(|values| values.cell(slot))
            .ok_or_else(|| invalid("A strip's slot holds no cell"))?;
        let recorded = table.recorded(column, slot);
        if held.holds(offset, cell, recorded) {
            return Ok(());
        }
        let at = offset as usize;
        let known = held.known(offset);
        let step = match raw(cell) {
            Some(new) if known => {
                let step = new.wrapping_sub(held.lane.bits(at));
                let stored = zigzag(step.wrapping_sub(held.velocity(at)) as i64);
                (varint_len(stored) <= whole_len(cell)).then_some((step, stored))
            }
            _ => None,
        };
        let patch = if known && step.is_none() && widen(cell, &mut self.scratch)? {
            held.lane.widen_items(at, &mut self.old)?;
            let (new, old) = (&self.scratch, &self.old);
            let bytes = matches!(cell, Cell::Opaque(_));
            let (front, back) = kept(old, new);
            let size = varint_len(front as u64)
                + varint_len(back as u64)
                + items_len(&new[front..new.len() - back], bytes);
            (size < items_len(new, bytes)).then_some((front, back))
        } else {
            None
        };
        let mut flags = 0;
        if !recorded {
            flags |= UNRECORDED;
        }
        if !present(cell) {
            flags |= MISSING;
        }
        if step.is_some() || patch.is_some() {
            flags |= DELTA;
        }
        self.header(offset, FLAG_BITS, u64::from(flags))?;
        match (step, patch) {
            (Some((step, stored)), _) => {
                values.varint(stored)?;
                held.set_velocity(at, step)?;
            }
            (None, Some((front, back))) => {
                values.varint(front as u64)?;
                values.varint(back as u64)?;
                put_middle(values, cell, front, back)?;
            }
            (None, None) => {
                put_value(values, cell)?;
                held.set_velocity(at, 0)?;
            }
        }
        held.keep(offset, cell, recorded)
    }

    /// A row's change: its slot's key at `offset`, or none.
    pub(super) fn row(
        &mut self,
        values: &mut Writer,
        offset: u32,
        key: Option<u128>,
    ) -> io::Result<()> {
        self.header(offset, 1, u64::from(key.is_some()))?;
        if let Some(key) = key {
            values.varint(key as u64)?;
            values.varint((key >> 64) as u64)?;
        }
        Ok(())
    }

    /// Writes the group into a page's headers `head` at `ordinal`, `previous` the ordinal of
    /// the group before or the page's first: the changes it holds, a fill counted, none when
    /// it holds none and is not written.
    pub(super) fn close(
        &mut self,
        head: &mut Writer,
        previous: u32,
        ordinal: u32,
    ) -> io::Result<u32> {
        if self.count == 0 && !self.fill {
            return Ok(0);
        }
        let delta = ordinal
            .checked_sub(previous)
            .filter(|&delta| delta > 0)
            .ok_or_else(|| invalid("A strip's hours do not follow one another"))?;
        head.varint(u64::from(delta))?;
        head.varint(self.count << 1 | u64::from(self.fill))?;
        head.raw(self.head.bytes())?;
        Ok(u32::try_from(self.count + u64::from(self.fill)).unwrap_or(u32::MAX))
    }
}

/// A page's raw bytes, in `into`'s allocation: its cells at its first ordinal `base`, then the
/// length of its changes' headers `head`, the headers, then their `values`.
#[cfg(test)]
pub(super) fn assemble(
    base: &Writer,
    head: &Writer,
    values: &Writer,
    into: Vec<u8>,
) -> io::Result<Writer> {
    let mut page = Writer::reusing(into);
    page.raw(base.bytes())?;
    page.varint(head.bytes().len() as u64)?;
    page.raw(head.bytes())?;
    page.raw(values.bytes())?;
    Ok(page)
}

fn get_key(reader: &mut Reader<'_>) -> io::Result<u128> {
    let low = reader.varint()?;
    Ok(u128::from(low) | u128::from(reader.varint()?) << 64)
}

/// Items a list cell is read into.
#[derive(Default)]
pub(super) struct Items {
    u32s: Vec<u32>,
    u64s: Vec<u64>,
    except: Vec<u32>,
}

impl Items {
    /// Holds no items: what it held was dropped.
    pub(super) fn clear(&mut self) {
        (self.u32s, self.u64s, self.except) = (Vec::new(), Vec::new(), Vec::new());
    }
}

fn whole<T: Int>(reader: &mut Reader<'_>, present: bool) -> io::Result<Option<T>> {
    if present {
        Ok(Some(T::unwire(reader.varint()?)?))
    } else {
        Ok(None)
    }
}

/// A value of `kind` written whole, missing when not `present`, `left` bytes from the end of
/// its reader's: a list's items, read into `u32s` or `u64s`.
fn get_value<'c, 'p: 'c>(
    reader: &mut Reader<'p>,
    left: usize,
    kind: Kind,
    present: bool,
    (u32s, u64s): (&'c mut Vec<u32>, &'c mut Vec<u64>),
) -> io::Result<Cell<'c>> {
    if !present && !optional(kind) {
        return Err(invalid("A strip cell holds no value"));
    }
    Ok(match kind {
        Kind::U8 => Cell::U8(reader.u8()?),
        Kind::I8 => Cell::I8(reader.u8()? as i8),
        Kind::Bool => Cell::Bool(match reader.u8()? {
            0 => false,
            1 => true,
            other => return Err(invalid(format!("A strip's bool cell {other}"))),
        }),
        Kind::U16 => Cell::U16(u16::unwire(reader.varint()?)?),
        Kind::I16 => Cell::I16(i16::unwire(reader.varint()?)?),
        Kind::U32 => Cell::U32(u32::unwire(reader.varint()?)?),
        Kind::Bytes => Cell::Bytes(u32::unwire(reader.varint()?)?),
        Kind::I32 => Cell::I32(i32::unwire(reader.varint()?)?),
        Kind::U64 => Cell::U64(reader.varint()?),
        Kind::I64 => Cell::I64(i64::unwire(reader.varint()?)?),
        Kind::OptionalI32 => Cell::OptionalI32(whole(reader, present)?),
        Kind::OptionalU32 => Cell::OptionalU32(whole(reader, present)?),
        Kind::OptionalI64 => Cell::OptionalI64(whole(reader, present)?),
        Kind::OptionalU64 => Cell::OptionalU64(whole(reader, present)?),
        // Every item is a byte or more of what is left.
        Kind::U32List => {
            let count = reader.count()?;
            scratch(u32s, count.min(left))?;
            reader.ints(count, u32s)?;
            Cell::U32List(&u32s[..])
        }
        Kind::U64List => {
            let count = reader.count()?;
            scratch(u64s, count.min(left))?;
            reader.ints(count, u64s)?;
            Cell::U64List(&u64s[..])
        }
        Kind::Opaque => {
            let length = reader.count()?;
            Cell::Opaque(reader.raw(length)?)
        }
    })
}

fn missing() -> io::Error {
    invalid("A strip names a slot past its table")
}

fn replace<T: Copy + PartialEq>(values: &mut Paged<T>, at: usize, value: T) -> io::Result<bool> {
    // A cell already holding the value leaves its page shared.
    if *values.get(at).ok_or_else(missing)? == value {
        return Ok(false);
    }
    *values.get_mut(at)?.ok_or_else(missing)? = value;
    Ok(true)
}

fn replace_bool(values: &mut Bools, slot: u32, value: bool) -> io::Result<bool> {
    if *values.get(slot as usize).ok_or_else(missing)? == value {
        return Ok(false);
    }
    values.put(slot, value)?;
    Ok(true)
}

fn set_optional<T: Copy + PartialEq>(
    values: &mut Paged<T>,
    bits: &mut PagedBits,
    slot: u32,
    value: Option<T>,
) -> io::Result<bool> {
    let held = *values.get(slot as usize).ok_or_else(missing)?;
    Ok(match value {
        Some(value) => {
            let present = bits.get(slot);
            if held != value {
                *values.get_mut(slot as usize)?.ok_or_else(missing)? = value;
            }
            if !present {
                bits.put(slot, true)?;
            }
            !present || held != value
        }
        None => {
            let changed = bits.get(slot);
            if changed {
                bits.put(slot, false)?;
            }
            changed
        }
    })
}

fn set_items<T: Copy + Default + PartialEq>(
    values: &mut Lists<T>,
    slot: u32,
    items: &[T],
) -> io::Result<bool> {
    if *values.get(slot as usize).ok_or_else(missing)? == *items {
        return Ok(false);
    }
    values.rewrite(
        &[slot],
        |_| missing(),
        |_, _, out| {
            engine_allocation::reserve(out, items.len())?;
            out.extend_from_slice(items);
            Ok(())
        },
    )?;
    Ok(true)
}

/// Sets whether `slot`'s cell is recorded; whether that changed.
fn set_recorded(unrecorded: &mut Bits, slot: u32, recorded: bool) -> io::Result<bool> {
    if unrecorded.get(slot) != recorded {
        return Ok(false);
    }
    if recorded {
        unrecorded.clear(slot);
    } else {
        unrecorded.set(slot)?;
    }
    Ok(true)
}

/// Sets `slot`'s cell of `column` (its unrecorded slots `unrecorded`) to `cell`; whether it
/// held another.
pub(super) fn set(
    column: &mut Column,
    unrecorded: &mut Bits,
    slot: u32,
    cell: Cell<'_>,
    recorded: bool,
) -> io::Result<bool> {
    let at = slot as usize;
    let changed = match (column, cell) {
        (Column::U8(values), Cell::U8(value)) => replace(values, at, value)?,
        (Column::I8(values), Cell::I8(value)) => replace(values, at, value)?,
        (Column::U16(values), Cell::U16(value)) => replace(values, at, value)?,
        (Column::I16(values), Cell::I16(value)) => replace(values, at, value)?,
        (Column::U32(values), Cell::U32(value)) => replace(values, at, value)?,
        (Column::I32(values), Cell::I32(value)) => replace(values, at, value)?,
        (Column::U64(values), Cell::U64(value)) => replace(values, at, value)?,
        (Column::I64(values), Cell::I64(value)) => replace(values, at, value)?,
        (Column::Bool(values), Cell::Bool(value)) => replace_bool(values, slot, value)?,
        (Column::Bytes(values), Cell::Bytes(value)) => replace(values, at, value)?,
        (Column::OptionalI32(values, bits), Cell::OptionalI32(value)) => {
            set_optional(values, bits, slot, value)?
        }
        (Column::OptionalU32(values, bits), Cell::OptionalU32(value)) => {
            set_optional(values, bits, slot, value)?
        }
        (Column::OptionalI64(values, bits), Cell::OptionalI64(value)) => {
            set_optional(values, bits, slot, value)?
        }
        (Column::OptionalU64(values, bits), Cell::OptionalU64(value)) => {
            set_optional(values, bits, slot, value)?
        }
        (Column::U32List(values), Cell::U32List(items)) => set_items(values, slot, items)?,
        (Column::U64List(values), Cell::U64List(items)) => set_items(values, slot, items)?,
        (Column::Opaque(values), Cell::Opaque(bytes)) => set_items(values, slot, bytes)?,
        _ => return Err(invalid("A strip cell differs from its column's kind")),
    };
    Ok(set_recorded(unrecorded, slot, recorded)? || changed)
}

fn shifted<T: Copy>(
    values: &mut Paged<T>,
    slot: u32,
    delta: u64,
    bits: (fn(T) -> u64, fn(u64) -> T),
) -> io::Result<bool> {
    if delta == 0 {
        values.get(slot as usize).ok_or_else(missing)?;
        return Ok(false);
    }
    let held = values.get_mut(slot as usize)?.ok_or_else(missing)?;
    *held = bits.1(bits.0(*held).wrapping_add(delta));
    Ok(true)
}

/// Moves `slot`'s present integer cell of `column` by `delta`, wrapping at its width; whether it
/// changed.
pub(super) fn shift(
    column: &mut Column,
    unrecorded: &mut Bits,
    slot: u32,
    delta: u64,
    recorded: bool,
) -> io::Result<bool> {
    let present = |bits: &PagedBits| {
        if bits.get(slot) {
            Ok(())
        } else {
            Err(invalid("A strip moves a missing cell"))
        }
    };
    let changed = match column {
        Column::U16(values) => shifted(values, slot, delta, (u64::from, |raw| raw as u16))?,
        Column::U32(values) => shifted(values, slot, delta, (u64::from, |raw| raw as u32))?,
        Column::U64(values) => shifted(values, slot, delta, (|value| value, |raw| raw))?,
        Column::I16(values) => shifted(
            values,
            slot,
            delta,
            (|value| value as i64 as u64, |raw| raw as i16),
        )?,
        Column::I32(values) => shifted(
            values,
            slot,
            delta,
            (|value| value as i64 as u64, |raw| raw as i32),
        )?,
        Column::I64(values) => shifted(
            values,
            slot,
            delta,
            (|value| value as u64, |raw| raw as i64),
        )?,
        Column::OptionalU32(values, bits) => {
            present(bits)?;
            shifted(values, slot, delta, (u64::from, |raw| raw as u32))?
        }
        Column::OptionalU64(values, bits) => {
            present(bits)?;
            shifted(values, slot, delta, (|value| value, |raw| raw))?
        }
        Column::OptionalI32(values, bits) => {
            present(bits)?;
            shifted(
                values,
                slot,
                delta,
                (|value| value as i64 as u64, |raw| raw as i32),
            )?
        }
        Column::OptionalI64(values, bits) => {
            present(bits)?;
            shifted(
                values,
                slot,
                delta,
                (|value| value as u64, |raw| raw as i64),
            )?
        }
        _ => return Err(invalid("A strip moves a cell that is not an integer")),
    };
    Ok(set_recorded(unrecorded, slot, recorded)? || changed)
}

fn spliced<T: Copy + Default + PartialEq>(
    values: &mut Lists<T>,
    slot: u32,
    (front, back): (usize, usize),
    middle: &[T],
) -> io::Result<bool> {
    let held = values.get(slot as usize).ok_or_else(missing)?;
    let length = held.len();
    if front.checked_add(back).is_none_or(|kept| kept > length) {
        return Err(invalid("A strip keeps more of a cell than it holds"));
    }
    if held[front..length - back] == *middle {
        return Ok(false);
    }
    values.rewrite(
        &[slot],
        |_| missing(),
        |_, held, out| {
            engine_allocation::reserve(out, front + middle.len() + back)?;
            out.extend_from_slice(&held[..front]);
            out.extend_from_slice(middle);
            out.extend_from_slice(&held[length - back..]);
            Ok(())
        },
    )?;
    Ok(true)
}

/// Sets `slot`'s list or bytes cell of `column` to its first `front` and last `back` items
/// around `middle`'s; whether it held another.
#[allow(clippy::too_many_arguments)]
pub(super) fn splice(
    column: &mut Column,
    unrecorded: &mut Bits,
    slot: u32,
    kept: (usize, usize),
    middle: Cell<'_>,
    recorded: bool,
) -> io::Result<bool> {
    let changed = match (column, middle) {
        (Column::U32List(values), Cell::U32List(items)) => {
            spliced(values, slot, kept, items)?
        }
        (Column::U64List(values), Cell::U64List(items)) => {
            spliced(values, slot, kept, items)?
        }
        (Column::Opaque(values), Cell::Opaque(bytes)) => spliced(values, slot, kept, bytes)?,
        _ => return Err(invalid("A strip patches a cell that is not a list")),
    };
    Ok(set_recorded(unrecorded, slot, recorded)? || changed)
}

/// One cell a page sets, at its block's offset; or one for every offset but the exceptions.
pub(super) enum Write<'c> {
    Cell {
        offset: u32,
        cell: Cell<'c>,
        recorded: bool,
    },
    /// The integer held at `offset` moved by `delta`, wrapping.
    Delta {
        offset: u32,
        delta: u64,
        recorded: bool,
    },
    /// The list or bytes held at `offset`, its first `front` and last `back` items kept around
    /// `middle`'s.
    Patch {
        offset: u32,
        front: usize,
        back: usize,
        middle: Cell<'c>,
        recorded: bool,
    },
    Fill {
        except: &'c [u32],
        cell: Cell<'c>,
        recorded: bool,
    },
    Row {
        offset: u32,
        key: Option<u128>,
    },
}

/// Where a page's reading stands: its next group's header, past its ordinal, the end of the
/// headers, the group's first value, the group's ordinal, none past the last, and the last
/// ordinal the page's directory entry names, which no change passes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Position {
    pub at: usize,
    pub end: usize,
    pub value: usize,
    pub next: Option<u32>,
    pub last: u32,
}

fn short() -> io::Error {
    invalid("A strip's page ends inside its changes")
}

/// The group whose header starts at `at` of `bytes`, the headers ending at `end` and its
/// values starting at `value`, `previous` the ordinal before it, through `last`, the page's
/// last ordinal: none past the headers, where the values end too and `previous` is `last`.
fn next(
    bytes: &[u8],
    at: usize,
    end: usize,
    value: usize,
    previous: u32,
    last: u32,
) -> io::Result<Position> {
    if at == end {
        if value != bytes.len() {
            return Err(invalid("A strip's values outlast its changes"));
        }
        if previous != last {
            return Err(invalid(
                "A strip's changes end short of its page's last ordinal",
            ));
        }
        return Ok(Position {
            at,
            end,
            value,
            next: None,
            last,
        });
    }
    let mut reader = Reader::new(bytes.get(at..end).ok_or_else(short)?);
    let delta = reader.varint_u32()?;
    let next = previous
        .checked_add(delta)
        .filter(|&next| delta > 0 && next <= last)
        .ok_or_else(|| invalid("A strip's changes pass its page's ordinals"))?;
    Ok(Position {
        at: at + reader.at(),
        end,
        value,
        next: Some(next),
        last,
    })
}

/// The offset a change's `gap` leads to from `previous`, the change before's, which it
/// becomes, in a block of `block` slots.
fn step(previous: &mut Option<u32>, gap: u64, block: u32) -> io::Result<u32> {
    let gap = u32::try_from(gap).map_err(|_| missing())?;
    let offset = match *previous {
        None => Some(gap),
        Some(previous) => previous
            .checked_add(1)
            .and_then(|next| next.checked_add(gap)),
    }
    .filter(|&offset| offset < block)
    .ok_or_else(missing)?;
    *previous = Some(offset);
    Ok(offset)
}

/// Reads a page's cells at its first ordinal, `first`, every one of the `extent` slots its
/// block held there: `kind` is its column's, none for its rows; its integers' last
/// differences, `velocity`, start anew. Where its changes start, through `last`, the page's
/// last ordinal.
#[allow(clippy::too_many_arguments)]
pub(super) fn base(
    bytes: &[u8],
    kind: Option<Kind>,
    (first, last): (u32, u32),
    extent: u32,
    velocity: &mut Vec<u64>,
    items: &mut Items,
    write: &mut dyn FnMut(Write<'_>) -> io::Result<()>,
) -> io::Result<Position> {
    let mut reader = Reader::new(bytes);
    if reader.varint_u32()? != extent {
        return Err(invalid("A strip's page holds other than its block's slots"));
    }
    let width = extent.div_ceil(8) as usize;
    match kind {
        None => {
            let live = reader.raw(width)?;
            for offset in 0..extent {
                let key = bit(live, offset)
                    .then(|| get_key(&mut reader))
                    .transpose()?;
                write(Write::Row { offset, key })?;
            }
        }
        Some(kind) => {
            let unrecorded = match reader.u8()? {
                0 => None,
                1 => Some(reader.raw(width)?),
                other => return Err(invalid(format!("A strip's cells' record form {other}"))),
            };
            let presence = if optional(kind) {
                Some(reader.raw(width)?)
            } else {
                None
            };
            for offset in 0..extent {
                let present = presence.is_none_or(|bits| bit(bits, offset));
                let left = bytes.len() - reader.at();
                let Items { u32s, u64s, .. } = &mut *items;
                let cell = get_value(&mut reader, left, kind, present, (u32s, u64s))?;
                write(Write::Cell {
                    offset,
                    cell,
                    recorded: unrecorded.is_none_or(|bits| !bit(bits, offset)),
                })?;
            }
        }
    }
    let length = reader.count()?;
    let start = reader.at();
    let end = start
        .checked_add(length)
        .filter(|&end| end <= bytes.len())
        .ok_or_else(short)?;
    velocity.clear();
    next(bytes, start, end, end, first, last)
}

/// Reads a page's changes from `from` through ordinal `until`, of a block of `block` slots,
/// its integers' last differences `velocity`.
#[allow(clippy::too_many_arguments)]
pub(super) fn changes(
    bytes: &[u8],
    kind: Option<Kind>,
    block: u32,
    mut from: Position,
    until: u32,
    velocity: &mut Vec<u64>,
    items: &mut Items,
    write: &mut dyn FnMut(Write<'_>) -> io::Result<()>,
) -> io::Result<Position> {
    while let Some(ordinal) = from.next.filter(|&next| next <= until) {
        let mut head = Reader::new(bytes.get(from.at..from.end).ok_or_else(short)?);
        let rest = bytes.get(from.value..).ok_or_else(short)?;
        let mut values = Reader::new(rest);
        let group = head.varint()?;
        let mut previous = None;
        match kind {
            None => {
                if group & 1 != 0 {
                    return Err(invalid("A table's rows strip holds a fill"));
                }
                for _ in 0..group >> 1 {
                    let code = head.varint()?;
                    let offset = step(&mut previous, code >> 1, block)?;
                    let key = (code & 1 != 0).then(|| get_key(&mut values)).transpose()?;
                    write(Write::Row { offset, key })?;
                }
            }
            Some(kind) => {
                if group & 1 != 0 {
                    let fill = head.varint()?;
                    let count = usize::try_from(fill >> 1).map_err(|_| missing())?;
                    let Items { u32s, u64s, except } = &mut *items;
                    except.clear();
                    engine_allocation::reserve(except, count.min(block as usize))?;
                    let mut at = None;
                    for _ in 0..count {
                        except.push(step(&mut at, head.varint()?, block)?);
                    }
                    let left = rest.len() - values.at();
                    let cell = get_value(&mut values, left, kind, fill & 1 == 0, (u32s, u64s))?;
                    write(Write::Fill {
                        except: &except[..],
                        cell,
                        recorded: true,
                    })?;
                }
                for _ in 0..group >> 1 {
                    let code = head.varint()?;
                    let offset = step(&mut previous, code >> FLAG_BITS, block)?;
                    let flags = code as u32 & ((1 << FLAG_BITS) - 1);
                    let recorded = flags & UNRECORDED == 0;
                    let present = flags & MISSING == 0;
                    let at = offset as usize;
                    let left = rest.len() - values.at();
                    let Items { u32s, u64s, .. } = &mut *items;
                    if flags & DELTA == 0 {
                        let cell = get_value(&mut values, left, kind, present, (u32s, u64s))?;
                        if let Some(velocity) = velocity.get_mut(at) {
                            *velocity = 0;
                        }
                        write(Write::Cell {
                            offset,
                            cell,
                            recorded,
                        })?;
                    } else if !present {
                        return Err(invalid("A strip moves a missing cell"));
                    } else if list(kind) {
                        let front = values.count()?;
                        let back = values.count()?;
                        let left = rest.len() - values.at();
                        let middle = get_value(&mut values, left, kind, true, (u32s, u64s))?;
                        write(Write::Patch {
                            offset,
                            front,
                            back,
                            middle,
                            recorded,
                        })?;
                    } else {
                        if velocity.len() <= at {
                            engine_allocation::resize(velocity, block as usize, 0)?;
                        }
                        let delta = velocity[at].wrapping_add(unzigzag(values.varint()?) as u64);
                        velocity[at] = delta;
                        write(Write::Delta {
                            offset,
                            delta,
                            recorded,
                        })?;
                    }
                }
            }
        }
        from = next(
            bytes,
            from.at + head.at(),
            from.end,
            from.value + values.at(),
            ordinal,
            from.last,
        )?;
    }
    Ok(from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_whole_value_past_the_held_offsets_leaves_the_differences_held() {
        let mut held = Held::default();
        held.keep(0, Cell::U64(1), true).unwrap();
        held.set_velocity(0, 5).unwrap();
        held.set_velocity(1, 0).unwrap();
        held.keep(1, Cell::U64(2), true).unwrap();
        assert_eq!((held.velocity(0), held.velocity(1)), (5, 0));
        held.set_velocity(0, 0).unwrap();
        assert_eq!(held.velocity(0), 0);
    }
}
