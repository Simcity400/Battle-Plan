//! Every table's current rows: one typed column per field, indexed by the slot the
//! writer assigned the row. Runs apply in place, column by column; the writer certifies
//! each change, so nothing here compares values. An error leaves the table partly
//! changed: its reader starts again from the next restart.

use crate::{
    invalid,
    leaf::{Int, Le, SLOT_WORDS, Writer},
    section::{PatchItems, Patches},
};
use std::{collections::HashMap, io};

#[cfg(test)]
mod tests;

/// A field's cell type, as the recording's schema names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    U8,
    I8,
    U16,
    I16,
    U32,
    I32,
    U64,
    I64,
    Bool,
    OptionalI32,
    OptionalU32,
    OptionalI64,
    OptionalU64,
    /// A dictionary id.
    Bytes,
    U32List,
    U64List,
    Opaque,
}

/// Slots, one bit each; bits past the end are clear. Every bit names a `u32` slot.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Bits {
    words: Vec<u64>,
}

impl Bits {
    pub fn from_words(words: Vec<u64>) -> io::Result<Self> {
        if words.len() > SLOT_WORDS {
            return Err(invalid("Slot bits pass the last slot"));
        }
        Ok(Self { words })
    }

    pub fn words(&self) -> &[u64] {
        &self.words
    }

    pub fn get(&self, bit: u32) -> bool {
        self.words
            .get(bit as usize / 64)
            .is_some_and(|word| word >> (bit % 64) & 1 == 1)
    }

    pub fn count(&self) -> usize {
        self.words
            .iter()
            .map(|word| word.count_ones() as usize)
            .sum()
    }

