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
    state::{Bits, Cell, Column, Kind, Table},
};
use engine_allocation::credit::{Credit, MemoryBudget};
use std::{
    io,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicUsize, Ordering},
    },
};

/// The bytes a share asks its budget for at once, so most writes admit nothing.
const GRAIN: usize = 64 << 10;

/// What a cursor's shares draw on: its budget and what a move waited for, drawn first; and
/// what the shares hold, with the most they held or asked for at once since the move began.
#[derive(Debug)]
pub(super) struct Ledger {
    pub(super) budget: MemoryBudget,
    pub(super) spare: Mutex<Option<Credit>>,
    held: AtomicUsize,
    peak: AtomicUsize,
}

impl Ledger {
    pub(super) fn new(budget: MemoryBudget) -> Self {
        Self {
            budget,
            spare: Mutex::new(None),
            held: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
        }
    }

    /// A move begins: its most is what the shares hold.
    pub(super) fn begin(&self) {
        self.peak
            .store(self.held.load(Ordering::Acquire), Ordering::Release);
    }

    /// The most the shares held, or held and asked for, at once since the move began: kept as
    /// workers give back what they held, so a retry waits for all the move needed at once.
    pub(super) fn peak(&self) -> usize {
        self.peak.load(Ordering::Acquire)
    }

    fn holds(&self, bytes: usize) {
        let now = self
            .held
            .fetch_add(bytes, Ordering::AcqRel)
            .saturating_add(bytes);
        self.peak.fetch_max(now, Ordering::AcqRel);
    }

    fn asks(&self, bytes: usize) {
        let now = self.held.load(Ordering::Acquire).saturating_add(bytes);
        self.peak.fetch_max(now, Ordering::AcqRel);
    }

    fn drops(&self, bytes: usize) {
        self.held.fetch_sub(bytes, Ordering::AcqRel);
    }
}

/// Some of a cursor's memory, held exactly and admitted before it is allocated: none for a
/// lookup, which holds what it reads only while it answers.
#[derive(Debug, Default)]
pub(super) struct Share {
    ledger: Option<Arc<Ledger>>,
    credit: Option<Credit>,
    held: usize,
}

/// A share's budget had no more for it.
#[derive(Debug)]
struct Short;

impl std::fmt::Display for Short {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("A change-run index's cursor is short of memory")
    }
}

impl std::error::Error for Short {}

/// Whether `error` is a share's budget having no more for it.
pub(super) fn short_of(error: &io::Error) -> bool {
    error.get_ref().is_some_and(|error| error.is::<Short>())
}

impl Share {
    pub(super) fn of(ledger: &Arc<Ledger>) -> io::Result<Self> {
        Ok(Self {
            credit: Some(ledger.budget.admit(0)?),
            ledger: Some(Arc::clone(ledger)),
            held: 0,
        })
    }

    /// Admits `bytes` more before they are allocated: from the ledger's spare first, then from
    /// its budget in grains, else exactly. A budget with no more refuses ([`short_of`]).
    pub(super) fn take(&mut self, bytes: usize) -> io::Result<()> {
        let Some(need) = self.held.checked_add(bytes) else {
            if let Some(ledger) = &self.ledger {
                ledger.asks(usize::MAX);
            }
            return Err(io::Error::new(io::ErrorKind::WouldBlock, Short));
        };
        let (Some(credit), Some(ledger)) = (&mut self.credit, &self.ledger) else {
            self.held = need;
            return Ok(());
        };
        if need <= credit.bytes() {
            self.held = need;
            ledger.holds(bytes);
            return Ok(());
        }
        // Credit moves only under the spare's lock, counted before it moves: a refusal sees
        // every byte another share holds or is taking.
        let mut spare = ledger.spare.lock().unwrap_or_else(PoisonError::into_inner);
        ledger.holds(bytes);
        let retried = spare.is_some();
        if let Some(spare) = spare.as_mut() {
            let part = spare.bytes().min(need - credit.bytes());
            credit.absorb(spare.split_off(part));
        }
        // In a retry it takes exactly what it holds, so its credit is never more.
        let grain = if retried {
            need
        } else {
            need.checked_next_multiple_of(GRAIN).unwrap_or(need)
        };
        if need > credit.bytes() && credit.resize(grain).is_err() && credit.resize(need).is_err() {
            ledger.drops(bytes);
            return Err(io::Error::new(io::ErrorKind::WouldBlock, Short));
        }
        self.held = need;
        Ok(())
    }

