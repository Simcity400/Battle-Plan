
use super::{Edits, Given, SLOTS, Store, TableState, label, valid_slot};
use crate::{
    invalid,
    leaf::Reader,
    manifest::{Manifest, Section, Status},
    section::{self, Scratch},
    state::{Cell, Kind, Values},
};
use std::io;

#[derive(Clone, Copy)]
pub struct Names {
    pub table: fn(usize) -> String,
    pub column: fn(usize, usize) -> String,
}

impl Default for Names {
    fn default() -> Self {
        Self { table: |table| format!("table {table}"), column: |_, column| format!("{column}") }
    }
}

#[derive(Clone, Debug, Default)]
struct Log {
    rows: Vec<Option<(u64, u128)>>,
    given: Given,
    refused: Option<(u32, Box<[u8]>)>,
}

impl Log {
    fn row(&self, slot: u32) -> Option<(u64, u128)> {
        self.rows.get(slot as usize).copied().flatten()
    }
}

#[derive(Clone, Copy)]
enum Held {
    Unrecorded,
    Scalar(Cell<'static>),
    U32s(usize, usize),
    U64s(usize, usize),
    Bytes(usize, usize),
}

#[derive(Default)]
struct HeldRows {
    cells: Vec<Held>,
    u32s: Vec<u32>,
    u64s: Vec<u64>,
    bytes: Vec<u8>,
}

impl HeldRows {
    fn clear(&mut self) {
        self.cells.clear();
        self.u32s.clear();
        self.u64s.clear();
        self.bytes.clear();
    }

    fn hold(&mut self, cell: Cell<'_>) -> Held {
        match cell {
            Cell::U32List(items) => {
                let start = self.u32s.len();
                self.u32s.extend_from_slice(items);
                Held::U32s(start, self.u32s.len())
            }
            Cell::U64List(items) => {
                let start = self.u64s.len();
                self.u64s.extend_from_slice(items);
                Held::U64s(start, self.u64s.len())
            }
            Cell::Opaque(items) => {
                let start = self.bytes.len();
                self.bytes.extend_from_slice(items);
                Held::Bytes(start, self.bytes.len())
            }
            Cell::U8(value) => Held::Scalar(Cell::U8(value)),
            Cell::I8(value) => Held::Scalar(Cell::I8(value)),
            Cell::U16(value) => Held::Scalar(Cell::U16(value)),
            Cell::I16(value) => Held::Scalar(Cell::I16(value)),
            Cell::U32(value) => Held::Scalar(Cell::U32(value)),
            Cell::I32(value) => Held::Scalar(Cell::I32(value)),
            Cell::U64(value) => Held::Scalar(Cell::U64(value)),
            Cell::I64(value) => Held::Scalar(Cell::I64(value)),
            Cell::Bool(value) => Held::Scalar(Cell::Bool(value)),
            Cell::OptionalI32(value) => Held::Scalar(Cell::OptionalI32(value)),
            Cell::OptionalU32(value) => Held::Scalar(Cell::OptionalU32(value)),
            Cell::OptionalI64(value) => Held::Scalar(Cell::OptionalI64(value)),
            Cell::OptionalU64(value) => Held::Scalar(Cell::OptionalU64(value)),
            Cell::Bytes(id) => Held::Scalar(Cell::Bytes(id)),
        }
    }

    fn cell(&self, held: Held) -> Option<Cell<'_>> {
        match held {
            Held::Unrecorded => None,
            Held::Scalar(cell) => Some(cell),
            Held::U32s(start, end) => Some(Cell::U32List(&self.u32s[start..end])),
            Held::U64s(start, end) => Some(Cell::U64List(&self.u64s[start..end])),
            Held::Bytes(start, end) => Some(Cell::Opaque(&self.bytes[start..end])),
        }
    }
}

enum Refused {
    Written { slot: u32, column: usize },
    GivenAgain { key: u128 },
    Width,
}

#[derive(Default)]
pub struct Journal {
    scratch: Scratch,
    lists: ListPieces,
    read: Vec<u32>,
    changed: Vec<u32>,
    deleted: Vec<u32>,
    inserted: Vec<u32>,
    keys: Vec<u128>,
    except: Vec<u32>,
    positions: Vec<u32>,
    slots: Vec<u32>,
    hunks: Vec<u32>,
    edits: Vec<u32>,
    triples: Vec<(u32, u32, u32)>,
    items: Vec<u8>,
    sectioned: Vec<bool>,
    maps: Vec<Option<Vec<u32>>>,
    mapped: Vec<u32>,
    mapped_inserted: Vec<u32>,
    left: Vec<bool>,
    reads: Vec<bool>,
    logs: Vec<Option<Box<Log>>>,
    held: HeldRows,
    reasons: u32,
    names: Names,
}