    /// The set bits, ascending.
    pub fn ones(&self) -> impl Iterator<Item = u32> + '_ {
        self.words.iter().enumerate().flat_map(|(index, &word)| {
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

    fn set(&mut self, bit: u32) -> io::Result<()> {
        let word = bit as usize / 64;
        if word >= self.words.len() {
            engine_allocation::resize_default(&mut self.words, word + 1)?;
        }
        self.words[word] |= 1 << (bit % 64);
        Ok(())
    }

    fn clear(&mut self, bit: u32) {
        if let Some(word) = self.words.get_mut(bit as usize / 64) {
            *word &= !(1 << (bit % 64));
        }
    }
}

/// One field's values at every slot. An optional field keeps a presence bit per slot,
/// so a missing value is never read as zero.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Column {
    U8(Vec<u8>),
    I8(Vec<i8>),
    U16(Vec<u16>),
    I16(Vec<i16>),
    U32(Vec<u32>),
    I32(Vec<i32>),
    U64(Vec<u64>),
    I64(Vec<i64>),
    Bool(Vec<bool>),
    OptionalI32(Vec<i32>, Bits),
    OptionalU32(Vec<u32>, Bits),
    OptionalI64(Vec<i64>, Bits),
    OptionalU64(Vec<u64>, Bits),
    Bytes(Vec<u32>),
    U32List(Vec<Box<[u32]>>),
    U64List(Vec<Box<[u64]>>),
    Opaque(Vec<Box<[u8]>>),
}

/// The values one run carries, in slot order. Optional kinds carry a presence bit per
/// slot, then values for the present slots only. Lists and opaque cells carry each
/// cell's end offset into one flat array of items.
#[derive(Clone, Copy, Debug)]
pub enum Values<'a> {
    U8(&'a [u8]),
    I8(&'a [i8]),
    U16(&'a [u16]),
    I16(&'a [i16]),
    U32(&'a [u32]),
    I32(&'a [i32]),
    U64(&'a [u64]),
    I64(&'a [i64]),
    /// One byte per value, 0 or 1.
    Bool(&'a [u8]),
    OptionalI32 {
        present: &'a [u64],
        values: &'a [i32],
    },
    OptionalU32 {
        present: &'a [u64],
        values: &'a [u32],
    },
    OptionalI64 {
        present: &'a [u64],
        values: &'a [i64],
    },
    OptionalU64 {
        present: &'a [u64],
        values: &'a [u64],
    },
    Bytes(&'a [u32]),
    U32List {
        ends: &'a [u32],
        items: &'a [u32],
    },
    U64List {
        ends: &'a [u32],
        items: &'a [u64],
    },
    Opaque {
        ends: &'a [u32],
        items: &'a [u8],
    },
}

/// One slot's value of one column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cell<'a> {
    U8(u8),
    I8(i8),
    U16(u16),
    I16(i16),
    U32(u32),
    I32(i32),
    U64(u64),
    I64(i64),
    Bool(bool),
    OptionalI32(Option<i32>),
    OptionalU32(Option<u32>),
    OptionalI64(Option<i64>),
    OptionalU64(Option<u64>),
    Bytes(u32),
    U32List(&'a [u32]),
    U64List(&'a [u64]),
    Opaque(&'a [u8]),
}

fn grow<T: Default>(values: &mut Vec<T>, slots: usize) -> io::Result<()> {
    if values.len() < slots {
        engine_allocation::resize_default(values, slots)?;
    }
    Ok(())
}

fn at<T>(values: &mut [T], slot: u32) -> io::Result<&mut T> {
    values
        .get_mut(slot as usize)
        .ok_or_else(|| invalid(format!("Column run names slot {slot} past the table")))
}

fn same_length(slots: &[u32], values: usize) -> io::Result<()> {
    if slots.len() != values {
        return Err(invalid(format!(
            "Column run has {values} values for {} slots",
            slots.len()
        )));
    }
    Ok(())
}

fn set_plain<T: Copy>(column: &mut [T], slots: &[u32], values: &[T]) -> io::Result<()> {
    same_length(slots, values.len())?;
    for (&slot, &value) in slots.iter().zip(values) {
        *at(column, slot)? = value;
    }
    Ok(())
}

fn set_bool(column: &mut [bool], slots: &[u32], values: &[u8]) -> io::Result<()> {
    same_length(slots, values.len())?;
    for (&slot, &value) in slots.iter().zip(values) {
        *at(column, slot)? = match value {
            0 => false,
            1 => true,
            other => return Err(invalid(format!("Bool column value {other}"))),
        };
    }
    Ok(())
}

fn presence(present: &[u64], index: usize) -> io::Result<bool> {
    present
        .get(index / 64)
        .map(|word| word >> (index % 64) & 1 == 1)
        .ok_or_else(|| invalid("Optional run's presence bits end before its slots"))
}

fn set_optional<T: Copy>(
    column: &mut [T],
    bits: &mut Bits,
    slots: &[u32],
    present: &[u64],
    values: &[T],
) -> io::Result<()> {
    let mut next = 0;
    for (index, &slot) in slots.iter().enumerate() {
        if presence(present, index)? {
            let value = *values
                .get(next)
                .ok_or_else(|| invalid("Optional run has fewer values than present slots"))?;
            *at(column, slot)? = value;
            bits.set(slot)?;
            next += 1;
        } else {
            at(column, slot)?;
            bits.clear(slot);
        }
    }
    if next != values.len() {
        return Err(invalid("Optional run has more values than present slots"));
    }
    Ok(())
}

fn cell_range(ends: &[u32], index: usize, items: usize) -> io::Result<std::ops::Range<usize>> {
    let start = index
        .checked_sub(1)
        .map_or(0, |previous| ends[previous] as usize);
    let end = ends[index] as usize;
    if start > end || end > items {
        return Err(invalid(
            "Cell run's end offsets are out of order or past its items",
        ));
    }
    Ok(start..end)
}

fn set_cells<T: Copy>(
    column: &mut [Box<[T]>],
    slots: &[u32],
    ends: &[u32],
    items: &[T],
) -> io::Result<()> {
    same_length(slots, ends.len())?;
    if ends.last().map_or(0, |&end| end as usize) != items.len() {
        return Err(invalid("Cell run's items do not end at its last offset"));
    }
    for (index, &slot) in slots.iter().enumerate() {
        let range = cell_range(ends, index, items.len())?;
        *at(column, slot)? = engine_allocation::copy_vec(&items[range])?.into_boxed_slice();
    }
    Ok(())
}

trait Wrapping: Copy {
    fn wrapping(self, delta: Self) -> Self;
}

macro_rules! wrapping {
    ($($kind:ty),+) => {
        $(impl Wrapping for $kind {
            fn wrapping(self, delta: Self) -> Self {
                self.wrapping_add(delta)
            }
        })+
    };
}
wrapping!(u8, i8, u16, i16, u32, i32, u64, i64);

// The writer encodes an exact difference at the column's width, so wrapping reproduces it.
fn add_plain<T: Wrapping>(column: &mut [T], slots: &[u32], deltas: &[T]) -> io::Result<()> {
    same_length(slots, deltas.len())?;
    for (&slot, &delta) in slots.iter().zip(deltas) {
        let value = at(column, slot)?;
        *value = value.wrapping(delta);
    }
    Ok(())
}

fn add_optional<T: Wrapping>(
    column: &mut [T],
    bits: &Bits,
    slots: &[u32],
    deltas: &[T],
) -> io::Result<()> {
    if let Some(&slot) = slots.iter().find(|&&slot| !bits.get(slot)) {
        return Err(invalid(format!("ADD to missing value at slot {slot}")));
    }
    add_plain(column, slots, deltas)
}

fn one<T: Copy>(values: &[T]) -> io::Result<T> {
    match values {
        [value] => Ok(*value),
        _ => Err(invalid(format!(
            "FILL carries {} values, not one",
            values.len()
        ))),
    }
}

fn fill_plain<T: Copy>(column: &mut [T], slots: &[u32], value: T) -> io::Result<()> {
    for &slot in slots {
        *at(column, slot)? = value;
    }
    Ok(())
}

fn fill_optional<T: Copy>(
    column: &mut [T],
    bits: &mut Bits,
    slots: &[u32],
    present: &[u64],
    values: &[T],
) -> io::Result<()> {
    let value = if presence(present, 0)? {
        Some(one(values)?)
    } else if values.is_empty() {
        None
    } else {
        return Err(invalid("FILL to missing carries a value"));
    };
    for &slot in slots {
        let cell = at(column, slot)?;
        match value {
            Some(value) => {
                *cell = value;
                bits.set(slot)?;
            }
            None => bits.clear(slot),
        }
    }
    Ok(())
}

fn fill_cells<T: Copy>(
    column: &mut [Box<[T]>],
    slots: &[u32],
    ends: &[u32],
    items: &[T],
) -> io::Result<()> {
    if ends.len() != 1 || ends[0] as usize != items.len() {
        return Err(invalid("FILL carries one cell"));
    }
    for &slot in slots {
        *at(column, slot)? = engine_allocation::copy_vec(items)?.into_boxed_slice();
    }
    Ok(())
}

impl Values<'_> {
    pub fn kind(&self) -> Kind {
        match self {
            Self::U8(_) => Kind::U8,
            Self::I8(_) => Kind::I8,
            Self::U16(_) => Kind::U16,
            Self::I16(_) => Kind::I16,
            Self::U32(_) => Kind::U32,
            Self::I32(_) => Kind::I32,
            Self::U64(_) => Kind::U64,
            Self::I64(_) => Kind::I64,
            Self::Bool(_) => Kind::Bool,
            Self::OptionalI32 { .. } => Kind::OptionalI32,
            Self::OptionalU32 { .. } => Kind::OptionalU32,
            Self::OptionalI64 { .. } => Kind::OptionalI64,
            Self::OptionalU64 { .. } => Kind::OptionalU64,
            Self::Bytes(_) => Kind::Bytes,
            Self::U32List { .. } => Kind::U32List,
            Self::U64List { .. } => Kind::U64List,
            Self::Opaque { .. } => Kind::Opaque,
        }
    }
}

impl Column {
    pub fn new(kind: Kind) -> Self {
        match kind {
            Kind::U8 => Self::U8(Vec::new()),
            Kind::I8 => Self::I8(Vec::new()),
            Kind::U16 => Self::U16(Vec::new()),
            Kind::I16 => Self::I16(Vec::new()),
            Kind::U32 => Self::U32(Vec::new()),
            Kind::I32 => Self::I32(Vec::new()),
            Kind::U64 => Self::U64(Vec::new()),
            Kind::I64 => Self::I64(Vec::new()),
            Kind::Bool => Self::Bool(Vec::new()),
            Kind::OptionalI32 => Self::OptionalI32(Vec::new(), Bits::default()),
            Kind::OptionalU32 => Self::OptionalU32(Vec::new(), Bits::default()),
            Kind::OptionalI64 => Self::OptionalI64(Vec::new(), Bits::default()),
            Kind::OptionalU64 => Self::OptionalU64(Vec::new(), Bits::default()),
            Kind::Bytes => Self::Bytes(Vec::new()),
            Kind::U32List => Self::U32List(Vec::new()),
            Kind::U64List => Self::U64List(Vec::new()),
            Kind::Opaque => Self::Opaque(Vec::new()),
        }
    }

    pub fn kind(&self) -> Kind {
        match self {
            Self::U8(_) => Kind::U8,
            Self::I8(_) => Kind::I8,
            Self::U16(_) => Kind::U16,
            Self::I16(_) => Kind::I16,
            Self::U32(_) => Kind::U32,
            Self::I32(_) => Kind::I32,
            Self::U64(_) => Kind::U64,
            Self::I64(_) => Kind::I64,
            Self::Bool(_) => Kind::Bool,
            Self::OptionalI32(..) => Kind::OptionalI32,
            Self::OptionalU32(..) => Kind::OptionalU32,
            Self::OptionalI64(..) => Kind::OptionalI64,
            Self::OptionalU64(..) => Kind::OptionalU64,
            Self::Bytes(_) => Kind::Bytes,
            Self::U32List(_) => Kind::U32List,
            Self::U64List(_) => Kind::U64List,
            Self::Opaque(_) => Kind::Opaque,
        }
    }

    /// The value at `slot`, live or not: a deleted row's slot keeps stale scalars.
    pub fn cell(&self, slot: u32) -> Option<Cell<'_>> {
        let at = slot as usize;
        Some(match self {
            Self::U8(values) => Cell::U8(*values.get(at)?),
            Self::I8(values) => Cell::I8(*values.get(at)?),
            Self::U16(values) => Cell::U16(*values.get(at)?),
            Self::I16(values) => Cell::I16(*values.get(at)?),
            Self::U32(values) => Cell::U32(*values.get(at)?),
            Self::I32(values) => Cell::I32(*values.get(at)?),
            Self::U64(values) => Cell::U64(*values.get(at)?),
            Self::I64(values) => Cell::I64(*values.get(at)?),
            Self::Bool(values) => Cell::Bool(*values.get(at)?),
            Self::OptionalI32(values, bits) => {
                Cell::OptionalI32(bits.get(slot).then_some(*values.get(at)?))
            }
            Self::OptionalU32(values, bits) => {
                Cell::OptionalU32(bits.get(slot).then_some(*values.get(at)?))
            }
            Self::OptionalI64(values, bits) => {
                Cell::OptionalI64(bits.get(slot).then_some(*values.get(at)?))
            }
            Self::OptionalU64(values, bits) => {
                Cell::OptionalU64(bits.get(slot).then_some(*values.get(at)?))
            }
            Self::Bytes(values) => Cell::Bytes(*values.get(at)?),
            Self::U32List(values) => Cell::U32List(values.get(at)?),
            Self::U64List(values) => Cell::U64List(values.get(at)?),
            Self::Opaque(values) => Cell::Opaque(values.get(at)?),
        })
    }

    fn grow(&mut self, slots: usize) -> io::Result<()> {
        match self {
            Self::U8(values) => grow(values, slots),
            Self::I8(values) => grow(values, slots),
            Self::U16(values) => grow(values, slots),
            Self::I16(values) => grow(values, slots),
            Self::U32(values) | Self::Bytes(values) => grow(values, slots),
            Self::I32(values) => grow(values, slots),
            Self::U64(values) => grow(values, slots),
            Self::I64(values) => grow(values, slots),
            Self::Bool(values) => grow(values, slots),
            Self::OptionalI32(values, _) => grow(values, slots),
            Self::OptionalU32(values, _) => grow(values, slots),
            Self::OptionalI64(values, _) => grow(values, slots),
            Self::OptionalU64(values, _) => grow(values, slots),
            Self::U32List(values) => grow(values, slots),
            Self::U64List(values) => grow(values, slots),
            Self::Opaque(values) => grow(values, slots),
        }
    }

    fn set(&mut self, slots: &[u32], values: Values<'_>) -> io::Result<()> {
        match (self, values) {
            (Self::U8(column), Values::U8(values)) => set_plain(column, slots, values),
            (Self::I8(column), Values::I8(values)) => set_plain(column, slots, values),
            (Self::U16(column), Values::U16(values)) => set_plain(column, slots, values),
            (Self::I16(column), Values::I16(values)) => set_plain(column, slots, values),
            (Self::U32(column), Values::U32(values)) => set_plain(column, slots, values),
            (Self::I32(column), Values::I32(values)) => set_plain(column, slots, values),
            (Self::U64(column), Values::U64(values)) => set_plain(column, slots, values),
            (Self::I64(column), Values::I64(values)) => set_plain(column, slots, values),
            (Self::Bool(column), Values::Bool(values)) => set_bool(column, slots, values),
            (Self::OptionalI32(column, bits), Values::OptionalI32 { present, values }) => {
                set_optional(column, bits, slots, present, values)
            }
            (Self::OptionalU32(column, bits), Values::OptionalU32 { present, values }) => {
                set_optional(column, bits, slots, present, values)
            }
            (Self::OptionalI64(column, bits), Values::OptionalI64 { present, values }) => {
                set_optional(column, bits, slots, present, values)
            }
            (Self::OptionalU64(column, bits), Values::OptionalU64 { present, values }) => {
                set_optional(column, bits, slots, present, values)
            }
            (Self::Bytes(column), Values::Bytes(values)) => set_plain(column, slots, values),
            (Self::U32List(column), Values::U32List { ends, items }) => {
                set_cells(column, slots, ends, items)
            }
            (Self::U64List(column), Values::U64List { ends, items }) => {
                set_cells(column, slots, ends, items)
            }
            (Self::Opaque(column), Values::Opaque { ends, items }) => {
                set_cells(column, slots, ends, items)
            }
            (column, values) => Err(mismatch(column.kind(), values.kind())),
        }
    }

    /// Optional columns take deltas of their integer type, for present values only.
    fn add(&mut self, slots: &[u32], deltas: Values<'_>) -> io::Result<()> {
        match (self, deltas) {
            (Self::U8(column), Values::U8(deltas)) => add_plain(column, slots, deltas),
            (Self::I8(column), Values::I8(deltas)) => add_plain(column, slots, deltas),
            (Self::U16(column), Values::U16(deltas)) => add_plain(column, slots, deltas),
            (Self::I16(column), Values::I16(deltas)) => add_plain(column, slots, deltas),
            (Self::U32(column), Values::U32(deltas)) => add_plain(column, slots, deltas),
            (Self::I32(column), Values::I32(deltas)) => add_plain(column, slots, deltas),
            (Self::U64(column), Values::U64(deltas)) => add_plain(column, slots, deltas),
            (Self::I64(column), Values::I64(deltas)) => add_plain(column, slots, deltas),
            (Self::OptionalI32(column, bits), Values::I32(deltas)) => {
                add_optional(column, bits, slots, deltas)
            }
            (Self::OptionalU32(column, bits), Values::U32(deltas)) => {
                add_optional(column, bits, slots, deltas)
            }
            (Self::OptionalI64(column, bits), Values::I64(deltas)) => {
                add_optional(column, bits, slots, deltas)
            }
            (Self::OptionalU64(column, bits), Values::U64(deltas)) => {
                add_optional(column, bits, slots, deltas)
            }
            (column, deltas) => Err(invalid(format!(
                "ADD of {:?} deltas to a {:?} column",
                deltas.kind(),
                column.kind()
            ))),
        }
    }

    fn fill(&mut self, slots: &[u32], value: Values<'_>) -> io::Result<()> {
        match (self, value) {
            (Self::U8(column), Values::U8(value)) => fill_plain(column, slots, one(value)?),
            (Self::I8(column), Values::I8(value)) => fill_plain(column, slots, one(value)?),
            (Self::U16(column), Values::U16(value)) => fill_plain(column, slots, one(value)?),
            (Self::I16(column), Values::I16(value)) => fill_plain(column, slots, one(value)?),
            (Self::U32(column), Values::U32(value)) => fill_plain(column, slots, one(value)?),
            (Self::I32(column), Values::I32(value)) => fill_plain(column, slots, one(value)?),
            (Self::U64(column), Values::U64(value)) => fill_plain(column, slots, one(value)?),
            (Self::I64(column), Values::I64(value)) => fill_plain(column, slots, one(value)?),
            (Self::Bool(column), Values::Bool(value)) => {
                let value = match one(value)? {
                    0 => false,
                    1 => true,
                    other => return Err(invalid(format!("Bool column value {other}"))),
                };
                fill_plain(column, slots, value)
            }
            (Self::OptionalI32(column, bits), Values::OptionalI32 { present, values }) => {
                fill_optional(column, bits, slots, present, values)
            }
            (Self::OptionalU32(column, bits), Values::OptionalU32 { present, values }) => {
                fill_optional(column, bits, slots, present, values)
            }
            (Self::OptionalI64(column, bits), Values::OptionalI64 { present, values }) => {
                fill_optional(column, bits, slots, present, values)
            }
            (Self::OptionalU64(column, bits), Values::OptionalU64 { present, values }) => {
                fill_optional(column, bits, slots, present, values)
            }
            (Self::Bytes(column), Values::Bytes(value)) => fill_plain(column, slots, one(value)?),
            (Self::U32List(column), Values::U32List { ends, items }) => {
                fill_cells(column, slots, ends, items)
            }
            (Self::U64List(column), Values::U64List { ends, items }) => {
                fill_cells(column, slots, ends, items)
            }
            (Self::Opaque(column), Values::Opaque { ends, items }) => {
                fill_cells(column, slots, ends, items)
            }
            (column, value) => Err(mismatch(column.kind(), value.kind())),
        }
    }

    /// The values at `slots`, in the layout a run of this column carries.
    pub(crate) fn write(&self, writer: &mut Writer, slots: &[u32]) -> io::Result<()> {
        if slots.last().is_some_and(|&last| self.cell(last).is_none()) {
            return Err(invalid("Column has no value at a written slot"));
        }
        match self {
            Self::U8(values) => write_plain(writer, values, slots),
            Self::I8(values) => write_plain(writer, values, slots),
            Self::U16(values) => write_ints(writer, values, slots),
            Self::I16(values) => write_ints(writer, values, slots),
            Self::U32(values) | Self::Bytes(values) => write_ints(writer, values, slots),
            Self::I32(values) => write_ints(writer, values, slots),
            Self::U64(values) => write_ints(writer, values, slots),
            Self::I64(values) => write_ints(writer, values, slots),
            Self::Bool(values) => {
                let bytes = engine_allocation::collect_vec(
                    slots.iter().map(|&slot| u8::from(values[slot as usize])),
                )?;
                writer.values(&bytes)
            }
            Self::OptionalI32(values, bits) => write_optional(writer, values, bits, slots),
            Self::OptionalU32(values, bits) => write_optional(writer, values, bits, slots),
            Self::OptionalI64(values, bits) => write_optional(writer, values, bits, slots),
            Self::OptionalU64(values, bits) => write_optional(writer, values, bits, slots),
            Self::U32List(values) => write_cells(writer, values, slots, Writer::ints),
            Self::U64List(values) => write_cells(writer, values, slots, Writer::ints),
            Self::Opaque(values) => write_cells(writer, values, slots, Writer::raw),
        }
    }

    fn patch(&mut self, slots: &[u32], patches: Patches<'_>) -> io::Result<()> {
        match (self, patches.items) {
            (Self::U32List(column), PatchItems::U32(items)) => {
                patch_cells(column, slots, patches, items)
            }
            (Self::U64List(column), PatchItems::U64(items)) => {
                patch_cells(column, slots, patches, items)
            }
            (Self::Opaque(column), PatchItems::Bytes(items)) => {
                patch_cells(column, slots, patches, items)
            }
            (column, _) => Err(invalid(format!("PATCH to a {:?} column", column.kind()))),
        }
    }

    /// A deleted row's cells release what they hold; its optional values become missing.
    fn release(&mut self, slot: u32) {
        let at = slot as usize;
        match self {
            Self::OptionalI32(_, bits)
            | Self::OptionalU32(_, bits)
            | Self::OptionalI64(_, bits)
            | Self::OptionalU64(_, bits) => bits.clear(slot),
            Self::U32List(values) => {
                if let Some(cell) = values.get_mut(at) {
                    *cell = Box::default();
                }
            }
            Self::U64List(values) => {
                if let Some(cell) = values.get_mut(at) {
                    *cell = Box::default();
                }
            }
            Self::Opaque(values) => {
                if let Some(cell) = values.get_mut(at) {
                    *cell = Box::default();
                }
            }
            _ => {}
        }
    }
}

fn write_plain<T: Le>(writer: &mut Writer, column: &[T], slots: &[u32]) -> io::Result<()> {
    let values = engine_allocation::collect_vec(slots.iter().map(|&slot| column[slot as usize]))?;
    writer.values(&values)
}

fn write_ints<T: Int>(writer: &mut Writer, column: &[T], slots: &[u32]) -> io::Result<()> {
    let values = engine_allocation::collect_vec(slots.iter().map(|&slot| column[slot as usize]))?;
    writer.ints(&values)
}

fn write_optional<T: Int>(
    writer: &mut Writer,
    column: &[T],
    bits: &Bits,
    slots: &[u32],
) -> io::Result<()> {
    let mut present = engine_allocation::filled(slots.len().div_ceil(64), 0u64)?;
    let mut values = Vec::new();
    for (index, &slot) in slots.iter().enumerate() {
        if bits.get(slot) {
            present[index / 64] |= 1 << (index % 64);
            engine_allocation::reserve(&mut values, 1)?;
            values.push(column[slot as usize]);
        }
    }
    writer.values(&present)?;
    writer.ints(&values)
}

fn write_cells<T: Copy>(
    writer: &mut Writer,
    column: &[Box<[T]>],
    slots: &[u32],
    items_of: fn(&mut Writer, &[T]) -> io::Result<()>,
) -> io::Result<()> {
    let mut ends = engine_allocation::vec(slots.len())?;
    let mut items = Vec::new();
    for &slot in slots {
        let cell = &column[slot as usize];
        engine_allocation::reserve(&mut items, cell.len())?;
        items.extend_from_slice(cell);
        ends.push(
            u32::try_from(items.len()).map_err(|_| invalid("Cell run passes 4 GiB of items"))?,
        );
    }
    writer.lengths(&ends)?;
    items_of(writer, &items)
}

/// Each slot's held cell with its value's hunks applied: kept items, removed items and
/// inserted items, then the rest of the held cell.
fn patch_cells<T: Copy>(
    column: &mut [Box<[T]>],
    slots: &[u32],
    patches: Patches<'_>,
    items: &[T],
) -> io::Result<()> {
    same_length(slots, patches.hunks.len())?;
    let (mut edit, mut item) = (0usize, 0usize);
    for (&slot, &count) in slots.iter().zip(patches.hunks) {
        let held = at(column, slot)?;
        let edits = patches
            .edits
            .get(edit..edit + count as usize)
            .ok_or_else(|| invalid("PATCH run ends inside its hunks"))?;
        edit += count as usize;
        let removed: usize = edits.iter().map(|edit| edit.1 as usize).sum();
        let inserted: usize = edits.iter().map(|edit| edit.2 as usize).sum();
        let mut cell = engine_allocation::vec(held.len().saturating_sub(removed) + inserted)?;
        let mut from = 0usize;
        for &(skip, removed, inserted) in edits {
            let kept = from
                .checked_add(skip as usize)
                .filter(|&end| end <= held.len())
                .ok_or_else(|| invalid("PATCH keeps items past its cell"))?;
            cell.extend_from_slice(&held[from..kept]);
            from = kept
                .checked_add(removed as usize)
                .filter(|&end| end <= held.len())
                .ok_or_else(|| invalid("PATCH removes items past its cell"))?;
            let end = item
                .checked_add(inserted as usize)
                .filter(|&end| end <= items.len())
                .ok_or_else(|| invalid("PATCH run ends inside its items"))?;
            cell.extend_from_slice(&items[item..end]);
            item = end;
        }
        cell.extend_from_slice(&held[from..]);
        *held = cell.into_boxed_slice();
    }
    if edit != patches.edits.len() || item != items.len() {
        return Err(invalid("PATCH run carries edits past its values"));
    }
    Ok(())
}

fn mismatch(column: Kind, values: Kind) -> io::Error {
    invalid(format!("{values:?} values for a {column:?} column"))
}

fn ascending(slots: &[u32]) -> io::Result<()> {
    if slots.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(invalid("Slots of a run are not strictly ascending"));
    }
    Ok(())
}

/// One table's live rows: their keys and one column per field, at writer-assigned slots.
#[derive(Clone, Debug)]
pub struct Table {
    keys: Vec<u128>,
    live: Bits,
    columns: Vec<Column>,
    /// Per column, the slots whose cell the recording did not record: unknown, never the
    /// value the column holds there.
    unrecorded: Vec<Bits>,
    slots: usize,
    /// Each live row's slot by its key, for joins between tables.
    by_key: HashMap<u128, u32, foldhash::fast::RandomState>,
}

/// Tables are equal when they hold the same rows at the same slots: what a freed slot
/// still holds is not part of the table.
impl PartialEq for Table {
    fn eq(&self, other: &Self) -> bool {
        self.slots == other.slots
            && self.kinds().eq(other.kinds())
            && self.live.ones().eq(other.live.ones())
            && self.live.ones().all(|slot| {
                self.keys[slot as usize] == other.keys[slot as usize]
                    && self
                        .columns
                        .iter()
                        .zip(&other.columns)
                        .all(|(mine, theirs)| mine.cell(slot) == theirs.cell(slot))
                    && (0..self.columns.len())
                        .all(|column| self.recorded(column, slot) == other.recorded(column, slot))
            })
    }
}

impl Eq for Table {}

impl Table {
    pub fn new(kinds: &[Kind]) -> io::Result<Self> {
        Ok(Self {
            keys: Vec::new(),
            live: Bits::default(),
            columns: engine_allocation::collect_vec(kinds.iter().map(|&kind| Column::new(kind)))?,
            unrecorded: engine_allocation::collect_vec(kinds.iter().map(|_| Bits::default()))?,
            slots: 0,
            by_key: HashMap::default(),
        })
    }

    /// A restart's table: `slots` slots, `live` of them holding rows with these keys and
    /// each column's values for the live slots in slot order.
    pub fn restore(
        kinds: &[Kind],
        slots: usize,
        live: Bits,
        keys: &[u128],
        values: &[Values<'_>],
    ) -> io::Result<Self> {
        if values.len() != kinds.len() {
            return Err(invalid("Restart table's columns differ from its schema"));
        }
        let mut table = Self::with_rows(kinds, slots, live, keys)?;
        let live_slots = engine_allocation::collect_vec(table.live.ones())?;
        for (column, &values) in table.columns.iter_mut().zip(values) {
            column.set(&live_slots, values)?;
        }
        Ok(table)
    }

    /// A restart's rows before their values: every column's values for the live slots
    /// follow, one [`Self::set`] per column, before the table is read.
    pub fn with_rows(kinds: &[Kind], slots: usize, live: Bits, keys: &[u128]) -> io::Result<Self> {
        let mut table = Self::new(kinds)?;
        if slots.div_ceil(64) > SLOT_WORDS {
            return Err(invalid("Restart table's slot count passes the last slot"));
        }
        if live.count() != keys.len() {
            return Err(invalid("Restart table's keys differ from its live slots"));
        }
        if live
            .ones()
            .last()
            .is_some_and(|last| last as usize >= slots)
        {
            return Err(invalid("Restart table's live slots exceed its slot count"));
        }
        table.resize(slots)?;
        table.index(keys.len())?;
        for (slot, &key) in live.ones().zip(keys) {
            table.keys[slot as usize] = key;
            table.key_slot(key, slot)?;
        }
        table.live = live;
        Ok(table)
    }

    pub fn kinds(&self) -> impl Iterator<Item = Kind> + '_ {
        self.columns.iter().map(Column::kind)
    }

    /// Every column's values at the live slots, in slot order, as a restart carries them.
    pub(crate) fn write_columns(&self, writer: &mut Writer, live: &[u32]) -> io::Result<()> {
        for column in &self.columns {
            column.write(writer, live)?;
        }
        Ok(())
    }

    /// One more than the highest slot ever assigned.
    pub fn slots(&self) -> usize {
        self.slots
    }

    pub fn live(&self) -> &Bits {
        &self.live
    }

    pub fn key(&self, slot: u32) -> Option<u128> {
        self.live.get(slot).then(|| self.keys[slot as usize])
    }

    /// The slot of the live row with `key`.
    pub fn slot(&self, key: u128) -> Option<u32> {
        self.by_key.get(&key).copied()
    }

    fn index(&mut self, additional: usize) -> io::Result<()> {
        engine_allocation::reserve_map(&mut self.by_key, additional)
    }

    fn key_slot(&mut self, key: u128, slot: u32) -> io::Result<()> {
        if let Some(held) = self.by_key.get(&key) {
            return Err(invalid(format!(
                "Row key {key:#x} at slot {slot} is already held at slot {held}"
            )));
        }
        self.by_key.insert(key, slot);
        Ok(())
    }

    pub fn column(&self, column: usize) -> Option<&Column> {
        self.columns.get(column)
    }

    /// Whether the recording recorded `slot`'s cell in `column`; when it did not, the cell is
    /// unknown whatever the column holds there.
    pub fn recorded(&self, column: usize, slot: u32) -> bool {
        !self
            .unrecorded
            .get(column)
            .is_some_and(|bits| bits.get(slot))
    }

    /// The live `slots` whose cell in `column` the recording did not record.
    pub fn unrecord(&mut self, column: usize, slots: &[u32]) -> io::Result<()> {
        self.live_slots(slots)?;
        let bits = self
            .unrecorded
            .get_mut(column)
            .ok_or_else(|| invalid(format!("Run names column {column} past the schema")))?;
        for &slot in slots {
            bits.set(slot)?;
        }
        Ok(())
    }

    /// `column`'s unrecorded slots, a bit each.
    pub(crate) fn unrecorded_bits(&self, column: usize) -> Option<&Bits> {
        self.unrecorded.get(column)
    }

    /// The live slots whose cell in `column` is unrecorded, ascending.
    pub(crate) fn unrecorded_slots(&self, column: usize) -> impl Iterator<Item = u32> + '_ {
        self.unrecorded[column]
            .ones()
            .filter(|&slot| self.live.get(slot))
    }

    /// A value written to `slots` of `column` is recorded.
    fn recorded_again(&mut self, column: usize, slots: &[u32]) {
        if let Some(bits) = self.unrecorded.get_mut(column) {
            for &slot in slots {
                bits.clear(slot);
            }
        }
    }

    fn resize(&mut self, slots: usize) -> io::Result<()> {
        if slots <= self.slots {
            return Ok(());
        }
        grow(&mut self.keys, slots)?;
        for column in &mut self.columns {
            column.grow(slots)?;
        }
        self.slots = slots;
        Ok(())
    }

    fn live_slots(&self, slots: &[u32]) -> io::Result<()> {
        ascending(slots)?;
        if let Some(&slot) = slots.iter().find(|&&slot| !self.live.get(slot)) {
            return Err(invalid(format!(
                "Run names slot {slot}, which holds no row"
            )));
        }
        Ok(())
    }

    fn column_mut(&mut self, column: usize) -> io::Result<&mut Column> {
        self.columns
            .get_mut(column)
            .ok_or_else(|| invalid(format!("Run names column {column} past the schema")))
    }

    /// New rows at free slots, with their keys and each column's values in slot order.
    pub fn insert(
        &mut self,
        slots: &[u32],
        keys: &[u128],
        values: &[Values<'_>],
    ) -> io::Result<()> {
        if values.len() != self.columns.len() {
            return Err(invalid("Insert carries other than one run per column"));
        }
        self.insert_keys(slots, keys)?;
        for (column, &values) in values.iter().enumerate() {
            self.set(column, slots, values)?;
        }
        Ok(())
    }

    /// New rows at free slots with these keys; every column's values for them follow,
    /// one [`Self::set`] per column, before the table is read.
    pub fn insert_keys(&mut self, slots: &[u32], keys: &[u128]) -> io::Result<()> {
        ascending(slots)?;
        if keys.len() != slots.len() {
            return Err(invalid("Insert carries other than one key per slot"));
        }
        if let Some(&slot) = slots.iter().find(|&&slot| self.live.get(slot)) {
            return Err(invalid(format!(
                "Insert names slot {slot}, which holds a row"
            )));
        }
        if let Some(&last) = slots.last() {
            self.resize(last as usize + 1)?;
        }
        self.index(slots.len())?;
        for (&slot, &key) in slots.iter().zip(keys) {
            self.key_slot(key, slot)?;
            self.keys[slot as usize] = key;
            self.live.set(slot)?;
        }
        Ok(())
    }

    pub fn delete(&mut self, slots: &[u32]) -> io::Result<()> {
        self.live_slots(slots)?;
        for &slot in slots {
            self.live.clear(slot);
            self.by_key.remove(&self.keys[slot as usize]);
            for column in &mut self.columns {
                column.release(slot);
            }
            for bits in &mut self.unrecorded {
                bits.clear(slot);
            }
        }
        Ok(())
    }

    /// Changed values: each differs from the one it replaces.
    pub fn set(&mut self, column: usize, slots: &[u32], values: Values<'_>) -> io::Result<()> {
        self.live_slots(slots)?;
        self.column_mut(column)?.set(slots, values)?;
        self.recorded_again(column, slots);
        Ok(())
    }

    /// Edits to held list or opaque cells.
    pub fn patch(&mut self, column: usize, slots: &[u32], patches: Patches<'_>) -> io::Result<()> {
        self.live_slots(slots)?;
        self.column_mut(column)?.patch(slots, patches)?;
        self.recorded_again(column, slots);
        Ok(())
    }

    pub fn add(&mut self, column: usize, slots: &[u32], deltas: Values<'_>) -> io::Result<()> {
        self.live_slots(slots)?;
        self.column_mut(column)?.add(slots, deltas)?;
        self.recorded_again(column, slots);
        Ok(())
    }

    /// One value for every live row but the `except` slots, into `targets` (the slots set,
    /// ascending).
    pub fn fill(
        &mut self,
        column: usize,
        except: &[u32],
        value: Values<'_>,
        targets: &mut Vec<u32>,
    ) -> io::Result<()> {
        self.live_slots(except)?;
        targets.clear();
        engine_allocation::reserve(targets, self.live.count() - except.len())?;
        let mut skip = except.iter().copied().peekable();
        for slot in self.live.ones() {
            if skip.next_if_eq(&slot).is_none() {
                targets.push(slot);
            }
        }
        self.column_mut(column)?.fill(targets, value)?;
        self.recorded_again(column, targets);
        Ok(())
    }
}
