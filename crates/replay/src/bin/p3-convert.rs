//! Offline converter: a recording to the columnar change grammar, through
//! replay-columnar's own writers, gated by applying every section to the
//! reader's state and comparing it with the recording's rows: every written
//! slot each hour, every live slot at every unit's end and at the end, and every
//! hour's stored metrics against its manifest.
//!
//! Output: a version 3 `H4COLUMN` recording (`replay_columnar::envelope`), a run per
//! continuous span of one branch. A run's units are replay-columnar's framing (the
//! manifest stream and one stream per table) on the envelope's unit grid and, every
//! `--restart-units` units, a restart: the dictionary chunk of the run's strings first used
//! since its previous restart (`walk::Dictionary::read_chunk` bytes), then each table's
//! replay-columnar restart bytes. Every extent is one zstd frame at `--level`. Restarts,
//! pages, run directories and the root take a `--window-log` window; each unit's streams
//! share a walker's `--budget-mb` decode memory.

use replay::{
    container::{Replay, ReplayStream},
    native_columns::column,
};
use replay_columnar::{
    envelope::{self, Extent, IndexPage, RestartDirectory, RestartTable, RunDirectory},
    framing,
    leaf::Writer as Bytes,
    manifest::{self, Manifest, Section, Status},
    restart::{self, Acquisition},
    section::{self, Op, PatchItems, Patches, Run, Scratch, TableHour},
    state::{Bits, Cell, Kind as Column, Table, Values},
};
use replay_data::{
    codec::Decoder,
    rows::{Kind, TableId, TableRows},
};
use replay_frame::frame::{Metrics, READ_TIME_NOT_RECORDED, TableUpdate, TableView};
use serde_json::json;
use std::{
    collections::{BTreeSet, HashMap},
    ffi::OsString,
    fs::File,
    io::{self, BufWriter, Seek, Write},
    path::{Path, PathBuf},
    time::Instant,
};

const FILL_MINIMUM: usize = 16;

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[derive(Default)]
struct Dictionary {
    ids: HashMap<Vec<u8>, u32>,
    bytes: Vec<Vec<u8>>,
    appended: usize,
    restarted: usize,
}

impl Dictionary {
    fn id(&mut self, bytes: &[u8]) -> u32 {
        if let Some(&id) = self.ids.get(bytes) {
            return id;
        }
        let id = self.bytes.len() as u32;
        self.ids.insert(bytes.to_vec(), id);
        self.bytes.push(bytes.to_vec());
        id
    }
}

/// A field's value parsed from replay-data field bytes.
#[derive(Debug, PartialEq, Eq)]
enum Parsed<'a> {
    Int(i128),
    Missing,
    Bytes(&'a [u8]),
    U32s(Vec<u32>),
    U64s(Vec<u64>),
}

fn int<const N: usize>(bytes: &[u8], signed: bool) -> io::Result<i128> {
    let array: [u8; N] = bytes.try_into().map_err(|_| invalid("fixed field width"))?;
    let mut wide = [0_u8; 16];
    wide[..N].copy_from_slice(&array);
    let value = i128::from_le_bytes(wide);
    let shift = 128 - 8 * N as u32;
    Ok(if signed { (value << shift) >> shift } else { value })
}

fn prefixed(field: &[u8]) -> io::Result<(usize, &[u8])> {
    let count = field.get(..4).ok_or_else(|| invalid("field has no length"))?;
    Ok((u32::from_le_bytes(count.try_into().unwrap()) as usize, &field[4..]))
}

fn parse(kind: Kind, field: &[u8]) -> io::Result<Parsed<'_>> {
    let optional = |width: usize, signed: bool| -> io::Result<Parsed<'_>> {
        match field.split_first() {
            Some((0, [])) => Ok(Parsed::Missing),
            Some((1, rest)) if rest.len() == width => Ok(Parsed::Int(match width {
                4 => int::<4>(rest, signed)?,
                _ => int::<8>(rest, signed)?,
            })),
            _ => Err(invalid("optional field is malformed")),
        }
    };
    Ok(match kind {
        Kind::U8 | Kind::Bool => Parsed::Int(int::<1>(field, false)?),
        Kind::I8 => Parsed::Int(int::<1>(field, true)?),
        Kind::U16 => Parsed::Int(int::<2>(field, false)?),
        Kind::I16 => Parsed::Int(int::<2>(field, true)?),
        Kind::U32 => Parsed::Int(int::<4>(field, false)?),
        Kind::I32 => Parsed::Int(int::<4>(field, true)?),
        Kind::U64 => Parsed::Int(int::<8>(field, false)?),
        Kind::I64 => Parsed::Int(int::<8>(field, true)?),
        Kind::OptionalI32 => optional(4, true)?,
        Kind::OptionalU32 => optional(4, false)?,
        Kind::OptionalI64 => optional(8, true)?,
        Kind::OptionalU64 => optional(8, false)?,
        Kind::Bytes | Kind::Opaque => {
            let (length, bytes) = prefixed(field)?;
            if bytes.len() != length {
                return Err(invalid("byte field length"));
            }
            Parsed::Bytes(bytes)
        }
        Kind::U32List => {
            let (count, items) = prefixed(field)?;
            if items.len() != count * 4 {
                return Err(invalid("list field count"));
            }
            Parsed::U32s(items.chunks_exact(4).map(|item| u32::from_le_bytes(item.try_into().unwrap())).collect())
        }
        Kind::U64List => {
            let (count, items) = prefixed(field)?;
            if items.len() != count * 8 {
                return Err(invalid("list field count"));
            }
            Parsed::U64s(items.chunks_exact(8).map(|item| u64::from_le_bytes(item.try_into().unwrap())).collect())
        }
    })
}

/// The cell a parsed field must read back as.
fn same(cell: Cell<'_>, parsed: &Parsed<'_>, dictionary: &Dictionary) -> bool {
    match (cell, parsed) {
        (Cell::U8(value), Parsed::Int(expected)) => i128::from(value) == *expected,
        (Cell::I8(value), Parsed::Int(expected)) => i128::from(value) == *expected,
        (Cell::U16(value), Parsed::Int(expected)) => i128::from(value) == *expected,
        (Cell::I16(value), Parsed::Int(expected)) => i128::from(value) == *expected,
        (Cell::U32(value), Parsed::Int(expected)) => i128::from(value) == *expected,
        (Cell::I32(value), Parsed::Int(expected)) => i128::from(value) == *expected,
        (Cell::U64(value), Parsed::Int(expected)) => i128::from(value) == *expected,
        (Cell::I64(value), Parsed::Int(expected)) => i128::from(value) == *expected,
        (Cell::Bool(value), Parsed::Int(expected)) => i128::from(value) == *expected,
        (Cell::OptionalI32(value), parsed) => optional_same(value.map(i128::from), parsed),
        (Cell::OptionalU32(value), parsed) => optional_same(value.map(i128::from), parsed),
        (Cell::OptionalI64(value), parsed) => optional_same(value.map(i128::from), parsed),
        (Cell::OptionalU64(value), parsed) => optional_same(value.map(i128::from), parsed),
        (Cell::Bytes(id), Parsed::Bytes(expected)) => dictionary.bytes.get(id as usize).is_some_and(|bytes| bytes.as_slice() == *expected),
        (Cell::Opaque(value), Parsed::Bytes(expected)) => value == *expected,
        (Cell::U32List(value), Parsed::U32s(expected)) => value == expected.as_slice(),
        (Cell::U64List(value), Parsed::U64s(expected)) => value == expected.as_slice(),
        _ => false,
    }
}

/// The plain integer kind an ADD to `kind` carries, and its width in bytes.
fn add_base(kind: Kind) -> Option<(Kind, u32, bool)> {
    Some(match kind {
        Kind::U8 => (Kind::U8, 1, false),
        Kind::I8 => (Kind::I8, 1, true),
        Kind::U16 => (Kind::U16, 2, false),
        Kind::I16 => (Kind::I16, 2, true),
        Kind::U32 | Kind::OptionalU32 => (Kind::U32, 4, false),
        Kind::I32 | Kind::OptionalI32 => (Kind::I32, 4, true),
        Kind::U64 | Kind::OptionalU64 => (Kind::U64, 8, false),
        Kind::I64 | Kind::OptionalI64 => (Kind::I64, 8, true),
        _ => return None,
    })
}

/// Bytes a value needs once zigzagged at its width (signed) or as is.
fn significant(value: i128, signed: bool) -> u32 {
    let magnitude = if signed { ((value << 1) ^ (value >> 127)) as u128 } else { value as u128 };
    (128 - magnitude.leading_zeros()).div_ceil(8)
}

/// `new - old` wrapped to `width` bytes, as a signed value of that width.
fn wrapped_delta(new: i128, old: i128, width: u32) -> i128 {
    let shift = 128 - 8 * width;
    (new.wrapping_sub(old) << shift) >> shift
}

fn optional_same(value: Option<i128>, parsed: &Parsed<'_>) -> bool {
    match (value, parsed) {
        (None, Parsed::Missing) => true,
        (Some(value), Parsed::Int(expected)) => value == *expected,
        _ => false,
    }
}

