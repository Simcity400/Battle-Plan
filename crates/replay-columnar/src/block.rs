//! One table's hours of one block, written once in final form: every kind of change held in
//! its own stream across the block's hours, so like values sit together for the compressor.
//!
//! An hour's changes are what a walker's [`Changes`] and table hold after it, never how a
//! writer chose to express them: the rows each column wrote (`W`), marked without a change
//! (`N`) or left unrecorded (`U`), each column's FILL, the deleted, inserted, touched and (for
//! a truncated table) read rows. A set is written as the rows that differ from the same set
//! the last time it was written, or listed whole when that is shorter. The changed rows are
//! the union of every column's `W` and `N` and a residual list, normally empty; or, in a block
//! written by rows, the changed rows as a set and each one's masks as their difference from
//! that row's last masks in the block. The encoder writes both and keeps the shorter.
//!
//! A numeric column's written values are one of: raw; the difference from the held value;
//! that difference less the row's last difference in the block. The encoder writes all three
//! and keeps the shortest. A list or opaque cell is a literal, a repeat of a literal earlier in
//! the block, an edit of the held cell (kept front and back, new middle), or same-length
//! item-wise differences from it; the encoder picks the shortest.
//!
//! Which tables and columns an hour names, and its op flags, come from the hour's record:
//! the decoder is given the record's [`Section`] for each present hour.
//!
//! Payload: present hours (varint) | flags (u8) | each column's model (u8: a numeric column's
//! value model, 0 for any other) | hours per group (varint) | each group of that many present
//! hours, the last of the rest: a bit per stream not empty in the group | each such stream's
//! length (varint) | the group's streams' bytes in stream order. A walker reads one group's
//! bytes at a time; what the streams carry from hour to hour runs on across groups.

use crate::{
    invalid,
    leaf::Reader,
    manifest::{self, Section},
    section::{Changes, column_bit, union, within},
    state::{Cell, Kind, ListItem, Table, Values},
};
use std::{collections::HashMap, io};

const READ: usize = 0;
const DELETED: usize = 1;
const INSERTED: usize = 2;
const KEYS: usize = 3;
const TOUCHED: usize = 4;
const RESIDUAL: usize = 5;
/// In a block written by rows ([`BY_ROWS`]): the changed rows, each one's change mask, and
/// the marks without a change of those that have them.
const CHANGED: usize = 6;
const MASKS: usize = 7;
const NOOPS: usize = 8;
const TABLE_STREAMS: usize = 9;

/// A block's header flag: its changed rows are written as each row's change masks rather than
/// each column's rows.
const BY_ROWS: u8 = 1;

const OP: usize = 0;
const W: usize = 1;
const N: usize = 2;
const U: usize = 3;
const EXCEPT: usize = 4;
const FILL: usize = 5;
const INSERT: usize = 6;
const VALUES: usize = 7;
const ARGS: usize = 8;
const ITEMS: usize = 9;
const COLUMN_STREAMS: usize = 10;

const HAS_W: u8 = 1;
const HAS_N: u8 = 2;
const HAS_U: u8 = 4;
const HAS_FILL: u8 = 8;

/// Numeric value models.
const RAW: u8 = 0;
const DELTA: u8 = 1;
const DELTA2: u8 = 2;
const MODELS: usize = 3;

/// List and opaque cell forms.
const LITERAL: u8 = 0;
const REPEAT: u8 = 1;
const EDIT: u8 = 2;
const DIFFERENCES: u8 = 3;

fn stream(offsets: &[usize], column: usize, which: usize) -> usize {
    offsets[column] + which
}

/// Where each column's streams start, and past the last, how many streams there are.
fn offsets(columns: usize) -> Vec<usize> {
    (0..=columns).map(|column| TABLE_STREAMS + column * COLUMN_STREAMS).collect()
}

// ---------------------------------------------------------------------------------------
// Leaf encodings

