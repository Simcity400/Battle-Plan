//! The writer's side of the change grammar: one table's held columns, an hour's candidate
//! rows compared once with them, and the section that hour writes. [`TableWriter::commit`]
//! applies the section through the reader's own [`section::apply`], so after the same hours
//! the writer holds exactly the table, status, sample hour and view a walker holds.

use crate::{
    invalid,
    manifest::{self, Status},
    restart::{self, Acquisition, Restored},
    section::{self, Changes, Op, Run, Scratch, TableHour},
    state::{Cell, Kind, Table, Values},
};
use std::{
    cmp::Reverse,
    collections::{BinaryHeap, HashMap},
    io,
    sync::atomic::{AtomicU64, Ordering},
};

#[cfg(test)]
mod tests;

type Hasher = foldhash::fast::RandomState;

/// Names each writer, so a preparation commits only to the writer that made it.
static WRITERS: AtomicU64 = AtomicU64::new(1);

/// A run's strings: exact bytes with append-only ids, each appended before its first use.
#[derive(Default)]
pub struct Dictionary {
    ids: HashMap<Box<[u8]>, u32, Hasher>,
    len: u32,
    appended: Vec<Box<[u8]>>,
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

    /// The id of `bytes`, appending it when the run has not used it.
    pub fn intern(&mut self, bytes: &[u8]) -> io::Result<u32> {
        if let Some(&id) = self.ids.get(bytes) {
            return Ok(id);
        }
        let id = self.len;
        let next = id
            .checked_add(1)
            .ok_or_else(|| invalid("Dictionary ids exceed 32 bits"))?;
        let mut owned = engine_allocation::vec(bytes.len())?;
        owned.extend_from_slice(bytes);
        let owned = engine_allocation::boxed_slice(owned)?;
        let mut copy = engine_allocation::vec(bytes.len())?;
        copy.extend_from_slice(bytes);
        let copy = engine_allocation::boxed_slice(copy)?;
        self.ids
            .try_reserve(1)
            .map_err(|_| engine_allocation::oom())?;
        engine_allocation::reserve(&mut self.appended, 1)?;
        self.ids.insert(owned, id);
        self.appended.push(copy);
        self.len = next;
        Ok(id)
    }

    /// The strings appended since the last take, in id order: the hour's manifest appends.
    pub fn take_appended(&mut self) -> Vec<Box<[u8]>> {
        std::mem::take(&mut self.appended)
    }
}

/// Which held rows an hour's visits cover.
#[derive(Clone, Copy, Debug)]
pub enum Coverage<'a> {
    /// A complete census: a held row the hour does not visit is deleted.
    Complete,
    /// A partial read: the held rows visited are the rows read, and nothing is deleted.
    Partial,
    /// Proven sparse changes on a complete table: only the visited rows change, and
    /// `removed` rows are deleted.
    Changes { removed: &'a [u128] },
}

/// One candidate cell.
#[derive(Clone, Copy, Debug)]
pub enum Input<'a> {
    Value(Cell<'a>),
    /// A Bytes column's string, interned in the run's dictionary.
    Text(&'a [u8]),
    /// The recording did not record this cell.
    Unrecorded,
}

/// The sample hour a table's view holds after an hour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sample {
    /// The read bit decides: a read table is sampled at the hour, another keeps its last.
    FromReadBit,
    /// Not known: an untimed snapshot.
    Reset,
    /// A carried acquisition's hour.
    Carried(i32),
}

/// A table's evidence for one hour, beside its rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Evidence {
    pub status: Status,
    /// The manifest's read bit.
    pub read: bool,
    pub sample: Sample,
}

/// One cell's value kept by its own type: integers by their bits, lists and opaque
/// cells as spans of item arrays.
#[derive(Default)]
struct Pending {
    /// The held slot, or the insert's index, of each entry.
    at: Vec<u32>,
    words: Vec<u64>,
    present: Vec<bool>,
    spans: Vec<(u32, u32)>,
    u32s: Vec<u32>,
    u64s: Vec<u64>,
    bytes: Vec<u8>,
}