/// One column's values, built from field bytes for a run, an insert or a FILL.
enum Built {
    U8(Vec<u8>),
    I8(Vec<i8>),
    U16(Vec<u16>),
    I16(Vec<i16>),
    U32(Vec<u32>),
    I32(Vec<i32>),
    U64(Vec<u64>),
    I64(Vec<i64>),
    Bool(Vec<u8>),
    OptionalI32(Vec<u64>, Vec<i32>, usize),
    OptionalU32(Vec<u64>, Vec<u32>, usize),
    OptionalI64(Vec<u64>, Vec<i64>, usize),
    OptionalU64(Vec<u64>, Vec<u64>, usize),
    Bytes(Vec<u32>),
    U32List(Vec<u32>, Vec<u32>),
    U64List(Vec<u32>, Vec<u64>),
    Opaque(Vec<u32>, Vec<u8>),
}

fn mark(present: &mut Vec<u64>, count: &mut usize, set: bool) {
    if *count % 64 == 0 {
        present.push(0);
    }
    if set {
        present[*count / 64] |= 1 << (*count % 64);
    }
    *count += 1;
}

impl Built {
    fn new(kind: Kind) -> Self {
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
            Kind::OptionalI32 => Self::OptionalI32(Vec::new(), Vec::new(), 0),
            Kind::OptionalU32 => Self::OptionalU32(Vec::new(), Vec::new(), 0),
            Kind::OptionalI64 => Self::OptionalI64(Vec::new(), Vec::new(), 0),
            Kind::OptionalU64 => Self::OptionalU64(Vec::new(), Vec::new(), 0),
            Kind::Bytes => Self::Bytes(Vec::new()),
            Kind::U32List => Self::U32List(Vec::new(), Vec::new()),
            Kind::U64List => Self::U64List(Vec::new(), Vec::new()),
            Kind::Opaque => Self::Opaque(Vec::new(), Vec::new()),
        }
    }

    fn push(&mut self, kind: Kind, field: &[u8], dictionary: &mut Dictionary) -> io::Result<()> {
        let parsed = parse(kind, field)?;
        let value = |parsed: &Parsed<'_>| match parsed {
            Parsed::Int(value) => Ok(*value),
            _ => Err(invalid("expected an integer")),
        };
        match (self, &parsed) {
            (Self::U8(values), parsed) | (Self::Bool(values), parsed) => values.push(value(parsed)? as u8),
            (Self::I8(values), parsed) => values.push(value(parsed)? as i8),
            (Self::U16(values), parsed) => values.push(value(parsed)? as u16),
            (Self::I16(values), parsed) => values.push(value(parsed)? as i16),
            (Self::U32(values), parsed) => values.push(value(parsed)? as u32),
            (Self::I32(values), parsed) => values.push(value(parsed)? as i32),
            (Self::U64(values), parsed) => values.push(value(parsed)? as u64),
            (Self::I64(values), parsed) => values.push(value(parsed)? as i64),
            (Self::OptionalI32(present, values, count), parsed) => {
                mark(present, count, *parsed != Parsed::Missing);
                if *parsed != Parsed::Missing {
                    values.push(value(parsed)? as i32);
                }
            }
            (Self::OptionalU32(present, values, count), parsed) => {
                mark(present, count, *parsed != Parsed::Missing);
                if *parsed != Parsed::Missing {
                    values.push(value(parsed)? as u32);
                }
            }
            (Self::OptionalI64(present, values, count), parsed) => {
                mark(present, count, *parsed != Parsed::Missing);
                if *parsed != Parsed::Missing {
                    values.push(value(parsed)? as i64);
                }
            }
            (Self::OptionalU64(present, values, count), parsed) => {
                mark(present, count, *parsed != Parsed::Missing);
                if *parsed != Parsed::Missing {
                    values.push(value(parsed)? as u64);
                }
            }
            (Self::Bytes(ids), Parsed::Bytes(bytes)) => ids.push(dictionary.id(bytes)),
            (Self::Opaque(ends, items), Parsed::Bytes(bytes)) => {
                items.extend_from_slice(bytes);
                ends.push(items.len() as u32);
            }
            (Self::U32List(ends, items), Parsed::U32s(list)) => {
                items.extend_from_slice(list);
                ends.push(items.len() as u32);
            }
            (Self::U64List(ends, items), Parsed::U64s(list)) => {
                items.extend_from_slice(list);
                ends.push(items.len() as u32);
            }
            _ => return Err(invalid("field does not fit its column")),
        }
        Ok(())
    }

    /// A stand-in for a cell the recording did not record, which the section marks unrecorded.
    fn push_unrecorded(&mut self, dictionary: &mut Dictionary) {
        match self {
            Self::U8(values) | Self::Bool(values) => values.push(0),
            Self::I8(values) => values.push(0),
            Self::U16(values) => values.push(0),
            Self::I16(values) => values.push(0),
            Self::U32(values) => values.push(0),
            Self::I32(values) => values.push(0),
            Self::U64(values) => values.push(0),
            Self::I64(values) => values.push(0),
            Self::OptionalI32(present, _, count)
            | Self::OptionalU32(present, _, count)
            | Self::OptionalI64(present, _, count)
            | Self::OptionalU64(present, _, count) => mark(present, count, false),
            Self::Bytes(ids) => ids.push(dictionary.id(b"")),
            Self::Opaque(ends, items) => ends.push(items.len() as u32),
            Self::U32List(ends, items) => ends.push(items.len() as u32),
            Self::U64List(ends, items) => ends.push(items.len() as u32),
        }
    }

    /// A plain integer, wrapped to the column's width.
    fn push_int(&mut self, value: i128) -> io::Result<()> {
        match self {
            Self::U8(values) => values.push(value as u8),
            Self::I8(values) => values.push(value as i8),
            Self::U16(values) => values.push(value as u16),
            Self::I16(values) => values.push(value as i16),
            Self::U32(values) => values.push(value as u32),
            Self::I32(values) => values.push(value as i32),
            Self::U64(values) => values.push(value as u64),
            Self::I64(values) => values.push(value as i64),
            _ => return Err(invalid("ADD to a column without integer deltas")),
        }
        Ok(())
    }

    fn values(&self) -> Values<'_> {
        match self {
            Self::U8(values) => Values::U8(values),
            Self::I8(values) => Values::I8(values),
            Self::U16(values) => Values::U16(values),
            Self::I16(values) => Values::I16(values),
            Self::U32(values) => Values::U32(values),
            Self::I32(values) => Values::I32(values),
            Self::U64(values) => Values::U64(values),
            Self::I64(values) => Values::I64(values),
            Self::Bool(values) => Values::Bool(values),
            Self::OptionalI32(present, values, _) => Values::OptionalI32 { present, values },
            Self::OptionalU32(present, values, _) => Values::OptionalU32 { present, values },
            Self::OptionalI64(present, values, _) => Values::OptionalI64 { present, values },
            Self::OptionalU64(present, values, _) => Values::OptionalU64 { present, values },
            Self::Bytes(ids) => Values::Bytes(ids),
            Self::U32List(ends, items) => Values::U32List { ends, items },
            Self::U64List(ends, items) => Values::U64List { ends, items },
            Self::Opaque(ends, items) => Values::Opaque { ends, items },
        }
    }
}

/// A row's field bytes and where each field ends; a field the recording did not record has
/// no bytes and its bit in `unrecorded`.
struct Row {
    bytes: Vec<u8>,
    ends: Vec<u32>,
    unrecorded: u64,
}

impl Row {
    fn read(kinds: &[Kind], rows: &TableRows, index: usize, buffer: &mut Vec<u8>) -> io::Result<Self> {
        buffer.clear();
        let unrecorded = rows.encode_row_recorded_fields(index, buffer)?;
        let mut decoder = Decoder::new(buffer.as_slice());
        let mut ends = Vec::with_capacity(kinds.len());
        for (column, kind) in kinds.iter().enumerate() {
            if unrecorded >> column & 1 == 0 {
                kind.decode(&mut decoder)?;
            }
            ends.push((buffer.len() - decoder.remaining()) as u32);
        }
        Ok(Self {
            bytes: buffer.clone(),
            ends,
            unrecorded,
        })
    }

    fn recorded(&self, column: usize) -> bool {
        self.unrecorded >> column & 1 == 0
    }

    /// Whether `column` holds the same cell in both rows, recorded or not.
    fn same_cell(&self, other: &Self, column: usize) -> bool {
        self.recorded(column) == other.recorded(column) && self.field(column) == other.field(column)
    }

    fn field(&self, column: usize) -> &[u8] {
        let start = if column == 0 { 0 } else { self.ends[column - 1] as usize };
        &self.bytes[start..self.ends[column] as usize]
    }
}

#[derive(Clone, Copy, Default)]
struct Totals {
    sections: u64,
    bytes: u64,
    inserts: u64,
    deletes: u64,
    changed: u64,
    set_values: u64,
    add_values: u64,
    patch_values: u64,
    fills: u64,
    fill_slots: u64,
    fill_exceptions: u64,
    truncated: u64,
}