pub(crate) fn put_varint(out: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        out.push(value as u8 | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

pub(crate) fn varint_bytes(value: u64) -> usize {
    (64 - value.max(1).leading_zeros() as usize).div_ceil(7)
}

pub(crate) fn zig(value: i64) -> u64 {
    ((value << 1) ^ (value >> 63)) as u64
}

pub(crate) fn unzig(value: u64) -> i64 {
    ((value >> 1) as i64) ^ -((value & 1) as i64)
}

fn put_u128(out: &mut Vec<u8>, mut value: u128) {
    while value >= 0x80 {
        out.push(value as u8 | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

fn get_u128(reader: &mut Reader<'_>) -> io::Result<u128> {
    let mut value = 0u128;
    for shift in (0..128).step_by(7) {
        let byte = reader.u8()?;
        let part = u128::from(byte & 0x7f);
        if shift == 126 && part > 3 {
            return Err(invalid("A key varint overflows 128 bits"));
        }
        value |= part << shift;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err(invalid("A key varint overflows 128 bits"))
}

/// Ascending `list`, its first slot then each gap less one.
pub(crate) fn put_list(out: &mut Vec<u8>, list: &[u32]) {
    put_varint(out, list.len() as u64);
    let mut next = 0u32;
    for &slot in list {
        put_varint(out, u64::from(slot - next));
        next = slot + 1;
    }
}

pub(crate) fn get_list(reader: &mut Reader<'_>, into: &mut Vec<u32>) -> io::Result<()> {
    let count = reader.count()?;
    get_gaps(reader, count, into)
}

pub(crate) fn get_gaps(reader: &mut Reader<'_>, count: usize, into: &mut Vec<u32>) -> io::Result<()> {
    into.clear();
    if count > reader_left(reader) {
        return Err(invalid("A slot list holds more slots than its bytes"));
    }
    engine_allocation::reserve(into, count)?;
    let mut next = 0u64;
    for _ in 0..count {
        let slot = next + reader.varint()?;
        if slot > u64::from(u32::MAX) {
            return Err(invalid("A slot list passes the last slot"));
        }
        into.push(slot as u32);
        next = slot + 1;
    }
    Ok(())
}

fn reader_left(reader: &Reader<'_>) -> usize {
    reader.left()
}

/// The ascending symmetric difference of ascending `a` and `b`.
fn toggles(a: &[u32], b: &[u32], into: &mut Vec<u32>) {
    into.clear();
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Less => {
                into.push(a[i]);
                i += 1;
            }
            std::cmp::Ordering::Greater => {
                into.push(b[j]);
                j += 1;
            }
            std::cmp::Ordering::Equal => {
                i += 1;
                j += 1;
            }
        }
    }
    into.extend_from_slice(&a[i..]);
    into.extend_from_slice(&b[j..]);
}

fn list_bytes(list: &[u32]) -> usize {
    let mut next = 0u32;
    let mut bytes = varint_bytes(list.len() as u64);
    for &slot in list {
        bytes += varint_bytes(u64::from(slot - next));
        next = slot + 1;
    }
    bytes
}

/// `set` as the rows it differs from `previous` by, or whole when that is shorter; `previous`
/// becomes `set`.
pub(crate) fn put_set(out: &mut Vec<u8>, previous: &mut Vec<u32>, set: &[u32], scratch: &mut Vec<u32>) {
    toggles(previous, set, scratch);
    let (whole, by_change) = (list_bytes(set), list_bytes(scratch));
    let (list, flag) = if by_change <= whole {
        (&scratch[..], 0)
    } else {
        (set, 1)
    };
    put_varint(out, (list.len() as u64) << 1 | flag);
    let mut next = 0u32;
    for &slot in list {
        put_varint(out, u64::from(slot - next));
        next = slot + 1;
    }
    previous.clear();
    previous.extend_from_slice(set);
}

pub(crate) fn get_set(
    reader: &mut Reader<'_>,
    previous: &mut Vec<u32>,
    scratch: &mut Vec<u32>,
    merged: &mut Vec<u32>,
) -> io::Result<()> {
    let head = reader.varint()?;
    let count = usize::try_from(head >> 1).map_err(|_| invalid("A set passes the address space"))?;
    get_gaps(reader, count, scratch)?;
    if head & 1 == 1 {
        std::mem::swap(previous, scratch);
    } else {
        engine_allocation::reserve(merged, previous.len() + scratch.len())?;
        toggles(previous, scratch, merged);
        std::mem::swap(previous, merged);
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------
// Numeric cells: every integer kind as its bits, `None` for a missing optional value.

fn width(kind: Kind) -> u32 {
    match kind {
        Kind::U8 | Kind::I8 | Kind::Bool => 8,
        Kind::U16 | Kind::I16 => 16,
        Kind::U32 | Kind::I32 | Kind::Bytes | Kind::OptionalI32 | Kind::OptionalU32 => 32,
        _ => 64,
    }
}

fn mask(width: u32) -> u64 {
    if width == 64 { u64::MAX } else { (1 << width) - 1 }
}

fn signed(kind: Kind) -> bool {
    matches!(
        kind,
        Kind::I8 | Kind::I16 | Kind::I32 | Kind::I64 | Kind::OptionalI32 | Kind::OptionalI64
    )
}

fn optional(kind: Kind) -> bool {
    matches!(
        kind,
        Kind::OptionalI32 | Kind::OptionalU32 | Kind::OptionalI64 | Kind::OptionalU64
    )
}

fn numeric(kind: Kind) -> bool {
    !matches!(kind, Kind::U32List | Kind::U64List | Kind::Opaque)
}

fn extend(bits: u64, width: u32) -> i64 {
    let shift = 64 - width;
    ((bits << shift) as i64) >> shift
}

/// A numeric cell's bits; `None` for a missing value.
fn bits(cell: Cell<'_>) -> Option<u64> {
    Some(match cell {
        Cell::U8(value) => u64::from(value),
        Cell::I8(value) => value as u8 as u64,
        Cell::U16(value) => u64::from(value),
        Cell::I16(value) => value as u16 as u64,
        Cell::U32(value) | Cell::Bytes(value) => u64::from(value),
        Cell::I32(value) => value as u32 as u64,
        Cell::U64(value) => value,
        Cell::I64(value) => value as u64,
        Cell::Bool(value) => u64::from(value),
        Cell::OptionalI32(value) => value? as u32 as u64,
        Cell::OptionalU32(value) => u64::from(value?),
        Cell::OptionalI64(value) => value? as u64,
        Cell::OptionalU64(value) => value?,
        Cell::U32List(_) | Cell::U64List(_) | Cell::Opaque(_) => return None,
    })
}

fn cell_bits(table: &Table, column: usize, slot: u32) -> Option<u64> {
    table.column(column).and_then(|held| held.cell(slot)).and_then(bits)
}

/// A value of `kind`, raw: a signed one zigzagged; an optional one shifted over its presence,
/// past 64 bits for a 64-bit kind's widest values.
fn raw_code(kind: Kind, value: Option<u64>) -> u128 {
    let wire = |bits: u64| {
        u128::from(if signed(kind) {
            zig(extend(bits, width(kind)))
        } else {
            bits
        })
    };
    match (optional(kind), value) {
        (false, Some(bits)) => wire(bits),
        (true, None) => 0,
        (true, Some(bits)) => wire(bits) + 1,
        (false, None) => 0,
    }
}

/// A value code as `kind`'s streams hold it: only an optional 64-bit kind's passes 64 bits.
fn get_code(reader: &mut Reader<'_>, kind: Kind) -> io::Result<u128> {
    if optional(kind) && width(kind) == 64 {
        get_u128(reader)
    } else {
        reader.varint().map(u128::from)
    }
}

fn raw_value(kind: Kind, code: u128) -> io::Result<Option<u64>> {
    let code = if optional(kind) {
        match code {
            0 => return Ok(None),
            code => code - 1,
        }
    } else {
        code
    };
    let code = u64::try_from(code).map_err(|_| invalid("A value passes its column's width"))?;
    let width = width(kind);
    let bits = if signed(kind) {
        let value = unzig(code);
        if width < 64 && extend(value as u64, width) != value {
            return Err(invalid("A value passes its column's width"));
        }
        value as u64 & mask(width)
    } else {
        if code & !mask(width) != 0 {
            return Err(invalid("A value passes its column's width"));
        }
        code
    };
    if kind == Kind::Bool && bits > 1 {
        return Err(invalid("A bool value is neither 0 nor 1"));
    }
    Ok(Some(bits))
}

/// A difference at `width`, signed and zigzagged, shifted over presence for an optional kind.
fn difference_code(kind: Kind, difference: Option<u64>) -> u128 {
    match difference {
        None => 0,
        Some(bits) => {
            u128::from(zig(extend(bits & mask(width(kind)), width(kind)))) + u128::from(optional(kind))
        }
    }
}

fn difference_value(kind: Kind, code: u128) -> io::Result<Option<u64>> {
    let code = if optional(kind) {
        match code {
            0 => return Ok(None),
            code => code - 1,
        }
    } else {
        code
    };
    let code = u64::try_from(code).map_err(|_| invalid("A difference passes its column's width"))?;
    let value = unzig(code);
    let width = width(kind);
    if width < 64 && extend(value as u64, width) != value {
        return Err(invalid("A difference passes its column's width"));
    }
    Ok(Some(value as u64 & mask(width)))
}

// ---------------------------------------------------------------------------------------
// List and opaque items.

trait Item: Copy + Eq + Default + std::hash::Hash {
    fn put(self, out: &mut Vec<u8>);
    fn get(reader: &mut Reader<'_>) -> io::Result<Self>;
    fn bytes(self) -> usize;
    fn minus(self, other: Self) -> Self;
    fn plus(self, other: Self) -> Self;
    fn put_difference(self, out: &mut Vec<u8>);
    fn get_difference(reader: &mut Reader<'_>) -> io::Result<Self>;
    fn difference_bytes(self) -> usize;
    /// `count` items appended to `into`.
    fn get_many(reader: &mut Reader<'_>, count: usize, into: &mut Vec<Self>) -> io::Result<()> {
        engine_allocation::reserve(into, count)?;
        for _ in 0..count {
            into.push(Self::get(reader)?);
        }
        Ok(())
    }
    /// `old` with each item's difference added, appended to `into`.
    fn get_differences(reader: &mut Reader<'_>, old: &[Self], into: &mut Vec<Self>) -> io::Result<()> {
        engine_allocation::reserve(into, old.len())?;
        for &item in old {
            into.push(item.plus(Self::get_difference(reader)?));
        }
        Ok(())
    }
}

impl Item for u8 {
    fn put(self, out: &mut Vec<u8>) {
        out.push(self);
    }
    fn get(reader: &mut Reader<'_>) -> io::Result<Self> {
        reader.u8()
    }
    fn bytes(self) -> usize {
        1
    }
    fn minus(self, other: Self) -> Self {
        self.wrapping_sub(other)
    }
    fn plus(self, other: Self) -> Self {
        self.wrapping_add(other)
    }
    fn put_difference(self, out: &mut Vec<u8>) {
        out.push(self);
    }
    fn get_difference(reader: &mut Reader<'_>) -> io::Result<Self> {
        reader.u8()
    }
    fn difference_bytes(self) -> usize {
        // A zero byte costs the compressor little; count it as an eighth.
        if self == 0 { 0 } else { 8 }
    }
    fn get_many(reader: &mut Reader<'_>, count: usize, into: &mut Vec<Self>) -> io::Result<()> {
        engine_allocation::reserve(into, count)?;
        into.extend_from_slice(reader.raw(count)?);
        Ok(())
    }
    fn get_differences(reader: &mut Reader<'_>, old: &[Self], into: &mut Vec<Self>) -> io::Result<()> {
        let differences = reader.raw(old.len())?;
        engine_allocation::reserve(into, old.len())?;
        into.extend(old.iter().zip(differences).map(|(&item, &difference)| item.wrapping_add(difference)));
        Ok(())
    }
}

macro_rules! varint_item {
    ($kind:ty, $signed:ty) => {
        impl Item for $kind {
            fn put(self, out: &mut Vec<u8>) {
                put_varint(out, self as u64);
            }
            fn get(reader: &mut Reader<'_>) -> io::Result<Self> {
                <$kind>::try_from(reader.varint()?)
                    .map_err(|_| invalid("A list item passes its width"))
            }
            fn bytes(self) -> usize {
                varint_bytes(self as u64) * 8
            }
            fn minus(self, other: Self) -> Self {
                self.wrapping_sub(other)
            }
            fn plus(self, other: Self) -> Self {
                self.wrapping_add(other)
            }
            fn put_difference(self, out: &mut Vec<u8>) {
                put_varint(out, zig(self as $signed as i64));
            }
            fn get_difference(reader: &mut Reader<'_>) -> io::Result<Self> {
                let value = unzig(reader.varint()?);
                <$signed>::try_from(value)
                    .map(|value| value as $kind)
                    .map_err(|_| invalid("A list difference passes its width"))
            }
            fn difference_bytes(self) -> usize {
                varint_bytes(zig(self as $signed as i64)) * 8
            }
        }
    };
}
varint_item!(u32, i32);
varint_item!(u64, i64);

fn items_cost<T: Item>(items: &[T]) -> usize {
    items.iter().map(|&item| item.bytes()).sum()
}

/// How a list cell is written: its form and, for an edit, the held items it keeps at its
/// front and back.
#[derive(Clone, Copy)]
enum Form {
    Literal,
    Repeat(u32),
    Edit { front: usize, back: usize },
    Differences,
}

fn choose<T: Item>(new: &[T], old: Option<&[T]>, repeat: Option<u32>) -> Form {
    if let Some(index) = repeat {
        return Form::Repeat(index);
    }
    let literal = items_cost(new) + 8 * varint_bytes(new.len() as u64);
    let Some(old) = old else {
        return Form::Literal;
    };
    let front = new.iter().zip(old).take_while(|(a, b)| a == b).count();
    let room = new.len().min(old.len()) - front;
    let back = new
        .iter()
        .rev()
        .zip(old.iter().rev())
        .take(room)
        .take_while(|(a, b)| a == b)
        .count();
    let middle = &new[front..new.len() - back];
    let edit = items_cost(middle)
        + 8 * (varint_bytes(front as u64)
            + varint_bytes(back as u64)
            + varint_bytes(middle.len() as u64));
    let differences = if new.len() == old.len() && !middle.is_empty() {
        new.iter()
            .zip(old)
            .map(|(&a, &b)| a.minus(b).difference_bytes())
            .sum::<usize>()
            + new.len()
    } else {
        usize::MAX
    };
    if differences < edit && differences < literal {
        Form::Differences
    } else if edit < literal {
        Form::Edit { front, back }
    } else {
        Form::Literal
    }
}

// ---------------------------------------------------------------------------------------
// The encoder.

/// One column's streams and what its next hour is written against.
#[derive(Default)]
struct ColumnState {
    previous_w: Vec<u32>,
    previous_n: Vec<u32>,
    previous_except: Vec<u32>,
    /// Each row's last difference in the block (for [`DELTA2`]), by slot.
    last: Vec<u64>,
    /// A list column's literals in the block: by content hash, their indexes.
    repeats: HashMap<u64, Vec<u32>, foldhash::fast::FixedState>,
    literals: Vec<Box<[u8]>>,
}

/// The generic forms' streams and literals one cell is written to.
struct Generic<'g> {
    values: &'g mut Vec<u8>,
    args: &'g mut Vec<u8>,
    items: &'g mut Vec<u8>,
    repeats: &'g mut HashMap<u64, Vec<u32>, foldhash::fast::FixedState>,
    literals: &'g mut Vec<Box<[u8]>>,
}

fn generic<'g>(state: &'g mut ColumnState, streams: &'g mut [Vec<u8>], offsets: &[usize], column: usize) -> Generic<'g> {
    let base = stream(offsets, column, VALUES);
    let [values, args, items] = &mut streams[base..base + 3] else {
        unreachable!("three streams")
    };
    Generic {
        values,
        args,
        items,
        repeats: &mut state.repeats,
        literals: &mut state.literals,
    }
}

/// Writes one table's hours of one block.
pub struct TableEncoder {
    kinds: Vec<Kind>,
    offsets: Vec<usize>,
    /// The table as the decoder holds it before the hour.
    held: Table,
    hours: u32,
    previous_read: Vec<u32>,
    previous_touched: Vec<u32>,
    previous_key: u128,
    /// The changed rows' last set and each row's last masks, written by rows.
    previous_changed: Vec<u32>,
    row_written: Vec<u64>,
    row_noops: Vec<u64>,
    columns: Vec<ColumnState>,
    streams: Vec<Vec<u8>>,
    /// Each numeric column's candidate value streams, by model.
    models: Vec<[Vec<u8>; MODELS]>,
    /// Present hours per group, and at each group's end every stream's length, then every
    /// column's candidate value streams' lengths.
    group: u32,
    ends: Vec<usize>,
    scratch: Vec<u32>,
    sets: Sets,
}

#[derive(Default)]
struct Sets {
    w: Vec<Vec<u32>>,
    n: Vec<Vec<u32>>,
    u: Vec<Vec<u32>>,
    derived: Vec<u32>,
    residual: Vec<u32>,
    except: Vec<u32>,
    targets: Vec<u32>,
}

impl TableEncoder {
    /// An encoder of a block starting from `held`, the table as every walker holds it then.
    pub fn new(kinds: &[Kind], held: Table, group: u32) -> io::Result<Self> {
        if kinds.len() > 64 {
            return Err(invalid("A table of more than 64 columns has no change masks"));
        }
        if group == 0 {
            return Err(invalid("A block group holds no hours"));
        }
        let columns = kinds.len();
        let offsets = offsets(columns);
        Ok(Self {
            kinds: kinds.to_vec(),
            held,
            hours: 0,
            previous_read: Vec::new(),
            previous_touched: Vec::new(),
            previous_key: 0,
            previous_changed: Vec::new(),
            row_written: Vec::new(),
            row_noops: Vec::new(),
            columns: (0..columns).map(|_| ColumnState::default()).collect(),
            streams: (0..offsets[columns]).map(|_| Vec::new()).collect(),
            offsets,
            models: (0..columns).map(|_| Default::default()).collect(),
            group,
            ends: Vec::new(),
            scratch: Vec::new(),
            sets: Sets {
                w: vec![Vec::new(); columns],
                n: vec![Vec::new(); columns],
                u: vec![Vec::new(); columns],
                ..Sets::default()
            },
        })
    }

    /// The table as the decoder holds it after the hours written so far.
    pub fn held(&self) -> &Table {
        &self.held
    }

    /// Writes an hour with a section: `table` and `changes` as a walker holds them after it,
    /// `section` its record's entry, `truncated` whether it read the table in part.
    pub fn hour(
        &mut self,
        table: &Table,
        changes: &Changes,
        section: &Section,
        truncated: bool,
    ) -> io::Result<()> {
        self.hours += 1;
        let kinds = self.kinds.clone();
        let Self {
            held,
            streams,
            columns,
            models,
            scratch,
            sets,
            offsets,
            ..
        } = self;
        if truncated {
            put_set(&mut streams[READ], &mut self.previous_read, &changes.read, scratch);
        }
        if section.ops & manifest::DELETES != 0 {
            put_list(&mut streams[DELETED], &changes.deleted);
        } else if !changes.deleted.is_empty() {
            return Err(invalid("An hour deletes rows its record does not flag"));
        }
        if section.ops & manifest::INSERTS != 0 {
            put_list(&mut streams[INSERTED], &changes.inserted);
            for &slot in &changes.inserted {
                let key = table.key(slot).ok_or_else(|| invalid("An inserted row is not live"))?;
                put_u128(
                    &mut streams[KEYS],
                    zig128(key.wrapping_sub(self.previous_key) as i128),
                );
                self.previous_key = key;
            }
        } else if !changes.inserted.is_empty() {
            return Err(invalid("An hour inserts rows its record does not flag"));
        }
        if section.ops & manifest::TOUCHES != 0 {
            put_set(
                &mut streams[TOUCHED],
                &mut self.previous_touched,
                &changes.touched,
                scratch,
            );
        } else if !changes.touched.is_empty() {
            return Err(invalid("An hour touches rows its record does not flag"));
        }
        // The decoder's table before the hour, with the hour's deletes and inserts.
        for &slot in &changes.deleted {
            for state in columns.iter_mut() {
                if let Some(last) = state.last.get_mut(slot as usize) {
                    *last = 0;
                }
            }
        }
        held.delete(&changes.deleted)?;
        if !changes.inserted.is_empty() {
            let keys: Vec<u128> = changes
                .inserted
                .iter()
                .map(|&slot| table.key(slot).expect("checked live"))
                .collect();
            held.insert_keys(&changes.inserted, &keys)?;
            for column in 0..kinds.len() {
                for &slot in &changes.inserted {
                    if let Some(last) = columns[column].last.get_mut(slot as usize) {
                        *last = 0;
                    }
                }
                put_inserts(
                    kinds[column],
                    &mut columns[column],
                    streams,
                    offsets,
                    column,
                    table,
                    &changes.inserted,
                )?;
                copy(held, table, column, &changes.inserted)?;
            }
        }
        // Each column's rows, from the masks.
        for column in 0..kinds.len() {
            sets.w[column].clear();
            sets.n[column].clear();
            sets.u[column].clear();
        }
        for (at, &slot) in changes.changed.iter().enumerate() {
            let (mask, written) = (changes.masks[at], changes.written[at]);
            let mut bits = mask;
            while bits != 0 {
                let column = bits.trailing_zeros() as usize;
                bits &= bits - 1;
                if column >= kinds.len() {
                    return Err(invalid("A change mask names a column past the table"));
                }
                if written >> column & 1 == 1 {
                    sets.w[column].push(slot);
                } else {
                    sets.n[column].push(slot);
                }
            }
        }
        sets.derived.clear();
        for column in 0..kinds.len() {
            let unrecorded = &mut sets.u[column];
            let (written, inserted) = (&sets.w[column], &changes.inserted);
            let (mut a, mut b) = (written.iter().peekable(), inserted.iter().peekable());
            loop {
                let slot = match (a.peek(), b.peek()) {
                    (Some(&&x), Some(&&y)) if x < y => *a.next().unwrap(),
                    (Some(&&x), Some(&&y)) if y < x => *b.next().unwrap(),
                    (Some(_), Some(_)) => {
                        b.next();
                        *a.next().unwrap()
                    }
                    (Some(_), None) => *a.next().unwrap(),
                    (None, Some(_)) => *b.next().unwrap(),
                    (None, None) => break,
                };
                if !table.recorded(column, slot) {
                    unrecorded.push(slot);
                }
            }
        }
        for column in 0..kinds.len() {
            let filled = changes.filled.contains(&column);
            let active = !sets.w[column].is_empty()
                || !sets.n[column].is_empty()
                || !sets.u[column].is_empty()
                || filled;
            let named = section.columns.get(column / 64).is_some_and(|word| word >> (column % 64) & 1 == 1);
            if !named {
                if active {
                    return Err(invalid("An hour changes a column its record does not name"));
                }
                continue;
            }
            let state = &mut columns[column];
            let kind = kinds[column];
            let op = u8::from(!sets.w[column].is_empty()) * HAS_W
                | u8::from(!sets.n[column].is_empty()) * HAS_N
                | u8::from(!sets.u[column].is_empty()) * HAS_U
                | u8::from(filled) * HAS_FILL;
            streams[stream(offsets, column, OP)].push(op);
            if op & HAS_W != 0 {
                put_set(&mut streams[stream(offsets, column, W)], &mut state.previous_w, &sets.w[column], scratch);
            }
            if op & HAS_N != 0 {
                put_set(&mut streams[stream(offsets, column, N)], &mut state.previous_n, &sets.n[column], scratch);
            }
            if op & HAS_U != 0 {
                put_list(&mut streams[stream(offsets, column, U)], &sets.u[column]);
            }
            if filled {
                sets.targets.clear();
                let bit = column_bit(column);
                sets.targets.extend(
                    changes
                        .fill_slots
                        .iter()
                        .zip(&changes.fill_masks)
                        .filter(|(_, mask)| *mask & bit != 0)
                        .map(|(&slot, _)| slot),
                );
                sets.except.clear();
                let mut targets = sets.targets.iter().peekable();
                for slot in table.live().ones() {
                    if targets.next_if_eq(&&slot).is_none() {
                        sets.except.push(slot);
                    }
                }
                put_set(&mut streams[stream(offsets, column, EXCEPT)], &mut state.previous_except, &sets.except, scratch);
                let value = sets.targets.first().map(|&slot| (slot, table));
                put_fill(kind, &mut streams[stream(offsets, column, FILL)], value, column)?;
                copy(held, table, column, &sets.targets)?;
            }
            if op & HAS_W != 0 {
                let slots = &sets.w[column];
                if numeric(kind) {
                    let [raw, delta, delta2] = &mut models[column];
                    let width = width(kind);
                    if state.last.len() < table.slots() {
                        state.last.resize(table.slots(), 0);
                    }
                    for &slot in slots {
                        let new = cell_bits(table, column, slot);
                        put_u128(raw, raw_code(kind, new));
                        let difference = new.map(|new| {
                            new.wrapping_sub(cell_bits(held, column, slot).unwrap_or(0)) & mask(width)
                        });
                        put_u128(delta, difference_code(kind, difference));
                        let second = difference.map(|difference| {
                            let last = &mut state.last[slot as usize];
                            let second = difference.wrapping_sub(*last);
                            *last = difference;
                            second
                        });
                        put_u128(delta2, difference_code(kind, second));
                    }
                } else {
                    put_cells(state, streams, offsets, column, table, Some(&*held), slots)?;
                }
                copy(held, table, column, slots)?;
            }
            if op & HAS_U != 0 {
                held.unrecord(column, &sets.u[column])?;
            }
            for &slot in &sets.w[column] {
                sets.derived.push(slot);
            }
            for &slot in &sets.n[column] {
                sets.derived.push(slot);
            }
        }
        sets.derived.sort_unstable();
        sets.derived.dedup();
        sets.residual.clear();
        let mut derived = sets.derived.iter().peekable();
        for &slot in &changes.changed {
            if derived.next_if_eq(&&slot).is_none() {
                sets.residual.push(slot);
            }
        }
        if derived.next().is_some() {
            return Err(invalid("A change mask names a row the hour did not change"));
        }
        put_list(&mut streams[RESIDUAL], &sets.residual);
        // The same rows by row.
        put_set(&mut streams[CHANGED], &mut self.previous_changed, &changes.changed, scratch);
        if let Some(&last) = changes.changed.last()
            && self.row_written.len() <= last as usize
        {
            self.row_written.resize(last as usize + 1, 0);
            self.row_noops.resize(last as usize + 1, 0);
        }
        for (at, &slot) in changes.changed.iter().enumerate() {
            let (written, noops) = (changes.written[at], changes.masks[at] & !changes.written[at]);
            let slot = slot as usize;
            let differs = noops != self.row_noops[slot];
            put_varint(&mut streams[MASKS], (written ^ self.row_written[slot]) << 1 | u64::from(differs));
            if differs {
                put_varint(&mut streams[NOOPS], noops ^ self.row_noops[slot]);
            }
            self.row_written[slot] = written;
            self.row_noops[slot] = noops;
        }
        if self.hours % self.group == 0 {
            self.cut();
        }
        Ok(())
    }

    /// Ends a group: every stream's length now.
    fn cut(&mut self) {
        self.ends.extend(self.streams.iter().map(Vec::len));
        for models in &self.models {
            self.ends.extend(models.iter().map(Vec::len));
        }
    }

    /// The block's payload. `estimate` gives the stored size of candidate bytes, to choose
    /// each column's numeric model and how changed rows are written.
    pub fn finish(mut self, estimate: &mut dyn FnMut(&[u8]) -> usize) -> io::Result<Vec<u8>> {
        if self.hours % self.group != 0 {
            self.cut();
        }
        let count = self.streams.len();
        let width = count + MODELS * self.kinds.len();
        let mut modes = vec![RAW; self.kinds.len()];
        for (column, models) in self.models.iter_mut().enumerate() {
            if !numeric(self.kinds[column]) || models[0].is_empty() {
                continue;
            }
            let best = (0..MODELS)
                .min_by_key(|&model| estimate(&models[model]))
                .expect("three models");
            modes[column] = best as u8;
            let values = stream(&self.offsets, column, VALUES);
            self.streams[values] = std::mem::take(&mut models[best]);
            for ends in self.ends.chunks_exact_mut(width) {
                ends[values] = ends[count + MODELS * column + best];
            }
        }
        let mut by_columns = std::mem::take(&mut self.streams[RESIDUAL]);
        let residual = by_columns.len();
        for column in 0..self.kinds.len() {
            for which in [W, N] {
                by_columns.extend_from_slice(&self.streams[stream(&self.offsets, column, which)]);
            }
        }
        let mut by_rows = Vec::new();
        for which in [CHANGED, MASKS, NOOPS] {
            by_rows.extend_from_slice(&self.streams[which]);
        }
        // By rows, the residual rows are the changed rows' own.
        let mut dropped = vec![RESIDUAL];
        let flags = if estimate(&by_rows) < estimate(&by_columns) {
            for column in 0..self.kinds.len() {
                for which in [W, N] {
                    let index = stream(&self.offsets, column, which);
                    self.streams[index].clear();
                    dropped.push(index);
                }
            }
            BY_ROWS
        } else {
            by_columns.truncate(residual);
            self.streams[RESIDUAL] = by_columns;
            dropped.clear();
            for which in [CHANGED, MASKS, NOOPS] {
                self.streams[which].clear();
                dropped.push(which);
            }
            0
        };
        for ends in self.ends.chunks_exact_mut(width) {
            for &index in &dropped {
                ends[index] = 0;
            }
        }
        let total: usize = self.streams.iter().map(Vec::len).sum();
        let mut out = Vec::with_capacity(total + self.kinds.len() + 16);
        put_varint(&mut out, u64::from(self.hours));
        out.push(flags);
        out.extend_from_slice(&modes);
        put_varint(&mut out, u64::from(self.group));
        let mut starts = vec![0usize; count];
        let mut present = vec![0u8; count.div_ceil(8)];
        for ends in self.ends.chunks_exact(width) {
            present.fill(0);
            for index in 0..count {
                if ends[index] > starts[index] {
                    present[index / 8] |= 1 << (index % 8);
                }
            }
            out.extend_from_slice(&present);
            for index in 0..count {
                if ends[index] > starts[index] {
                    put_varint(&mut out, (ends[index] - starts[index]) as u64);
                }
            }
            for index in 0..count {
                out.extend_from_slice(&self.streams[index][starts[index]..ends[index]]);
                starts[index] = ends[index];
            }
        }
        Ok(out)
    }

    /// The decoder's table after the block: the encoder's, which the next block starts from.
    pub fn into_held(self) -> Table {
        self.held
    }
}

fn zig128(value: i128) -> u128 {
    ((value << 1) ^ (value >> 127)) as u128
}

fn unzig128(value: u128) -> i128 {
    ((value >> 1) as i128) ^ -((value & 1) as i128)
}

/// `table`'s cells of `column` at `slots`, written into `held`.
fn copy(held: &mut Table, table: &Table, column: usize, slots: &[u32]) -> io::Result<()> {
    if slots.is_empty() {
        return Ok(());
    }
    let mut buffer = Buffer::default();
    let source = table.column(column).ok_or_else(|| invalid("A column past the table"))?;
    buffer.clear();
    for &slot in slots {
        buffer.push_cell(source.cell(slot).ok_or_else(|| invalid("A cell past the table"))?);
    }
    held.set(column, slots, buffer.values(source.kind()))
}

fn put_fill(
    kind: Kind,
    out: &mut Vec<u8>,
    value: Option<(u32, &Table)>,
    column: usize,
) -> io::Result<()> {
    let Some((slot, table)) = value else {
        // A FILL that sets no row: its value reaches no cell.
        out.push(0);
        return Ok(());
    };
    out.push(1);
    let cell = table
        .column(column)
        .and_then(|held| held.cell(slot))
        .ok_or_else(|| invalid("A filled cell past the table"))?;
    match cell {
        Cell::U32List(items) => put_literal(out, items),
        Cell::U64List(items) => put_literal(out, items),
        Cell::Opaque(items) => put_literal(out, items),
        cell => put_u128(out, raw_code(kind, bits(cell))),
    }
    Ok(())
}

fn put_literal<T: Item>(out: &mut Vec<u8>, items: &[T]) {
    put_varint(out, items.len() as u64);
    for &item in items {
        item.put(out);
    }
}

fn put_inserts(
    kind: Kind,
    state: &mut ColumnState,
    streams: &mut [Vec<u8>],
    offsets: &[usize],
    column: usize,
    table: &Table,
    slots: &[u32],
) -> io::Result<()> {
    if numeric(kind) {
        let out = &mut streams[stream(offsets, column, INSERT)];
        for &slot in slots {
            put_u128(out, raw_code(kind, cell_bits(table, column, slot)));
        }
        Ok(())
    } else {
        put_cells(state, streams, offsets, column, table, None, slots)
    }
}

fn put_cells(
    state: &mut ColumnState,
    streams: &mut [Vec<u8>],
    offsets: &[usize],
    column: usize,
    table: &Table,
    held: Option<&Table>,
    slots: &[u32],
) -> io::Result<()> {
    let source = table.column(column).ok_or_else(|| invalid("A column past the table"))?;
    let old_column = held.and_then(|held| held.column(column));
    for &slot in slots {
        let new = source.cell(slot).ok_or_else(|| invalid("A cell past the table"))?;
        let old = old_column.and_then(|old| old.cell(slot));
        match (new, old) {
            (Cell::U32List(new), old) => {
                let old = old.and_then(|old| match old {
                    Cell::U32List(old) => Some(old),
                    _ => None,
                });
                put_cell(generic(state, streams, offsets, column), new, old);
            }
            (Cell::U64List(new), old) => {
                let old = old.and_then(|old| match old {
                    Cell::U64List(old) => Some(old),
                    _ => None,
                });
                put_cell(generic(state, streams, offsets, column), new, old);
            }
            (Cell::Opaque(new), old) => {
                let old = old.and_then(|old| match old {
                    Cell::Opaque(old) => Some(old),
                    _ => None,
                });
                put_cell(generic(state, streams, offsets, column), new, old);
            }
            _ => return Err(invalid("A list column holds another kind")),
        }
    }
    Ok(())
}

fn content_hash<T: Item>(items: &[T]) -> u64 {
    use std::hash::BuildHasher;
    foldhash::fast::FixedState::with_seed(0x6834_636f_6c34).hash_one(items)
}

fn as_bytes<T: Item>(items: &[T]) -> Vec<u8> {
    let mut out = Vec::with_capacity(items.len() * 2);
    for &item in items {
        item.put(&mut out);
    }
    out
}

fn put_cell<T: Item>(out: Generic<'_>, new: &[T], old: Option<&[T]>) {
    let Generic {
        values,
        args,
        items,
        repeats,
        literals,
    } = out;
    let hash = content_hash(new);
    let bytes = as_bytes(new);
    let repeat = repeats.get(&hash).and_then(|indexes| {
        indexes
            .iter()
            .copied()
            .find(|&index| *literals[index as usize] == *bytes)
    });
    match choose(new, old, repeat) {
        Form::Repeat(index) => {
            values.push(REPEAT);
            put_varint(args, u64::from(index));
        }
        Form::Literal => {
            values.push(LITERAL);
            put_varint(args, new.len() as u64);
            items.extend_from_slice(&bytes);
            let index = literals.len() as u32;
            literals.push(bytes.into_boxed_slice());
            repeats.entry(hash).or_default().push(index);
        }
        Form::Edit { front, back } => {
            values.push(EDIT);
            let middle = &new[front..new.len() - back];
            put_varint(args, front as u64);
            put_varint(args, back as u64);
            put_varint(args, middle.len() as u64);
            for &item in middle {
                item.put(items);
            }
        }
        Form::Differences => {
            values.push(DIFFERENCES);
            for (&a, &b) in new.iter().zip(old.expect("a held cell")) {
                a.minus(b).put_difference(items);
            }
        }
    }
}

// ---------------------------------------------------------------------------------------
// Typed values for a table's columns.

/// Cells collected for one [`Table::set`].
#[derive(Default)]
pub(crate) struct Buffer {
    u8s: Vec<u8>,
    i8s: Vec<i8>,
    u16s: Vec<u16>,
    i16s: Vec<i16>,
    u32s: Vec<u32>,
    i32s: Vec<i32>,
    u64s: Vec<u64>,
    i64s: Vec<i64>,
    present: Vec<u64>,
    count: usize,
    ends: Vec<u32>,
    items32: Vec<u32>,
    items64: Vec<u64>,
    bytes: Vec<u8>,
}

impl Buffer {
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
        self.count = 0;
        self.ends.clear();
        self.items32.clear();
        self.items64.clear();
        self.bytes.clear();
    }

    fn presence(&mut self, present: bool) {
        if self.count % 64 == 0 {
            self.present.push(0);
        }
        if present {
            *self.present.last_mut().expect("a word") |= 1 << (self.count % 64);
        }
        self.count += 1;
    }

    fn push_bits(&mut self, kind: Kind, value: Option<u64>) {
        let bits = value.unwrap_or(0);
        match kind {
            Kind::U8 | Kind::Bool => self.u8s.push(bits as u8),
            Kind::I8 => self.i8s.push(bits as u8 as i8),
            Kind::U16 => self.u16s.push(bits as u16),
            Kind::I16 => self.i16s.push(bits as u16 as i16),
            Kind::U32 | Kind::Bytes => self.u32s.push(bits as u32),
            Kind::I32 => self.i32s.push(bits as u32 as i32),
            Kind::U64 => self.u64s.push(bits),
            Kind::I64 => self.i64s.push(bits as i64),
            Kind::OptionalI32 => {
                self.presence(value.is_some());
                if value.is_some() {
                    self.i32s.push(bits as u32 as i32);
                }
            }
            Kind::OptionalU32 => {
                self.presence(value.is_some());
                if value.is_some() {
                    self.u32s.push(bits as u32);
                }
            }
            Kind::OptionalI64 => {
                self.presence(value.is_some());
                if value.is_some() {
                    self.i64s.push(bits as i64);
                }
            }
            Kind::OptionalU64 => {
                self.presence(value.is_some());
                if value.is_some() {
                    self.u64s.push(bits);
                }
            }
            Kind::U32List | Kind::U64List | Kind::Opaque => {}
        }
    }

    fn push_cell(&mut self, cell: Cell<'_>) {
        match cell {
            Cell::U32List(items) => {
                self.items32.extend_from_slice(items);
                self.ends.push(self.items32.len() as u32);
            }
            Cell::U64List(items) => {
                self.items64.extend_from_slice(items);
                self.ends.push(self.items64.len() as u32);
            }
            Cell::Opaque(items) => {
                self.bytes.extend_from_slice(items);
                self.ends.push(self.bytes.len() as u32);
            }
            cell => {
                let kind = cell_kind(cell);
                self.push_bits(kind, bits(cell));
            }
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
                items: &self.items32,
            },
            Kind::U64List => Values::U64List {
                ends: &self.ends,
                items: &self.items64,
            },
            Kind::Opaque => Values::Opaque {
                ends: &self.ends,
                items: &self.bytes,
            },
        }
    }
}

fn cell_kind(cell: Cell<'_>) -> Kind {
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

// ---------------------------------------------------------------------------------------
// The decoder.

/// One column's decoding state.
#[derive(Default)]
struct ColumnDecoder {
    previous_w: Vec<u32>,
    previous_n: Vec<u32>,
    previous_except: Vec<u32>,
    last: Vec<u64>,
    /// Each literal's place in the payload: its offset and item count.
    literals: Vec<(usize, usize)>,
}

/// Reads one table's hours of one block, applying each to the table a walker holds.
pub struct TableDecoder<'a> {
    kinds: Vec<Kind>,
    offsets: Vec<usize>,
    modes: Vec<u8>,
    /// The group's streams, and the payload's groups after it.
    streams: Vec<Reader<'a>>,
    rest: Reader<'a>,
    /// The payload, which repeats of literals in earlier groups read.
    payload: &'a [u8],
    group: u32,
    hours: u32,
    read_hours: u32,
    by_rows: bool,
    previous_read: Vec<u32>,
    previous_touched: Vec<u32>,
    previous_key: u128,
    previous_changed: Vec<u32>,
    row_written: Vec<u64>,
    row_noops: Vec<u64>,
    columns: Vec<ColumnDecoder>,
    scratch: DecodeScratch,
}

#[derive(Default)]
struct DecodeScratch {
    list: Vec<u32>,
    merged: Vec<u32>,
    keys: Vec<u128>,
    w: Vec<u32>,
    n: Vec<u32>,
    u: Vec<u32>,
    targets: Vec<u32>,
    positions: Vec<u32>,
    masks: Vec<u64>,
    /// This hour's changed rows, unordered, while the columns name them.
    rows: Rows,
    /// Written by rows: each column's written and marked rows this hour.
    row_w: Vec<Vec<u32>>,
    row_n: Vec<Vec<u32>>,
    buffer: Buffer,
    cell32: Vec<u32>,
    cell64: Vec<u64>,
    cell8: Vec<u8>,
}

/// The rows an hour's columns change, gathered by slot: a bit for each row named, the words
/// holding them, and each row's column bits, written and marked.
#[derive(Default)]
struct Rows {
    named: Vec<u64>,
    words: Vec<u32>,
    count: usize,
    by_slot: Vec<(u64, u64)>,
}

impl Rows {
    fn add(&mut self, slot: u32, mask: u64, written: u64) -> io::Result<()> {
        let at = slot as usize;
        if self.by_slot.len() <= at {
            engine_allocation::resize_default(&mut self.by_slot, at + 1)?;
            engine_allocation::resize_default(&mut self.named, (at + 1).div_ceil(64))?;
        }
        let (word, bit) = (at / 64, 1u64 << (at % 64));
        if self.named[word] & bit == 0 {
            if self.named[word] == 0 {
                engine_allocation::reserve(&mut self.words, 1)?;
                self.words.push(word as u32);
            }
            self.named[word] |= bit;
            self.count += 1;
        }
        let entry = &mut self.by_slot[at];
        entry.0 |= mask;
        entry.1 |= written;
        Ok(())
    }

    /// Every named row in slot order into `changes`, leaving none named.
    fn drain(&mut self, changes: &mut Changes) -> io::Result<()> {
        self.words.sort_unstable();
        engine_allocation::reserve(&mut changes.changed, self.count)?;
        engine_allocation::reserve(&mut changes.masks, self.count)?;
        engine_allocation::reserve(&mut changes.written, self.count)?;
        for &word in &self.words {
            let mut bits = std::mem::take(&mut self.named[word as usize]);
            while bits != 0 {
                let slot = word * 64 + bits.trailing_zeros();
                bits &= bits - 1;
                let (mask, written) = std::mem::take(&mut self.by_slot[slot as usize]);
                changes.changed.push(slot);
                changes.masks.push(mask);
                changes.written.push(written);
            }
        }
        self.words.clear();
        self.count = 0;
        Ok(())
    }
}

impl ColumnDecoder {
    fn copied(&self) -> io::Result<Self> {
        Ok(Self {
            previous_w: engine_allocation::copy_vec(&self.previous_w)?,
            previous_n: engine_allocation::copy_vec(&self.previous_n)?,
            previous_except: engine_allocation::copy_vec(&self.previous_except)?,
            last: engine_allocation::copy_vec(&self.last)?,
            literals: engine_allocation::copy_vec(&self.literals)?,
        })
    }
}

impl<'a> TableDecoder<'a> {
    /// The bytes its own state holds, its payload aside.
    pub fn held_bytes(&self) -> usize {
        let words = self.previous_read.capacity()
            + self.previous_touched.capacity()
            + self.previous_changed.capacity()
            + self
                .columns
                .iter()
                .map(|column| {
                    column.previous_w.capacity()
                        + column.previous_n.capacity()
                        + column.previous_except.capacity()
                        + 2 * column.last.capacity()
                        + 4 * column.literals.capacity()
                })
                .sum::<usize>();
        4 * words
            + 8 * (self.row_written.capacity() + self.row_noops.capacity())
            + self.streams.capacity() * std::mem::size_of::<Reader<'_>>()
            + self.offsets.capacity() * std::mem::size_of::<usize>()
            + self.kinds.capacity()
            + self.modes.capacity()
    }

    /// The decoder where it stands, over `to`, a copy of `from`, which its payload lies in.
    pub fn rebased<'b>(&self, from: &[u8], to: &'b [u8]) -> io::Result<TableDecoder<'b>> {
        let mut streams = engine_allocation::vec(self.streams.len())?;
        for stream in &self.streams {
            streams.push(stream.rebased(from, to)?);
        }
        let mut columns = engine_allocation::vec(self.columns.len())?;
        for column in &self.columns {
            columns.push(column.copied()?);
        }
        let count = self.kinds.len();
        Ok(TableDecoder {
            kinds: engine_allocation::copy_vec(&self.kinds)?,
            offsets: engine_allocation::copy_vec(&self.offsets)?,
            modes: engine_allocation::copy_vec(&self.modes)?,
            streams,
            rest: self.rest.rebased(from, to)?,
            payload: crate::leaf::rebased(self.payload, from, to)?,
            group: self.group,
            hours: self.hours,
            read_hours: self.read_hours,
            by_rows: self.by_rows,
            previous_read: engine_allocation::copy_vec(&self.previous_read)?,
            previous_touched: engine_allocation::copy_vec(&self.previous_touched)?,
            previous_key: self.previous_key,
            previous_changed: engine_allocation::copy_vec(&self.previous_changed)?,
            row_written: engine_allocation::copy_vec(&self.row_written)?,
            row_noops: engine_allocation::copy_vec(&self.row_noops)?,
            columns,
            scratch: DecodeScratch {
                row_w: engine_allocation::collect_vec((0..count).map(|_| Vec::new()))?,
                row_n: engine_allocation::collect_vec((0..count).map(|_| Vec::new()))?,
                ..DecodeScratch::default()
            },
        })
    }

    pub fn new(kinds: &[Kind], payload: &'a [u8]) -> io::Result<Self> {
        let mut reader = Reader::new(payload);
        let hours = reader.varint_u32()?;
        let flags = reader.u8()?;
        if flags & !BY_ROWS != 0 {
            return Err(invalid("A block names unknown flags"));
        }
        let modes = reader.raw(kinds.len())?.to_vec();
        if modes
            .iter()
            .zip(kinds)
            .any(|(&mode, &kind)| mode as usize >= MODELS || (!numeric(kind) && mode != RAW))
        {
            return Err(invalid("A column names an unknown value model"));
        }
        let group = reader.varint_u32()?;
        if group == 0 {
            return Err(invalid("A block group holds no hours"));
        }
        let offsets = offsets(kinds.len());
        let count = offsets[kinds.len()];
        let mut decoder = Self {
            kinds: kinds.to_vec(),
            offsets,
            modes,
            streams: (0..count).map(|_| Reader::new(&[])).collect(),
            rest: Reader::new(&payload[reader.at()..]),
            payload,
            group,
            hours,
            read_hours: 0,
            by_rows: flags & BY_ROWS != 0,
            previous_read: Vec::new(),
            previous_touched: Vec::new(),
            previous_key: 0,
            previous_changed: Vec::new(),
            row_written: Vec::new(),
            row_noops: Vec::new(),
            columns: (0..kinds.len()).map(|_| ColumnDecoder::default()).collect(),
            scratch: DecodeScratch {
                row_w: (0..kinds.len()).map(|_| Vec::new()).collect(),
                row_n: (0..kinds.len()).map(|_| Vec::new()).collect(),
                ..DecodeScratch::default()
            },
        };
        decoder.open_group()?;
        Ok(decoder)
    }

    /// Points the streams at the next group's bytes.
    fn open_group(&mut self) -> io::Result<()> {
        let count = self.streams.len();
        let rest = &mut self.rest;
        let present = rest.raw(count.div_ceil(8))?;
        let mut lengths = [0usize; 64 * COLUMN_STREAMS + TABLE_STREAMS];
        for (index, length) in lengths[..count].iter_mut().enumerate() {
            if present[index / 8] >> (index % 8) & 1 == 1 {
                *length = rest.count()?;
            }
        }
        for (stream, &length) in self.streams.iter_mut().zip(&lengths[..count]) {
            if length > rest.left() {
                return Err(invalid("A block stream passes its payload"));
            }
            *stream = Reader::new(rest.raw(length)?);
        }
        Ok(())
    }

    /// Hours with a section the block holds.
    pub fn hours(&self) -> u32 {
        self.hours
    }

    /// Applies the next present hour to `table`, into `changes`: `section` the hour's record
    /// entry, `truncated` whether it read the table in part.
    pub fn hour(
        &mut self,
        table: &mut Table,
        changes: &mut Changes,
        section: &Section,
        truncated: bool,
    ) -> io::Result<()> {
        if self.read_hours == self.hours {
            return Err(invalid("A block holds fewer hours of a table than its records name"));
        }
        if self.read_hours > 0 && self.read_hours % self.group == 0 {
            if self.streams.iter().any(|stream| stream.left() != 0) {
                return Err(invalid("A block group holds more than its hours"));
            }
            self.open_group()?;
        }
        self.read_hours += 1;
        let Self {
            kinds,
            offsets,
            modes,
            streams,
            payload,
            columns,
            scratch,
            ..
        } = self;
        let payload = *payload;
        changes.truncated = truncated;
        changes.read.clear();
        changes.changed.clear();
        changes.masks.clear();
        changes.written.clear();
        changes.deleted.clear();
        changes.deleted_keys.clear();
        changes.inserted.clear();
        changes.touched.clear();
        changes.filled.clear();
        changes.fill_slots.clear();
        changes.fill_masks.clear();
        if truncated {
            get_set(&mut streams[READ], &mut self.previous_read, &mut scratch.list, &mut scratch.merged)?;
            if self.previous_read.iter().any(|&slot| !table.live().get(slot)) {
                return Err(invalid("A partly read table reads a slot holding no row"));
            }
            engine_allocation::reserve(&mut changes.read, self.previous_read.len())?;
            changes.read.extend_from_slice(&self.previous_read);
        }
        if section.ops & manifest::DELETES != 0 {
            get_list(&mut streams[DELETED], &mut changes.deleted)?;
            if truncated && !within(&changes.deleted, &changes.read) {
                return Err(invalid("A partly read table deletes a slot it did not read"));
            }
            engine_allocation::reserve(&mut changes.deleted_keys, changes.deleted.len())?;
            changes
                .deleted_keys
                .extend(changes.deleted.iter().filter_map(|&slot| table.key(slot)));
            table.delete(&changes.deleted)?;
            for &slot in &changes.deleted {
                for column in columns.iter_mut() {
                    if let Some(last) = column.last.get_mut(slot as usize) {
                        *last = 0;
                    }
                }
            }
        }
        if section.ops & manifest::INSERTS != 0 {
            get_list(&mut streams[INSERTED], &mut changes.inserted)?;
            let extent = table.slots();
            let fresh = changes.inserted.iter().filter(|&&slot| slot as usize >= extent);
            for (at, &slot) in fresh.enumerate() {
                if slot as usize != extent + at {
                    return Err(invalid("An insert names a slot past those it adds"));
                }
            }
            scratch.keys.clear();
            engine_allocation::reserve(&mut scratch.keys, changes.inserted.len())?;
            for _ in &changes.inserted {
                let difference = unzig128(get_u128(&mut streams[KEYS])?);
                self.previous_key = self.previous_key.wrapping_add(difference as u128);
                scratch.keys.push(self.previous_key);
            }
            table.insert_keys(&changes.inserted, &scratch.keys)?;
            for column in 0..kinds.len() {
                for &slot in &changes.inserted {
                    if let Some(last) = columns[column].last.get_mut(slot as usize) {
                        *last = 0;
                    }
                }
                let kind = kinds[column];
                if numeric(kind) {
                    scratch.buffer.clear();
                    let reader = &mut streams[stream(offsets, column, INSERT)];
                    for _ in &changes.inserted {
                        let value = raw_value(kind, get_code(reader, kind)?)?;
                        scratch.buffer.push_bits(kind, value);
                    }
                    table.set(column, &changes.inserted, scratch.buffer.values(kind))?;
                } else {
                    let cells = Cells { state: &mut columns[column], streams, offsets, payload, column, edits: false };
                    cells.decode(kind, table, &changes.inserted)?;
                }
            }
        }
        if section.ops & manifest::TOUCHES != 0 {
            get_set(&mut streams[TOUCHED], &mut self.previous_touched, &mut scratch.list, &mut scratch.merged)?;
            engine_allocation::reserve(&mut changes.touched, self.previous_touched.len())?;
            changes.touched.extend_from_slice(&self.previous_touched);
        }
        // Rows changed, with the column bits each carries.
        scratch.positions.clear();
        scratch.masks.clear();
        let mut rows = std::mem::take(&mut scratch.rows);
        let by_rows = self.by_rows;
        if by_rows {
            get_set(&mut streams[CHANGED], &mut self.previous_changed, &mut scratch.list, &mut scratch.merged)?;
            for (w, n) in scratch.row_w.iter_mut().zip(&mut scratch.row_n) {
                w.clear();
                n.clear();
            }
            if let Some(&last) = self.previous_changed.last()
                && self.row_written.len() <= last as usize
            {
                engine_allocation::resize_default(&mut self.row_written, last as usize + 1)?;
                engine_allocation::resize_default(&mut self.row_noops, last as usize + 1)?;
            }
            let columns_mask = if kinds.len() == 64 { u64::MAX } else { (1 << kinds.len()) - 1 };
            let named = section.columns.first().copied().unwrap_or(0);
            engine_allocation::reserve(&mut changes.changed, self.previous_changed.len())?;
            engine_allocation::reserve(&mut changes.masks, self.previous_changed.len())?;
            engine_allocation::reserve(&mut changes.written, self.previous_changed.len())?;
            for &slot in &self.previous_changed {
                let code = streams[MASKS].varint()?;
                let at = slot as usize;
                let written = self.row_written[at] ^ (code >> 1);
                let noops = if code & 1 == 1 {
                    self.row_noops[at] ^ streams[NOOPS].varint()?
                } else {
                    self.row_noops[at]
                };
                if (written | noops) & !(columns_mask & named) != 0 || written & noops != 0 {
                    return Err(invalid("A row's change mask names a column its record does not"));
                }
                self.row_written[at] = written;
                self.row_noops[at] = noops;
                let mut bits = written | noops;
                while bits != 0 {
                    let column = bits.trailing_zeros() as usize;
                    bits &= bits - 1;
                    if written >> column & 1 == 1 {
                        scratch.row_w[column].push(slot);
                    } else {
                        scratch.row_n[column].push(slot);
                    }
                }
                changes.changed.push(slot);
                changes.masks.push(written | noops);
                changes.written.push(written);
            }
        }
        for column in 0..kinds.len() {
            let named = section.columns.get(column / 64).is_some_and(|word| word >> (column % 64) & 1 == 1);
            if !named {
                continue;
            }
            let kind = kinds[column];
            let bit = column_bit(column);
            let op = streams[stream(offsets, column, OP)].u8()?;
            let state = &mut columns[column];
            scratch.w.clear();
            scratch.n.clear();
            scratch.u.clear();
            if by_rows {
                if (op & HAS_W != 0) == scratch.row_w[column].is_empty()
                    || (op & HAS_N != 0) == scratch.row_n[column].is_empty()
                {
                    return Err(invalid("A column's op disagrees with its rows' change masks"));
                }
                std::mem::swap(&mut scratch.w, &mut scratch.row_w[column]);
                std::mem::swap(&mut scratch.n, &mut scratch.row_n[column]);
            } else {
                if op & HAS_W != 0 {
                    get_set(&mut streams[stream(offsets, column, W)], &mut state.previous_w, &mut scratch.list, &mut scratch.merged)?;
                    scratch.w.extend_from_slice(&state.previous_w);
                }
                if op & HAS_N != 0 {
                    get_set(&mut streams[stream(offsets, column, N)], &mut state.previous_n, &mut scratch.list, &mut scratch.merged)?;
                    scratch.n.extend_from_slice(&state.previous_n);
                }
            }
            if op & HAS_U != 0 {
                get_list(&mut streams[stream(offsets, column, U)], &mut scratch.u)?;
            }
            if op & HAS_FILL != 0 {
                get_set(
                    &mut streams[stream(offsets, column, EXCEPT)],
                    &mut state.previous_except,
                    &mut scratch.list,
                    &mut scratch.merged,
                )?;
                scratch.buffer.clear();
                let fill = &mut streams[stream(offsets, column, FILL)];
                let sets = fill.u8()?;
                match (sets, kind) {
                    (0, Kind::U32List | Kind::U64List | Kind::Opaque) => scratch.buffer.ends.push(0),
                    (0, kind) => scratch.buffer.push_bits(kind, (!optional(kind)).then_some(0)),
                    (1, Kind::U32List) => {
                        get_literal::<u32>(fill, &mut scratch.cell32)?;
                        scratch.buffer.items32.extend_from_slice(&scratch.cell32);
                        scratch.buffer.ends.push(scratch.buffer.items32.len() as u32);
                    }
                    (1, Kind::U64List) => {
                        get_literal::<u64>(fill, &mut scratch.cell64)?;
                        scratch.buffer.items64.extend_from_slice(&scratch.cell64);
                        scratch.buffer.ends.push(scratch.buffer.items64.len() as u32);
                    }
                    (1, Kind::Opaque) => {
                        get_literal::<u8>(fill, &mut scratch.cell8)?;
                        scratch.buffer.bytes.extend_from_slice(&scratch.cell8);
                        scratch.buffer.ends.push(scratch.buffer.bytes.len() as u32);
                    }
                    (1, kind) => {
                        let value = raw_value(kind, get_code(fill, kind)?)?;
                        scratch.buffer.push_bits(kind, value);
                    }
                    _ => return Err(invalid("A FILL's value marker is malformed")),
                }
                table.fill(column, &state.previous_except, scratch.buffer.values(kind), &mut scratch.targets)?;
                if truncated
                    && scratch.targets.iter().any(|slot| {
                        changes.read.binary_search(slot).is_err()
                            && changes.inserted.binary_search(slot).is_err()
                    })
                {
                    return Err(invalid("A partly read table fills a slot it did not read"));
                }
                union(
                    &mut changes.fill_slots,
                    &mut changes.fill_masks,
                    &scratch.targets,
                    bit,
                    &mut scratch.positions,
                    &mut scratch.masks,
                )?;
                engine_allocation::reserve(&mut changes.filled, 1)?;
                changes.filled.push(column);
            }
            if op & HAS_W != 0 {
                scratch.buffer.clear();
                let mut added = false;
                if numeric(kind) {
                    let reader = &mut streams[stream(offsets, column, VALUES)];
                    let mode = modes[column];
                    let width = width(kind);
                    if mode == DELTA2 && state.last.len() < table.slots() {
                        engine_allocation::resize_default(&mut state.last, table.slots())?;
                    }
                    // A required integer's differences add to its held cells in one pass.
                    if mode != RAW && !optional(kind) && !matches!(kind, Kind::Bool | Kind::Bytes) {
                        for &slot in &scratch.w {
                            let code = get_code(reader, kind)?;
                            let difference = match mode {
                                DELTA => difference_value(kind, code)?,
                                _ => difference_value(kind, code)?.map(|second| {
                                    let last = &mut state.last[slot as usize];
                                    *last = last.wrapping_add(second) & mask(width);
                                    *last
                                }),
                            };
                            scratch.buffer.push_bits(kind, difference);
                        }
                        table.add(column, &scratch.w, scratch.buffer.values(kind))?;
                        added = true;
                    } else {
                        let held = table.column(column);
                        let held_bits = |slot: u32| held.and_then(|held| held.cell(slot)).and_then(bits).unwrap_or(0);
                        for &slot in &scratch.w {
                            let code = get_code(reader, kind)?;
                            let value = match mode {
                                RAW => raw_value(kind, code)?,
                                DELTA => difference_value(kind, code)?
                                    .map(|difference| held_bits(slot).wrapping_add(difference) & mask(width)),
                                _ => difference_value(kind, code)?.map(|second| {
                                    let last = &mut state.last[slot as usize];
                                    let difference = last.wrapping_add(second) & mask(width);
                                    *last = difference;
                                    held_bits(slot).wrapping_add(difference) & mask(width)
                                }),
                            };
                            if kind == Kind::Bool && value.is_some_and(|value| value > 1) {
                                return Err(invalid("A bool value is neither 0 nor 1"));
                            }
                            scratch.buffer.push_bits(kind, value);
                        }
                    }
                } else {
                    let cells = Cells { state, streams, offsets, payload, column, edits: true };
                    cells.decode(kind, table, &scratch.w)?;
                    added = true;
                }
                if !added {
                    table.set(column, &scratch.w, scratch.buffer.values(kind))?;
                }
            }
            if op & HAS_U != 0 {
                if scratch.u.iter().any(|slot| {
                    scratch.w.binary_search(slot).is_err() && changes.inserted.binary_search(slot).is_err()
                }) {
                    return Err(invalid("Unrecorded cells name a slot the hour did not write"));
                }
                table.unrecord(column, &scratch.u)?;
            }
            if by_rows {
                continue;
            }
            for &slot in &scratch.w {
                rows.add(slot, bit, bit)?;
            }
            for &slot in &scratch.n {
                rows.add(slot, bit, 0)?;
            }
        }
        if by_rows {
            scratch.rows = rows;
            if truncated && !within(&changes.changed, &changes.read) {
                return Err(invalid("A partly read table changes a slot it did not read"));
            }
            return Ok(());
        }
        get_list(&mut streams[RESIDUAL], &mut scratch.list)?;
        for &slot in &scratch.list {
            rows.add(slot, 0, 0)?;
        }
        let drained = rows.drain(changes);
        scratch.rows = rows;
        drained?;
        if truncated && !within(&changes.changed, &changes.read) {
            return Err(invalid("A partly read table changes a slot it did not read"));
        }
        Ok(())
    }

    /// Refuses a block holding more hours or stream bytes than its records named.
    pub fn finish(&self) -> io::Result<()> {
        if self.read_hours != self.hours
            || self.rest.left() != 0
            || self.streams.iter().any(|stream| stream.left() != 0)
        {
            return Err(invalid("A block holds more than its records name"));
        }
        Ok(())
    }
}

fn get_literal<T: Item>(reader: &mut Reader<'_>, into: &mut Vec<T>) -> io::Result<()> {
    let count = reader.count()?;
    if count > reader.left() {
        return Err(invalid("A literal passes its stream"));
    }
    into.clear();
    engine_allocation::reserve(into, count)?;
    for _ in 0..count {
        into.push(T::get(reader)?);
    }
    Ok(())
}

/// One list or opaque column's cell streams, decoded straight into a table's pages.
struct Cells<'s, 'a> {
    state: &'s mut ColumnDecoder,
    streams: &'s mut [Reader<'a>],
    offsets: &'s [usize],
    payload: &'s [u8],
    column: usize,
    /// Whether a cell may name its held one; an inserted row's may not.
    edits: bool,
}

impl Cells<'_, '_> {
    fn decode(self, kind: Kind, table: &mut Table, slots: &[u32]) -> io::Result<()> {
        match kind {
            Kind::U32List => self.rewrite::<u32>(table, slots),
            Kind::U64List => self.rewrite::<u64>(table, slots),
            _ => self.rewrite::<u8>(table, slots),
        }
    }

    fn rewrite<T: Item + ListItem>(self, table: &mut Table, slots: &[u32]) -> io::Result<()> {
        let Self { state, streams, offsets, payload, column, edits } = self;
        table.rewrite::<T>(column, slots, |_, held, out| {
            let form = streams[stream(offsets, column, VALUES)].u8()?;
            decode_cell(state, streams, offsets, payload, column, form, edits.then_some(held), out)
        })
    }
}

#[allow(clippy::too_many_arguments)]
fn decode_cell<T: Item>(
    state: &mut ColumnDecoder,
    streams: &mut [Reader<'_>],
    offsets: &[usize],
    payload: &[u8],
    column: usize,
    form: u8,
    old: Option<&[T]>,
    into: &mut Vec<T>,
) -> io::Result<()> {
    let [args, items] = &mut streams[stream(offsets, column, ARGS)..=stream(offsets, column, ITEMS)] else {
        unreachable!("two streams")
    };
    match form {
        LITERAL => {
            let count = args.count()?;
            if count > items.left() {
                return Err(invalid("A literal passes its stream"));
            }
            // The items stream lies within the payload.
            let offset = items.rest().as_ptr() as usize - payload.as_ptr() as usize;
            T::get_many(items, count, into)?;
            engine_allocation::reserve(&mut state.literals, 1)?;
            state.literals.push((offset, count));
        }
        REPEAT => {
            let index = args.count()?;
            let &(offset, count) = state
                .literals
                .get(index)
                .ok_or_else(|| invalid("A repeat names no earlier literal"))?;
            T::get_many(&mut Reader::new(&payload[offset..]), count, into)?;
        }
        EDIT => {
            let old = old.ok_or_else(|| invalid("An edit names no held cell"))?;
            let (front, back, middle) = (args.count()?, args.count()?, args.count()?);
            if front.checked_add(back).is_none_or(|kept| kept > old.len()) || middle > items.left() {
                return Err(invalid("An edit keeps more than its held cell"));
            }
            engine_allocation::reserve(into, front + back + middle)?;
            into.extend_from_slice(&old[..front]);
            T::get_many(items, middle, into)?;
            into.extend_from_slice(&old[old.len() - back..]);
        }
        DIFFERENCES => {
            let old = old.ok_or_else(|| invalid("Differences name no held cell"))?;
            T::get_differences(items, old, into)?;
        }
        _ => return Err(invalid("A cell names an unknown form")),
    }
    Ok(())
}
