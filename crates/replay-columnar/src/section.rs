
use crate::{
    invalid,
    leaf::{Int, Reader, Writer},
    state::{Kind, Table, Values},
};
use std::io;

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Set,
    Add,
    Noop,
    Fill,
    Patch,
}

#[derive(Clone, Copy, Debug)]
pub enum PatchItems<'a> {
    U32(&'a [u32]),
    U64(&'a [u64]),
    Bytes(&'a [u8]),
}

#[derive(Clone, Copy, Debug)]
pub struct Patches<'a> {
    pub hunks: &'a [u32],
    pub edits: &'a [(u32, u32, u32)],
    pub items: PatchItems<'a>,
}

impl Op {
    fn tag(self) -> u8 {
        match self {
            Self::Set => 0,
            Self::Add => 1,
            Self::Noop => 2,
            Self::Fill => 3,
            Self::Patch => 4,
        }
    }

    fn of(tag: u8) -> io::Result<Self> {
        Ok(match tag {
            0 => Self::Set,
            1 => Self::Add,
            2 => Self::Noop,
            3 => Self::Fill,
            4 => Self::Patch,
            other => return Err(invalid(format!("Column run op {other}"))),
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Run<'a> {
    pub column: usize,
    pub op: Op,
    pub positions: &'a [u32],
    pub values: Option<Values<'a>>,
    pub except: &'a [u32],
    pub patches: Option<Patches<'a>>,
}

#[derive(Clone, Copy, Debug)]
pub struct TableHour<'a> {
    pub read: Option<&'a [u32]>,
    pub changed: &'a [u32],
    pub deleted: &'a [u32],
    pub inserted: &'a [u32],
    pub keys: &'a [u128],
    pub inserts: &'a [Values<'a>],
    pub runs: &'a [Run<'a>],
    pub unrecorded: &'a [(usize, &'a [u32])],
    pub touched: &'a [u32],
}

pub(crate) fn put_values(writer: &mut Writer, values: Values<'_>) -> io::Result<()> {
    match values {
        Values::U8(values) | Values::Bool(values) => writer.values(values),
        Values::I8(values) => writer.values(values),
        Values::U16(values) => writer.ints(values),
        Values::I16(values) => writer.ints(values),
        Values::U32(values) | Values::Bytes(values) => writer.ints(values),
        Values::I32(values) => writer.ints(values),
        Values::U64(values) => writer.ints(values),
        Values::I64(values) => writer.ints(values),
        Values::OptionalI32 { present, values } => optional(writer, present, values),
        Values::OptionalU32 { present, values } => optional(writer, present, values),
        Values::OptionalI64 { present, values } => optional(writer, present, values),
        Values::OptionalU64 { present, values } => optional(writer, present, values),
        Values::U32List { ends, items } => {
            writer.lengths(ends)?;
            writer.ints(items)
        }
        Values::U64List { ends, items } => {
            writer.lengths(ends)?;
            writer.ints(items)
        }
        Values::Opaque { ends, items } => {
            writer.lengths(ends)?;
            writer.raw(items)
        }
    }
}

fn optional<T: Int>(writer: &mut Writer, present: &[u64], values: &[T]) -> io::Result<()> {
    writer.values(present)?;
    writer.ints(values)
}

fn put_patches(writer: &mut Writer, kind: Kind, run: &Run<'_>) -> io::Result<()> {
    let Some(patches) = run.patches else {
        return Err(invalid("PATCH run carries no edits"));
    };
    let inserted: u64 = patches.edits.iter().map(|edit| u64::from(edit.2)).sum();
    let items = match patches.items {
        PatchItems::U32(items) if kind == Kind::U32List => items.len(),
        PatchItems::U64(items) if kind == Kind::U64List => items.len(),
        PatchItems::Bytes(items) if kind == Kind::Opaque => items.len(),
        _ => {
            return Err(invalid(format!(
                "PATCH items differ from a {kind:?} column"
            )));
        }
    };
    let hunks: u64 = patches.hunks.iter().map(|&count| u64::from(count)).sum();
    if patches.hunks.len() != run.positions.len()
        || hunks != patches.edits.len() as u64
        || inserted != items as u64
    {
        return Err(invalid("PATCH edits differ from its positions or items"));
    }
    writer.ints(patches.hunks)?;
    for &(skip, removed, inserted) in patches.edits {
        writer.ints(&[skip, removed, inserted])?;
    }
    match patches.items {
        PatchItems::U32(items) => writer.ints(items),
        PatchItems::U64(items) => writer.ints(items),
        PatchItems::Bytes(items) => writer.raw(items),
    }
}