const NONE: u32 = u32::MAX;

const LIST_PIECE: usize = 1 << 18;

const NUMBER_PIECE: usize = 1 << 16;

fn each_number(reader: &mut Reader<'_>, scratch: &mut Scratch, kind: Kind, count: usize, mut each: impl FnMut(usize, Cell<'_>) -> io::Result<()>) -> io::Result<()> {
    let pieces = !matches!(kind, Kind::OptionalI32 | Kind::OptionalU32 | Kind::OptionalI64 | Kind::OptionalU64);
    let step = if pieces { NUMBER_PIECE } else { count.max(1) };
    let mut from = 0;
    while from < count || (count == 0 && from == 0) {
        let length = step.min(count - from);
        let values = {
            let _label = label("adapter: values");
            section::read_values(reader, scratch, kind, length)?
        };
        each_cell(values, length, |at, cell| each(from + at, cell))?;
        from += length;
        if count == 0 {
            break;
        }
    }
    Ok(())
}

fn is_list(kind: Kind) -> bool {
    matches!(kind, Kind::U32List | Kind::U64List | Kind::Opaque)
}

#[derive(Default)]
struct ListPieces {
    ends: Vec<u32>,
    u32s: Vec<u32>,
    u64s: Vec<u64>,
    bytes: Vec<u8>,
}

fn each_list(reader: &mut Reader<'_>, held: &mut ListPieces, kind: Kind, count: usize, mut each: impl FnMut(usize, Cell<'_>) -> io::Result<()>) -> io::Result<()> {
    {
        let _label = label("adapter: values");
        reader.lengths(count, &mut held.ends)?;
    }
    let (mut row, mut start) = (0_usize, 0_usize);
    while row < count {
        let mut last = row;
        while last + 1 < count && held.ends[last + 1] as usize - start <= LIST_PIECE {
            last += 1;
        }
        let end = held.ends[last] as usize;
        let mut cell_start = start;
        macro_rules! piece {
            ($buffer:ident, $read:ident, $cell:ident) => {{
                {
                    let _label = label("adapter: values");
                    reader.$read(end - start, &mut held.$buffer)?;
                }
                for at in row..=last {
                    let cell_end = held.ends[at] as usize;
                    each(at, Cell::$cell(&held.$buffer[cell_start - start..cell_end - start]))?;
                    cell_start = cell_end;
                }
            }};
        }
        match kind {
            Kind::U32List => piece!(u32s, ints, U32List),
            Kind::U64List => piece!(u64s, ints, U64List),
            Kind::Opaque => piece!(bytes, values, Opaque),
            other => return Err(invalid(format!("A {other:?} column read as a list"))),
        }
        start = end;
        row = last + 1;
    }
    Ok(())
}

fn to_store(map: Option<&Vec<u32>>, slot: u32) -> io::Result<u32> {
    match map {
        None => Ok(slot),
        Some(map) => map.get(slot as usize).copied().filter(|&held| held != NONE).ok_or_else(|| invalid(format!("A section names slot {slot}, which holds no row"))),
    }
}

fn map_all(map: Option<&Vec<u32>>, slots: &[u32], out: &mut Vec<u32>, sorted: bool) -> io::Result<()> {
    out.clear();
    for &slot in slots {
        out.push(to_store(map, slot)?);
    }
    if sorted && map.is_some() {
        out.sort_unstable();
    }
    Ok(())
}

pub fn each_cell(values: Values<'_>, count: usize, mut each: impl FnMut(usize, Cell<'_>) -> io::Result<()>) -> io::Result<()> {
    macro_rules! plain {
        ($values:expr, $cell:ident) => {
            for (at, &value) in $values.iter().enumerate() {
                each(at, Cell::$cell(value))?;
            }
        };
    }
    macro_rules! optional {
        ($present:expr, $values:expr, $cell:ident, $count:expr) => {{
            let mut next = $values.iter();
            for at in 0..$count {
                let value = if $present.get(at / 64).is_some_and(|word| word >> (at % 64) & 1 == 1) {
                    Some(*next.next().ok_or_else(|| invalid("Presence bits pass their values"))?)
                } else {
                    None
                };
                each(at, Cell::$cell(value))?;
            }
        }};
    }
    macro_rules! list {
        ($ends:expr, $items:expr, $cell:ident) => {{
            let mut start = 0;
            for (at, &end) in $ends.iter().enumerate() {
                let cell = $items.get(start..end as usize).ok_or_else(|| invalid("A list cell passes its items"))?;
                each(at, Cell::$cell(cell))?;
                start = end as usize;
            }
        }};
    }
    match values {
        Values::U8(values) => plain!(values, U8),
        Values::I8(values) => plain!(values, I8),
        Values::U16(values) => plain!(values, U16),
        Values::I16(values) => plain!(values, I16),
        Values::U32(values) => plain!(values, U32),
        Values::I32(values) => plain!(values, I32),
        Values::U64(values) => plain!(values, U64),
        Values::I64(values) => plain!(values, I64),
        Values::Bytes(values) => plain!(values, Bytes),
        Values::Bool(values) => {
            for (at, &value) in values.iter().enumerate() {
                let value = match value {
                    0 => false,
                    1 => true,
                    other => return Err(invalid(format!("Bool column value {other}"))),
                };
                each(at, Cell::Bool(value))?;
            }
        }
        Values::OptionalI32 { present, values } => optional!(present, values, OptionalI32, count),
        Values::OptionalU32 { present, values } => optional!(present, values, OptionalU32, count),
        Values::OptionalI64 { present, values } => optional!(present, values, OptionalI64, count),
        Values::OptionalU64 { present, values } => optional!(present, values, OptionalU64, count),
        Values::U32List { ends, items } => list!(ends, items, U32List),
        Values::U64List { ends, items } => list!(ends, items, U64List),
        Values::Opaque { ends, items } => list!(ends, items, Opaque),
    }
    Ok(())
}