struct Writer {
    id: TableId,
    kinds: &'static [Kind],
    columns: Vec<Column>,
    slots: HashMap<u128, u32>,
    rows: Vec<Option<(u128, Row)>>,
    free: BTreeSet<u32>,
    /// The reader's state, applied from every written section.
    gate: Table,
    totals: Totals,
    status: Status,
    acquisition: Acquisition,
    /// A truncated table's rows read this hour: its read and inserted slots.
    view: Vec<u32>,
    /// Per column: values written (SET, ADD, FILL, insert) and their field bytes.
    column_values: Vec<(u64, u64)>,
    /// This table's sections written so far in the current unit.
    unit: Bytes,
    /// This table's units: raw and stored bytes.
    stored: (u64, u64),
    /// This table's restarts: raw bytes, and compressed alone.
    restart_bytes: (u64, u64),
}

fn position(rows: &TableRows, key: u128) -> io::Result<usize> {
    let (mut low, mut high) = (0, rows.len());
    while low < high {
        let middle = (low + high) / 2;
        match rows.get(middle).ok_or_else(|| invalid("row index"))?.key().cmp(&key) {
            std::cmp::Ordering::Less => low = middle + 1,
            std::cmp::Ordering::Greater => high = middle,
            std::cmp::Ordering::Equal => return Ok(middle),
        }
    }
    Err(invalid("changed key is absent from its hour's rows"))
}

/// Varint bytes of `value`.
fn varint_len(value: u64) -> u64 {
    u64::from(64 - value.max(1).leading_zeros()).div_ceil(7)
}

/// Hunks turning `held` into `new`, as `(skip, removed, inserted)`, and the inserted items.
/// Equal lengths edit their differing runs, merged across fewer than `gap` equal items;
/// otherwise one hunk replaces what lies between the common prefix and suffix.
fn hunks<T: Copy + PartialEq>(held: &[T], new: &[T], gap: usize) -> (Vec<(u32, u32, u32)>, Vec<T>) {
    let prefix = held.iter().zip(new).take_while(|(a, b)| a == b).count();
    let limit = held.len().min(new.len()) - prefix;
    let suffix = held.iter().rev().zip(new.iter().rev()).take(limit).take_while(|(a, b)| a == b).count();
    let (mut edits, mut items) = (Vec::new(), Vec::new());
    if held.len() != new.len() {
        let (removed, inserted) = (held.len() - prefix - suffix, new.len() - prefix - suffix);
        edits.push((prefix as u32, removed as u32, inserted as u32));
        items.extend_from_slice(&new[prefix..prefix + inserted]);
        return (edits, items);
    }
    let end = held.len() - suffix;
    let (mut at, mut kept) = (prefix, 0);
    while at < end {
        if held[at] == new[at] {
            at += 1;
            continue;
        }
        let (start, mut stop, mut probe, mut equal) = (at, at + 1, at + 1, 0);
        while probe < end && equal < gap {
            if held[probe] == new[probe] {
                equal += 1;
            } else {
                equal = 0;
                stop = probe + 1;
            }
            probe += 1;
        }
        edits.push(((start - kept) as u32, (stop - start) as u32, (stop - start) as u32));
        items.extend_from_slice(&new[start..stop]);
        (kept, at) = (stop, stop);
    }
    (edits, items)
}

/// A PATCH run's values as the writer builds them.
enum PatchBuilt {
    U32(Vec<u32>),
    U64(Vec<u64>),
    Bytes(Vec<u8>),
}

/// One column run's payload.
enum RunData {
    Values(Built),
    Patch {
        hunks: Vec<u32>,
        edits: Vec<(u32, u32, u32)>,
        items: PatchBuilt,
    },
}

/// What the source says changed in a table this hour.
enum Changes<'a> {
    Unchanged,
    Keys { changed: &'a [u128], deleted: &'a [u128] },
    Full,
}

impl Writer {
    fn new(id: TableId) -> io::Result<Self> {
        let kinds = id.schema();
        let columns: Vec<Column> = kinds.iter().map(|&kind| column(kind)).collect();
        Ok(Self {
            id,
            kinds,
            gate: Table::new(&columns)?,
            columns,
            slots: HashMap::new(),
            rows: Vec::new(),
            free: BTreeSet::new(),
            totals: Totals::default(),
            status: Status::NotRecorded,
            acquisition: Acquisition::default(),
            view: Vec::new(),
            column_values: vec![(0, 0); kinds.len()],
            unit: Bytes::new(),
            stored: (0, 0),
            restart_bytes: (0, 0),
        })
    }

    /// Empties the table for a new run, keeping its report totals.
    fn reset(&mut self) -> io::Result<()> {
        self.gate = Table::new(&self.columns)?;
        self.slots.clear();
        self.rows.clear();
        self.free.clear();
        self.status = Status::NotRecorded;
        self.acquisition = Acquisition::default();
        self.view.clear();
        self.unit = Bytes::new();
        Ok(())
    }

    /// The run's dictionary chunks, by restart ordinal, holding an id the table's state
    /// references: its recorded text cells at live slots and its refusal. `starts` holds each
    /// chunk's first id, ascending from 0.
    fn chunks(&self, starts: &[u32]) -> Vec<u32> {
        let mut used = vec![false; starts.len()];
        // The last chunk starting at or before an id holds it: an empty chunk starts where
        // the next does.
        let mut take = |id: u32| used[starts.partition_point(|&start| start <= id) - 1] = true;
        if let Some(id) = self.acquisition.refusal {
            take(id);
        }
        for column in 0..self.columns.len() {
            if let Some(replay_columnar::state::Column::Bytes(ids)) = self.gate.column(column) {
                for slot in self.gate.live().ones() {
                    if self.gate.recorded(column, slot)
                        && let Some(&id) = ids.get(slot as usize)
                    {
                        take(id);
                    }
                }
            }
        }
        (0..).zip(used).filter_map(|(chunk, used)| used.then_some(chunk)).collect()
    }