fn span(start: usize, len: usize) -> io::Result<(u32, u32)> {
    let end = start
        .checked_add(len)
        .filter(|&end| u32::try_from(end).is_ok())
        .ok_or_else(|| invalid("Cell items exceed 32 bits"))?;
    Ok((start as u32, (end - start) as u32))
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
    }

    fn len(&self) -> usize {
        self.at.len()
    }

    fn push(&mut self, at: u32, cell: Cell<'_>) -> io::Result<()> {
        let (word, present) = match cell {
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
            Cell::U32List(items) => {
                engine_allocation::reserve(&mut self.spans, 1)?;
                engine_allocation::reserve(&mut self.u32s, items.len())?;
                self.spans.push(span(self.u32s.len(), items.len())?);
                self.u32s.extend_from_slice(items);
                (0, true)
            }
            Cell::U64List(items) => {
                engine_allocation::reserve(&mut self.spans, 1)?;
                engine_allocation::reserve(&mut self.u64s, items.len())?;
                self.spans.push(span(self.u64s.len(), items.len())?);
                self.u64s.extend_from_slice(items);
                (0, true)
            }
            Cell::Opaque(items) => {
                engine_allocation::reserve(&mut self.spans, 1)?;
                engine_allocation::reserve(&mut self.bytes, items.len())?;
                self.spans.push(span(self.bytes.len(), items.len())?);
                self.bytes.extend_from_slice(items);
                (0, true)
            }
        };
        engine_allocation::reserve(&mut self.at, 1)?;
        engine_allocation::reserve(&mut self.words, 1)?;
        engine_allocation::reserve(&mut self.present, 1)?;
        self.at.push(at);
        self.words.push(word);
        self.present.push(present);
        Ok(())
    }
}