fn counts(values: Values<'_>, count: usize) -> bool {
    match values {
        Values::U8(values) | Values::Bool(values) => values.len() == count,
        Values::I8(values) => values.len() == count,
        Values::U16(values) => values.len() == count,
        Values::I16(values) => values.len() == count,
        Values::U32(values) | Values::Bytes(values) => values.len() == count,
        Values::I32(values) => values.len() == count,
        Values::U64(values) => values.len() == count,
        Values::I64(values) => values.len() == count,
        Values::OptionalI32 { present, .. }
        | Values::OptionalU32 { present, .. }
        | Values::OptionalI64 { present, .. }
        | Values::OptionalU64 { present, .. } => present.len() == count.div_ceil(64),
        Values::U32List { ends, .. }
        | Values::U64List { ends, .. }
        | Values::Opaque { ends, .. } => ends.len() == count,
    }
}

pub fn write(kinds: &[Kind], hour: &TableHour<'_>) -> io::Result<Vec<u8>> {
    write_with(kinds, hour, Vec::new())
}

pub(crate) fn write_with(
    kinds: &[Kind],
    hour: &TableHour<'_>,
    bytes: Vec<u8>,
) -> io::Result<Vec<u8>> {
    if hour.keys.len() != hour.inserted.len()
        || (!hour.inserted.is_empty() && hour.inserts.len() != kinds.len())
    {
        return Err(invalid(
            "Inserts carry other than one key per slot and one run per column",
        ));
    }
    let mut writer = Writer::reusing(bytes);
    if let Some(read) = hour.read {
        if !within(hour.changed, read) || !within(hour.deleted, read) {
            return Err(invalid(
                "A partly read table changes a slot it did not read",
            ));
        }
        writer.slots(read)?;
    }
    writer.slots(hour.changed)?;
    writer.slots(hour.deleted)?;
    writer.slots(hour.inserted)?;
    writer.keys(hour.keys)?;
    if !hour.inserted.is_empty() {
        for (&kind, &values) in kinds.iter().zip(hour.inserts) {
            if values.kind() != kind || !counts(values, hour.inserted.len()) {
                return Err(invalid("Insert values differ from their column or slots"));
            }
            put_values(&mut writer, values)?;
        }
    }
    writer.varint(hour.runs.len() as u64)?;
    for run in hour.runs {
        let kind = *kinds
            .get(run.column)
            .ok_or_else(|| invalid("Run names a column past the schema"))?;
        writer.varint(run.column as u64)?;
        writer.u8(run.op.tag())?;
        if run.patches.is_some() != (run.op == Op::Patch) {
            return Err(invalid("Only a PATCH run carries edits"));
        }
        match (run.op, run.values) {
            (Op::Fill, Some(value)) => {
                writer.slots(run.except)?;
                if value.kind() != kind || !counts(value, 1) {
                    return Err(invalid("FILL value differs from its column's kind"));
                }
                put_values(&mut writer, value)?;
            }
            (Op::Noop, None) => writer.positions(hour.changed.len(), run.positions)?,
            (Op::Set | Op::Add, Some(values)) => {
                writer.positions(hour.changed.len(), run.positions)?;
                let expected = match run.op {
                    Op::Add => base(kind)?,
                    _ => kind,
                };
                if values.kind() != expected || !counts(values, run.positions.len()) {
                    return Err(invalid("Run values differ from its column or positions"));
                }
                put_values(&mut writer, values)?;
            }
            (Op::Patch, None) => {
                writer.positions(hour.changed.len(), run.positions)?;
                put_patches(&mut writer, kind, run)?;
            }
            _ => return Err(invalid("Run carries values its op does not take")),
        }
    }
    writer.varint(hour.unrecorded.len() as u64)?;
    for (at, &(column, slots)) in hour.unrecorded.iter().enumerate() {
        if column >= kinds.len()
            || at > 0 && hour.unrecorded[at - 1].0 >= column
            || slots.is_empty()
            || slots.iter().any(|slot| {
                hour.changed.binary_search(slot).is_err()
                    && hour.inserted.binary_search(slot).is_err()
            })
        {
            return Err(invalid(
                "Unrecorded cells are not changed or inserted slots of columns in order",
            ));
        }
        writer.varint(column as u64)?;
        writer.slots(slots)?;
    }
    writer.slots(hour.touched)?;
    Ok(writer.into_bytes())
}