    /// Writes this hour's section, applies it to the gate and checks every slot it wrote.
    fn hour(
        &mut self,
        rows: &TableRows,
        changes: Changes<'_>,
        complete: bool,
        dictionary: &mut Dictionary,
        scratch: &mut Scratch,
        buffer: &mut Vec<u8>,
    ) -> io::Result<Option<(Vec<u8>, Vec<u64>, u8)>> {
        if complete {
            self.view.clear();
        } else {
            self.totals.truncated += 1;
        }
        let mut deleted: Vec<u32> = Vec::new();
        let mut fresh: Vec<(u128, Row)> = Vec::new();
        let mut updated: Vec<(u32, Row)> = Vec::new();
        let mut read: Vec<u32> = Vec::new();
        let mut note = |slots: &HashMap<u128, u32>, key: u128, row: Row| match slots.get(&key) {
            Some(&slot) => {
                read.push(slot);
                updated.push((slot, row));
            }
            None => fresh.push((key, row)),
        };
        match changes {
            Changes::Unchanged => return Ok(None),
            Changes::Keys { changed, deleted: gone } => {
                for key in gone {
                    if let Some(&slot) = self.slots.get(key) {
                        deleted.push(slot);
                    }
                }
                for &key in changed {
                    note(&self.slots, key, Row::read(self.kinds, rows, position(rows, key)?, buffer)?);
                }
            }
            Changes::Full => {
                let mut seen = BTreeSet::new();
                for index in 0..rows.len() {
                    let key = rows.get(index).ok_or_else(|| invalid("row index"))?.key();
                    seen.insert(key);
                    note(&self.slots, key, Row::read(self.kinds, rows, index, buffer)?);
                }
                if complete {
                    deleted.extend(self.slots.iter().filter(|(key, _)| !seen.contains(*key)).map(|(_, &slot)| slot));
                }
            }
        }
        let width = self.kinds.len();
        let mut changed_columns: Vec<Vec<u32>> = vec![Vec::new(); width];
        updated.retain(|(slot, row)| {
            let (_, old) = self.rows[*slot as usize].as_ref().expect("live slot");
            let mut any = false;
            for (column, slots) in changed_columns.iter_mut().enumerate() {
                if !row.same_cell(old, column) {
                    slots.push(*slot);
                    any = true;
                }
            }
            any
        });
        // A partly read table always says which rows it read.
        if complete && deleted.is_empty() && fresh.is_empty() && updated.is_empty() {
            return Ok(None);
        }
        read.sort_unstable();
        updated.sort_unstable_by_key(|(slot, _)| *slot);
        for slots in &mut changed_columns {
            slots.sort_unstable();
        }
        deleted.sort_unstable();
        // Deletes, then inserts at the lowest free slots.
        for &slot in &deleted {
            let (key, _) = self.rows[slot as usize].take().expect("deleted slot is live");
            self.slots.remove(&key);
            self.free.insert(slot);
        }
        fresh.sort_unstable_by_key(|(key, _)| *key);
        let mut inserted: Vec<(u32, u128)> = Vec::with_capacity(fresh.len());
        for (key, row) in fresh {
            let slot = match self.free.pop_first() {
                Some(slot) => slot,
                None => {
                    self.rows.push(None);
                    (self.rows.len() - 1) as u32
                }
            };
            self.slots.insert(key, slot);
            self.rows[slot as usize] = Some((key, row));
            inserted.push((slot, key));
        }
        inserted.sort_unstable_by_key(|(slot, _)| *slot);
        let new_rows: HashMap<u32, &Row> = updated.iter().map(|(slot, row)| (*slot, row)).collect();
        let old_rows: HashMap<u32, &Row> = updated
            .iter()
            .map(|(slot, _)| (*slot, &self.rows[*slot as usize].as_ref().expect("live slot").1))
            .collect();
        // Cells this hour leaves unrecorded: the changed ones, then the inserted ones.
        let mut unrecord: Vec<Vec<u32>> = vec![Vec::new(); width];
        let mut unrecorded_changed: BTreeSet<u32> = BTreeSet::new();
        for (column, slots) in changed_columns.iter_mut().enumerate() {
            slots.retain(|slot| {
                if new_rows[slot].recorded(column) {
                    return true;
                }
                unrecord[column].push(*slot);
                unrecorded_changed.insert(*slot);
                false
            });
        }
        for (slot, _) in &inserted {
            let (_, row) = self.rows[*slot as usize].as_ref().expect("inserted slot");
            for (column, slots) in unrecord.iter_mut().enumerate() {
                if !row.recorded(column) {
                    slots.push(*slot);
                }
            }
        }
        for slots in &mut unrecord {
            slots.sort_unstable();
        }
        // FILL: every changed slot of the column took the same value, covering at least half the live rows.
        let live = self.slots.len();
        let fills: Vec<bool> = changed_columns
            .iter()
            .enumerate()
            .map(|(column, slots)| {
                slots.len() >= FILL_MINIMUM
                    && slots.len() * 2 >= live
                    && slots.iter().all(|slot| new_rows[slot].field(column) == new_rows[&slots[0]].field(column))
            })
            .collect();
        let changed: Vec<u32> = changed_columns
            .iter()
            .zip(&fills)
            .filter(|(_, fill)| !**fill)
            .flat_map(|(slots, _)| slots.iter().copied())
            .chain(unrecorded_changed.iter().copied())
            .collect::<BTreeSet<u32>>()
            .into_iter()
            .collect();
        let index_of: HashMap<u32, u32> = changed.iter().enumerate().map(|(at, slot)| (*slot, at as u32)).collect();
        // Insert values per column.
        let mut inserts: Vec<Built> = Vec::new();
        if !inserted.is_empty() {
            for (column, &kind) in self.kinds.iter().enumerate() {
                let mut built = Built::new(kind);
                for (slot, _) in &inserted {
                    let (_, row) = self.rows[*slot as usize].as_ref().expect("inserted slot");
                    if !row.recorded(column) {
                        built.push_unrecorded(dictionary);
                        continue;
                    }
                    built.push(kind, row.field(column), dictionary)?;
                    self.column_values[column].0 += 1;
                    self.column_values[column].1 += row.field(column).len() as u64;
                }
                inserts.push(built);
            }
        }
        // Runs, ascending column; a column with both takes its SET run first.
        let inserted_slots: Vec<u32> = inserted.iter().map(|(slot, _)| *slot).collect();
        let keys: Vec<u128> = inserted.iter().map(|(_, key)| *key).collect();
        let mut run_values: Vec<(usize, Op, Vec<u32>, RunData, Vec<u32>)> = Vec::new();
        for (column, slots) in changed_columns.iter().enumerate() {
            if slots.is_empty() {
                continue;
            }
            let kind = self.kinds[column];
            let mut built = Built::new(kind);
            if fills[column] {
                built.push(kind, new_rows[&slots[0]].field(column), dictionary)?;
                let covered: BTreeSet<u32> = slots.iter().copied().collect();
                let except: Vec<u32> = self.slots.values().copied().filter(|slot| !covered.contains(slot)).collect::<BTreeSet<u32>>().into_iter().collect();
                self.totals.fills += 1;
                self.totals.fill_slots += slots.len() as u64;
                self.totals.fill_exceptions += except.len() as u64;
                run_values.push((column, Op::Fill, Vec::new(), RunData::Values(built), except));
                continue;
            }
            // An integer change whose delta is shorter than its value is an ADD.
            let mut added: Vec<(u32, i128)> = Vec::new();
            let mut set: Vec<u32> = Vec::new();
            for &slot in slots {
                let delta = add_base(kind).and_then(|(_, width, signed)| {
                    match (parse(kind, new_rows[&slot].field(column)).ok()?, parse(kind, old_rows[&slot].field(column)).ok()?) {
                        (Parsed::Int(new), Parsed::Int(old)) => {
                            let delta = wrapped_delta(new, old, width);
                            (significant(delta, true) < significant(new, signed)).then_some(delta)
                        }
                        _ => None,
                    }
                });
                match delta {
                    Some(delta) => added.push((slot, delta)),
                    None => set.push(slot),
                }
            }
            // An opaque or list change whose hunks against the held cell are shorter is a PATCH.
            let mut patched: Vec<u32> = Vec::new();
            let (mut patch_hunks, mut patch_edits) = (Vec::new(), Vec::new());
            let mut patch_items = match kind {
                Kind::Opaque => Some(PatchBuilt::Bytes(Vec::new())),
                Kind::U32List => Some(PatchBuilt::U32(Vec::new())),
                Kind::U64List => Some(PatchBuilt::U64(Vec::new())),
                _ => None,
            };
            if let Some(items) = &mut patch_items {
                let edit_bytes = |edits: &[(u32, u32, u32)]| {
                    varint_len(edits.len() as u64)
                        + edits.iter().map(|&(a, b, c)| varint_len(a.into()) + varint_len(b.into()) + varint_len(c.into())).sum::<u64>()
                };
                set.retain(|&slot| {
                    let (new, held) = (parse(kind, new_rows[&slot].field(column)), parse(kind, old_rows[&slot].field(column)));
                    let (edits, inserted_bytes, whole_bytes, push): (Vec<(u32, u32, u32)>, u64, u64, Box<dyn FnOnce(&mut PatchBuilt) + '_>) =
                        match (new, held) {
                            (Ok(Parsed::Bytes(new)), Ok(Parsed::Bytes(held))) => {
                                let (edits, inserted) = hunks(held, new, 3);
                                let bytes = inserted.len() as u64;
                                (edits, bytes, varint_len(new.len() as u64) + new.len() as u64, Box::new(move |items| {
                                    if let PatchBuilt::Bytes(items) = items {
                                        items.extend_from_slice(&inserted);
                                    }
                                }))
                            }
                            (Ok(Parsed::U32s(new)), Ok(Parsed::U32s(held))) => {
                                let (edits, inserted) = hunks(&held, &new, 1);
                                let bytes = inserted.iter().map(|&item| varint_len(item.into())).sum();
                                let whole = varint_len(new.len() as u64) + new.iter().map(|&item| varint_len(item.into())).sum::<u64>();
                                (edits, bytes, whole, Box::new(move |items| {
                                    if let PatchBuilt::U32(items) = items {
                                        items.extend_from_slice(&inserted);
                                    }
                                }))
                            }
                            (Ok(Parsed::U64s(new)), Ok(Parsed::U64s(held))) => {
                                let (edits, inserted) = hunks(&held, &new, 1);
                                let bytes = inserted.iter().map(|&item| varint_len(item)).sum();
                                let whole = varint_len(new.len() as u64) + new.iter().map(|&item| varint_len(item)).sum::<u64>();
                                (edits, bytes, whole, Box::new(move |items| {
                                    if let PatchBuilt::U64(items) = items {
                                        items.extend_from_slice(&inserted);
                                    }
                                }))
                            }
                            _ => return true,
                        };
                    if edit_bytes(&edits) + inserted_bytes >= whole_bytes {
                        return true;
                    }
                    patch_hunks.push(edits.len() as u32);
                    patch_edits.extend_from_slice(&edits);
                    push(items);
                    patched.push(slot);
                    false
                });
            }
            if !set.is_empty() {
                for slot in &set {
                    built.push(kind, new_rows[slot].field(column), dictionary)?;
                    self.column_values[column].0 += 1;
                    self.column_values[column].1 += new_rows[slot].field(column).len() as u64;
                }
                self.totals.set_values += set.len() as u64;
                let positions = set.iter().map(|slot| index_of[slot]).collect();
                run_values.push((column, Op::Set, positions, RunData::Values(built), Vec::new()));
            }
            if !patched.is_empty() {
                self.totals.patch_values += patched.len() as u64;
                let positions = patched.iter().map(|slot| index_of[slot]).collect();
                let items = patch_items.expect("patched cell column");
                run_values.push((column, Op::Patch, positions, RunData::Patch { hunks: patch_hunks, edits: patch_edits, items }, Vec::new()));
            }
            if !added.is_empty() {
                let (base, width, _) = add_base(kind).expect("integer column");
                let mut deltas = Built::new(base);
                for &(_, delta) in &added {
                    deltas.push_int(delta)?;
                }
                self.column_values[column].0 += added.len() as u64;
                self.column_values[column].1 += u64::from(width) * added.len() as u64;
                self.totals.add_values += added.len() as u64;
                let positions = added.iter().map(|(slot, _)| index_of[slot]).collect();
                run_values.push((column, Op::Add, positions, RunData::Values(deltas), Vec::new()));
            }
        }
        let insert_values: Vec<Values<'_>> = inserts.iter().map(Built::values).collect();
        let runs: Vec<Run<'_>> = run_values
            .iter()
            .map(|(column, op, positions, data, except)| Run {
                column: *column,
                op: *op,
                positions,
                values: match data {
                    RunData::Values(built) => Some(built.values()),
                    RunData::Patch { .. } => None,
                },
                except,
                patches: match data {
                    RunData::Values(_) => None,
                    RunData::Patch { hunks, edits, items } => Some(Patches {
                        hunks,
                        edits,
                        items: match items {
                            PatchBuilt::U32(items) => PatchItems::U32(items),
                            PatchBuilt::U64(items) => PatchItems::U64(items),
                            PatchBuilt::Bytes(items) => PatchItems::Bytes(items),
                        },
                    }),
                },
            })
            .collect();
        if !complete {
            self.view.clear();
            self.view.extend(read.iter().copied());
            self.view.extend(inserted_slots.iter().copied());
            self.view.sort_unstable();
        }
        let unrecorded: Vec<(usize, &[u32])> = unrecord
            .iter()
            .enumerate()
            .filter(|(_, slots)| !slots.is_empty())
            .map(|(column, slots)| (column, slots.as_slice()))
            .collect();
        let hour_view = TableHour {
            read: (!complete).then_some(read.as_slice()),
            changed: &changed,
            deleted: &deleted,
            inserted: &inserted_slots,
            keys: &keys,
            inserts: &insert_values,
            runs: &runs,
            unrecorded: &unrecorded,
            touched: &[],
        };
        let bytes = section::write(&self.columns, &hour_view)?;
        let mut columns = vec![0_u64; width.div_ceil(64)];
        for (column, _, _, _, _) in &run_values {
            columns[column / 64] |= 1 << (column % 64);
        }
        for &(column, _) in &unrecorded {
            columns[column / 64] |= 1 << (column % 64);
        }
        let mut ops = 0;
        if !inserted.is_empty() {
            ops |= manifest::INSERTS;
        }
        if !deleted.is_empty() {
            ops |= manifest::DELETES;
        }
        if fills.iter().any(|fill| *fill) {
            ops |= manifest::FILLS;
        }
        // Install the updates, apply the section to the gate and check what it wrote.
        let written: Vec<u32> = updated.iter().map(|(slot, _)| *slot).chain(inserted_slots.iter().copied()).collect();
        for (slot, row) in updated {
            self.rows[slot as usize].as_mut().expect("live slot").1 = row;
        }
        let mut applied = section::Changes::default();
        section::apply(&bytes, &self.columns, !complete, &mut self.gate, scratch, &mut applied)?;
        let reused: BTreeSet<u32> = inserted_slots.iter().copied().collect();
        for &slot in deleted.iter().filter(|slot| !reused.contains(slot)) {
            if self.gate.live().get(slot) {
                return Err(invalid(format!("{:?} slot {slot} is live after its delete", self.id)));
            }
        }
        for slot in written {
            self.check(slot, dictionary)?;
        }
        self.totals.sections += 1;
        self.totals.bytes += bytes.len() as u64;
        self.totals.inserts += inserted.len() as u64;
        self.totals.deletes += deleted.len() as u64;
        self.totals.changed += changed.len() as u64;
        Ok(Some((bytes, columns, ops)))
    }

    fn check(&self, slot: u32, dictionary: &Dictionary) -> io::Result<()> {
        let (key, row) = self.rows[slot as usize].as_ref().expect("checked slot is live");
        if !self.gate.live().get(slot) || self.gate.key(slot) != Some(*key) {
            return Err(invalid(format!("{:?} slot {slot} lost its row or key", self.id)));
        }
        for (column, &kind) in self.kinds.iter().enumerate() {
            if self.gate.recorded(column, slot) != row.recorded(column) {
                return Err(invalid(format!("{:?} slot {slot} column {column} recorded differently", self.id)));
            }
            if !row.recorded(column) {
                continue;
            }
            let cell = self
                .gate
                .column(column)
                .and_then(|values| values.cell(slot))
                .ok_or_else(|| invalid("gate column missing"))?;
            if !same(cell, &parse(kind, row.field(column))?, dictionary) {
                return Err(invalid(format!("{:?} slot {slot} column {column} differs", self.id)));
            }
        }
        Ok(())
    }

    /// Every live slot, and nothing else live.
    fn check_all(&self, dictionary: &Dictionary) -> io::Result<()> {
        if self.gate.live().count() != self.slots.len() {
            return Err(invalid(format!("{:?} gate holds a different row count", self.id)));
        }
        for &slot in self.slots.values() {
            self.check(slot, dictionary)?;
        }
        Ok(())
    }
}

/// Each stream's zstd window log for one unit: the largest power of two within its share
/// of a walker's decode-memory budget by raw size, then one step larger for the largest
/// streams while the unit stays within the budget. A window never grows past its stream.
fn unit_windows(raws: &[usize], budget: usize) -> Vec<u32> {
    const SMALLEST: u32 = 15;
    const LARGEST: u32 = 24;
    let total: usize = raws.iter().sum();
    let mut logs: Vec<u32> = raws
        .iter()
        .map(|&raw| {
            let share = (budget as u128 * raw as u128 / total.max(1) as u128) as usize;
            (usize::BITS - 1 - share.max(1).leading_zeros()).clamp(SMALLEST, LARGEST)
        })
        .collect();
    let held = |raw: usize, log: u32| raw.min(1 << log);
    let mut used: usize = raws.iter().zip(&logs).map(|(&raw, &log)| held(raw, log)).sum();
    let mut order: Vec<usize> = (0..raws.len()).collect();
    order.sort_unstable_by_key(|&stream| std::cmp::Reverse(raws[stream]));
    loop {
        let mut grew = false;
        for &stream in &order {
            let (raw, log) = (raws[stream], logs[stream]);
            if log < LARGEST && (1 << log) < raw {
                let added = held(raw, log + 1) - held(raw, log);
                if used + added <= budget {
                    logs[stream] += 1;
                    used += added;
                    grew = true;
                }
            }
        }
        if !grew {
            return logs;
        }
    }
}

/// The output file and the runs written so far.
struct Output {
    file: BufWriter<File>,
    level: i32,
    /// Restart, page and root extents' window; units take theirs from the budget.
    window_log: u32,
    /// A walker's decode-memory budget across the streams of one unit.
    budget: usize,
    /// Per unit grid position, the decode memory its windows need.
    walker_memory: Vec<usize>,
    /// Each finished run's directory.
    runs: Vec<Extent>,
    restarts: u64,
    /// Raw bytes of every page and the root.
    index_bytes: u64,
}

impl Output {
    /// One compressed extent with a `window_log` window, described by its envelope.
    #[allow(clippy::too_many_arguments)]
    fn extent(&mut self, kind: envelope::Kind, run: u32, stream: u32, first: u32, hours: u32, raw: &[u8], window_log: u32) -> io::Result<Extent> {
        let mut compressor = zstd::bulk::Compressor::new(self.level)?;
        compressor.set_parameter(zstd::zstd_safe::CParameter::WindowLog(window_log))?;
        let stored = compressor.compress(raw)?;
        let offset = self.file.stream_position()?;
        self.file.write_all(&stored)?;
        Ok(Extent { kind, run, stream, first, hours, offset, stored: stored.len() as u64, raw: raw.len() as u64, digest: envelope::digest(&stored) })
    }

