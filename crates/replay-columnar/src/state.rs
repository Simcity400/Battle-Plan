
use crate::{
    invalid,
    leaf::{Int, Le, SLOT_WORDS, Writer},
    section::{PatchItems, Patches},
};
use std::io;

mod paged;
#[cfg(test)]
mod tests;

pub use paged::{Bools, Lists, PAGE, Paged, PagedBits};
use paged::{WeakLists, WeakPaged};

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
    Bytes,
    U32List,
    U64List,
    Opaque,
}

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

    pub(crate) fn all_set(&self, len: usize) -> bool {
        let full = len / 64;
        let rem = len % 64;
        if self.words.len() < full + usize::from(rem != 0) {
            return false;
        }
        self.words[..full].iter().all(|&word| word == u64::MAX)
            && (rem == 0 || (self.words[full] & ((1u64 << rem) - 1)) == ((1u64 << rem) - 1))
    }

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

    pub(crate) fn set(&mut self, bit: u32) -> io::Result<()> {
        let word = bit as usize / 64;
        if word >= self.words.len() {
            engine_allocation::resize_default(&mut self.words, word + 1)?;
        }
        self.words[word] |= 1 << (bit % 64);
        Ok(())
    }

    pub(crate) fn clear(&mut self, bit: u32) {
        if let Some(word) = self.words.get_mut(bit as usize / 64) {
            *word &= !(1 << (bit % 64));
        }
    }

    fn copy(&self) -> io::Result<Self> {
        Ok(Self {
            words: engine_allocation::copy_vec(&self.words)?,
        })
    }

    fn held_bytes(&self) -> usize {
        self.words.capacity() * size_of::<u64>()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Column {
    U8(Paged<u8>),
    I8(Paged<i8>),
    U16(Paged<u16>),
    I16(Paged<i16>),
    U32(Paged<u32>),
    I32(Paged<i32>),
    U64(Paged<u64>),
    I64(Paged<i64>),
    Bool(Bools),
    OptionalI32(Paged<i32>, PagedBits),
    OptionalU32(Paged<u32>, PagedBits),
    OptionalI64(Paged<i64>, PagedBits),
    OptionalU64(Paged<u64>, PagedBits),
    Bytes(Paged<u32>),
    U32List(Lists<u32>),
    U64List(Lists<u64>),
    Opaque(Lists<u8>),
}

#[derive(Debug)]
enum WeakColumn {
    U8(WeakPaged<u8>),
    I8(WeakPaged<i8>),
    U16(WeakPaged<u16>),
    I16(WeakPaged<i16>),
    U32(WeakPaged<u32>),
    I32(WeakPaged<i32>),
    U64(WeakPaged<u64>),
    I64(WeakPaged<i64>),
    Bool((WeakPaged<u64>, usize)),
    OptionalI32(WeakPaged<i32>, WeakPaged<u64>),
    OptionalU32(WeakPaged<u32>, WeakPaged<u64>),
    OptionalI64(WeakPaged<i64>, WeakPaged<u64>),
    OptionalU64(WeakPaged<u64>, WeakPaged<u64>),
    Bytes(WeakPaged<u32>),
    U32List(WeakLists<u32>),
    U64List(WeakLists<u64>),
    Opaque(WeakLists<u8>),
}

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

fn past(slot: u32) -> io::Error {
    invalid(format!("Column run names slot {slot} past the table"))
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

fn set_plain<T: Copy + PartialEq>(
    column: &mut Paged<T>,
    slots: &[u32],
    values: &[T],
) -> io::Result<()> {
    same_length(slots, values.len())?;
    column.write_each(slots, past, |index| Ok(values[index]))
}

fn bool_of(value: u8) -> io::Result<bool> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        other => Err(invalid(format!("Bool column value {other}"))),
    }
}

fn set_bool(column: &mut Bools, slots: &[u32], values: &[u8]) -> io::Result<()> {
    same_length(slots, values.len())?;
    column.put_each(slots, past, |index| bool_of(values[index]))
}

fn presence(present: &[u64], index: usize) -> io::Result<bool> {
    present
        .get(index / 64)
        .map(|word| word >> (index % 64) & 1 == 1)
        .ok_or_else(|| invalid("Optional run's presence bits end before its slots"))
}