fn difference_of(cell: Cell<'_>) -> io::Result<u64> {
    Ok(match cell {
        Cell::U8(value) => value.into(),
        Cell::I8(value) => value as i64 as u64,
        Cell::U16(value) => value.into(),
        Cell::I16(value) => value as i64 as u64,
        Cell::U32(value) => value.into(),
        Cell::I32(value) => value as i64 as u64,
        Cell::U64(value) => value,
        Cell::I64(value) => value as u64,
        other => return Err(invalid(format!("An ADD of a {other:?}"))),
    })
}

fn within(slots: &[u32], of: &[u32]) -> bool {
    section::within(slots, of)
}

impl Journal {
    pub fn leaving(tables: &[usize], names: Names) -> Self {
        let mut adapter = Self { names, ..Self::default() };
        for &table in tables {
            if adapter.left.len() <= table {
                adapter.left.resize(table + 1, false);
            }
            adapter.left[table] = true;
        }
        adapter
    }

    fn leaves(&self, table: usize) -> bool {
        self.left.get(table).copied().unwrap_or(false)
    }

    pub fn save(&self) -> Vec<u8> {
        use crate::unit::put_varint;
        let mut out = Vec::new();
        let held: Vec<(usize, &Vec<u32>)> = self.maps.iter().enumerate().filter_map(|(table, map)| map.as_ref().map(|map| (table, map))).collect();
        put_varint(&mut out, held.len() as u64);
        for (table, map) in held {
            put_varint(&mut out, table as u64);
            put_varint(&mut out, map.len() as u64);
            for &slot in map {
                put_varint(&mut out, u64::from(slot.wrapping_add(1)));
            }
        }
        let logs: Vec<(usize, &Log)> = self.logs.iter().enumerate().filter_map(|(table, log)| log.as_deref().map(|log| (table, log))).collect();
        put_varint(&mut out, logs.len() as u64);
        for (table, log) in logs {
            put_varint(&mut out, table as u64);
            match &log.refused {
                None => put_varint(&mut out, 0),
                Some((id, text)) => {
                    put_varint(&mut out, u64::from(*id) + 1);
                    put_varint(&mut out, text.len() as u64);
                    out.extend_from_slice(text);
                }
            }
            put_varint(&mut out, log.rows.len() as u64);
            for row in &log.rows {
                match row {
                    None => put_varint(&mut out, 0),
                    Some((ordinal, key)) => {
                        put_varint(&mut out, ordinal + 1);
                        out.extend_from_slice(&key.to_le_bytes());
                    }
                }
            }
            put_varint(&mut out, log.given.len() as u64);
            for (start, end) in log.given.ranges() {
                out.extend_from_slice(&start.to_le_bytes());
                out.extend_from_slice(&end.to_le_bytes());
            }
        }
        put_varint(&mut out, u64::from(self.reasons));
        out
    }