pub(crate) fn base(kind: Kind) -> io::Result<Kind> {
    Ok(match kind {
        Kind::U8
        | Kind::I8
        | Kind::U16
        | Kind::I16
        | Kind::U32
        | Kind::I32
        | Kind::U64
        | Kind::I64 => kind,
        Kind::OptionalI32 => Kind::I32,
        Kind::OptionalU32 => Kind::U32,
        Kind::OptionalI64 => Kind::I64,
        Kind::OptionalU64 => Kind::U64,
        other => return Err(invalid(format!("ADD to a {other:?} column"))),
    })
}

#[derive(Default)]
pub struct Scratch {
    u8s: Vec<u8>,
    i8s: Vec<i8>,
    u16s: Vec<u16>,
    i16s: Vec<i16>,
    u32s: Vec<u32>,
    i32s: Vec<i32>,
    u64s: Vec<u64>,
    i64s: Vec<i64>,
    words: Vec<u64>,
    ends: Vec<u32>,
    keys: Vec<u128>,
    hunks: Vec<u32>,
    edits: Vec<u32>,
    triples: Vec<(u32, u32, u32)>,
    slots: Vec<u32>,
    positions: Vec<u32>,
    except: Vec<u32>,
    masks: Vec<u64>,
}

pub(crate) fn within(slots: &[u32], of: &[u32]) -> bool {
    let mut of = of.iter().peekable();
    slots.iter().all(|slot| {
        while of.next_if(|&&candidate| candidate < *slot).is_some() {}
        of.next_if_eq(&slot).is_some()
    })
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Changes {
    pub truncated: bool,
    pub read: Vec<u32>,
    pub changed: Vec<u32>,
    pub masks: Vec<u64>,
    pub written: Vec<u64>,
    pub deleted: Vec<u32>,
    pub deleted_keys: Vec<u128>,
    pub inserted: Vec<u32>,
    pub touched: Vec<u32>,
    pub filled: Vec<usize>,
    pub fill_slots: Vec<u32>,
    pub fill_masks: Vec<u64>,
}

pub(crate) fn column_bit(column: usize) -> u64 {
    u32::try_from(column)
        .ok()
        .and_then(|column| 1_u64.checked_shl(column))
        .unwrap_or(0)
}

fn mark(masks: &mut [u64], positions: &[u32], bit: u64) {
    for &position in positions {
        if let Some(mask) = masks.get_mut(position as usize) {
            *mask |= bit;
        }
    }
}

macro_rules! read_plain {
    ($reader:expr, $buffer:expr, $count:expr, $variant:ident) => {{
        $reader.values($count, &mut $buffer)?;
        Values::$variant(&$buffer)
    }};
}

macro_rules! read_ints {
    ($reader:expr, $buffer:expr, $count:expr, $variant:ident) => {{
        $reader.ints($count, &mut $buffer)?;
        Values::$variant(&$buffer)
    }};
}

macro_rules! read_optional {
    ($reader:expr, $scratch:expr, $buffer:ident, $count:expr, $variant:ident) => {{
        $reader.values($count.div_ceil(64), &mut $scratch.words)?;
        let present = present_count(&$scratch.words, $count)?;
        $reader.ints(present, &mut $scratch.$buffer)?;
        Values::$variant {
            present: &$scratch.words,
            values: &$scratch.$buffer,
        }
    }};
}

macro_rules! read_cells {
    ($reader:expr, $scratch:expr, $buffer:ident, $count:expr, $variant:ident, $items:ident) => {{
        $reader.lengths($count, &mut $scratch.ends)?;
        let items = $scratch.ends.last().map_or(0, |&end| end as usize);
        $reader.$items(items, &mut $scratch.$buffer)?;
        Values::$variant {
            ends: &$scratch.ends,
            items: &$scratch.$buffer,
        }
    }};
}

fn present_count(words: &[u64], count: usize) -> io::Result<usize> {
    if let Some(&last) = words.last()
        && count % 64 != 0
        && last >> (count % 64) != 0
    {
        return Err(invalid("Presence bits pass their values"));
    }
    Ok(words.iter().map(|word| word.count_ones() as usize).sum())
}

pub(crate) fn read_values<'s>(
    reader: &mut Reader<'_>,
    scratch: &'s mut Scratch,
    kind: Kind,
    count: usize,
) -> io::Result<Values<'s>> {
    Ok(match kind {
        Kind::U8 => read_plain!(reader, scratch.u8s, count, U8),
        Kind::Bool => read_plain!(reader, scratch.u8s, count, Bool),
        Kind::I8 => read_plain!(reader, scratch.i8s, count, I8),
        Kind::U16 => read_ints!(reader, scratch.u16s, count, U16),
        Kind::I16 => read_ints!(reader, scratch.i16s, count, I16),
        Kind::U32 => read_ints!(reader, scratch.u32s, count, U32),
        Kind::Bytes => read_ints!(reader, scratch.u32s, count, Bytes),
        Kind::I32 => read_ints!(reader, scratch.i32s, count, I32),
        Kind::U64 => read_ints!(reader, scratch.u64s, count, U64),
        Kind::I64 => read_ints!(reader, scratch.i64s, count, I64),
        Kind::OptionalI32 => read_optional!(reader, scratch, i32s, count, OptionalI32),
        Kind::OptionalU32 => read_optional!(reader, scratch, u32s, count, OptionalU32),
        Kind::OptionalI64 => read_optional!(reader, scratch, i64s, count, OptionalI64),
        Kind::OptionalU64 => read_optional!(reader, scratch, u64s, count, OptionalU64),
        Kind::U32List => read_cells!(reader, scratch, u32s, count, U32List, ints),
        Kind::U64List => read_cells!(reader, scratch, u64s, count, U64List, ints),
        Kind::Opaque => read_cells!(reader, scratch, u8s, count, Opaque, values),
    })
}