    /// `bytes` it held are dropped.
    pub(super) fn give(&mut self, bytes: usize) {
        let dropped = bytes.min(self.held);
        self.held -= dropped;
        self.surplus(dropped, GRAIN);
    }

    /// Counts `dropped` bytes out of the ledger and gives back what its credit holds past what
    /// it holds: in a move retried on what it waited for, all of it to the move's spare in the
    /// same step, so no byte leaves demand before it is back in what the move waited for;
    /// otherwise what passes `slack` over the grain above what it holds, to its budget.
    fn surplus(&mut self, dropped: usize, slack: usize) {
        let (Some(credit), Some(ledger)) = (&mut self.credit, &self.ledger) else {
            return;
        };
        if credit.bytes() <= self.held {
            ledger.drops(dropped);
            return;
        }
        let mut spare = ledger.spare.lock().unwrap_or_else(PoisonError::into_inner);
        ledger.drops(dropped);
        if let Some(spare) = spare.as_mut() {
            spare.absorb(credit.split_off(credit.bytes() - self.held));
            return;
        }
        drop(spare);
        let keep = self
            .held
            .checked_next_multiple_of(GRAIN)
            .unwrap_or(self.held);
        if credit.bytes() > keep.saturating_add(slack) {
            let _ = credit.resize(keep);
        }
    }

    /// Holds `bytes`: more admitted, or less given back.
    pub(super) fn set(&mut self, bytes: usize) -> io::Result<()> {
        if bytes > self.held {
            self.take(bytes - self.held)
        } else {
            self.give(self.held - bytes);
            Ok(())
        }
    }

    /// Gives back what it holds past the grain above what it holds.
    pub(super) fn settle(&mut self) {
        self.surplus(0, 0);
    }

    /// Holds nothing: all it held was dropped.
    pub(super) fn clear(&mut self) {
        if let Some(ledger) = &self.ledger {
            ledger.drops(self.held);
        }
        self.held = 0;
        if let Some(credit) = &mut self.credit {
            let _ = credit.resize(0);
        }
    }
}

impl Drop for Share {
    /// What it still holds leaves its ledger's count with it.
    fn drop(&mut self) {
        if let Some(ledger) = &self.ledger {
            ledger.drops(self.held);
        }
    }
}

/// The bytes `cell`'s items take held.
pub(super) fn items_bytes(cell: Cell<'_>) -> usize {
    match cell {
        Cell::U32List(items) => size_of_val(items),
        Cell::U64List(items) => size_of_val(items),
        Cell::Opaque(bytes) => bytes.len(),
        _ => 0,
    }
}