    pub fn load(&mut self, bytes: &[u8]) -> io::Result<()> {
        use crate::unit::get_varint;
        let wide = |at: &mut usize| -> io::Result<u128> {
            let raw = bytes.get(*at..*at + 16).ok_or_else(|| invalid("Adapter state ends inside a key"))?;
            *at += 16;
            Ok(u128::from_le_bytes(raw.try_into().expect("sixteen bytes")))
        };
        let mut at = 0;
        self.maps.clear();
        for _ in 0..get_varint(bytes, &mut at)? {
            let table = get_varint(bytes, &mut at)? as usize;
            let count = get_varint(bytes, &mut at)? as usize;
            let slot = |at: &mut usize| -> io::Result<u32> {
                let held = get_varint(bytes, at)?;
                held.checked_sub(1).map_or(Ok(NONE), |slot| u32::try_from(slot).ok().filter(|&slot| slot < SLOTS).ok_or_else(|| invalid("Adapter state names a store slot past the format's slot width")))
            };
            let map = (0..count).map(|_| slot(&mut at)).collect::<io::Result<_>>()?;
            if self.maps.len() <= table {
                self.maps.resize(table + 1, None);
            }
            self.maps[table] = Some(map);
        }
        self.logs.clear();
        for _ in 0..get_varint(bytes, &mut at)? {
            let table = get_varint(bytes, &mut at)? as usize;
            let mut log = Log::default();
            if let Some(id) = get_varint(bytes, &mut at)?.checked_sub(1) {
                let length = get_varint(bytes, &mut at)? as usize;
                let text = bytes.get(at..at.saturating_add(length)).ok_or_else(|| invalid("Adapter state ends inside a refusal"))?;
                at += length;
                log.refused = Some((u32::try_from(id).map_err(|_| invalid("A refusal's string id passes 32 bits"))?, text.into()));
            }
            let rows = get_varint(bytes, &mut at)? as usize;
            if rows > bytes.len() {
                return Err(invalid("Adapter state counts more log rows than it holds"));
            }
            log.rows.reserve(rows);
            for _ in 0..rows {
                log.rows.push(match get_varint(bytes, &mut at)?.checked_sub(1) {
                    None => None,
                    Some(ordinal) => Some((ordinal, wide(&mut at)?)),
                });
            }
            for _ in 0..get_varint(bytes, &mut at)? {
                let start = wide(&mut at)?;
                let end = wide(&mut at)?;
                log.given.hold_range(start, end);
            }
            if self.logs.len() <= table {
                self.logs.resize_with(table + 1, || None);
            }
            self.logs[table] = Some(Box::new(log));
        }
        self.reasons = u32::try_from(get_varint(bytes, &mut at)?).map_err(|_| invalid("Adapter state counts more refusals than string ids"))?;
        if at != bytes.len() {
            return Err(invalid("Adapter state holds bytes past its end"));
        }
        Ok(())
    }

    pub fn begin_hour(&mut self, store: &mut Store, manifest: &Manifest<'_>) -> io::Result<()> {
        let tables = store.tables().len();
        if manifest.statuses.len() != tables || manifest.read.len() != tables.div_ceil(64) {
            return Err(invalid("Manifest disagrees with the schema's tables"));
        }
        store.begin_hour(manifest.hour, &manifest.appends)?;
        if store.first_of_run() {
            self.logs.clear();
            self.reasons = 0;
        }
        self.sectioned.clear();
        self.sectioned.resize(tables, false);
        self.reads.clear();
        self.reads.resize(tables, false);
        for entry in &manifest.sections {
            if self.leaves(entry.table) {
                return Err(invalid(format!("The journal holds a section of {}, which the recorder writes", (self.names.table)(entry.table))));
            }
            *self.sectioned.get_mut(entry.table).ok_or_else(|| invalid("A section names a table past the schema"))? = true;
        }
        let mut samples = manifest.samples.iter().peekable();
        for table in 0..tables {
            if self.leaves(table) {
                continue;
            }
            let mut status = manifest.statuses[table];
            let mut sampled = store.table_state(table).and_then(|held| held.sampled_at());
            if manifest.read[table / 64] >> (table % 64) & 1 != 0 {
                sampled = Some(manifest.hour);
            }
            while samples.next_if(|(held, _)| *held < table).is_some() {}
            if let Some(&&(_, hour)) = samples.peek().filter(|(held, _)| *held == table) {
                sampled = hour;
            }
            match self.logs.get(table).and_then(Option::as_deref).and_then(|log| log.refused.as_ref()) {
                Some((reason, text)) => {
                    store.pin_string(*reason, text)?;
                    status = Status::Unavailable { reason: *reason };
                }
                None if matches!(status, Status::Truncated { .. }) && !self.sectioned[table] => {
                    return Err(invalid("A partly read table names no rows it read"));
                }
                None => {}
            }
            let read = manifest.read[table / 64] >> (table % 64) & 1 != 0;
            self.reads[table] = read;
            store.status(table, status, read, sampled, self.sectioned[table])?;
        }
        Ok(())
    }

    pub fn end_hour(&mut self, store: &mut Store, sink: &mut dyn crate::unit::PartSink) -> io::Result<()> {
        self.applied(store);
        store.end_hour(sink)?;
        self.moved(store);
        Ok(())
    }

    pub fn applied(&mut self, store: &mut Store) {
        for (table, &sectioned) in self.sectioned.iter().enumerate() {
            store.after(table, sectioned);
        }
        store.settle();
    }