fn slots_at(changed: &[u32], positions: &[u32], into: &mut Vec<u32>) -> io::Result<()> {
    into.clear();
    engine_allocation::reserve(into, positions.len())?;
    into.extend(positions.iter().map(|&position| changed[position as usize]));
    Ok(())
}

fn fill_run(
    reader: &mut Reader<'_>,
    scratch: &mut Scratch,
    table: &mut Table,
    column: usize,
    kind: Kind,
    except: &[u32],
    targets: &mut Vec<u32>,
) -> io::Result<()> {
    let value = read_values(reader, scratch, kind, 1)?;
    table.fill(column, except, value, targets)
}

pub(crate) fn union(
    slots: &mut Vec<u32>,
    masks: &mut Vec<u64>,
    more: &[u32],
    bit: u64,
    merged: &mut Vec<u32>,
    merged_masks: &mut Vec<u64>,
) -> io::Result<()> {
    if slots.as_slice() == more {
        for mask in masks.iter_mut() {
            *mask |= bit;
        }
        return Ok(());
    }
    if slots.is_empty() {
        engine_allocation::reserve(slots, more.len())?;
        slots.extend_from_slice(more);
        masks.clear();
        engine_allocation::reserve(masks, more.len())?;
        masks.resize(more.len(), bit);
        return Ok(());
    }
    merged.clear();
    merged_masks.clear();
    engine_allocation::reserve(merged, slots.len() + more.len())?;
    engine_allocation::reserve(merged_masks, slots.len() + more.len())?;
    let mut left = slots.iter().copied().zip(masks.iter().copied()).peekable();
    let mut right = more.iter().copied().peekable();
    loop {
        let (slot, mask) = match (left.peek().copied(), right.peek().copied()) {
            (Some((a, mask)), Some(b)) if a < b => {
                left.next();
                (a, mask)
            }
            (Some((a, _)), Some(b)) if b < a => {
                right.next();
                (b, bit)
            }
            (Some((a, mask)), Some(_)) => {
                left.next();
                right.next();
                (a, mask | bit)
            }
            (Some((a, mask)), None) => {
                left.next();
                (a, mask)
            }
            (None, Some(b)) => {
                right.next();
                (b, bit)
            }
            (None, None) => break,
        };
        merged.push(slot);
        merged_masks.push(mask);
    }
    std::mem::swap(slots, merged);
    std::mem::swap(masks, merged_masks);
    Ok(())
}