/// One run's values, laid out as [`Values`] borrows them.
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
    }

    /// `pending`'s entries at `order` (indices into it), as `kind`'s values.
    fn fill(&mut self, kind: Kind, pending: &Pending, order: &[u32]) -> io::Result<()> {
        self.clear();
        let count = order.len();
        let words = order.iter().map(|&at| pending.words[at as usize]);
        match kind {
            Kind::U8 | Kind::Bool => {
                engine_allocation::reserve(&mut self.u8s, count)?;
                self.u8s.extend(words.map(|word| word as u8));
            }
            Kind::I8 => {
                engine_allocation::reserve(&mut self.i8s, count)?;
                self.i8s.extend(words.map(|word| word as i8));
            }
            Kind::U16 => {
                engine_allocation::reserve(&mut self.u16s, count)?;
                self.u16s.extend(words.map(|word| word as u16));
            }
            Kind::I16 => {
                engine_allocation::reserve(&mut self.i16s, count)?;
                self.i16s.extend(words.map(|word| word as i16));
            }
            Kind::U32 | Kind::Bytes => {
                engine_allocation::reserve(&mut self.u32s, count)?;
                self.u32s.extend(words.map(|word| word as u32));
            }
            Kind::I32 => {
                engine_allocation::reserve(&mut self.i32s, count)?;
                self.i32s.extend(words.map(|word| word as i32));
            }
            Kind::U64 => {
                engine_allocation::reserve(&mut self.u64s, count)?;
                self.u64s.extend(words);
            }
            Kind::I64 => {
                engine_allocation::reserve(&mut self.i64s, count)?;
                self.i64s.extend(words.map(|word| word as i64));
            }
            Kind::OptionalI32 | Kind::OptionalU32 | Kind::OptionalI64 | Kind::OptionalU64 => {
                self.present = engine_allocation::filled(count.div_ceil(64), 0u64)?;
                for (index, &at) in order.iter().enumerate() {
                    if !pending.present[at as usize] {
                        continue;
                    }
                    self.present[index / 64] |= 1 << (index % 64);
                    let word = pending.words[at as usize];
                    match kind {
                        Kind::OptionalI32 => {
                            engine_allocation::reserve(&mut self.i32s, 1)?;
                            self.i32s.push(word as i32);
                        }
                        Kind::OptionalU32 => {
                            engine_allocation::reserve(&mut self.u32s, 1)?;
                            self.u32s.push(word as u32);
                        }
                        Kind::OptionalI64 => {
                            engine_allocation::reserve(&mut self.i64s, 1)?;
                            self.i64s.push(word as i64);
                        }
                        _ => {
                            engine_allocation::reserve(&mut self.u64s, 1)?;
                            self.u64s.push(word);
                        }
                    }
                }
            }
            Kind::U32List | Kind::U64List | Kind::Opaque => {
                engine_allocation::reserve(&mut self.ends, count)?;
                let mut end = 0u32;
                for &at in order {
                    let (start, len) = pending.spans[at as usize];
                    let range = start as usize..(start + len) as usize;
                    match kind {
                        Kind::U32List => {
                            engine_allocation::reserve(&mut self.u32s, range.len())?;
                            self.u32s.extend_from_slice(&pending.u32s[range]);
                        }
                        Kind::U64List => {
                            engine_allocation::reserve(&mut self.u64s, range.len())?;
                            self.u64s.extend_from_slice(&pending.u64s[range]);
                        }
                        _ => {
                            engine_allocation::reserve(&mut self.u8s, range.len())?;
                            self.u8s.extend_from_slice(&pending.bytes[range]);
                        }
                    }
                    end = end
                        .checked_add(len)
                        .ok_or_else(|| invalid("Cell items exceed 32 bits"))?;
                    self.ends.push(end);
                }
            }
        }
        Ok(())
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

/// The value an unrecorded inserted cell holds: never read, since the cell is unrecorded.
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

/// One table's held state and the buffers its hours reuse.
pub struct TableWriter {
    kinds: Box<[Kind]>,
    table: Table,
    status: Status,
    acquisition: Acquisition,
    /// A truncated table's view: its last hour's read and inserted slots, ascending.
    view: Vec<u32>,
    /// Slots below the table's extent that hold no row, lowest first.
    free: BinaryHeap<Reverse<u32>>,
    /// This writer's name and the hours begun, so only its latest preparation commits.
    identity: u64,
    generation: u64,
    /// A commit failed part way: the held table is not one a walker holds.
    poisoned: bool,
    // Per hour, reused. `seen` holds one bit per visited held slot and is zero between
    // hours: the hour clears only the bits `visited` names.
    seen: Vec<u64>,
    visited: Vec<u32>,
    updates: Vec<Pending>,
    unrecord: Vec<Vec<u32>>,
    noops: Vec<Vec<u32>>,
    touched: Vec<u32>,
    /// A changes hour's removed keys, ascending.
    removed: Vec<u128>,
    /// The hour's new keys, in visit order: an insert's index is its position.
    new_keys: Vec<u128>,
    inserts: Vec<Pending>,
    insert_unrecorded: Vec<Vec<u32>>,
    run_outs: Vec<Out>,
    insert_outs: Vec<Out>,
    scratch: Scratch,
    changes: Changes,
}

impl TableWriter {
    pub fn new(kinds: &[Kind]) -> io::Result<Self> {
        Self::with_table(kinds, Table::new(kinds)?)
    }

    fn with_table(kinds: &[Kind], table: Table) -> io::Result<Self> {
        let columns = kinds.len();
        let many = |count: usize| engine_allocation::collect_vec((0..count).map(|_| Vec::new()));
        Ok(Self {
            kinds: engine_allocation::boxed_slice(engine_allocation::collect_vec(
                kinds.iter().copied(),
            )?)?,
            table,
            status: Status::NotRecorded,
            acquisition: Acquisition::default(),
            view: Vec::new(),
            free: BinaryHeap::new(),
            identity: WRITERS.fetch_add(1, Ordering::Relaxed),
            generation: 0,
            poisoned: false,
            seen: Vec::new(),
            visited: Vec::new(),
            updates: engine_allocation::collect_vec((0..columns).map(|_| Pending::default()))?,
            unrecord: many(columns)?,
            noops: many(columns)?,
            touched: Vec::new(),
            removed: Vec::new(),
            new_keys: Vec::new(),
            inserts: engine_allocation::collect_vec((0..columns).map(|_| Pending::default()))?,
            insert_unrecorded: many(columns)?,
            run_outs: engine_allocation::collect_vec((0..columns).map(|_| Out::default()))?,
            insert_outs: engine_allocation::collect_vec((0..columns).map(|_| Out::default()))?,
            scratch: Scratch::default(),
            changes: Changes::default(),
        })
    }

    /// A writer continuing from a restart: the next hour writes what it would have written
    /// had the writer never stopped.
    pub fn restore(kinds: &[Kind], restored: Restored) -> io::Result<Self> {
        if !restored.table.kinds().eq(kinds.iter().copied()) {
            return Err(invalid("Restart table's columns differ from its schema"));
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
        writer.free = BinaryHeap::from(free);
        Ok(writer)
    }

    /// The restart a block beginning now carries.
    pub fn restart(&self) -> io::Result<Vec<u8>> {
        self.usable()?;
        restart::write(&self.table, self.status, &self.acquisition, &self.view)
    }

    pub fn table(&self) -> &Table {
        &self.table
    }

    pub fn status(&self) -> Status {
        self.status
    }

    pub fn acquisition(&self) -> &Acquisition {
        &self.acquisition
    }

    /// The slots of the rows a truncated table read, ascending.
    pub fn view(&self) -> &[u32] {
        &self.view
    }

    fn usable(&self) -> io::Result<()> {
        if self.poisoned {
            return Err(invalid(
                "Column writer failed part way through a commit; restore it from a restart",
            ));
        }
        Ok(())
    }

    /// Starts an hour's rows. Nothing held changes until its preparation commits.
    pub fn begin<'w>(
        &'w mut self,
        dictionary: &'w mut Dictionary,
        coverage: Coverage<'w>,
    ) -> io::Result<Hour<'w>> {
        self.usable()?;
        self.generation += 1;
        for &slot in &self.visited {
            self.seen[slot as usize / 64] &= !(1 << (slot % 64));
        }
        self.visited.clear();
        engine_allocation::resize_default(&mut self.seen, self.table.slots().div_ceil(64))?;
        for pending in self.updates.iter_mut().chain(&mut self.inserts) {
            pending.clear();
        }
        for slots in self
            .unrecord
            .iter_mut()
            .chain(&mut self.noops)
            .chain(&mut self.insert_unrecorded)
        {
            slots.clear();
        }
        self.touched.clear();
        self.new_keys.clear();
        self.removed.clear();
        if let Coverage::Changes { removed } = coverage {
            engine_allocation::reserve(&mut self.removed, removed.len())?;
            self.removed.extend_from_slice(removed);
            self.removed.sort_unstable();
            if self.removed.windows(2).any(|pair| pair[0] == pair[1]) {
                return Err(invalid("A removed row key is named twice"));
            }
        }
        Ok(Hour {
            writer: self,
            dictionary,
            coverage,
            inserted: 0,
            failed: false,
        })
    }



    /// Applies a section to the held table and keeps the free slots it leaves.
    fn apply(&mut self, bytes: &[u8], truncated: bool) -> io::Result<()> {
        let extent = self.table.slots();
        section::apply(
            bytes,
            &self.kinds,
            truncated,
            &mut self.table,
            &mut self.scratch,
            &mut self.changes,
        )?;
        let changes = &self.changes;
        // Inserts below the old extent the hour did not free itself took the lowest free
        // slots, in order, as the preparation did.
        for &slot in &changes.inserted {
            if slot as usize >= extent || changes.deleted.binary_search(&slot).is_ok() {
                continue;
            }
            if self.free.pop() != Some(Reverse(slot)) {
                return Err(invalid("Inserted slots are not the lowest free ones"));
            }
        }
        for &slot in &changes.deleted {
            if changes.inserted.binary_search(&slot).is_err() {
                self.free
                    .try_reserve(1)
                    .map_err(|_| engine_allocation::oom())?;
                self.free.push(Reverse(slot));
            }
        }
        Ok(())
    }

    /// Applies an hour: its section (none for an hour that changed no row, or one that
    /// read none) and its evidence, as a walker applies its manifest and section.
    pub fn commit(
        &mut self,
        prepared: Option<Prepared>,
        evidence: Evidence,
        hour: i32,
    ) -> io::Result<()> {
        self.usable()?;
        let truncated = matches!(evidence.status, Status::Truncated { .. });
        let readable = matches!(evidence.status, Status::Complete | Status::Truncated { .. });
        let written = match prepared {
            Some(prepared) => {
                if (prepared.identity, prepared.generation) != (self.identity, self.generation) {
                    return Err(invalid("Column preparation is not the latest hour's"));
                }
                if !readable {
                    return Err(invalid("A table with no rows this hour carries a section"));
                }
                if prepared.partial != truncated {
                    return Err(invalid(
                        "A partial read and a truncated status come together",
                    ));
                }
                prepared.section
            }
            None => None,
        };
        if truncated && written.is_none() {
            return Err(invalid("A partly read table names no rows it read"));
        }
        if let Some(bytes) = &written
            && let Err(error) = self.apply(bytes, truncated)
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
        self.status = evidence.status;
        self.view.clear();
        if truncated {
            let changes = &self.changes;
            let live = self.table.live();
            let reserved = engine_allocation::reserve(
                &mut self.view,
                changes.read.len() + changes.inserted.len(),
            );
            if let Err(error) = reserved {
                self.poisoned = true;
                return Err(error);
            }
            // A read row the hour deleted is gone; a slot it deleted and filled again holds
            // the inserted row, once.
            self.view
                .extend(changes.read.iter().copied().filter(|&slot| live.get(slot)));
            self.view.extend_from_slice(&changes.inserted);
            self.view.sort_unstable();
            self.view.dedup();
        }
        Ok(())
    }
}