    pub fn shed(&mut self, store: &mut Store) {
        let Self { maps, left, sectioned, reads, logs, reasons, names, .. } = std::mem::take(self);
        *self = Self { maps, left, sectioned, reads, logs, reasons, names, ..Self::default() };
        store.shed();
    }

    pub fn moved(&mut self, store: &mut Store) {
        for (table, moved) in store.take_moves() {
            if self.maps.len() <= table {
                self.maps.resize(table + 1, None);
            }
            let map = match self.maps[table].take() {
                Some(map) => map.into_iter().map(|held| if held == NONE { NONE } else { moved.get(held as usize).copied().flatten().unwrap_or(NONE) }).collect(),
                None => moved.iter().map(|held| held.unwrap_or(NONE)).collect(),
            };
            self.maps[table] = Some(map);
        }
    }

    pub fn section(&mut self, store: &mut Store, entry: &Section, bytes: &[u8]) -> io::Result<()> {
        if bytes.len() as u64 != entry.bytes {
            return Err(invalid("Section length differs from its manifest's"));
        }
        let table = entry.table;
        if store.logs().get(table) == Some(&true) {
            return self.log_section(store, table, bytes);
        }
        if self.logs.get(table).and_then(Option::as_deref).is_some_and(|log| log.refused.is_some()) {
            return Ok(());
        }
        let truncated = store.table_state(table).is_some_and(|held| matches!(held.status(), Status::Truncated { .. }));
        let kinds: Vec<Kind> = store.kinds(table).to_vec();
        if self.maps.len() <= table {
            self.maps.resize(table + 1, None);
        }
        let mut reader = Reader::new(bytes);
        let rows = label("adapter: rows");
        if truncated {
            reader.slots(&mut self.read)?;
        }
        reader.slots(&mut self.changed)?;
        reader.slots(&mut self.deleted)?;
        reader.slots(&mut self.inserted)?;
        drop(rows);
        if self.passes_slots(store, table) {
            if self.logs.len() <= table {
                self.logs.resize_with(table + 1, || None);
            }
            let mut log = Log::default();
            let refused = self.refuse(store, table, &mut log, Refused::Width);
            self.logs[table] = Some(Box::new(log));
            return refused;
        }
        let mut map = self.maps[table].take();
        let mut changes = store.table(table)?;
        if truncated {
            map_all(map.as_ref(), &self.read, &mut self.mapped, true)?;
            changes.read_rows(&self.mapped)?;
        }
        if truncated && (!within(&self.changed, &self.read) || !within(&self.deleted, &self.read)) {
            return Err(invalid("A partly read table changes a slot it did not read"));
        }
        for &slot in &self.deleted {
            changes.delete(to_store(map.as_ref(), slot)?)?;
            if let Some(map) = &mut map {
                map[slot as usize] = NONE;
            }
        }
        self.mapped_inserted.clear();
        if map.is_some() {
            let _label = label("adapter: rows");
            self.mapped_inserted.reserve(self.inserted.len());
        }
        let mut key = 0_u128;
        for &slot in &self.inserted {
            reader.keys(1, &mut self.keys)?;
            key = key.wrapping_add(self.keys[0]);
            match &mut map {
                None => changes.insert_given_at(slot, key)?,
                Some(map) => {
                    let held = changes.insert_given(key)?;
                    if map.len() <= slot as usize {
                        map.resize(slot as usize + 1, NONE);
                    }
                    map[slot as usize] = held;
                    self.mapped_inserted.push(held);
                }
            }
        }
        if !self.inserted.is_empty() {
            for (column, &kind) in kinds.iter().enumerate() {
                let inserted = if map.is_some() { &self.mapped_inserted } else { &self.inserted };
                if is_list(kind) {
                    each_list(&mut reader, &mut self.lists, kind, inserted.len(), |at, cell| changes.write(inserted[at], column, cell))?;
                    continue;
                }
                each_number(&mut reader, &mut self.scratch, kind, inserted.len(), |at, cell| changes.write(inserted[at], column, cell))?;
            }
        }
        map_all(map.as_ref(), &self.changed, &mut self.mapped, false)?;
        let changed = std::mem::take(&mut self.mapped);
        for _ in 0..reader.count()? {
            let column = reader.count()?;
            let kind = *kinds.get(column).ok_or_else(|| invalid("Run names a column past the schema"))?;
            match reader.u8()? {
                3 => {
                    reader.slots(&mut self.except)?;
                    let mut except = Vec::with_capacity(self.except.len());
                    map_all(map.as_ref(), &self.except, &mut except, true)?;
                    let value = section::read_values(&mut reader, &mut self.scratch, kind, 1)?;
                    let mut targets = Vec::new();
                    let except = &except;
                    each_cell(value, 1, |_, cell| {
                        targets = changes.fill(column, cell, except)?;
                        Ok(())
                    })?;
                    let mut read_store = Vec::new();
                    map_all(map.as_ref(), &self.read, &mut read_store, true)?;
                    let mut inserted_store = if map.is_some() { self.mapped_inserted.clone() } else { self.inserted.clone() };
                    inserted_store.sort_unstable();
                    if truncated
                        && targets.iter().any(|slot| read_store.binary_search(slot).is_err() && inserted_store.binary_search(slot).is_err())
                    {
                        return Err(invalid("A partly read table fills a slot it did not read"));
                    }
                }
                2 => {
                    reader.positions(self.changed.len(), &mut self.positions)?;
                    for &position in &self.positions {
                        changes.rewrite(changed[position as usize], column)?;
                    }
                }
                4 => {
                    reader.positions(self.changed.len(), &mut self.positions)?;
                    reader.ints(self.positions.len(), &mut self.hunks)?;
                    let total = self.hunks.iter().try_fold(0_usize, |total, &count| total.checked_add(count as usize)).ok_or_else(|| invalid("PATCH hunks overflow"))?;
                    reader.ints(total.saturating_mul(3), &mut self.edits)?;
                    self.triples.clear();
                    self.triples.extend(self.edits.chunks_exact(3).map(|edit| (edit[0], edit[1], edit[2])));
                    let inserted = self.triples.iter().try_fold(0_usize, |total, edit| total.checked_add(edit.2 as usize)).ok_or_else(|| invalid("PATCH items overflow"))?;
                    self.items.clear();
                    match kind {
                        Kind::U32List => {
                            let mut items: Vec<u32> = Vec::new();
                            reader.ints(inserted, &mut items)?;
                            items.iter().for_each(|item| self.items.extend_from_slice(&item.to_le_bytes()));
                        }
                        Kind::U64List => {
                            let mut items: Vec<u64> = Vec::new();
                            reader.ints(inserted, &mut items)?;
                            items.iter().for_each(|item| self.items.extend_from_slice(&item.to_le_bytes()));
                        }
                        Kind::Opaque => reader.values(inserted, &mut self.items)?,
                        other => return Err(invalid(format!("PATCH to a {other:?} column"))),
                    }
                    let width = super::item_width(kind);
                    let (mut edit, mut item) = (0_usize, 0_usize);
                    for (at, &position) in self.positions.iter().enumerate() {
                        let count = self.hunks[at] as usize;
                        let hunks = &self.triples[edit..edit + count];
                        let bytes = hunks.iter().map(|hunk| hunk.2 as usize).sum::<usize>() * width;
                        changes.edit(changed[position as usize], column, Edits { hunks, items: &self.items[item..item + bytes] })?;
                        edit += count;
                        item += bytes;
                    }
                }
                tag @ (0 | 1) => {
                    reader.positions(self.changed.len(), &mut self.positions)?;
                    let carried = if tag == 1 { section::base(kind)? } else { kind };
                    if tag == 0 && is_list(kind) {
                        let (changed, positions) = (&changed, &self.positions);
                        each_list(&mut reader, &mut self.lists, kind, positions.len(), |at, cell| changes.write(changed[positions[at] as usize], column, cell))?;
                        continue;
                    }
                    let (changed, positions) = (&changed, &self.positions);
                    if tag == 1 {
                        each_number(&mut reader, &mut self.scratch, carried, positions.len(), |at, cell| changes.add(changed[positions[at] as usize], column, difference_of(cell)?))?;
                    } else {
                        each_number(&mut reader, &mut self.scratch, carried, positions.len(), |at, cell| changes.write(changed[positions[at] as usize], column, cell))?;
                    }
                }
                other => return Err(invalid(format!("Column run op {other}"))),
            }
        }
        let mut previous = None;
        for _ in 0..reader.count()? {
            let column = reader.count()?;
            if column >= kinds.len() || previous.is_some_and(|before| before >= column) {
                return Err(invalid("Unrecorded cells name columns out of order"));
            }
            previous = Some(column);
            reader.slots(&mut self.slots)?;
            for &slot in &self.slots {
                if self.changed.binary_search(&slot).is_err() && self.inserted.binary_search(&slot).is_err() {
                    return Err(invalid("Unrecorded cells name a slot the hour did not change"));
                }
                changes.unrecord(to_store(map.as_ref(), slot)?, column)?;
            }
        }
        reader.slots(&mut self.slots)?;
        for &slot in &self.slots {
            changes.touch(to_store(map.as_ref(), slot)?)?;
        }
        reader.finish()?;
        if truncated {
            changes.read_done();
        }
        self.mapped = changed;
        self.maps[table] = map;
        Ok(())
    }
}