/// Makes room in `into` for `count` items, admitted to `share` before it grows.
fn scratch<T>(into: &mut Vec<T>, count: usize, share: &mut Share) -> io::Result<()> {
    if count > into.capacity() {
        share.take((count - into.capacity()) * size_of::<T>())?;
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

/// The cells a column's open page last set, as its next changes are taken against: at each
/// offset whether a present value is known and recorded, a scalar's bits or a list's items,
/// and an integer's last difference.
#[derive(Default)]
pub(super) struct Held {
    known: Vec<bool>,
    recorded: Vec<bool>,
    values: Vec<u64>,
    velocity: Vec<u64>,
    items: Vec<Vec<u64>>,
    scratch: Vec<u64>,
}

impl Held {
    fn keep(&mut self, offset: u32, cell: Cell<'_>, recorded: bool) -> io::Result<()> {
        let at = offset as usize;
        if self.known.len() <= at {
            engine_allocation::resize(&mut self.known, at + 1, false)?;
            engine_allocation::resize(&mut self.recorded, at + 1, false)?;
            engine_allocation::resize(&mut self.values, at + 1, 0)?;
            engine_allocation::resize(&mut self.velocity, at + 1, 0)?;
        }
        self.known[at] = present(cell);
        self.recorded[at] = recorded;
        if let Some(bits) = scalar(cell) {
            self.values[at] = bits;
        } else if matches!(cell, Cell::U32List(_) | Cell::U64List(_) | Cell::Opaque(_)) {
            if self.items.len() <= at {
                engine_allocation::resize_default(&mut self.items, at + 1)?;
            }
            widen(cell, &mut self.items[at])?;
        }
        Ok(())
    }

    fn known(&self, offset: u32) -> bool {
        self.known.get(offset as usize).copied().unwrap_or(false)
    }

    /// Whether `cell`, recorded or not, is the cell held at `offset`.
    fn holds(&self, offset: u32, cell: Cell<'_>, recorded: bool) -> bool {
        let at = offset as usize;
        if !self.known(offset) || self.recorded[at] != recorded {
            return false;
        }
        let items = || self.items.get(at).map_or(&[][..], Vec::as_slice);
        match cell {
            Cell::U32List(new) => {
                items().len() == new.len()
                    && items().iter().zip(new).all(|(&a, &b)| a == u64::from(b))
            }
            Cell::U64List(new) => items() == new,
            Cell::Opaque(new) => {
                items().len() == new.len()
                    && items().iter().zip(new).all(|(&a, &b)| a == u64::from(b))
            }
            _ => scalar(cell) == Some(self.values[at]),
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
                let step = new.wrapping_sub(held.values[at]);
                let stored = zigzag(step.wrapping_sub(held.velocity[at]) as i64);
                (varint_len(stored) <= whole_len(cell)).then_some((step, stored))
            }
            _ => None,
        };
        let patch = if known && step.is_none() && widen(cell, &mut held.scratch)? {
            let new = &held.scratch;
            let old = held.items.get(at).map_or(&[][..], Vec::as_slice);
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
                held.velocity[at] = step;
            }
            (None, Some((front, back))) => {
                values.varint(front as u64)?;
                values.varint(back as u64)?;
                put_middle(values, cell, front, back)?;
            }
            (None, None) => {
                put_value(values, cell)?;
                if let Some(velocity) = held.velocity.get_mut(at) {
                    *velocity = 0;
                }
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

/// Items a list cell is read into, admitted to `share` as they grow.
#[derive(Default)]
pub(super) struct Items {
    u32s: Vec<u32>,
    u64s: Vec<u64>,
    except: Vec<u32>,
    pub(super) share: Share,
}

impl Items {
    pub(super) fn with(share: Share) -> Self {
        Self {
            share,
            ..Self::default()
        }
    }

    /// Holds no items: what it held was dropped.
    pub(super) fn clear(&mut self) {
        (self.u32s, self.u64s, self.except) = (Vec::new(), Vec::new(), Vec::new());
        self.share.clear();
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
/// its reader's: a list's items, read into `u32s` or `u64s`, admitted to `share` first.
fn get_value<'c, 'p: 'c>(
    reader: &mut Reader<'p>,
    left: usize,
    kind: Kind,
    present: bool,
    (u32s, u64s): (&'c mut Vec<u32>, &'c mut Vec<u64>),
    share: &mut Share,
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
            scratch(u32s, count.min(left), share)?;
            reader.ints(count, u32s)?;
            Cell::U32List(&u32s[..])
        }
        Kind::U64List => {
            let count = reader.count()?;
            scratch(u64s, count.min(left), share)?;
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

fn replace<T: PartialEq>(held: Option<&mut T>, value: T) -> io::Result<bool> {
    let held = held.ok_or_else(missing)?;
    if *held == value {
        return Ok(false);
    }
    *held = value;
    Ok(true)
}

fn set_optional<T: Copy + PartialEq>(
    values: &mut [T],
    bits: &mut Bits,
    slot: u32,
    value: Option<T>,
) -> io::Result<bool> {
    let held = values.get_mut(slot as usize).ok_or_else(missing)?;
    Ok(match value {
        Some(value) => {
            let changed = !bits.get(slot) || *held != value;
            *held = value;
            bits.set(slot)?;
            changed
        }
        None => {
            let changed = bits.get(slot);
            bits.clear(slot);
            changed
        }
    })
}

fn set_items<T: Copy + PartialEq>(
    values: &mut [Box<[T]>],
    slot: u32,
    items: &[T],
    share: &mut Share,
) -> io::Result<bool> {
    let held = values.get_mut(slot as usize).ok_or_else(missing)?;
    if **held == *items {
        return Ok(false);
    }
    share.take(size_of_val(items))?;
    let old = size_of_val(&**held);
    *held = engine_allocation::copy_vec(items)?.into_boxed_slice();
    share.give(old);
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

/// Sets `slot`'s cell of `column` (its unrecorded slots `unrecorded`) to `cell`, its items
/// admitted to `share`; whether it held another.
pub(super) fn set(
    column: &mut Column,
    unrecorded: &mut Bits,
    slot: u32,
    cell: Cell<'_>,
    recorded: bool,
    share: &mut Share,
) -> io::Result<bool> {
    let at = slot as usize;
    let changed = match (column, cell) {
        (Column::U8(values), Cell::U8(value)) => replace(values.get_mut(at), value)?,
        (Column::I8(values), Cell::I8(value)) => replace(values.get_mut(at), value)?,
        (Column::U16(values), Cell::U16(value)) => replace(values.get_mut(at), value)?,
        (Column::I16(values), Cell::I16(value)) => replace(values.get_mut(at), value)?,
        (Column::U32(values), Cell::U32(value)) => replace(values.get_mut(at), value)?,
        (Column::I32(values), Cell::I32(value)) => replace(values.get_mut(at), value)?,
        (Column::U64(values), Cell::U64(value)) => replace(values.get_mut(at), value)?,
        (Column::I64(values), Cell::I64(value)) => replace(values.get_mut(at), value)?,
        (Column::Bool(values), Cell::Bool(value)) => replace(values.get_mut(at), value)?,
        (Column::Bytes(values), Cell::Bytes(value)) => replace(values.get_mut(at), value)?,
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
        (Column::U32List(values), Cell::U32List(items)) => set_items(values, slot, items, share)?,
        (Column::U64List(values), Cell::U64List(items)) => set_items(values, slot, items, share)?,
        (Column::Opaque(values), Cell::Opaque(bytes)) => set_items(values, slot, bytes, share)?,
        _ => return Err(invalid("A strip cell differs from its column's kind")),
    };
    Ok(set_recorded(unrecorded, slot, recorded)? || changed)
}

fn shifted<T: Copy>(
    values: &mut [T],
    slot: u32,
    delta: u64,
    bits: (fn(T) -> u64, fn(u64) -> T),
) -> io::Result<bool> {
    let held = values.get_mut(slot as usize).ok_or_else(missing)?;
    *held = bits.1(bits.0(*held).wrapping_add(delta));
    Ok(delta != 0)
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
    let present = |bits: &Bits| {
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

fn spliced<T: Copy + PartialEq>(
    values: &mut [Box<[T]>],
    slot: u32,
    (front, back): (usize, usize),
    middle: &[T],
    share: &mut Share,
) -> io::Result<bool> {
    let held = values.get_mut(slot as usize).ok_or_else(missing)?;
    let length = held.len();
    if front.checked_add(back).is_none_or(|kept| kept > length) {
        return Err(invalid("A strip keeps more of a cell than it holds"));
    }
    if held[front..length - back] == *middle {
        return Ok(false);
    }
    let count = front + middle.len() + back;
    share.take(count * size_of::<T>())?;
    let mut items = engine_allocation::vec(count)?;
    items.extend_from_slice(&held[..front]);
    items.extend_from_slice(middle);
    items.extend_from_slice(&held[length - back..]);
    let old = size_of_val(&**held);
    *held = items.into_boxed_slice();
    share.give(old);
    Ok(true)
}

/// Sets `slot`'s list or bytes cell of `column` to its first `front` and last `back` items
/// around `middle`'s, its items admitted to `share`; whether it held another.
#[allow(clippy::too_many_arguments)]
pub(super) fn splice(
    column: &mut Column,
    unrecorded: &mut Bits,
    slot: u32,
    kept: (usize, usize),
    middle: Cell<'_>,
    recorded: bool,
    share: &mut Share,
) -> io::Result<bool> {
    let changed = match (column, middle) {
        (Column::U32List(values), Cell::U32List(items)) => {
            spliced(values, slot, kept, items, share)?
        }
        (Column::U64List(values), Cell::U64List(items)) => {
            spliced(values, slot, kept, items, share)?
        }
        (Column::Opaque(values), Cell::Opaque(bytes)) => spliced(values, slot, kept, bytes, share)?,
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
                let Items {
                    u32s, u64s, share, ..
                } = &mut *items;
                let cell = get_value(&mut reader, left, kind, present, (u32s, u64s), share)?;
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
                    let Items {
                        u32s,
                        u64s,
                        except,
                        share,
                    } = &mut *items;
                    except.clear();
                    engine_allocation::reserve(except, count.min(block as usize))?;
                    let mut at = None;
                    for _ in 0..count {
                        except.push(step(&mut at, head.varint()?, block)?);
                    }
                    let left = rest.len() - values.at();
                    let cell =
                        get_value(&mut values, left, kind, fill & 1 == 0, (u32s, u64s), share)?;
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
                    let Items {
                        u32s, u64s, share, ..
                    } = &mut *items;
                    if flags & DELTA == 0 {
                        let cell =
                            get_value(&mut values, left, kind, present, (u32s, u64s), share)?;
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
                        let middle = get_value(&mut values, left, kind, true, (u32s, u64s), share)?;
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