fn value_run(
    reader: &mut Reader<'_>,
    scratch: &mut Scratch,
    table: &mut Table,
    column: usize,
    kind: Kind,
    op: Op,
    slots: &[u32],
) -> io::Result<()> {
    if op == Op::Add {
        let deltas = read_values(reader, scratch, base(kind)?, slots.len())?;
        table.add(column, slots, deltas)
    } else {
        let values = read_values(reader, scratch, kind, slots.len())?;
        table.set(column, slots, values)
    }
}

fn patch_run(
    reader: &mut Reader<'_>,
    scratch: &mut Scratch,
    table: &mut Table,
    column: usize,
    kind: Kind,
    slots: &[u32],
) -> io::Result<()> {
    reader.ints(slots.len(), &mut scratch.hunks)?;
    let hunks = scratch
        .hunks
        .iter()
        .try_fold(0usize, |total, &count| total.checked_add(count as usize))
        .ok_or_else(|| invalid("PATCH hunks overflow"))?;
    reader.ints(hunks.saturating_mul(3), &mut scratch.edits)?;
    scratch.triples.clear();
    engine_allocation::reserve(&mut scratch.triples, hunks)?;
    scratch.triples.extend(
        scratch
            .edits
            .chunks_exact(3)
            .map(|edit| (edit[0], edit[1], edit[2])),
    );
    let inserted = scratch
        .triples
        .iter()
        .try_fold(0usize, |total, edit| total.checked_add(edit.2 as usize))
        .ok_or_else(|| invalid("PATCH items overflow"))?;
    let items = match kind {
        Kind::U32List => {
            reader.ints(inserted, &mut scratch.u32s)?;
            PatchItems::U32(&scratch.u32s)
        }
        Kind::U64List => {
            reader.ints(inserted, &mut scratch.u64s)?;
            PatchItems::U64(&scratch.u64s)
        }
        Kind::Opaque => {
            reader.values(inserted, &mut scratch.u8s)?;
            PatchItems::Bytes(&scratch.u8s)
        }
        other => return Err(invalid(format!("PATCH to a {other:?} column"))),
    };
    table.patch(
        column,
        slots,
        Patches {
            hunks: &scratch.hunks,
            edits: &scratch.triples,
            items,
        },
    )
}