    /// Every stream's unit of the `hours` ordinals of run `run` from `first`: the manifests'
    /// first, then each table's in stream order.
    fn units(&mut self, run: u32, manifests: &mut Bytes, writers: &mut [Writer], first: u32, hours: u32) -> io::Result<Vec<Extent>> {
        let streams = std::iter::once((framing::MANIFESTS, std::mem::take(manifests).into_bytes()))
            .chain(writers.iter_mut().enumerate().map(|(table, writer)| (framing::table_stream(table), std::mem::take(&mut writer.unit).into_bytes())));
        let mut written = Vec::new();
        for (stream, raw) in streams {
            if !raw.is_empty() {
                written.push((stream, raw));
            }
        }
        let raws: Vec<usize> = written.iter().map(|(_, raw)| raw.len()).collect();
        let logs = unit_windows(&raws, self.budget);
        self.walker_memory.push(raws.iter().zip(&logs).map(|(&raw, &log)| raw.min(1 << log)).sum());
        let mut units = Vec::with_capacity(written.len());
        for ((stream, raw), log) in written.into_iter().zip(logs) {
            let unit = self.extent(envelope::Kind::Unit, run, stream, first, hours, &raw, log)?;
            if stream != framing::MANIFESTS {
                let writer = &mut writers[stream as usize - 1];
                writer.stored.0 += unit.raw;
                writer.stored.1 += unit.stored;
            }
            units.push(unit);
        }
        Ok(units)
    }
}

/// Raw and stored bytes written per kind, and dictionary entries over every run, for the
/// report.
#[derive(Default)]
struct Written {
    manifests: (u64, u64),
    chunks: (u64, u64),
    states: (u64, u64),
    entries: u64,
}

/// The run being written: its ordinals so far, the chain after the last, and the unit
/// position under way.
struct RunState {
    run: u32,
    branch: u64,
    epoch: u8,
    first_hour: i32,
    last_hour: i32,
    ordinal: u32,
    chain: [u8; 32],
    /// The position's first and last hour so far; none before its first ordinal.
    position: Option<(i32, i32)>,
    sectioned: Vec<u64>,
    read: Vec<u64>,
    status_changed: Vec<u64>,
    /// Each table's status at the run's last ordinal; none before its first.
    statuses: Option<Vec<Status>>,
    /// Each restart's dictionary chunk's first id so far, by restart ordinal.
    chunks: Vec<u32>,
    pages: Vec<Extent>,
}

impl RunState {
    fn new(run: u32, branch: u64, hour: i32, identity: &[u8], tables: usize) -> Self {
        let words = tables.div_ceil(64);
        Self {
            run,
            branch,
            // A converted recording's later runs start at a branch change or a gap.
            epoch: if run == 0 { envelope::epoch::START } else { envelope::epoch::GAP },
            first_hour: hour,
            last_hour: hour,
            ordinal: 0,
            chain: envelope::chain_start(identity, run),
            position: None,
            sectioned: vec![0; words],
            read: vec![0; words],
            status_changed: vec![0; words],
            statuses: None,
            chunks: Vec::new(),
            pages: Vec::new(),
        }
    }
}

fn set(bits: &mut [u64], table: usize) {
    bits[table / 64] |= 1 << (table % 64);
}

/// Closes the run's unit position: every stream's unit, the restart ending it when
/// `restart` (the run's dictionary strings first used since its last restart, then every
/// table's state alone), and its page.
#[allow(clippy::too_many_arguments)]
fn close_position(out: &mut Output, writers: &mut [Writer], manifests: &mut Bytes, dictionary: &mut Dictionary, state: &mut RunState, restart: bool, written: &mut Written) -> io::Result<()> {
    let Some((first_hour, last_hour)) = state.position.take() else {
        return Ok(());
    };
    for writer in writers.iter() {
        writer.check_all(dictionary)?;
    }
    let first = (state.ordinal - 1) / envelope::UNIT_ORDINALS * envelope::UNIT_ORDINALS;
    let hours = state.ordinal - first;
    let units = out.units(state.run, manifests, writers, first, hours)?;
    if let Some(manifests) = units.first() {
        written.manifests = (written.manifests.0 + manifests.raw, written.manifests.1 + manifests.stored);
    }
    let restart = if restart {
        let after = state.ordinal - 1;
        let ids = |id: usize| u32::try_from(id).map_err(|_| invalid("A run's dictionary passes 2^32 ids"));
        let (first_id, count) = (ids(dictionary.restarted)?, ids(dictionary.bytes.len() - dictionary.restarted)?);
        let mut chunk = Bytes::new();
        chunk.varint(u64::from(count))?;
        for entry in &dictionary.bytes[dictionary.restarted..] {
            chunk.varint(entry.len() as u64)?;
            chunk.raw(entry)?;
        }
        dictionary.restarted = dictionary.bytes.len();
        let chunk = out.extent(envelope::Kind::DictionaryChunk, state.run, 0, after, 0, chunk.bytes(), out.window_log)?;
        written.chunks = (written.chunks.0 + chunk.raw, written.chunks.1 + chunk.stored);
        let ordinal = state.chunks.len() as u32;
        state.chunks.push(first_id);
        let mut tables = Vec::with_capacity(writers.len());
        for (table, writer) in writers.iter_mut().enumerate() {
            let bytes = restart::write(&writer.gate, writer.status, &writer.acquisition, &writer.view)?;
            let extent = out.extent(envelope::Kind::RestartTable, state.run, table as u32, after, 0, &bytes, out.window_log)?;
            writer.restart_bytes = (writer.restart_bytes.0 + extent.raw, writer.restart_bytes.1 + extent.stored);
            written.states = (written.states.0 + extent.raw, written.states.1 + extent.stored);
            tables.push(RestartTable { table: table as u32, extent, chunks: writer.chunks(&state.chunks) });
        }
        out.restarts += 1;
        Some(RestartDirectory { after, hour: last_hour, ordinal, chunk, ids: (first_id, count), tables })
    } else {
        None
    };
    let take = |bits: &mut Vec<u64>| Bits::from_words(std::mem::replace(bits, vec![0; bits.len()]));
    let page = IndexPage {
        run: state.run,
        first,
        hours,
        first_hour,
        last_hour,
        units,
        restart,
        sectioned: take(&mut state.sectioned)?,
        read: take(&mut state.read)?,
        status_changed: take(&mut state.status_changed)?,
        chain: state.chain,
    }
    .write()?;
    out.index_bytes += page.len() as u64;
    let page = out.extent(envelope::Kind::IndexPage, state.run, 0, first, hours, &page, out.window_log)?;
    state.pages.push(page);
    Ok(())
}

/// Closes the run's last position, short or not, and writes the run's directory.
fn finish_run(out: &mut Output, writers: &mut [Writer], manifests: &mut Bytes, dictionary: &mut Dictionary, mut state: RunState, written: &mut Written) -> io::Result<()> {
    close_position(out, writers, manifests, dictionary, &mut state, false, written)?;
    written.entries += dictionary.bytes.len() as u64;
    let directory = RunDirectory {
        branch: state.branch,
        epoch: state.epoch,
        first_hour: state.first_hour,
        ordinals: state.ordinal,
        chain: state.chain,
        pages: state.pages,
    }
    .write(state.run)?;
    out.index_bytes += directory.len() as u64;
    let window_log = out.window_log;
    let directory = out.extent(envelope::Kind::RunDirectory, state.run, 0, 0, 0, &directory, window_log)?;
    out.runs.push(directory);
    Ok(())
}

struct Options {
    input: PathBuf,
    output: PathBuf,
    /// The envelope's unit hours times `--restart-units`, which fits a `u32`.
    restart_hours: u32,
    level: i32,
    window_log: u32,
    budget_mb: u32,
}

fn options(mut args: impl Iterator<Item = OsString>) -> io::Result<Options> {
    let usage = || invalid("p3-convert <recording.battleplan> <out.h4col> [--restart-units 9] [--level 3] [--window-log 24] [--budget-mb 30]");
    let (Some(input), Some(output)) = (args.next(), args.next()) else {
        return Err(usage());
    };
    let (mut restart_units, mut level, mut window_log, mut budget_mb) = (9_u32, 3_i32, 24_u32, 30_u32);
    while let Some(flag) = args.next() {
        let value = args.next().and_then(|value| value.into_string().ok()).ok_or_else(usage)?;
        let number = || value.parse::<i64>().map_err(|_| usage());
        match flag.to_str() {
            Some("--restart-units") => restart_units = u32::try_from(number()?).map_err(|_| usage())?,
            Some("--level") => level = i32::try_from(number()?).map_err(|_| usage())?,
            Some("--window-log") => window_log = u32::try_from(number()?).map_err(|_| usage())?,
            Some("--budget-mb") => budget_mb = u32::try_from(number()?).map_err(|_| usage())?,
            _ => return Err(usage()),
        }
    }
    if restart_units == 0 {
        return Err(usage());
    }
    let restart_hours = envelope::UNIT_ORDINALS
        .checked_mul(restart_units)
        .ok_or_else(|| invalid("--restart-units units pass 2^32 hours"))?;
    Ok(Options {
        input: PathBuf::from(input),
        output: PathBuf::from(output),
        restart_hours,
        level,
        window_log,
        budget_mb,
    })
}

/// What one file is, whichever name or hard link reaches it.
#[cfg(windows)]
fn identity(file: &File) -> io::Result<(u64, [u8; 16])> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{FILE_ID_INFO, FileIdInfo, GetFileInformationByHandleEx};
    let mut info: FILE_ID_INFO = unsafe { std::mem::zeroed() };
    // SAFETY: an open handle and an owned FILE_ID_INFO of the size passed.
    if unsafe { GetFileInformationByHandleEx(file.as_raw_handle(), FileIdInfo, std::ptr::from_mut(&mut info).cast(), std::mem::size_of::<FILE_ID_INFO>() as u32) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((info.VolumeSerialNumber, info.FileId.Identifier))
}

#[cfg(unix)]
fn identity(file: &File) -> io::Result<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    let metadata = file.metadata()?;
    Ok((metadata.dev(), metadata.ino()))
}