/// An hour's rows while they are visited.
pub struct Hour<'w> {
    writer: &'w mut TableWriter,
    dictionary: &'w mut Dictionary,
    coverage: Coverage<'w>,
    inserted: u32,
    /// A row, NOOP or touch was refused: the hour is partly staged and never finishes.
    failed: bool,
}

/// An hour's section and what its manifest says of it.
#[derive(Debug)]
pub struct Prepared {
    identity: u64,
    generation: u64,
    partial: bool,
    /// None: the hour changed no row of a fully read table, so it writes no section.
    pub section: Option<Vec<u8>>,
    /// One bit per column a run or unrecorded cell names.
    pub columns: Vec<u64>,
    /// [`manifest::INSERTS`], [`manifest::DELETES`] and [`manifest::TOUCHES`].
    pub ops: u8,
}

impl Hour<'_> {
    fn held_slot(&self, key: u128) -> Option<u32> {
        self.writer.table.slot(key)
    }

    fn mark_seen(&mut self, slot: u32) -> io::Result<()> {
        let word = &mut self.writer.seen[slot as usize / 64];
        let bit = 1u64 << (slot % 64);
        if *word & bit != 0 {
            return Err(invalid(format!("An hour visits row slot {slot} twice")));
        }
        engine_allocation::reserve(&mut self.writer.visited, 1)?;
        *word |= bit;
        self.writer.visited.push(slot);
        Ok(())
    }

    /// Runs one staging step; a refusal fails the whole hour.
    fn staged(&mut self, step: impl FnOnce(&mut Self) -> io::Result<()>) -> io::Result<()> {
        if self.failed {
            return Err(invalid("An earlier row of this hour was refused"));
        }
        let result = step(self);
        self.failed = result.is_err();
        result
    }

    fn resolve<'c>(&mut self, column: usize, input: Input<'c>) -> io::Result<Option<Cell<'c>>> {
        let kind = self.writer.kinds[column];
        let cell = match input {
            Input::Unrecorded => return Ok(None),
            Input::Text(bytes) if kind == Kind::Bytes => {
                Cell::Bytes(self.dictionary.intern(bytes)?)
            }
            Input::Text(_) => {
                return Err(invalid(format!("Column {column} is not a Bytes column")));
            }
            Input::Value(cell) => cell,
        };
        if cell_kind(&cell) != kind {
            return Err(invalid(format!(
                "Column {column} holds {kind:?}, not {:?}",
                cell_kind(&cell)
            )));
        }
        if let Cell::Bytes(id) = cell
            && id as usize >= self.dictionary.len()
        {
            return Err(invalid(format!("Dictionary id {id} is not in the run's dictionary")));
        }
        Ok(Some(cell))
    }

    /// One candidate row, with one input per column in schema order. A held row's cells
    /// are compared once with what it holds; a new key becomes an insert.
    pub fn row(&mut self, key: u128, cells: &[Input<'_>]) -> io::Result<()> {
        self.staged(|hour| hour.stage_row(key, cells))
    }

    fn stage_row(&mut self, key: u128, cells: &[Input<'_>]) -> io::Result<()> {
        if cells.len() != self.writer.kinds.len() {
            return Err(invalid("A row carries other than one cell per column"));
        }
        if self.writer.removed.binary_search(&key).is_ok() {
            return Err(invalid(format!("Row key {key:#x} is both changed and removed")));
        }
        match self.held_slot(key) {
            Some(slot) => {
                self.mark_seen(slot)?;
                for (column, &input) in cells.iter().enumerate() {
                    let cell = self.resolve(column, input)?;
                    let recorded = self.writer.table.recorded(column, slot);
                    match cell {
                        None if recorded => {
                            engine_allocation::reserve(&mut self.writer.unrecord[column], 1)?;
                            self.writer.unrecord[column].push(slot);
                        }
                        None => {}
                        Some(cell) => {
                            let held = self
                                .writer
                                .table
                                .column(column)
                                .and_then(|values| values.cell(slot));
                            if !recorded || held != Some(cell) {
                                self.writer.updates[column].push(slot, cell)?;
                            }
                        }
                    }
                }
            }
            None => {
                let index = self.inserted;
                engine_allocation::reserve(&mut self.writer.new_keys, 1)?;
                self.writer.new_keys.push(key);
                for (column, &input) in cells.iter().enumerate() {
                    let cell = match self.resolve(column, input)? {
                        Some(cell) => cell,
                        None => {
                            engine_allocation::reserve(
                                &mut self.writer.insert_unrecorded[column],
                                1,
                            )?;
                            self.writer.insert_unrecorded[column].push(index);
                            placeholder(self.writer.kinds[column])
                        }
                    };
                    self.writer.inserts[column].push(index, cell)?;
                }
                self.inserted = index
                    .checked_add(1)
                    .ok_or_else(|| invalid("An hour inserts more than 2^32 rows"))?;
            }
        }
        Ok(())
    }

    /// The producer's explicit write of a value the held row already holds, preserved as
    /// a NOOP. The row must be visited this hour.
    pub fn noop(&mut self, key: u128, column: usize) -> io::Result<()> {
        self.staged(|hour| hour.stage_noop(key, column))
    }

    fn stage_noop(&mut self, key: u128, column: usize) -> io::Result<()> {
        let slot = self
            .held_slot(key)
            .ok_or_else(|| invalid(format!("NOOP names row key {key:#x}, which is not held")))?;
        if column >= self.writer.kinds.len() {
            return Err(invalid("NOOP names a column past the schema"));
        }
        engine_allocation::reserve(&mut self.writer.noops[column], 1)?;
        self.writer.noops[column].push(slot);
        Ok(())
    }

    /// The producer's touch of a held row that changes no cell.
    pub fn touch(&mut self, key: u128) -> io::Result<()> {
        self.staged(|hour| hour.stage_touch(key))
    }

    fn stage_touch(&mut self, key: u128) -> io::Result<()> {
        let slot = self
            .held_slot(key)
            .ok_or_else(|| invalid(format!("Touch names row key {key:#x}, which is not held")))?;
        engine_allocation::reserve(&mut self.writer.touched, 1)?;
        self.writer.touched.push(slot);
        Ok(())
    }

    /// The hour's section. Nothing held changes: [`TableWriter::commit`] applies it.
    pub fn finish(self) -> io::Result<Prepared> {
        let Self {
            writer,
            coverage,
            inserted,
            failed,
            ..
        } = self;
        if failed {
            return Err(invalid("An hour with a refused row writes no section"));
        }
        let columns = writer.kinds.len();
        let seen = |slot: u32| writer.seen[slot as usize / 64] >> (slot % 64) & 1 != 0;
        let partial = matches!(coverage, Coverage::Partial);
        // Deletes, ascending.
        let mut deleted: Vec<u32> = Vec::new();
        match coverage {
            Coverage::Complete => {
                for slot in writer.table.live().ones() {
                    if !seen(slot) {
                        engine_allocation::reserve(&mut deleted, 1)?;
                        deleted.push(slot);
                    }
                }
            }
            Coverage::Partial => {}
            Coverage::Changes { .. } => {
                engine_allocation::reserve(&mut deleted, writer.removed.len())?;
                for &key in &writer.removed {
                    let slot = writer.table.slot(key).ok_or_else(|| {
                        invalid(format!("Removed row key {key:#x} is not held"))
                    })?;
                    deleted.push(slot);
                }
                deleted.sort_unstable();
            }
        }
        // The read held slots of a partial read, ascending.
        let read = if partial {
            let mut read = engine_allocation::collect_vec(writer.visited.iter().copied())?;
            read.sort_unstable();
            Some(read)
        } else {
            None
        };
        // New rows by key, at the lowest free slots (deletes free theirs first), then at
        // new slots past the table's extent.
        let mut new_keys = engine_allocation::collect_vec(
            writer
                .new_keys
                .iter()
                .enumerate()
                .map(|(index, &key)| (key, index as u32)),
        )?;
        new_keys.sort_unstable();
        if let Some(pair) = new_keys.windows(2).find(|pair| pair[0].0 == pair[1].0) {
            return Err(invalid(format!("An hour inserts row key {:#x} twice", pair[0].0)));
        }
        let extent = u32::try_from(writer.table.slots())
            .map_err(|_| invalid("Row slots exceed 32 bits"))?;
        let fresh = new_keys.len().saturating_sub(writer.free.len() + deleted.len());
        if extent.checked_add(u32::try_from(fresh).unwrap_or(u32::MAX)).is_none() {
            return Err(invalid("Row slots exceed 32 bits"));
        }
        let mut slots_by_index = engine_allocation::filled(inserted as usize, 0u32)?;
        let mut keys_by_slot: Vec<(u32, u128, u32)> = engine_allocation::vec(new_keys.len())?;
        let mut popped: Vec<u32> = engine_allocation::vec(new_keys.len())?;
        {
            // The lowest free slots come off the heap here and go back below, so the held
            // heap is unchanged until the hour commits; nothing in between can fail.
            let mut reused = deleted.iter().copied().peekable();
            let mut next = extent;
            for &(key, index) in &new_keys {
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
                slots_by_index[index as usize] = slot;
                keys_by_slot.push((slot, key, index));
            }
            for &slot in &popped {
                writer.free.push(Reverse(slot));
            }
        }
        keys_by_slot.sort_unstable_by_key(|entry| entry.0);
        let inserted_slots = engine_allocation::collect_vec(keys_by_slot.iter().map(|e| e.0))?;
        let keys = engine_allocation::collect_vec(keys_by_slot.iter().map(|e| e.1))?;
        let insert_order = engine_allocation::collect_vec(keys_by_slot.iter().map(|e| e.2))?;
        // Changed held slots: those with a SET, a NOOP or a cell no longer recorded.
        let mut changed: Vec<u32> = Vec::new();
        for column in 0..columns {
            engine_allocation::reserve(
                &mut changed,
                writer.updates[column].len()
                    + writer.unrecord[column].len()
                    + writer.noops[column].len(),
            )?;
            changed.extend_from_slice(&writer.updates[column].at);
            changed.extend_from_slice(&writer.unrecord[column]);
            changed.extend_from_slice(&writer.noops[column]);
        }
        changed.sort_unstable();
        changed.dedup();
        if let Some(&slot) = changed.iter().find(|&&slot| !seen(slot)) {
            return Err(invalid(format!(
                "Row slot {slot} has a NOOP but was not visited this hour"
            )));
        }
        if let Some(&slot) = writer.touched.iter().find(|&&slot| !seen(slot)) {
            return Err(invalid(format!(
                "Row slot {slot} is touched but was not visited this hour"
            )));
        }
        if partial && !deleted.is_empty() {
            return Err(invalid("A partial read deletes no row"));
        }
        writer.touched.sort_unstable();
        writer.touched.dedup();
        let position = |slot: u32| changed.binary_search(&slot).expect("changed slot") as u32;
        // Runs: per column, its SETs ordered by slot, then its NOOPs for cells it did not set.
        let empty = |count: usize| -> io::Result<Vec<Vec<u32>>> {
            engine_allocation::collect_vec((0..count).map(|_| Vec::new()))
        };
        let mut set_order = empty(columns)?;
        let mut set_positions = empty(columns)?;
        let mut noop_positions = empty(columns)?;
        for column in 0..columns {
            let pending = &writer.updates[column];
            let order = &mut set_order[column];
            *order = engine_allocation::collect_vec(0..pending.len() as u32)?;
            order.sort_unstable_by_key(|&at| pending.at[at as usize]);
            let set_slots =
                engine_allocation::collect_vec(order.iter().map(|&at| pending.at[at as usize]))?;
            set_positions[column] =
                engine_allocation::collect_vec(set_slots.iter().map(|&slot| position(slot)))?;
            let noops = &mut writer.noops[column];
            noops.sort_unstable();
            noops.dedup();
            noops.retain(|slot| set_slots.binary_search(slot).is_err());
            noop_positions[column] =
                engine_allocation::collect_vec(noops.iter().map(|&slot| position(slot)))?;
        }
        for column in 0..columns {
            let kind = writer.kinds[column];
            writer.run_outs[column].fill(kind, &writer.updates[column], &set_order[column])?;
            writer.insert_outs[column].fill(kind, &writer.inserts[column], &insert_order)?;
        }
        let mut runs: Vec<Run<'_>> = Vec::new();
        for column in 0..columns {
            if !set_positions[column].is_empty() {
                engine_allocation::reserve(&mut runs, 1)?;
                runs.push(Run {
                    column,
                    op: Op::Set,
                    positions: &set_positions[column],
                    values: Some(writer.run_outs[column].values(writer.kinds[column])),
                    except: &[],
                    patches: None,
                });
            }
            if !noop_positions[column].is_empty() {
                engine_allocation::reserve(&mut runs, 1)?;
                runs.push(Run {
                    column,
                    op: Op::Noop,
                    positions: &noop_positions[column],
                    values: None,
                    except: &[],
                    patches: None,
                });
            }
        }
        let inserts = if inserted_slots.is_empty() {
            Vec::new()
        } else {
            engine_allocation::collect_vec(
                (0..columns).map(|column| writer.insert_outs[column].values(writer.kinds[column])),
            )?
        };
        // Unrecorded cells per column: held slots no longer recorded and inserted ones.
        let mut unrecorded_slots = empty(columns)?;
        for column in 0..columns {
            let slots = &mut unrecorded_slots[column];
            engine_allocation::reserve(
                slots,
                writer.unrecord[column].len() + writer.insert_unrecorded[column].len(),
            )?;
            slots.extend_from_slice(&writer.unrecord[column]);
            slots.extend(
                writer.insert_unrecorded[column]
                    .iter()
                    .map(|&index| slots_by_index[index as usize]),
            );
            slots.sort_unstable();
        }
        let unrecorded: Vec<(usize, &[u32])> = engine_allocation::collect_vec(
            unrecorded_slots
                .iter()
                .enumerate()
                .filter(|(_, slots)| !slots.is_empty())
                .map(|(column, slots)| (column, slots.as_slice())),
        )?;
        let mut column_bits = engine_allocation::filled(columns.div_ceil(64), 0u64)?;
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
        if !writer.touched.is_empty() {
            ops |= manifest::TOUCHES;
        }
        let quiet = changed.is_empty()
            && deleted.is_empty()
            && inserted_slots.is_empty()
            && writer.touched.is_empty();
        let bytes = if quiet && !partial {
            None
        } else {
            Some(section::write(
                &writer.kinds,
                &TableHour {
                    read: read.as_deref(),
                    changed: &changed,
                    deleted: &deleted,
                    inserted: &inserted_slots,
                    keys: &keys,
                    inserts: &inserts,
                    runs: &runs,
                    unrecorded: &unrecorded,
                    touched: &writer.touched,
                },
            )?)
        };
        Ok(Prepared {
            identity: writer.identity,
            generation: writer.generation,
            partial,
            section: bytes,
            columns: column_bits,
            ops,
        })
    }
}