impl Journal {
    fn passes_slots(&self, store: &Store, table: usize) -> bool {
        if self.maps[table].is_none() {
            return self.inserted.iter().any(|&slot| !valid_slot(slot));
        }
        let (inserted, slots) = (self.inserted.len() as u64, u64::from(store.table_state(table).map_or(0, TableState::slots)));
        slots + inserted > u64::from(SLOTS) && {
            let live = store.table_state(table).map_or(0, |held| held.live().count()) as u64;
            live.saturating_sub(self.deleted.len() as u64) + inserted > u64::from(SLOTS)
        }
    }

    fn log_section(&mut self, store: &mut Store, table: usize, bytes: &[u8]) -> io::Result<()> {
        if self.logs.len() <= table {
            self.logs.resize_with(table + 1, || None);
        }
        let mut log = self.logs[table].take().unwrap_or_default();
        let applied = match self.read_log(store, table, &log, bytes) {
            Ok(None) => self.apply_log(store, table, &mut log),
            Ok(Some(refused)) => self.refuse(store, table, &mut log, refused),
            Err(error) => Err(error),
        };
        self.logs[table] = Some(log);
        applied
    }

    fn read_log(&mut self, store: &Store, table: usize, log: &Log, bytes: &[u8]) -> io::Result<Option<Refused>> {
        if log.refused.is_some() {
            return Ok(None);
        }
        let kinds = store.kinds(table);
        let truncated = store.table_state(table).is_some_and(|held| matches!(held.status(), Status::Truncated { .. }));
        let mut reader = Reader::new(bytes);
        if truncated {
            reader.slots(&mut self.read)?;
        }
        reader.slots(&mut self.changed)?;
        reader.slots(&mut self.deleted)?;
        reader.slots(&mut self.inserted)?;
        if let Some(&slot) = self.deleted.iter().find(|&&slot| log.row(slot).is_none()) {
            return Err(invalid(format!("A section deletes slot {slot}, which holds no row")));
        }
        if let Some(&slot) = self.inserted.iter().find(|&&slot| log.row(slot).is_some() && self.deleted.binary_search(&slot).is_err()) {
            return Err(invalid(format!("Insert names slot {slot}, which holds a row")));
        }
        reader.keys(self.inserted.len(), &mut self.keys)?;
        let mut refused = (self.inserted.len() as u64 > u64::from(SLOTS)).then_some(Refused::Width);
        if refused.is_none() {
            let mut keys = self.keys.clone();
            keys.sort_unstable();
            if let Some(&key) = keys.iter().find(|&&key| log.given.holds(key)) {
                refused = Some(Refused::GivenAgain { key });
            } else if let Some(pair) = keys.windows(2).find(|pair| pair[0] == pair[1]) {
                refused = Some(Refused::GivenAgain { key: pair[0] });
            }
        }
        let columns = kinds.len();
        let rows = self.inserted.len();
        self.held.clear();
        self.held.cells.resize(rows * columns, Held::Unrecorded);
        for (column, &kind) in kinds.iter().enumerate().filter(|_| rows > 0) {
            let held = &mut self.held;
            let put = |at: usize, cell: Cell<'_>| {
                let cell = held.hold(cell);
                held.cells[at * columns + column] = cell;
                Ok(())
            };
            if is_list(kind) {
                each_list(&mut reader, &mut self.lists, kind, rows, put)?;
            } else {
                each_number(&mut reader, &mut self.scratch, kind, rows, put)?;
            }
        }
        for _ in 0..reader.count()? {
            let column = reader.count()?;
            let kind = *kinds.get(column).ok_or_else(|| invalid("Run names a column past the schema"))?;
            match reader.u8()? {
                3 => {
                    reader.slots(&mut self.except)?;
                    let value = section::read_values(&mut reader, &mut self.scratch, kind, 1)?;
                    let mut cell = Held::Unrecorded;
                    let held = &mut self.held;
                    each_cell(value, 1, |_, value| {
                        cell = held.hold(value);
                        Ok(())
                    })?;
                    let except = &self.except;
                    let excepted = |slot: u32| except.binary_search(&slot).is_ok();
                    let written = (0..=u32::MAX).zip(&log.rows).find(|&(slot, row)| row.is_some() && self.deleted.binary_search(&slot).is_err() && !excepted(slot)).map(|(slot, _)| slot);
                    if let Some(slot) = written {
                        refused = refused.or(Some(Refused::Written { slot, column }));
                    }
                    for (at, &slot) in self.inserted.iter().enumerate() {
                        if !excepted(slot) {
                            held.cells[at * columns + column] = cell;
                        }
                    }
                }
                op @ (0 | 1 | 2 | 4) => {
                    reader.positions(self.changed.len(), &mut self.positions)?;
                    if let Some(&position) = self.positions.first() {
                        refused = refused.or(Some(Refused::Written { slot: self.changed[position as usize], column }));
                    }
                    if op == 4 {
                        reader.ints(self.positions.len(), &mut self.hunks)?;
                        let total = self.hunks.iter().try_fold(0_usize, |total, &count| total.checked_add(count as usize)).ok_or_else(|| invalid("PATCH hunks overflow"))?;
                        reader.ints(total.saturating_mul(3), &mut self.edits)?;
                        let items = self.edits.chunks_exact(3).try_fold(0_usize, |total, edit| total.checked_add(edit[2] as usize)).ok_or_else(|| invalid("PATCH items overflow"))?;
                        match kind {
                            Kind::U32List => {
                                let mut skipped: Vec<u32> = Vec::new();
                                reader.ints(items, &mut skipped)?;
                            }
                            Kind::U64List => {
                                let mut skipped: Vec<u64> = Vec::new();
                                reader.ints(items, &mut skipped)?;
                            }
                            Kind::Opaque => reader.values(items, &mut self.items)?,
                            other => return Err(invalid(format!("PATCH to a {other:?} column"))),
                        }
                    } else if op != 2 {
                        let carried = if op == 1 { section::base(kind)? } else { kind };
                        section::read_values(&mut reader, &mut self.scratch, carried, self.positions.len())?;
                    }
                }
                other => return Err(invalid(format!("Column run op {other}"))),
            }
        }
        let mut previous = None;
        for _ in 0..reader.count()? {
            let column = reader.count()?;
            if column >= columns || previous.is_some_and(|before| before >= column) {
                return Err(invalid("Unrecorded cells name columns out of order"));
            }
            previous = Some(column);
            reader.slots(&mut self.slots)?;
            for &slot in &self.slots {
                match self.inserted.iter().position(|&held| held == slot) {
                    Some(at) => self.held.cells[at * columns + column] = Held::Unrecorded,
                    None if self.changed.binary_search(&slot).is_ok() => {
                        refused = refused.or(Some(Refused::Written { slot, column }));
                    }
                    None => return Err(invalid("Unrecorded cells name a slot the hour did not change")),
                }
            }
        }
        reader.slots(&mut self.slots)?;
        reader.finish()?;
        Ok(refused)
    }

    fn apply_log(&mut self, store: &mut Store, table: usize, log: &mut Log) -> io::Result<()> {
        if log.refused.is_some() {
            return Ok(());
        }
        let columns = store.kinds(table).len();
        let mut changes = store.table(table)?;
        for &slot in &self.deleted {
            let (ordinal, _) = log.rows[slot as usize].take().ok_or_else(|| invalid(format!("A section deletes slot {slot}, which holds no row")))?;
            changes.forget(ordinal)?;
        }
        let mut cells: Vec<Option<Cell<'_>>> = Vec::with_capacity(columns);
        for (at, (&slot, &key)) in self.inserted.iter().zip(&self.keys).enumerate() {
            cells.clear();
            cells.extend(self.held.cells[at * columns..(at + 1) * columns].iter().map(|&held| self.held.cell(held)));
            let ordinal = changes.append(key, &cells)?;
            if log.rows.len() <= slot as usize {
                log.rows.resize(slot as usize + 1, None);
            }
            log.rows[slot as usize] = Some((ordinal, key));
            log.given.add(key);
        }
        Ok(())
    }

    fn refuse(&mut self, store: &mut Store, table: usize, log: &mut Log, refused: Refused) -> io::Result<()> {
        let name = (self.names.table)(table);
        let text = match refused {
            Refused::Written { slot, column } => {
                let key = log.row(slot).map_or(0, |(_, key)| key);
                format!("a log row changed after it was written: {name} key {key:#x} column {}", (self.names.column)(table, column))
            }
            Refused::GivenAgain { key } => format!("a log row changed after it was written: {name} key {key:#x} given again"),
            Refused::Width => format!("{name}: more than 4,294,967,295 rows, the format's slot width"),
        };
        let reason = u32::MAX.checked_sub(self.reasons).ok_or_else(|| invalid("Refusals pass the string ids"))?;
        self.reasons = self.reasons.checked_add(1).ok_or_else(|| invalid("Refusals pass the string ids"))?;
        store.pin_string(reason, text.as_bytes())?;
        let sampled = store.table_state(table).and_then(|held| held.sampled_at());
        let read = self.reads.get(table).copied().unwrap_or(false);
        store.status(table, Status::Unavailable { reason }, read, sampled, true)?;
        log.refused = Some((reason, text.into_bytes().into()));
        Ok(())
    }
}