/// The unfinished output and the counts file beside `output`, refused where any of them, or
/// `output` itself, is the recording at `input`: writing one would destroy the recording.
fn written_paths(input: &Path, output: &Path) -> io::Result<(PathBuf, PathBuf)> {
    let source = identity(&File::open(input)?)?;
    let beside = |suffix: &str| {
        let mut path = output.as_os_str().to_owned();
        path.push(suffix);
        PathBuf::from(path)
    };
    let (partial, counts) = (beside(".partial"), beside(".counts.json"));
    for path in [output, &partial, &counts] {
        let reaches = match File::open(path) {
            Ok(file) => identity(&file)? == source,
            Err(error) if error.kind() == io::ErrorKind::NotFound => false,
            Err(error) => return Err(error),
        };
        if reaches {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{} is the recording being converted", path.display()),
            ));
        }
    }
    Ok((partial, counts))
}

/// Removes the unfinished output unless it was published.
struct Unfinished(Option<PathBuf>);

impl Drop for Unfinished {
    fn drop(&mut self) {
        if let Some(path) = self.0.take() {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn main() -> io::Result<()> {
    let Options { input, output, restart_hours, level, window_log, budget_mb } = options(std::env::args_os().skip(1))?;
    let source_bytes = std::fs::metadata(&input)?.len();
    let replay = Replay::open_path(&input)?;
    let (partial, counts) = written_paths(&input, &output)?;
    let mut stream = ReplayStream::new(&TableId::ALL)?;
    // Declared before `out`, so a failure closes the file before removing it.
    let mut unfinished = Unfinished(Some(partial.clone()));
    let mut out = Output {
        file: BufWriter::new(File::create(&partial)?),
        level,
        window_log,
        budget: budget_mb as usize * 1_000_000,
        walker_memory: Vec::new(),
        runs: Vec::new(),
        restarts: 0,
        index_bytes: 0,
    };
    out.file.write_all(envelope::MAGIC)?;
    out.file.write_all(&envelope::VERSION.to_le_bytes())?;
    let mut writers: Vec<Writer> = TableId::ALL
        .iter()
        .map(|&id| Writer::new(id))
        .collect::<io::Result<_>>()?;
    let column_counts: Vec<usize> = TableId::ALL.iter().map(|id| id.schema().len()).collect();
    let schema: Vec<Vec<Column>> = TableId::ALL.iter().map(|id| id.schema().iter().map(|&kind| column(kind)).collect()).collect();
    let mut content = Vec::new();
    replay_data::identity::IdentityCodec::encode(&replay.identity, &mut content)?;
    let identity = envelope::Identity {
        schema: envelope::schema_digest(&schema),
        campaign: replay.campaign_id.clone(),
        content,
        capture_mode: replay.capture_mode.byte(),
    };
    let identity_bytes = identity.chain_bytes()?;
    let mut run: Option<RunState> = None;
    let mut written = Written::default();
    let mut manifests = Bytes::new();
    let mut dictionary = Dictionary::default();
    let mut scratch = Scratch::default();
    let mut buffer = Vec::new();
    let mut changed_keys: Vec<Vec<u128>> = vec![Vec::new(); TableId::ALL.len()];
    let (mut encode_seconds, mut hours) = (0.0_f64, 0_u32);
    let started = Instant::now();
    while stream.next_block_recorded_with(
        &replay,
        |recorded, decoded| {
            let begin = Instant::now();
            // A branch change or a gap ends the run; the next starts from empty tables and
            // its own dictionary.
            let continues = run.as_ref().is_some_and(|run| run.branch == decoded.branch && run.last_hour.checked_add(1) == Some(decoded.hour));
            if !continues {
                if let Some(ended) = run.take() {
                    finish_run(&mut out, &mut writers, &mut manifests, &mut dictionary, ended, &mut written)?;
                    for writer in &mut writers {
                        writer.reset()?;
                    }
                    dictionary = Dictionary::default();
                }
                run = Some(RunState::new(out.runs.len() as u32, decoded.branch, decoded.hour, &identity_bytes, writers.len()));
            }
            let state = run.as_mut().expect("a run under way");
            let starts = state.ordinal == 0;
            let mut hour_raw: Vec<Vec<u8>> = Vec::new();
            let mut statuses = Vec::with_capacity(writers.len());
            let mut read = vec![0_u64; writers.len().div_ceil(64)];
            let mut samples: Vec<(usize, Option<i32>)> = Vec::new();
            let mut hour_sections: Vec<Section> = Vec::new();
            for (index, update) in recorded.tables.iter().enumerate() {
                let view = &decoded.tables[index];
                if decoded.metrics.table_ns[index] != 0 {
                    read[index / 64] |= 1 << (index % 64);
                }
                let keys = &mut changed_keys[index];
                keys.clear();
                if let TableUpdate::Delta { changed, .. } = update {
                    keys.extend(changed.iter().map(|delta| delta.key));
                }
                let (status, rows) = match &**view {
                    TableView::Complete(rows) => (Status::Complete, Some(rows)),
                    TableView::Truncated { rows, missing } => (Status::Truncated { missing: *missing }, Some(rows)),
                    TableView::Unavailable { reason } => (Status::Unavailable { reason: dictionary.id(reason.as_bytes()) }, None),
                    TableView::NotRecorded { .. } => (Status::NotRecorded, None),
                };
                statuses.push(status);
                let writer = &mut writers[index];
                let available = |status: Status| matches!(status, Status::Complete | Status::Truncated { .. });
                if available(status) != available(writer.status) || writer.acquisition.available_from.is_none() {
                    writer.acquisition.available_from = Some(decoded.hour);
                }
                writer.status = status;
                writer.acquisition.refusal = match status {
                    Status::Unavailable { reason } => Some(reason),
                    _ => None,
                };
                // The read bits sample a read table at its hour and leave the others; any
                // other sample hour (an untimed snapshot's none, a carried read) is listed.
                let implied = if decoded.metrics.table_ns[index] != 0 {
                    Some(decoded.hour)
                } else {
                    writer.acquisition.last_read
                };
                if view.sampled_at() != implied {
                    samples.push((index, view.sampled_at()));
                }
                writer.acquisition.last_read = view.sampled_at();
                if rows.is_none() {
                    writer.view.clear();
                }
                let written = match rows {
                    None => None,
                    Some(rows) => {
                        let complete = status == Status::Complete;
                        let changes = match update {
                            // A run's first hour writes every row: its walker starts empty.
                            _ if starts => Changes::Full,
                            TableUpdate::Unchanged { .. } if complete => Changes::Unchanged,
                            TableUpdate::Delta { deleted, .. } if complete => Changes::Keys {
                                changed: &changed_keys[index],
                                deleted,
                            },
                            _ => Changes::Full,
                        };
                        writers[index].hour(rows, changes, complete, &mut dictionary, &mut scratch, &mut buffer)?
                    }
                };
                let writer = &mut writers[index];
                if let Some((bytes, columns, ops)) = written {
                    framing::put_section(&mut writer.unit, &bytes)?;
                    hour_sections.push(Section {
                        table: index,
                        bytes: bytes.len() as u64,
                        columns,
                        ops,
                    });
                    set(&mut state.sectioned, index);
                    hour_raw.push(bytes);
                }
            }
            for (table, status) in statuses.iter().enumerate() {
                if state.statuses.as_ref().is_none_or(|before| before[table] != *status) {
                    set(&mut state.status_changed, table);
                }
            }
            state.statuses = Some(statuses.clone());
            for (word, bits) in state.read.iter_mut().zip(&read) {
                *word |= bits;
            }
            let appends: Vec<&[u8]> = dictionary.bytes[dictionary.appended..].iter().map(Vec::as_slice).collect();
            let manifest = Manifest {
                branch: decoded.branch,
                hour: decoded.hour,
                flags: if samples.is_empty() { 0 } else { manifest::SAMPLES },
                total_ns: decoded.metrics.total_ns,
                pool_bytes: decoded.metrics.pool_bytes,
                dropped_frames: decoded.metrics.dropped_frames,
                appends,
                statuses,
                read,
                samples,
                sections: hour_sections,
            }
            .write(&column_counts)?;
            dictionary.appended = dictionary.bytes.len();
            // A block keeps which tables were read, never for how long: the manifest holds it all.
            let read_back = Manifest::read(&manifest, &column_counts)?;
            let mut rebuilt = Metrics {
                total_ns: read_back.total_ns,
                pool_bytes: read_back.pool_bytes,
                dropped_frames: read_back.dropped_frames,
                ..Metrics::default()
            };
            for (table, ns) in rebuilt.table_ns.iter_mut().enumerate() {
                if read_back.read[table / 64] >> (table % 64) & 1 != 0 {
                    *ns = READ_TIME_NOT_RECORDED;
                }
            }
            if rebuilt != decoded.metrics.stored() {
                return Err(invalid(format!("hour {} metrics differ from the manifest's", decoded.hour)));
            }
            framing::put_manifest(&mut manifests, &manifest)?;
            state.chain = envelope::chain_hour(&state.chain, &manifest, hour_raw.iter().map(Vec::as_slice));
            state.position = Some(state.position.map_or((decoded.hour, decoded.hour), |(first, _)| (first, decoded.hour)));
            state.last_hour = decoded.hour;
            hours += 1;
            state.ordinal += 1;
            if state.ordinal % envelope::UNIT_ORDINALS == 0 {
                let restart = state.ordinal % restart_hours == 0;
                close_position(&mut out, &mut writers, &mut manifests, &mut dictionary, state, restart, &mut written)?;
            }
            encode_seconds += begin.elapsed().as_secs_f64();
            Ok(())
        },
        || Ok(()),
    )? {}
    for writer in &writers {
        writer.check_all(&dictionary)?;
    }
    if let Some(ended) = run.take() {
        finish_run(&mut out, &mut writers, &mut manifests, &mut dictionary, ended, &mut written)?;
    }
    // The source's control records go in with the live recorder's encoding (P4 PR2).
    let root = envelope::Root { identity, revision: 1, runs: std::mem::take(&mut out.runs), controls: Vec::new() }.write()?;
    out.index_bytes += root.len() as u64;
    let window_log = out.window_log;
    let root = out.extent(envelope::Kind::Root, 0, 0, 0, 0, &root, window_log)?;
    out.file.write_all(&envelope::footer(root.offset, root.stored, &root.digest))?;
    let file = out.file.into_inner().map_err(io::Error::from)?;
    // Durable before it is published under its name.
    file.sync_all()?;
    let total = file.metadata()?.len();
    drop(file);
    std::fs::rename(&partial, &output)?;
    unfinished.0 = None;
    let per_hour = |bytes: u64| bytes as f64 / f64::from(hours.max(1));
    let mut memory = out.walker_memory.clone();
    memory.sort_unstable();
    let (walker_peak, walker_median) = (memory.last().copied().unwrap_or(0), memory.get(memory.len() / 2).copied().unwrap_or(0));
    let fifty = |bytes: u64| per_hour(bytes) * 438_000.0 / 1e9;
    let restarts = out.restarts;
    let sections = writers.iter().fold((0, 0), |sum: (u64, u64), writer| (sum.0 + writer.stored.0, sum.1 + writer.stored.1));
    let restart_alone = writers.iter().fold((0, 0), |sum: (u64, u64), writer| (sum.0 + writer.restart_bytes.0, sum.1 + writer.restart_bytes.1));
    let largest: Vec<serde_json::Value> = {
        let mut columns: Vec<(u64, u64, String, usize)> = writers
            .iter()
            .flat_map(|writer| writer.column_values.iter().enumerate().map(move |(column, &(values, bytes))| (bytes, values, format!("{:?}", writer.id), column)))
            .filter(|(bytes, ..)| *bytes > 0)
            .collect();
        columns.sort_unstable_by(|a, b| b.0.cmp(&a.0));
        columns
            .into_iter()
            .take(40)
            .map(|(bytes, values, table, column)| json!({"table": table, "column": column, "kind": format!("{:?}", column_kind_name(&table, column)), "values": values, "field_bytes": bytes}))
            .collect()
    };
    let tables: Vec<serde_json::Value> = writers
        .iter()
        .filter(|writer| writer.totals.sections > 0 || writer.restart_bytes.0 > 0)
        .map(|writer| json!({
            "table": format!("{:?}", writer.id),
            "sections": writer.totals.sections,
            "section_bytes": writer.totals.bytes,
            "units_raw": writer.stored.0,
            "units_stored": writer.stored.1,
            "inserts": writer.totals.inserts,
            "deletes": writer.totals.deletes,
            "changed_slots": writer.totals.changed,
            "set_values": writer.totals.set_values,
            "add_values": writer.totals.add_values,
            "patch_values": writer.totals.patch_values,
            "fills": writer.totals.fills,
            "fill_slots": writer.totals.fill_slots,
            "fill_exceptions": writer.totals.fill_exceptions,
            "truncated_hours": writer.totals.truncated,
            "restart_raw": writer.restart_bytes.0,
            "restart_stored_alone": writer.restart_bytes.1,
        }))
        .collect();
    let report = json!({
        "hours": hours,
        "gate": "every written slot each hour, every live slot at every unit's end and at the end, and every hour's stored metrics: no difference",
        "unit_hours": envelope::UNIT_ORDINALS,
        "restart_hours": restart_hours,
        "level": level,
        "restart_window_log": window_log,
        "walker_decode_memory": {
            "budget_mb": budget_mb,
            "peak_mb": walker_peak as f64 / 1e6,
            "median_mb": walker_median as f64 / 1e6,
            "scope": "per unit grid position, the sum over its streams of min(unit raw bytes, 2^window)",
        },
        "source_bytes": source_bytes,
        "output_bytes": total,
        "ratio_to_source": total as f64 / source_bytes as f64,
        "gb_per_50_years": fifty(total),
        "source_gb_per_50_years": fifty(source_bytes),
        "encode_and_gate_ms_per_hour": encode_seconds * 1000.0 / f64::from(hours.max(1)),
        "wall_seconds": started.elapsed().as_secs_f64(),
        "dictionary_entries": written.entries,
        "manifests": {"raw": written.manifests.0, "stored": written.manifests.1},
        "sections": {"raw": sections.0, "stored": sections.1, "stored_per_hour": per_hour(sections.1)},
        "restarts": {
            "count": restarts,
            "chunks": {"raw": written.chunks.0, "stored": written.chunks.1},
            "states": {"raw": written.states.0, "stored": written.states.1},
            "stored_per_restart": (written.chunks.1 + written.states.1) as f64 / restarts.max(1) as f64,
            "tables_alone": {"raw": restart_alone.0, "stored": restart_alone.1},
        },
        "index_bytes": out.index_bytes,
        "largest_columns": largest,
        "tables": tables,
    });
    std::fs::write(counts, serde_json::to_vec_pretty(&report)?)?;
    Ok(())
}

/// A column's replay-data kind, by table name, for the report.
fn column_kind_name(table: &str, column: usize) -> Column {
    TableId::ALL
        .iter()
        .find(|id| format!("{id:?}") == table)
        .map_or(Column::Opaque, |id| replay::native_columns::column(id.schema()[column]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn arguments(text: &[&str]) -> impl Iterator<Item = OsString> {
        text.iter().map(OsString::from).collect::<Vec<_>>().into_iter()
    }

    /// A fresh folder of its own, removed afterwards.
    struct Folder(PathBuf);

    impl Folder {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!("p3-convert-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_restart_interval_past_32_bits_is_refused_before_converting() {
        let most = u32::MAX / envelope::UNIT_ORDINALS;
        let (past, most_text) = ((most + 1).to_string(), most.to_string());
        let refused = options(arguments(&["in", "out", "--restart-units", &past])).err().unwrap();
        assert_eq!(refused.kind(), io::ErrorKind::InvalidData);
        assert!(refused.to_string().contains("--restart-units"), "{refused}");
        let largest = options(arguments(&["in", "out", "--restart-units", &most_text])).unwrap();
        assert_eq!(largest.restart_hours, most * envelope::UNIT_ORDINALS);
        assert_eq!(options(arguments(&["in", "out"])).unwrap().restart_hours, 6480);
        assert!(options(arguments(&["in", "out", "--restart-units", "0"])).is_err());
        // Units are the envelope's grid.
        assert!(options(arguments(&["in", "out", "--unit-hours", "360"])).is_err());
    }

    #[test]
    fn no_written_path_may_be_the_recording_under_any_name() {
        let folder = Folder::new();
        let recording = folder.0.join("campaign.battleplan");
        std::fs::write(&recording, b"recording").unwrap();
        let refused = |output: &Path| written_paths(&recording, output).unwrap_err().kind() == io::ErrorKind::InvalidInput;
        assert!(refused(&recording));
        assert!(refused(&folder.0.join(".").join("campaign.battleplan")));
        let linked = folder.0.join("linked.h4col");
        std::fs::hard_link(&recording, &linked).unwrap();
        assert!(refused(&linked));
        let beside = folder.0.join("beside.h4col");
        std::fs::hard_link(&recording, folder.0.join("beside.h4col.partial")).unwrap();
        assert!(refused(&beside));
        let counted = folder.0.join("counted.h4col");
        std::fs::hard_link(&recording, folder.0.join("counted.h4col.counts.json")).unwrap();
        assert!(refused(&counted));
        let other = folder.0.join("other.h4col");
        std::fs::write(&other, b"an earlier conversion").unwrap();
        let (partial, counts) = written_paths(&recording, &other).unwrap();
        assert_eq!(partial, folder.0.join("other.h4col.partial"));
        assert_eq!(counts, folder.0.join("other.h4col.counts.json"));
        assert!(written_paths(&recording, &folder.0.join("new.h4col")).is_ok());
        assert_eq!(std::fs::read(&recording).unwrap(), b"recording");
    }

    #[test]
    fn an_unfinished_output_is_removed_and_a_published_one_kept() {
        let folder = Folder::new();
        let failed = folder.0.join("failed.h4col.partial");
        std::fs::write(&failed, b"H4COLUMN").unwrap();
        drop(Unfinished(Some(failed.clone())));
        assert!(!failed.exists());
        let published = folder.0.join("published.h4col");
        std::fs::write(&published, b"H4COLUMN").unwrap();
        let mut unfinished = Unfinished(Some(published.clone()));
        unfinished.0 = None;
        drop(unfinished);
        assert!(published.exists());
    }
}