pub fn apply(
    bytes: &[u8],
    kinds: &[Kind],
    truncated: bool,
    table: &mut Table,
    scratch: &mut Scratch,
    changes: &mut Changes,
) -> io::Result<()> {
    let mut reader = Reader::new(bytes);
    changes.truncated = truncated;
    changes.read.clear();
    if truncated {
        reader.slots(&mut changes.read)?;
    }
    reader.slots(&mut changes.changed)?;
    reader.slots(&mut changes.deleted)?;
    reader.slots(&mut changes.inserted)?;
    changes.masks.clear();
    engine_allocation::reserve(&mut changes.masks, changes.changed.len())?;
    changes.masks.resize(changes.changed.len(), 0);
    changes.written.clear();
    engine_allocation::reserve(&mut changes.written, changes.changed.len())?;
    changes.written.resize(changes.changed.len(), 0);
    changes.touched.clear();
    changes.filled.clear();
    changes.fill_slots.clear();
    changes.fill_masks.clear();
    if truncated && changes.read.iter().any(|&slot| !table.live().get(slot)) {
        return Err(invalid("A partly read table reads a slot holding no row"));
    }
    if truncated
        && (!within(&changes.changed, &changes.read) || !within(&changes.deleted, &changes.read))
    {
        return Err(invalid(
            "A partly read table changes a slot it did not read",
        ));
    }
    changes.deleted_keys.clear();
    engine_allocation::reserve(&mut changes.deleted_keys, changes.deleted.len())?;
    changes
        .deleted_keys
        .extend(changes.deleted.iter().filter_map(|&slot| table.key(slot)));
    table.delete(&changes.deleted)?;
    let fresh = changes
        .inserted
        .iter()
        .filter(|&&slot| slot as usize >= table.slots());
    for (at, &slot) in fresh.enumerate() {
        if !cfg!(feature = "sparse-inserts") && slot as usize != table.slots() + at {
            return Err(invalid(format!(
                "Insert names slot {slot} past the {} slots and those it adds",
                table.slots()
            )));
        }
    }
    let inserted = changes.inserted.len();
    reader.keys(inserted, &mut scratch.keys)?;
    if inserted > 0 {
        table.insert_keys(&changes.inserted, &scratch.keys)?;
        for (column, &kind) in kinds.iter().enumerate() {
            let values = read_values(&mut reader, scratch, kind, inserted)?;
            table.set(column, &changes.inserted, values)?;
        }
    }
    let runs = reader.count()?;
    for _ in 0..runs {
        let column = usize::try_from(reader.varint()?)
            .map_err(|_| invalid("Run names a column past the schema"))?;
        let kind = *kinds
            .get(column)
            .ok_or_else(|| invalid("Run names a column past the schema"))?;
        match Op::of(reader.u8()?)? {
            Op::Fill => {
                reader.slots(&mut scratch.except)?;
                let except = std::mem::take(&mut scratch.except);
                let mut targets = std::mem::take(&mut scratch.slots);
                let filled = fill_run(
                    &mut reader,
                    scratch,
                    table,
                    column,
                    kind,
                    &except,
                    &mut targets,
                );
                scratch.except = except;
                let filled = filled.and_then(|()| {
                    if truncated
                        && targets.iter().any(|slot| {
                            changes.read.binary_search(slot).is_err()
                                && changes.inserted.binary_search(slot).is_err()
                        })
                    {
                        return Err(invalid("A partly read table fills a slot it did not read"));
                    }
                    union(
                        &mut changes.fill_slots,
                        &mut changes.fill_masks,
                        &targets,
                        column_bit(column),
                        &mut scratch.positions,
                        &mut scratch.masks,
                    )
                });
                scratch.slots = targets;
                filled?;
                engine_allocation::reserve(&mut changes.filled, 1)?;
                changes.filled.push(column);
            }
            Op::Noop => {
                reader.positions(changes.changed.len(), &mut scratch.positions)?;
                mark(&mut changes.masks, &scratch.positions, column_bit(column));
            }
            Op::Patch => {
                reader.positions(changes.changed.len(), &mut scratch.positions)?;
                mark(&mut changes.masks, &scratch.positions, column_bit(column));
                mark(&mut changes.written, &scratch.positions, column_bit(column));
                let mut slots = std::mem::take(&mut scratch.slots);
                let applied = slots_at(&changes.changed, &scratch.positions, &mut slots)
                    .and_then(|()| patch_run(&mut reader, scratch, table, column, kind, &slots));
                scratch.slots = slots;
                applied?;
            }
            op @ (Op::Set | Op::Add) => {
                reader.positions(changes.changed.len(), &mut scratch.positions)?;
                mark(&mut changes.masks, &scratch.positions, column_bit(column));
                mark(&mut changes.written, &scratch.positions, column_bit(column));
                let mut slots = std::mem::take(&mut scratch.slots);
                let applied =
                    slots_at(&changes.changed, &scratch.positions, &mut slots).and_then(|()| {
                        value_run(&mut reader, scratch, table, column, kind, op, &slots)
                    });
                scratch.slots = slots;
                applied?;
            }
        }
    }
    let unrecorded = reader.count()?;
    let mut previous = None;
    for _ in 0..unrecorded {
        let column = reader.count()?;
        if column >= kinds.len() || previous.is_some_and(|before| before >= column) {
            return Err(invalid("Unrecorded cells name columns out of order"));
        }
        previous = Some(column);
        reader.slots(&mut scratch.slots)?;
        if scratch.slots.is_empty()
            || scratch.slots.iter().any(|slot| {
                changes.changed.binary_search(slot).is_err()
                    && changes.inserted.binary_search(slot).is_err()
            })
        {
            return Err(invalid(
                "Unrecorded cells name a slot the hour did not change",
            ));
        }
        table.unrecord(column, &scratch.slots)?;
        for slot in &scratch.slots {
            if let Ok(position) = changes.changed.binary_search(slot) {
                changes.masks[position] |= column_bit(column);
                changes.written[position] |= column_bit(column);
            }
        }
    }
    reader.slots(&mut changes.touched)?;
    reader.finish()
}