fn set_optional<T: Copy + PartialEq>(
    column: &mut Paged<T>,
    bits: &mut PagedBits,
    slots: &[u32],
    present: &[u64],
    values: &[T],
) -> io::Result<()> {
    let mut next = 0;
    column.write_each_optional(slots, past, |index| {
        if presence(present, index)? {
            let value = *values
                .get(next)
                .ok_or_else(|| invalid("Optional run has fewer values than present slots"))?;
            next += 1;
            Ok(Some(value))
        } else {
            Ok(None)
        }
    })?;
    if next != values.len() {
        return Err(invalid("Optional run has more values than present slots"));
    }
    bits.put_each(slots, |index| presence(present, index))
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

fn put_items<T: Copy>(out: &mut Vec<T>, items: &[T]) -> io::Result<()> {
    engine_allocation::reserve(out, items.len())?;
    out.extend_from_slice(items);
    Ok(())
}

fn set_cells<T: Copy + Default>(
    column: &mut Lists<T>,
    slots: &[u32],
    ends: &[u32],
    items: &[T],
) -> io::Result<()> {
    same_length(slots, ends.len())?;
    if ends.last().map_or(0, |&end| end as usize) != items.len() {
        return Err(invalid("Cell run's items do not end at its last offset"));
    }
    column.rewrite(slots, past, |index, _, out| {
        put_items(out, &items[cell_range(ends, index, items.len())?])
    })
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

fn add_plain<T: Wrapping>(column: &mut Paged<T>, slots: &[u32], deltas: &[T]) -> io::Result<()> {
    same_length(slots, deltas.len())?;
    column.each_mut(slots, past, |index, value| {
        *value = value.wrapping(deltas[index]);
        Ok(())
    })
}

fn add_optional<T: Wrapping>(
    column: &mut Paged<T>,
    bits: &PagedBits,
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

fn fill_plain<T: Copy + PartialEq>(
    column: &mut Paged<T>,
    slots: &[u32],
    value: T,
) -> io::Result<()> {
    column.write_each(slots, past, |_| Ok(value))
}

fn fill_optional<T: Copy + PartialEq>(
    column: &mut Paged<T>,
    bits: &mut PagedBits,
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
    match value {
        Some(value) => fill_plain(column, slots, value)?,
        None => {
            if let Some(&slot) = slots.iter().find(|&&slot| slot as usize >= column.len()) {
                return Err(past(slot));
            }
        }
    }
    bits.put_each(slots, |_| Ok(value.is_some()))
}

fn fill_cells<T: Copy + Default>(
    column: &mut Lists<T>,
    slots: &[u32],
    ends: &[u32],
    items: &[T],
) -> io::Result<()> {
    if ends.len() != 1 || ends[0] as usize != items.len() {
        return Err(invalid("FILL carries one cell"));
    }
    column.rewrite(slots, past, |_, _, out| put_items(out, items))
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

macro_rules! plain_columns {
    ($column:expr, $values:ident => $body:expr, $($rest:tt)*) => {
        match $column {
            Column::U8($values) => $body,
            Column::I8($values) => $body,
            Column::U16($values) => $body,
            Column::I16($values) => $body,
            Column::U32($values) => $body,
            Column::I32($values) => $body,
            Column::U64($values) => $body,
            Column::I64($values) => $body,
            Column::Bytes($values) => $body,
            $($rest)*
        }
    };
}

impl Column {
    pub fn new(kind: Kind) -> Self {
        match kind {
            Kind::U8 => Self::U8(Paged::default()),
            Kind::I8 => Self::I8(Paged::default()),
            Kind::U16 => Self::U16(Paged::default()),
            Kind::I16 => Self::I16(Paged::default()),
            Kind::U32 => Self::U32(Paged::default()),
            Kind::I32 => Self::I32(Paged::default()),
            Kind::U64 => Self::U64(Paged::default()),
            Kind::I64 => Self::I64(Paged::default()),
            Kind::Bool => Self::Bool(Bools::default()),
            Kind::OptionalI32 => Self::OptionalI32(Paged::default(), PagedBits::default()),
            Kind::OptionalU32 => Self::OptionalU32(Paged::default(), PagedBits::default()),
            Kind::OptionalI64 => Self::OptionalI64(Paged::default(), PagedBits::default()),
            Kind::OptionalU64 => Self::OptionalU64(Paged::default(), PagedBits::default()),
            Kind::Bytes => Self::Bytes(Paged::default()),
            Kind::U32List => Self::U32List(Lists::default()),
            Kind::U64List => Self::U64List(Lists::default()),
            Kind::Opaque => Self::Opaque(Lists::default()),
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

    pub fn held_bytes(&self) -> usize {
        plain_columns!(self, values => values.held_bytes(),
            Self::Bool(values) => values.held_bytes(),
            Self::OptionalI32(values, present) => values.held_bytes() + present.held_bytes(),
            Self::OptionalU32(values, present) => values.held_bytes() + present.held_bytes(),
            Self::OptionalI64(values, present) => values.held_bytes() + present.held_bytes(),
            Self::OptionalU64(values, present) => values.held_bytes() + present.held_bytes(),
            Self::U32List(values) => values.held_bytes(),
            Self::U64List(values) => values.held_bytes(),
            Self::Opaque(values) => values.held_bytes(),
        )
    }

    pub fn owned_bytes(&self) -> usize {
        plain_columns!(self, values => values.owned_bytes(),
            Self::Bool(values) => values.owned_bytes(),
            Self::OptionalI32(values, present) => values.owned_bytes() + present.owned_bytes(),
            Self::OptionalU32(values, present) => values.owned_bytes() + present.owned_bytes(),
            Self::OptionalI64(values, present) => values.owned_bytes() + present.owned_bytes(),
            Self::OptionalU64(values, present) => values.owned_bytes() + present.owned_bytes(),
            Self::U32List(values) => values.owned_bytes(),
            Self::U64List(values) => values.owned_bytes(),
            Self::Opaque(values) => values.owned_bytes(),
        )
    }

    pub fn shared_bytes(&self, other: &Self) -> usize {
        match (self, other) {
            (Self::U8(mine), Self::U8(theirs)) => mine.shared_bytes(theirs),
            (Self::I8(mine), Self::I8(theirs)) => mine.shared_bytes(theirs),
            (Self::U16(mine), Self::U16(theirs)) => mine.shared_bytes(theirs),
            (Self::I16(mine), Self::I16(theirs)) => mine.shared_bytes(theirs),
            (Self::U32(mine), Self::U32(theirs)) => mine.shared_bytes(theirs),
            (Self::I32(mine), Self::I32(theirs)) => mine.shared_bytes(theirs),
            (Self::U64(mine), Self::U64(theirs)) => mine.shared_bytes(theirs),
            (Self::I64(mine), Self::I64(theirs)) => mine.shared_bytes(theirs),
            (Self::Bytes(mine), Self::Bytes(theirs)) => mine.shared_bytes(theirs),
            (Self::Bool(mine), Self::Bool(theirs)) => mine.shared_bytes(theirs),
            (Self::OptionalI32(mine, held), Self::OptionalI32(theirs, present)) => {
                mine.shared_bytes(theirs) + held.shared_bytes(present)
            }
            (Self::OptionalU32(mine, held), Self::OptionalU32(theirs, present)) => {
                mine.shared_bytes(theirs) + held.shared_bytes(present)
            }
            (Self::OptionalI64(mine, held), Self::OptionalI64(theirs, present)) => {
                mine.shared_bytes(theirs) + held.shared_bytes(present)
            }
            (Self::OptionalU64(mine, held), Self::OptionalU64(theirs, present)) => {
                mine.shared_bytes(theirs) + held.shared_bytes(present)
            }
            (Self::U32List(mine), Self::U32List(theirs)) => mine.shared_bytes(theirs),
            (Self::U64List(mine), Self::U64List(theirs)) => mine.shared_bytes(theirs),
            (Self::Opaque(mine), Self::Opaque(theirs)) => mine.shared_bytes(theirs),
            _ => 0,
        }
    }

    pub fn common_bytes(&self, other: &Self) -> usize {
        match (self, other) {
            (Self::U8(mine), Self::U8(theirs)) => mine.common_bytes(theirs),
            (Self::I8(mine), Self::I8(theirs)) => mine.common_bytes(theirs),
            (Self::U16(mine), Self::U16(theirs)) => mine.common_bytes(theirs),
            (Self::I16(mine), Self::I16(theirs)) => mine.common_bytes(theirs),
            (Self::U32(mine), Self::U32(theirs)) => mine.common_bytes(theirs),
            (Self::I32(mine), Self::I32(theirs)) => mine.common_bytes(theirs),
            (Self::U64(mine), Self::U64(theirs)) => mine.common_bytes(theirs),
            (Self::I64(mine), Self::I64(theirs)) => mine.common_bytes(theirs),
            (Self::Bytes(mine), Self::Bytes(theirs)) => mine.common_bytes(theirs),
            (Self::Bool(mine), Self::Bool(theirs)) => mine.common_bytes(theirs),
            (Self::OptionalI32(mine, held), Self::OptionalI32(theirs, present)) => {
                mine.common_bytes(theirs) + held.common_bytes(present)
            }
            (Self::OptionalU32(mine, held), Self::OptionalU32(theirs, present)) => {
                mine.common_bytes(theirs) + held.common_bytes(present)
            }
            (Self::OptionalI64(mine, held), Self::OptionalI64(theirs, present)) => {
                mine.common_bytes(theirs) + held.common_bytes(present)
            }
            (Self::OptionalU64(mine, held), Self::OptionalU64(theirs, present)) => {
                mine.common_bytes(theirs) + held.common_bytes(present)
            }
            (Self::U32List(mine), Self::U32List(theirs)) => mine.common_bytes(theirs),
            (Self::U64List(mine), Self::U64List(theirs)) => mine.common_bytes(theirs),
            (Self::Opaque(mine), Self::Opaque(theirs)) => mine.common_bytes(theirs),
            _ => 0,
        }
    }

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

    pub(crate) fn grow(&mut self, slots: usize) -> io::Result<()> {
        plain_columns!(self, values => values.resize(slots),
            Self::Bool(values) => values.resize(slots),
            Self::OptionalI32(values, _) => values.resize(slots),
            Self::OptionalU32(values, _) => values.resize(slots),
            Self::OptionalI64(values, _) => values.resize(slots),
            Self::OptionalU64(values, _) => values.resize(slots),
            Self::U32List(values) => values.resize(slots),
            Self::U64List(values) => values.resize(slots),
            Self::Opaque(values) => values.resize(slots),
        )
    }

    pub fn share(&self) -> io::Result<Self> {
        Ok(match self {
            Self::U8(values) => Self::U8(values.share()?),
            Self::I8(values) => Self::I8(values.share()?),
            Self::U16(values) => Self::U16(values.share()?),
            Self::I16(values) => Self::I16(values.share()?),
            Self::U32(values) => Self::U32(values.share()?),
            Self::I32(values) => Self::I32(values.share()?),
            Self::U64(values) => Self::U64(values.share()?),
            Self::I64(values) => Self::I64(values.share()?),
            Self::Bool(values) => Self::Bool(values.share()?),
            Self::OptionalI32(values, bits) => Self::OptionalI32(values.share()?, bits.share()?),
            Self::OptionalU32(values, bits) => Self::OptionalU32(values.share()?, bits.share()?),
            Self::OptionalI64(values, bits) => Self::OptionalI64(values.share()?, bits.share()?),
            Self::OptionalU64(values, bits) => Self::OptionalU64(values.share()?, bits.share()?),
            Self::Bytes(values) => Self::Bytes(values.share()?),
            Self::U32List(values) => Self::U32List(values.share()?),
            Self::U64List(values) => Self::U64List(values.share()?),
            Self::Opaque(values) => Self::Opaque(values.share()?),
        })
    }

    fn downgrade(&self) -> io::Result<WeakColumn> {
        Ok(match self {
            Self::U8(values) => WeakColumn::U8(values.downgrade()?),
            Self::I8(values) => WeakColumn::I8(values.downgrade()?),
            Self::U16(values) => WeakColumn::U16(values.downgrade()?),
            Self::I16(values) => WeakColumn::I16(values.downgrade()?),
            Self::U32(values) => WeakColumn::U32(values.downgrade()?),
            Self::I32(values) => WeakColumn::I32(values.downgrade()?),
            Self::U64(values) => WeakColumn::U64(values.downgrade()?),
            Self::I64(values) => WeakColumn::I64(values.downgrade()?),
            Self::Bool(values) => WeakColumn::Bool(values.downgrade()?),
            Self::OptionalI32(values, bits) => {
                WeakColumn::OptionalI32(values.downgrade()?, bits.downgrade()?)
            }
            Self::OptionalU32(values, bits) => {
                WeakColumn::OptionalU32(values.downgrade()?, bits.downgrade()?)
            }
            Self::OptionalI64(values, bits) => {
                WeakColumn::OptionalI64(values.downgrade()?, bits.downgrade()?)
            }
            Self::OptionalU64(values, bits) => {
                WeakColumn::OptionalU64(values.downgrade()?, bits.downgrade()?)
            }
            Self::Bytes(values) => WeakColumn::Bytes(values.downgrade()?),
            Self::U32List(values) => WeakColumn::U32List(values.downgrade()?),
            Self::U64List(values) => WeakColumn::U64List(values.downgrade()?),
            Self::Opaque(values) => WeakColumn::Opaque(values.downgrade()?),
        })
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
                let value = bool_of(one(value)?)?;
                column.put_each(slots, past, |_| Ok(value))
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

    fn release(&mut self, slots: &[u32]) -> io::Result<()> {
        fn zero<T: Copy + Default + PartialEq>(
            values: &mut Paged<T>,
            slots: &[u32],
        ) -> io::Result<()> {
            for &slot in slots {
                let at = slot as usize;
                if values.get(at).is_some_and(|value| *value != T::default())
                    && let Some(value) = values.get_mut(at)?
                {
                    *value = T::default();
                }
            }
            Ok(())
        }
        fn missing(bits: &mut PagedBits, slots: &[u32]) -> io::Result<()> {
            for &slot in slots {
                if bits.get(slot) {
                    bits.put(slot, false)?;
                }
            }
            Ok(())
        }
        match self {
            Self::U8(values) => zero(values, slots),
            Self::I8(values) => zero(values, slots),
            Self::U16(values) => zero(values, slots),
            Self::I16(values) => zero(values, slots),
            Self::U32(values) | Self::Bytes(values) => zero(values, slots),
            Self::I32(values) => zero(values, slots),
            Self::U64(values) => zero(values, slots),
            Self::I64(values) => zero(values, slots),
            Self::Bool(values) => {
                for &slot in slots {
                    if values.get(slot as usize) == Some(&true) {
                        values.put(slot, false)?;
                    }
                }
                Ok(())
            }
            Self::OptionalI32(_, bits)
            | Self::OptionalU32(_, bits)
            | Self::OptionalI64(_, bits)
            | Self::OptionalU64(_, bits) => missing(bits, slots),
            Self::U32List(values) => values.clear(slots, past),
            Self::U64List(values) => values.clear(slots, past),
            Self::Opaque(values) => values.clear(slots, past),
        }
    }
}

macro_rules! some {
    ($upgraded:expr) => {
        match $upgraded {
            Some(upgraded) => upgraded,
            None => return Ok(None),
        }
    };
}

impl WeakColumn {
    fn upgrade(&self) -> io::Result<Option<Column>> {
        Ok(Some(match self {
            Self::U8(values) => Column::U8(some!(values.upgrade()?)),
            Self::I8(values) => Column::I8(some!(values.upgrade()?)),
            Self::U16(values) => Column::U16(some!(values.upgrade()?)),
            Self::I16(values) => Column::I16(some!(values.upgrade()?)),
            Self::U32(values) => Column::U32(some!(values.upgrade()?)),
            Self::I32(values) => Column::I32(some!(values.upgrade()?)),
            Self::U64(values) => Column::U64(some!(values.upgrade()?)),
            Self::I64(values) => Column::I64(some!(values.upgrade()?)),
            Self::Bool(values) => Column::Bool(some!(Bools::upgrade(values)?)),
            Self::OptionalI32(values, bits) => {
                Column::OptionalI32(some!(values.upgrade()?), some!(PagedBits::upgrade(bits)?))
            }
            Self::OptionalU32(values, bits) => {
                Column::OptionalU32(some!(values.upgrade()?), some!(PagedBits::upgrade(bits)?))
            }
            Self::OptionalI64(values, bits) => {
                Column::OptionalI64(some!(values.upgrade()?), some!(PagedBits::upgrade(bits)?))
            }
            Self::OptionalU64(values, bits) => {
                Column::OptionalU64(some!(values.upgrade()?), some!(PagedBits::upgrade(bits)?))
            }
            Self::Bytes(values) => Column::Bytes(some!(values.upgrade()?)),
            Self::U32List(values) => Column::U32List(some!(values.upgrade()?)),
            Self::U64List(values) => Column::U64List(some!(values.upgrade()?)),
            Self::Opaque(values) => Column::Opaque(some!(values.upgrade()?)),
        }))
    }
}

fn write_plain<T: Le + Copy>(
    writer: &mut Writer,
    column: &Paged<T>,
    slots: &[u32],
) -> io::Result<()> {
    let values = engine_allocation::collect_vec(slots.iter().map(|&slot| column[slot as usize]))?;
    writer.values(&values)
}

fn write_ints<T: Int + Copy>(
    writer: &mut Writer,
    column: &Paged<T>,
    slots: &[u32],
) -> io::Result<()> {
    let values = engine_allocation::collect_vec(slots.iter().map(|&slot| column[slot as usize]))?;
    writer.ints(&values)
}

fn write_optional<T: Int + Copy>(
    writer: &mut Writer,
    column: &Paged<T>,
    bits: &PagedBits,
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
    column: &Lists<T>,
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

fn patch_cells<T: Copy + Default>(
    column: &mut Lists<T>,
    slots: &[u32],
    patches: Patches<'_>,
    items: &[T],
) -> io::Result<()> {
    same_length(slots, patches.hunks.len())?;
    let (mut edit, mut item) = (0usize, 0usize);
    column.rewrite(slots, past, |index, held, cell| {
        let count = patches.hunks[index] as usize;
        let edits = patches
            .edits
            .get(edit..edit + count)
            .ok_or_else(|| invalid("PATCH run ends inside its hunks"))?;
        edit += count;
        let removed: usize = edits.iter().map(|edit| edit.1 as usize).sum();
        let inserted: usize = edits.iter().map(|edit| edit.2 as usize).sum();
        engine_allocation::reserve(cell, held.len().saturating_sub(removed) + inserted)?;
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
        Ok(())
    })?;
    if edit != patches.edits.len() || item != items.len() {
        return Err(invalid("PATCH run carries edits past its values"));
    }
    Ok(())
}

fn mismatch(column: Kind, values: Kind) -> io::Error {
    invalid(format!("{values:?} values for a {column:?} column"))
}

const EMPTY: u32 = u32::MAX;

fn tag(key: u128) -> u32 {
    let mut x = (key as u64) ^ ((key >> 64) as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^= x >> 31;
    (x as u32) & 0x7FFF_FFFF
}

#[derive(Clone, Debug)]
pub(crate) enum Keys {
    U64(Paged<u64>),
    U128(Paged<u128>),
}

impl Default for Keys {
    fn default() -> Self {
        Self::U64(Paged::default())
    }
}

#[derive(Debug)]
enum WeakKeys {
    U64(WeakPaged<u64>),
    U128(WeakPaged<u128>),
}

impl WeakKeys {
    fn upgrade(&self) -> io::Result<Option<Keys>> {
        Ok(match self {
            Self::U64(keys) => keys.upgrade()?.map(Keys::U64),
            Self::U128(keys) => keys.upgrade()?.map(Keys::U128),
        })
    }

    fn alive(&self) -> bool {
        match self {
            Self::U64(keys) => keys.alive(),
            Self::U128(keys) => keys.alive(),
        }
    }
}

impl Keys {
    #[inline]
    fn get(&self, at: usize) -> Option<u128> {
        match self {
            Self::U64(keys) => keys.get(at).map(|&key| u128::from(key)),
            Self::U128(keys) => keys.get(at).copied(),
        }
    }

    #[inline]
    fn at(&self, index: usize) -> u128 {
        match self {
            Self::U64(keys) => u128::from(keys[index]),
            Self::U128(keys) => keys[index],
        }
    }

    fn resize(&mut self, slots: usize) -> io::Result<()> {
        match self {
            Self::U64(keys) => keys.resize(slots),
            Self::U128(keys) => keys.resize(slots),
        }
    }

    fn write_slots(&mut self, slots: &[u32], values: &[u128]) -> io::Result<()> {
        if let Self::U64(narrow) = self
            && values.iter().any(|&key| key > u128::from(u64::MAX))
        {
            let widened = engine_allocation::collect_vec(narrow.iter().map(|&key| u128::from(key)))?;
            *self = Self::U128(Paged::from_slice(&widened, 0)?);
        }
        match self {
            Self::U64(keys) => keys.each_mut(slots, past, |index, held| {
                *held = values[index] as u64;
                Ok(())
            }),
            Self::U128(keys) => keys.each_mut(slots, past, |index, held| {
                *held = values[index];
                Ok(())
            }),
        }
    }

    fn matches_prefix(&self, prefix: &[u128]) -> bool {
        match self {
            Self::U64(keys) => {
                if prefix.len() > keys.len() {
                    return false;
                }
                keys.iter()
                    .take(prefix.len())
                    .zip(prefix)
                    .all(|(&held, &want)| u128::from(held) == want)
            }
            Self::U128(keys) => keys.matches_prefix(prefix),
        }
    }

    fn share(&self) -> io::Result<Self> {
        Ok(match self {
            Self::U64(keys) => Self::U64(keys.share()?),
            Self::U128(keys) => Self::U128(keys.share()?),
        })
    }

    fn downgrade(&self) -> io::Result<WeakKeys> {
        Ok(match self {
            Self::U64(keys) => WeakKeys::U64(keys.downgrade()?),
            Self::U128(keys) => WeakKeys::U128(keys.downgrade()?),
        })
    }

    fn held_bytes(&self) -> usize {
        match self {
            Self::U64(keys) => keys.held_bytes(),
            Self::U128(keys) => keys.held_bytes(),
        }
    }

    fn owned_bytes(&self) -> usize {
        match self {
            Self::U64(keys) => keys.owned_bytes(),
            Self::U128(keys) => keys.owned_bytes(),
        }
    }

    fn shared_bytes(&self, other: &Self) -> usize {
        match (self, other) {
            (Self::U64(mine), Self::U64(theirs)) => mine.shared_bytes(theirs),
            (Self::U128(mine), Self::U128(theirs)) => mine.shared_bytes(theirs),
            _ => 0,
        }
    }

    fn common_bytes(&self, other: &Self) -> usize {
        match (self, other) {
            (Self::U64(mine), Self::U64(theirs)) => mine.common_bytes(theirs),
            (Self::U128(mine), Self::U128(theirs)) => mine.common_bytes(theirs),
            _ => 0,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Slots {
    buckets: Paged<u32>,
    len: usize,
    max_slot: bool,
}

impl Default for Slots {
    fn default() -> Self {
        Self {
            buckets: Paged::filled(0, EMPTY).expect("an empty index allocates nothing"),
            len: 0,
            max_slot: false,
        }
    }
}

impl Slots {
    fn held_bytes(&self) -> usize {
        self.buckets.held_bytes()
    }

    fn owned_bytes(&self) -> usize {
        self.buckets.owned_bytes()
    }

    fn shared_bytes(&self, other: &Self) -> usize {
        self.buckets.shared_bytes(&other.buckets)
    }

    fn common_bytes(&self, other: &Self) -> usize {
        self.buckets.common_bytes(&other.buckets)
    }

    fn share(&self) -> io::Result<Self> {
        Ok(Self {
            buckets: self.buckets.share()?,
            len: self.len,
            max_slot: self.max_slot,
        })
    }

    fn put(&mut self, at: usize, bucket: u32) -> io::Result<()> {
        *self
            .buckets
            .get_mut(at)?
            .expect("a probe stays among the buckets") = bucket;
        Ok(())
    }

    pub(crate) fn get(&self, keys: &Keys, key: u128) -> Option<u32> {
        if self.max_slot && keys.get(EMPTY as usize) == Some(key) {
            return Some(EMPTY);
        }
        if self.buckets.is_empty() {
            return None;
        }
        let mask = self.buckets.len() - 1;
        let mut at = tag(key) as usize & mask;
        loop {
            let slot = self.buckets[at];
            if slot == EMPTY {
                return None;
            }
            if keys.get(slot as usize) == Some(key) {
                return Some(slot);
            }
            at = (at + 1) & mask;
        }
    }

    pub(crate) fn reserve(&mut self, keys: &Keys, additional: usize) -> io::Result<()> {
        let live = self
            .len
            .checked_add(additional)
            .ok_or_else(engine_allocation::oom)?;
        if live.saturating_mul(4) <= self.buckets.len().saturating_mul(3) {
            return Ok(());
        }
        let mut buckets = 8_usize;
        while buckets.saturating_mul(5) < live.saturating_mul(8) {
            buckets = buckets
                .checked_mul(2)
                .filter(|&buckets| buckets <= 1 << 31)
                .ok_or_else(engine_allocation::oom)?;
        }
        let mut rebuilt = engine_allocation::filled(buckets, EMPTY)?;
        let mask = buckets - 1;
        for &slot in self.buckets.iter() {
            if slot == EMPTY {
                continue;
            }
            let mut at = tag(keys.at(slot as usize)) as usize & mask;
            while rebuilt[at] != EMPTY {
                at = (at + 1) & mask;
            }
            rebuilt[at] = slot;
        }
        self.buckets = Paged::filled(0, EMPTY)?;
        self.buckets = Paged::from_slice(&rebuilt, EMPTY)?;
        Ok(())
    }

    pub(crate) fn insert(&mut self, keys: &Keys, key: u128, slot: u32) -> io::Result<()> {
        if slot == EMPTY {
            self.max_slot = true;
            return Ok(());
        }
        self.reserve(keys, 1)?;
        let mask = self.buckets.len() - 1;
        let mut at = tag(key) as usize & mask;
        loop {
            let held = self.buckets[at];
            if held == EMPTY {
                self.put(at, slot)?;
                self.len += 1;
                return Ok(());
            }
            if keys.get(held as usize) == Some(key) {
                return self.put(at, slot);
            }
            at = (at + 1) & mask;
        }
    }

    pub(crate) fn remove(&mut self, keys: &Keys, key: u128, slot: u32) -> io::Result<()> {
        if slot == EMPTY {
            self.max_slot = false;
            return Ok(());
        }
        if self.buckets.is_empty() {
            return Ok(());
        }
        let mask = self.buckets.len() - 1;
        let mut hole = tag(key) as usize & mask;
        loop {
            let held = self.buckets[hole];
            if held == EMPTY {
                return Ok(());
            }
            if held == slot {
                break;
            }
            hole = (hole + 1) & mask;
        }
        self.len -= 1;
        let mut cursor = (hole + 1) & mask;
        loop {
            let held = self.buckets[cursor];
            if held == EMPTY {
                self.put(hole, EMPTY)?;
                return Ok(());
            }
            let home = tag(keys.at(held as usize)) as usize & mask;
            if (hole.wrapping_sub(home) & mask) < (cursor.wrapping_sub(home) & mask) {
                self.put(hole, held)?;
                hole = cursor;
            }
            cursor = (cursor + 1) & mask;
        }
    }
}

fn ascending(slots: &[u32]) -> io::Result<()> {
    if slots.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(invalid("Slots of a run are not strictly ascending"));
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub struct Table {
    pub(crate) keys: Keys,
    pub(crate) live: Bits,
    pub(crate) columns: Vec<Column>,
    pub(crate) unrecorded: Vec<Bits>,
    pub(crate) slots: usize,
    pub(crate) by_key: Slots,
}

#[derive(Debug)]
pub struct WeakTable {
    keys: WeakKeys,
    live: Bits,
    columns: Vec<WeakColumn>,
    unrecorded: Vec<Bits>,
    slots: usize,
    by_key: (WeakPaged<u32>, usize, bool),
}

impl WeakTable {
    pub fn upgrade(&self) -> io::Result<Option<Table>> {
        let Some(keys) = self.keys.upgrade()? else {
            return Ok(None);
        };
        let Some(buckets) = self.by_key.0.upgrade()? else {
            return Ok(None);
        };
        let mut columns = engine_allocation::vec(self.columns.len())?;
        for column in &self.columns {
            let Some(column) = column.upgrade()? else {
                return Ok(None);
            };
            columns.push(column);
        }
        let mut unrecorded = engine_allocation::vec(self.unrecorded.len())?;
        for bits in &self.unrecorded {
            unrecorded.push(bits.copy()?);
        }
        Ok(Some(Table {
            keys,
            live: self.live.copy()?,
            columns,
            unrecorded,
            slots: self.slots,
            by_key: Slots {
                buckets,
                len: self.by_key.1,
                max_slot: self.by_key.2,
            },
        }))
    }

    pub fn alive(&self) -> bool {
        self.keys.alive()
    }

    pub fn held_bytes(&self) -> usize {
        self.live.held_bytes() + self.unrecorded.iter().map(Bits::held_bytes).sum::<usize>()
    }
}

impl PartialEq for Table {
    fn eq(&self, other: &Self) -> bool {
        self.slots == other.slots
            && self.kinds().eq(other.kinds())
            && self.live.ones().eq(other.live.ones())
            && self.live.ones().all(|slot| {
                self.keys.at(slot as usize) == other.keys.at(slot as usize)
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
            keys: Keys::default(),
            live: Bits::default(),
            columns: engine_allocation::collect_vec(kinds.iter().map(|&kind| Column::new(kind)))?,
            unrecorded: engine_allocation::collect_vec(kinds.iter().map(|_| Bits::default()))?,
            slots: 0,
            by_key: Slots::default(),
        })
    }

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
        let live_slots = engine_allocation::collect_vec(live.ones())?;
        table.keys.write_slots(&live_slots, keys)?;
        for (&slot, &key) in live_slots.iter().zip(keys) {
            table.key_slot(key, slot)?;
        }
        table.live = live;
        Ok(table)
    }

    pub fn share(&self) -> io::Result<Self> {
        let mut columns = engine_allocation::vec(self.columns.len())?;
        for column in &self.columns {
            columns.push(column.share()?);
        }
        let mut unrecorded = engine_allocation::vec(self.unrecorded.len())?;
        for bits in &self.unrecorded {
            unrecorded.push(bits.copy()?);
        }
        Ok(Self {
            keys: self.keys.share()?,
            live: self.live.copy()?,
            columns,
            unrecorded,
            slots: self.slots,
            by_key: self.by_key.share()?,
        })
    }

    pub fn downgrade(&self) -> io::Result<WeakTable> {
        let mut columns = engine_allocation::vec(self.columns.len())?;
        for column in &self.columns {
            columns.push(column.downgrade()?);
        }
        let mut unrecorded = engine_allocation::vec(self.unrecorded.len())?;
        for bits in &self.unrecorded {
            unrecorded.push(bits.copy()?);
        }
        Ok(WeakTable {
            keys: self.keys.downgrade()?,
            live: self.live.copy()?,
            columns,
            unrecorded,
            slots: self.slots,
            by_key: (
                self.by_key.buckets.downgrade()?,
                self.by_key.len,
                self.by_key.max_slot,
            ),
        })
    }

    pub fn kinds(&self) -> impl Iterator<Item = Kind> + '_ {
        self.columns.iter().map(Column::kind)
    }

    pub(crate) fn write_column(&self, column: usize, writer: &mut Writer, live: &[u32]) -> io::Result<()> {
        self.columns
            .get(column)
            .ok_or_else(|| invalid("A column past the table"))?
            .write(writer, live)
    }

    pub fn slots(&self) -> usize {
        self.slots
    }

    pub fn held_bytes(&self) -> usize {
        self.keys.held_bytes()
            + self.live.held_bytes()
            + self.unrecorded.iter().map(Bits::held_bytes).sum::<usize>()
            + self.columns.iter().map(Column::held_bytes).sum::<usize>()
            + self.by_key.held_bytes()
    }

    pub fn owned_bytes(&self) -> usize {
        self.keys.owned_bytes()
            + self.live.held_bytes()
            + self.unrecorded.iter().map(Bits::held_bytes).sum::<usize>()
            + self.columns.iter().map(Column::owned_bytes).sum::<usize>()
            + self.by_key.owned_bytes()
    }

    pub fn shared_bytes(&self, other: &Self) -> usize {
        self.keys.shared_bytes(&other.keys)
            + self
                .columns
                .iter()
                .zip(&other.columns)
                .map(|(mine, theirs)| mine.shared_bytes(theirs))
                .sum::<usize>()
            + self.by_key.shared_bytes(&other.by_key)
    }

    pub fn common_bytes(&self, other: &Self) -> usize {
        self.keys.common_bytes(&other.keys)
            + self
                .columns
                .iter()
                .zip(&other.columns)
                .map(|(mine, theirs)| mine.common_bytes(theirs))
                .sum::<usize>()
            + self.by_key.common_bytes(&other.by_key)
    }

    pub fn live(&self) -> &Bits {
        &self.live
    }

    pub fn key(&self, slot: u32) -> Option<u128> {
        self.live.get(slot).then(|| self.keys.at(slot as usize))
    }

    pub(crate) fn keys_match_prefix(&self, keys: &[u128]) -> bool {
        if keys.len() > self.slots || !self.live.all_set(keys.len()) {
            return false;
        }
        self.keys.matches_prefix(keys)
    }

    pub fn slot(&self, key: u128) -> Option<u32> {
        self.by_key.get(&self.keys, key)
    }

    fn index(&mut self, additional: usize) -> io::Result<()> {
        self.by_key.reserve(&self.keys, additional)
    }

    fn key_slot(&mut self, key: u128, slot: u32) -> io::Result<()> {
        if let Some(held) = self
            .by_key
            .get(&self.keys, key)
            .filter(|&held| held != slot)
        {
            return Err(invalid(format!(
                "Row key {key:#x} at slot {slot} is already held at slot {held}"
            )));
        }
        self.by_key.insert(&self.keys, key, slot)
    }

    pub fn column(&self, column: usize) -> Option<&Column> {
        self.columns.get(column)
    }

    pub fn recorded(&self, column: usize, slot: u32) -> bool {
        self.unrecorded
            .get(column)
            .is_some_and(|bits| !bits.get(slot))
    }

    pub fn widen(&mut self, kinds: &[Kind]) -> io::Result<()> {
        engine_allocation::reserve(&mut self.columns, kinds.len())?;
        engine_allocation::reserve(&mut self.unrecorded, kinds.len())?;
        for &kind in kinds {
            let mut column = Column::new(kind);
            column.grow(self.slots)?;
            let unrecorded = Bits::from_words(engine_allocation::copy_vec(self.live.words())?)?;
            self.columns.push(column);
            self.unrecorded.push(unrecorded);
        }
        Ok(())
    }

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

    pub(crate) fn unrecorded_bits(&self, column: usize) -> Option<&Bits> {
        self.unrecorded.get(column)
    }

    pub(crate) fn unrecorded_slots(&self, column: usize) -> impl Iterator<Item = u32> + '_ {
        self.unrecorded[column]
            .ones()
            .filter(|&slot| self.live.get(slot))
    }

    fn recorded_again(&mut self, column: usize, slots: &[u32]) {
        if let Some(bits) = self.unrecorded.get_mut(column) {
            for &slot in slots {
                bits.clear(slot);
            }
        }
    }

    pub(crate) fn resize(&mut self, slots: usize) -> io::Result<()> {
        if slots <= self.slots {
            return Ok(());
        }
        self.keys.resize(slots)?;
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
        self.keys.write_slots(slots, keys)?;
        for (&slot, &key) in slots.iter().zip(keys) {
            self.key_slot(key, slot)?;
            self.live.set(slot)?;
        }
        Ok(())
    }

    pub fn delete(&mut self, slots: &[u32]) -> io::Result<()> {
        self.live_slots(slots)?;
        for &slot in slots {
            self.live.clear(slot);
            self.by_key
                .remove(&self.keys, self.keys.at(slot as usize), slot)?;
            for bits in &mut self.unrecorded {
                bits.clear(slot);
            }
        }
        for column in &mut self.columns {
            column.release(slots)?;
        }
        Ok(())
    }

    pub fn set(&mut self, column: usize, slots: &[u32], values: Values<'_>) -> io::Result<()> {
        self.live_slots(slots)?;
        self.column_mut(column)?.set(slots, values)?;
        self.recorded_again(column, slots);
        Ok(())
    }

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
