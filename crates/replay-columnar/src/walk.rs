//! A recording's current state while it is walked: every schema table, what each changed
//! in the last hour applied, and the string dictionary. Where an hour's bytes live is the
//! framing's business: an hour applies from its manifest and its sections' bytes.

use crate::{
    invalid,
    leaf::{Reader, Writer},
    manifest::{Manifest, Status},
    restart::{self, Acquisition},
    section::{self, Changes, Scratch},
    state::{Column, Kind, Table},
};
use std::{collections::HashMap, fmt, io, sync::Arc};

#[cfg(test)]
mod tests;

/// Strings `first..first + len()` of a run's dictionary, packed end to end.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Packed {
    first: usize,
    bytes: Vec<u8>,
    ends: Vec<usize>,
}

impl Packed {
    /// A dictionary chunk's strings as ids `first..`: a count, then each string
    /// length-prefixed. The strings keep the chunk's own allocation.
    pub fn read(first: usize, mut bytes: Vec<u8>) -> io::Result<Self> {
        let mut reader = Reader::new(&bytes);
        let count = reader.count()?;
        let mut read = reader.at();
        // Each string takes at least the byte of its length.
        if count > bytes.len() - read {
            return Err(invalid(
                "A dictionary chunk counts more strings than it holds",
            ));
        }
        let mut ends = engine_allocation::vec(count)?;
        let mut written = 0;
        for _ in 0..count {
            let mut reader = Reader::new(&bytes[read..]);
            let length = reader.count()?;
            let start = read + reader.at();
            reader.raw(length)?;
            read = start + length;
            bytes.copy_within(start..read, written);
            written += length;
            ends.push(written);
        }
        if read != bytes.len() {
            return Err(invalid(
                "A dictionary chunk holds bytes past its last string",
            ));
        }
        bytes.truncate(written);
        Ok(Self { first, bytes, ends })
    }

    pub fn len(&self) -> usize {
        self.ends.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ends.is_empty()
    }

    fn end(&self) -> usize {
        self.first + self.ends.len()
    }

    /// String `id`; None outside these ids.
    fn get(&self, id: u32) -> Option<&[u8]> {
        self.at((id as usize).checked_sub(self.first)?)
    }

    fn at(&self, at: usize) -> Option<&[u8]> {
        let end = *self.ends.get(at)?;
        let start = at.checked_sub(1).map_or(0, |previous| self.ends[previous]);
        Some(&self.bytes[start..end])
    }

    fn push(&mut self, text: &[u8]) -> io::Result<()> {
        engine_allocation::reserve(&mut self.bytes, text.len())?;
        engine_allocation::reserve(&mut self.ends, 1)?;
        self.bytes.extend_from_slice(text);
        self.ends.push(self.bytes.len());
        Ok(())
    }

    /// The bytes it allocated.
    fn held(&self) -> usize {
        self.bytes.capacity() + self.ends.capacity() * size_of::<usize>()
    }
}

/// Where a run's dictionary reads the strings it no longer holds.
pub trait Chunks: Send + Sync {
    /// The run's dictionary chunk holding string `id`, whole.
    fn chunk(&self, id: u32) -> io::Result<Arc<Packed>>;
}

/// What holds a pinned range of strings.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Held {
    /// The run's chunk, shared with the other walkers holding it.
    Chunk,
    /// Strings the walk appended before a restart it crossed, not yet found in its chunk.
    Appended,
    /// Appended strings no readable chunk equals: nothing else holds them.
    Alone,
}

struct Pin {
    strings: Arc<Packed>,
    held: Held,
}

/// Single strings kept once the pins holding them were dropped.
#[derive(Default)]
struct Sparse {
    bytes: Vec<u8>,
    ranges: HashMap<u32, (usize, usize), foldhash::fast::RandomState>,
}

impl Sparse {
    fn get(&self, id: u32) -> Option<&[u8]> {
        if self.ranges.is_empty() {
            return None;
        }
        let &(start, end) = self.ranges.get(&id)?;
        Some(&self.bytes[start..end])
    }

    fn push(&mut self, id: u32, text: &[u8]) -> io::Result<()> {
        engine_allocation::reserve(&mut self.bytes, text.len())?;
        engine_allocation::reserve_map(&mut self.ranges, 1)?;
        let start = self.bytes.len();
        self.bytes.extend_from_slice(text);
        self.ranges.insert(id, (start, self.bytes.len()));
        Ok(())
    }

    fn held(&self) -> usize {
        self.bytes.capacity() + self.ranges.capacity() * size_of::<(u32, (usize, usize))>()
    }
}

/// The recording's strings by id, appended in order and never changed. A run's dictionary
/// ([`Self::run`]) holds what its walk appended, the chunks it pinned and single strings;
/// past its budget it drops what no live cell names ([`State::trim_dictionary`]) and reads
/// a string back from its chunk when a cell names it again.
#[derive(Default)]
pub struct Dictionary {
    /// The strings appended since the walk's start or the last restart it crossed.
    log: Packed,
    /// Chunks and earlier logs held whole, ascending and apart, all below the log.
    pins: Vec<Pin>,
    sparse: Sparse,
    /// Some id below the log is held by no pin.
    gaps: bool,
    chunks: Option<Arc<dyn Chunks>>,
    budget: usize,
    /// The bytes it held after its last trim.
    trimmed: usize,
}

impl fmt::Debug for Dictionary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Dictionary")
            .field("len", &self.len())
            .field("held_bytes", &self.held_bytes())
            .finish_non_exhaustive()
    }
}

/// Dictionaries are equal when they hold the same strings at the same ids.
impl PartialEq for Dictionary {
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len()
            && (0..self.len()).all(|id| self.get(id as u32) == other.get(id as u32))
    }
}

impl Eq for Dictionary {}

impl Dictionary {
    /// A run's dictionary, holding none of the run's first `len` strings until they are
    /// pinned, which reads what it dropped from `chunks` and trims past `budget` bytes.
    pub fn run(chunks: Arc<dyn Chunks>, budget: usize, len: usize) -> Self {
        Self {
            log: Packed {
                first: len,
                ..Packed::default()
            },
            gaps: len != 0,
            chunks: Some(chunks),
            budget,
            ..Self::default()
        }
    }

    pub fn len(&self) -> usize {
        self.log.end()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// String `id`, when the dictionary holds it.
    pub fn get(&self, id: u32) -> Option<&[u8]> {
        if let Some(at) = (id as usize).checked_sub(self.log.first) {
            return self.log.at(at);
        }
        let at = self
            .pins
            .partition_point(|pin| pin.strings.first <= id as usize);
        at.checked_sub(1)
            .and_then(|at| self.pins[at].strings.get(id))
            .or_else(|| self.sparse.get(id))
    }

    /// String `id`, read from its chunk when the dictionary does not hold it: `held` keeps
    /// the chunk read last for the next call.
    pub fn resolve<'d>(
        &'d self,
        id: u32,
        held: &'d mut Option<Arc<Packed>>,
    ) -> io::Result<Option<&'d [u8]>> {
        if let Some(text) = self.get(id) {
            return Ok(Some(text));
        }
        let Some(chunks) = self.chunks.as_ref().filter(|_| (id as usize) < self.len()) else {
            return Ok(None);
        };
        if held.as_ref().is_none_or(|chunk| chunk.get(id).is_none()) {
            *held = Some(chunks.chunk(id)?);
        }
        Ok(held.as_deref().and_then(|chunk| chunk.get(id)))
    }

    /// The bytes its strings hold, chunks it shares with other walkers included.
    pub fn held_bytes(&self) -> usize {
        self.log.held()
            + self.sparse.held()
            + self.pins.capacity() * size_of::<Pin>()
            + self
                .pins
                .iter()
                .map(|pin| pin.strings.held())
                .sum::<usize>()
    }

    pub fn append(&mut self, text: &[u8]) -> io::Result<()> {
        self.log.push(text)
    }

    /// Strings `first..` as one chunk: a count, then each string length-prefixed, as a
    /// manifest carries its appends.
    pub fn chunk(&self, first: usize) -> io::Result<Vec<u8>> {
        let mut writer = Writer::new();
        writer.varint(self.len().saturating_sub(first) as u64)?;
        for id in first..self.len() {
            let text = self
                .get(id as u32)
                .ok_or_else(|| invalid("A chunk's string is not held"))?;
            writer.varint(text.len() as u64)?;
            writer.raw(text)?;
        }
        Ok(writer.into_bytes())
    }

    /// Appends a chunk's strings, in order.
    pub fn read_chunk(&mut self, bytes: &[u8]) -> io::Result<()> {
        let mut reader = Reader::new(bytes);
        let count = reader.count()?;
        for _ in 0..count {
            let length = reader.count()?;
            self.append(reader.raw(length)?)?;
        }
        reader.finish()
    }

    /// Holds string `id`: a run's dictionary pins the chunk holding it when it held none.
    pub fn hold(&mut self, id: u32) -> io::Result<()> {
        if id as usize >= self.len() || self.get(id).is_some() {
            return Ok(());
        }
        let Some(chunks) = &self.chunks else {
            return Ok(());
        };
        let chunk = chunks.chunk(id)?;
        self.pin(chunk)
    }

    /// Holds `strings`, one of the run's chunks, whole.
    fn pin(&mut self, strings: Arc<Packed>) -> io::Result<()> {
        if strings.is_empty() {
            return Ok(());
        }
        let at = self
            .pins
            .partition_point(|pin| pin.strings.first < strings.first);
        if self
            .pins
            .get(at)
            .is_some_and(|pin| pin.strings.first == strings.first)
        {
            return Ok(());
        }
        engine_allocation::reserve(&mut self.pins, 1)?;
        self.pins.insert(
            at,
            Pin {
                strings,
                held: Held::Chunk,
            },
        );
        self.gaps = !self.whole();
        Ok(())
    }

    /// Whether the pins hold every id below the log.
    fn whole(&self) -> bool {
        let mut next = 0;
        for pin in &self.pins {
            if pin.strings.first != next {
                return false;
            }
            next = pin.strings.end();
        }
        next == self.log.first
    }

    /// Starts a new log at a restart the walk crosses: what it appended since is pinned,
    /// to be dropped once found equal to the restart's chunk.
    fn freeze(&mut self) -> io::Result<()> {
        if self.chunks.is_none() || self.log.is_empty() {
            return Ok(());
        }
        let strings = Packed {
            first: self.log.first,
            bytes: engine_allocation::copy_vec(&self.log.bytes)?,
            ends: engine_allocation::copy_vec(&self.log.ends)?,
        };
        engine_allocation::reserve(&mut self.pins, 1)?;
        self.log.first = self.log.end();
        self.log.bytes.clear();
        self.log.ends.clear();
        self.pins.push(Pin {
            strings: Arc::new(strings),
            held: Held::Appended,
        });
        Ok(())
    }

    /// Whether a run's dictionary holds more than its budget, and twice what it held after
    /// its last trim.
    fn over(&self) -> bool {
        self.chunks.is_some() && {
            let held = self.held_bytes();
            held > self.budget && held / 2 > self.trimmed
        }
    }

    /// Keeps of what is below the log only the strings `named` names (ascending, each once):
    /// a pin whose named strings hold a quarter of its bytes stays whole; the named strings
    /// of the others are kept alone. Appended strings go only once their chunk equals them.
    fn trim(&mut self, named: &[u32]) -> io::Result<()> {
        let named = &named[..named.partition_point(|&id| (id as usize) < self.log.first)];
        let mut kept = engine_allocation::vec(self.pins.len())?;
        for at in 0..self.pins.len() {
            let dense = {
                let strings = &self.pins[at].strings;
                let ids = &named[named.partition_point(|&id| (id as usize) < strings.first)
                    ..named.partition_point(|&id| (id as usize) < strings.end())];
                let bytes: usize = ids
                    .iter()
                    .filter_map(|&id| strings.get(id))
                    .map(<[u8]>::len)
                    .sum();
                !ids.is_empty() && bytes.saturating_mul(4) >= strings.bytes.len()
            };
            let held = self.pins[at].held;
            let keep = dense
                || match held {
                    Held::Chunk => false,
                    Held::Alone => true,
                    Held::Appended => !self.back(at)?,
                };
            kept.push(keep);
        }
        let mut sparse = Sparse::default();
        let mut pin = 0;
        for &id in named {
            while self
                .pins
                .get(pin)
                .is_some_and(|held| held.strings.end() <= id as usize)
            {
                pin += 1;
            }
            if kept.get(pin).copied().unwrap_or(false)
                && self.pins[pin].strings.first <= id as usize
            {
                continue;
            }
            if let Some(text) = self.get(id) {
                sparse.push(id, text)?;
            }
        }
        let mut at = 0;
        self.pins.retain(|_| {
            at += 1;
            kept[at - 1]
        });
        self.sparse = sparse;
        self.gaps = !self.whole();
        self.trimmed = self.held_bytes();
        Ok(())
    }

    /// Whether the appended strings pinned at `at` equal their restart's chunk, which then
    /// holds them in their place.
    fn back(&mut self, at: usize) -> io::Result<bool> {
        let Some(chunks) = &self.chunks else {
            return Ok(false);
        };
        let read = u32::try_from(self.pins[at].strings.first)
            .map_err(|_| invalid("Dictionary ids exceed 32 bits"))
            .and_then(|id| chunks.chunk(id));
        let chunk = match read {
            Ok(chunk) => chunk,
            Err(error) if error.kind() == io::ErrorKind::InvalidData => {
                self.pins[at].held = Held::Alone;
                return Ok(false);
            }
            Err(error) => return Err(error),
        };
        if *chunk != *self.pins[at].strings {
            self.pins[at].held = Held::Alone;
            return Ok(false);
        }
        self.pins[at] = Pin {
            strings: chunk,
            held: Held::Chunk,
        };
        Ok(true)
    }
}

/// The dictionary ids `table`'s recorded text cells at live slots name, and its status's
/// reason; `text` holds its text columns.
fn each_text_id(
    table: &Table,
    text: &[usize],
    status: Status,
    mut visit: impl FnMut(u32) -> io::Result<()>,
) -> io::Result<()> {
    if let Status::Unavailable { reason } = status {
        visit(reason)?;
    }
    for &column in text {
        let Some(Column::Bytes(ids)) = table.column(column) else {
            continue;
        };
        for slot in table.live().ones() {
            if table.recorded(column, slot)
                && let Some(&id) = ids.get(slot as usize)
            {
                visit(id)?;
            }
        }
    }
    Ok(())
}

/// Every schema table while a branch is walked.
pub struct State {
    pub(crate) kinds: Vec<Vec<Kind>>,
    columns: Vec<usize>,
    /// Each table's text columns.
    text: Vec<Vec<usize>>,
    pub(crate) tables: Vec<Table>,
    pub(crate) changes: Vec<Changes>,
    pub(crate) statuses: Vec<Status>,
    /// Each table's sample hour.
    pub(crate) sampled: Vec<Option<i32>>,
    /// Each truncated table's rows read: its last hour's read and inserted slots.
    pub(crate) views: Vec<Vec<u32>>,
    /// Tables whose changes the last hour set, to clear before the next.
    pub(crate) present: Vec<usize>,
    /// Tables whose sections are not applied: no reader of the walk reads them.
    skipped: Vec<bool>,
    pub(crate) dictionary: Dictionary,
    /// The ids a dictionary trim keeps.
    named: Vec<u32>,
    scratch: Scratch,
}

impl State {
    /// `schema` holds each table's column kinds, in the recording's table order.
    pub fn new(schema: Vec<Vec<Kind>>) -> io::Result<Self> {
        let count = schema.len();
        let mut tables = engine_allocation::vec(count)?;
        let mut text = engine_allocation::vec(count)?;
        for kinds in &schema {
            tables.push(Table::new(kinds)?);
            text.push(engine_allocation::collect_vec(
                (0..kinds.len()).filter(|&column| kinds[column] == Kind::Bytes),
            )?);
        }
        Ok(Self {
            columns: engine_allocation::collect_vec(schema.iter().map(Vec::len))?,
            text,
            tables,
            changes: engine_allocation::collect_vec((0..count).map(|_| Changes::default()))?,
            statuses: engine_allocation::filled(count, Status::NotRecorded)?,
            sampled: engine_allocation::filled(count, None)?,
            views: engine_allocation::collect_vec((0..count).map(|_| Vec::new()))?,
            skipped: engine_allocation::filled(count, false)?,
            kinds: schema,
            present: Vec::new(),
            dictionary: Dictionary::default(),
            named: Vec::new(),
            scratch: Scratch::default(),
        })
    }

    /// Each table's column count, as a manifest is read with.
    pub fn columns(&self) -> &[usize] {
        &self.columns
    }

    /// Leaves the sections of the tables `skipped` marks unapplied from the next hour on:
    /// they keep their rows and record no changes. Their statuses still follow each hour.
    pub fn skip(&mut self, skipped: &[bool]) -> io::Result<()> {
        if skipped.len() != self.skipped.len() {
            return Err(invalid("A selection disagrees with the schema's tables"));
        }
        self.skipped.copy_from_slice(skipped);
        Ok(())
    }

    /// Whether `table`'s sections are left unapplied.
    pub fn skips(&self, table: usize) -> bool {
        self.skipped.get(table).copied().unwrap_or(false)
    }

    pub fn table(&self, table: usize) -> Option<&Table> {
        self.tables.get(table)
    }

    /// What `table` changed in the last hour applied; empty when it had no section.
    pub fn changes(&self, table: usize) -> Option<&Changes> {
        self.changes.get(table)
    }

    pub fn status(&self, table: usize) -> Option<Status> {
        self.statuses.get(table).copied()
    }

    /// The hour of the acquisition `table`'s view holds; none when that is not known.
    pub fn sampled_at(&self, table: usize) -> Option<i32> {
        self.sampled.get(table).copied().flatten()
    }

    /// The slots of the rows a truncated `table` read, ascending; None while it is not
    /// truncated. Its other live slots hold rows from earlier reads.
    pub fn truncated_view(&self, table: usize) -> Option<&[u32]> {
        matches!(self.statuses.get(table)?, Status::Truncated { .. })
            .then(|| self.views[table].as_slice())
    }

    pub fn dictionary(&self) -> &Dictionary {
        &self.dictionary
    }

    /// The bytes its tables and dictionary hold, chunks shared with other walkers included.
    pub fn held_bytes(&self) -> usize {
        self.tables.iter().map(Table::held_bytes).sum::<usize>()
            + self
                .views
                .iter()
                .map(|view| view.capacity() * size_of::<u32>())
                .sum::<usize>()
            + self.dictionary.held_bytes()
    }

    /// Walks on with `dictionary`, a run's ([`Dictionary::run`]).
    pub fn set_dictionary(&mut self, dictionary: Dictionary) {
        self.dictionary = dictionary;
    }

    /// Holds `strings`, a chunk of the run the dictionary is of, whole.
    pub fn pin_strings(&mut self, strings: Arc<Packed>) -> io::Result<()> {
        self.dictionary.pin(strings)
    }

    /// A restart the walk crosses: the strings appended before it can later be read back
    /// from its chunk.
    pub fn freeze_dictionary(&mut self) -> io::Result<()> {
        self.dictionary.freeze()
    }

    /// Once a run's dictionary holds more than its budget, drops the strings no recorded
    /// text cell at a live slot, nor the reason of a status, of a table the walk reads names.
    pub fn trim_dictionary(&mut self) -> io::Result<()> {
        if !self.dictionary.over() {
            return Ok(());
        }
        let Self {
            tables,
            text,
            statuses,
            skipped,
            named,
            dictionary,
            ..
        } = self;
        named.clear();
        for (at, table) in tables.iter().enumerate() {
            if skipped[at] {
                continue;
            }
            each_text_id(table, &text[at], statuses[at], |id| {
                engine_allocation::reserve(named, 1)?;
                named.push(id);
                Ok(())
            })?;
        }
        named.sort_unstable();
        named.dedup();
        dictionary.trim(named)
    }

    /// The dictionary ids `table`'s recorded text cells at live slots name, and its
    /// status's reason.
    pub fn text_ids(
        &self,
        table: usize,
        visit: impl FnMut(u32) -> io::Result<()>,
    ) -> io::Result<()> {
        let held = self
            .tables
            .get(table)
            .ok_or_else(|| invalid("Table past the schema"))?;
        each_text_id(held, &self.text[table], self.statuses[table], visit)
    }

    /// Holds every string `table`'s last changes name, and its status's reason: what a run's
    /// dictionary holds once a move other than an hour's changed the table.
    pub(crate) fn hold_text(&mut self, table: usize) -> io::Result<()> {
        if let Some(&Status::Unavailable { reason }) = self.statuses.get(table) {
            self.dictionary.hold(reason)?;
        }
        self.hold_changed_text(table)
    }

    /// Holds every string the last hour's changes to `table` name.
    fn hold_changed_text(&mut self, table: usize) -> io::Result<()> {
        let (held, changes) = (&self.tables[table], &self.changes[table]);
        let written = changes
            .masks
            .iter()
            .chain(&changes.fill_masks)
            .fold(0_u64, |bits, &mask| bits | mask);
        for &column in &self.text[table] {
            // Masks hold the first 64 columns; the others count as written.
            if changes.inserted.is_empty() && column < 64 && written >> column & 1 == 0 {
                continue;
            }
            let Some(Column::Bytes(ids)) = held.column(column) else {
                continue;
            };
            let wrote = |mask: u64| column >= 64 || mask >> column & 1 != 0;
            let changed = changes
                .changed
                .iter()
                .zip(&changes.masks)
                .filter(|(_, mask)| wrote(**mask))
                .map(|(&slot, _)| slot);
            let filled = changes
                .fill_slots
                .iter()
                .zip(&changes.fill_masks)
                .filter(|(_, mask)| wrote(**mask))
                .map(|(&slot, _)| slot);
            for slot in changed
                .chain(changes.inserted.iter().copied())
                .chain(filled)
            {
                if held.live().get(slot)
                    && held.recorded(column, slot)
                    && let Some(&id) = ids.get(slot as usize)
                {
                    self.dictionary.hold(id)?;
                }
            }
        }
        Ok(())
    }

    /// Replaces `table` with a restart's.
    pub fn restore(&mut self, table: usize, bytes: &[u8]) -> io::Result<Acquisition> {
        let kinds = self
            .kinds
            .get(table)
            .ok_or_else(|| invalid("Restart names a table past the schema"))?;
        let restored = restart::read(bytes, kinds, &mut self.scratch)?;
        self.tables[table] = restored.table;
        self.statuses[table] = restored.status;
        self.sampled[table] = restored.acquisition.last_read;
        self.views[table] = restored.view;
        self.changes[table] = Changes::default();
        Ok(restored.acquisition)
    }

    /// Replaces the dictionary with a restart's chunks, in order.
    pub fn restore_dictionary<'b>(
        &mut self,
        chunks: impl IntoIterator<Item = &'b [u8]>,
    ) -> io::Result<()> {
        self.dictionary = Dictionary::default();
        for chunk in chunks {
            self.dictionary.read_chunk(chunk)?;
        }
        Ok(())
    }

    /// Applies one hour: its appends, statuses and every present section of a table it does
    /// not skip, whose bytes `section(index)` gives for the manifest's `index`th section.
    /// A run's dictionary holds every string a table it does not skip names after it.
    pub fn apply<'s>(
        &mut self,
        manifest: &Manifest<'_>,
        mut section: impl FnMut(usize) -> io::Result<&'s [u8]>,
    ) -> io::Result<()> {
        if manifest.statuses.len() != self.tables.len() {
            return Err(invalid("Manifest disagrees with the schema's tables"));
        }
        for &table in &self.present {
            let changes = &mut self.changes[table];
            changes.truncated = false;
            changes.read.clear();
            changes.changed.clear();
            changes.deleted.clear();
            changes.inserted.clear();
            changes.touched.clear();
            changes.filled.clear();
            changes.fill_slots.clear();
            changes.masks.clear();
            changes.written.clear();
            changes.deleted_keys.clear();
            changes.fill_masks.clear();
        }
        self.present.clear();
        for append in &manifest.appends {
            self.dictionary.append(append)?;
        }
        if manifest.read.len() != self.tables.len().div_ceil(64) {
            return Err(invalid("Manifest disagrees with the schema's tables"));
        }
        self.statuses.copy_from_slice(&manifest.statuses);
        if self.dictionary.gaps {
            for (table, status) in self.statuses.iter().enumerate() {
                if let Status::Unavailable { reason } = *status
                    && !self.skipped[table]
                {
                    self.dictionary.hold(reason)?;
                }
            }
        }
        for (table, sampled) in self.sampled.iter_mut().enumerate() {
            if manifest.read[table / 64] >> (table % 64) & 1 != 0 {
                *sampled = Some(manifest.hour);
            }
        }
        for &(table, hour) in &manifest.samples {
            *self
                .sampled
                .get_mut(table)
                .ok_or_else(|| invalid("A sample names a table past the schema"))? = hour;
        }
        let mut sections = manifest.sections.iter().peekable();
        for (table, view) in self.views.iter_mut().enumerate() {
            let truncated = matches!(self.statuses[table], Status::Truncated { .. });
            let present = sections.next_if(|entry| entry.table == table).is_some();
            if truncated && !present {
                return Err(invalid("A partly read table names no rows it read"));
            }
            if !truncated {
                view.clear();
            }
        }
        for (index, entry) in manifest.sections.iter().enumerate() {
            let table = entry.table;
            if self.skipped[table] {
                continue;
            }
            let bytes = section(index)?;
            if bytes.len() as u64 != entry.bytes {
                return Err(invalid("Section length differs from its manifest's"));
            }
            let truncated = matches!(self.statuses[table], Status::Truncated { .. });
            section::apply(
                bytes,
                &self.kinds[table],
                truncated,
                &mut self.tables[table],
                &mut self.scratch,
                &mut self.changes[table],
            )?;
            if truncated {
                let changes = &self.changes[table];
                let live = self.tables[table].live();
                let view = &mut self.views[table];
                view.clear();
                engine_allocation::reserve(view, changes.read.len() + changes.inserted.len())?;
                // A read row the hour deleted is gone; a slot it deleted and filled again holds
                // the inserted row, once.
                view.extend(changes.read.iter().copied().filter(|&slot| live.get(slot)));
                view.extend_from_slice(&changes.inserted);
                view.sort_unstable();
                view.dedup();
            }
            engine_allocation::reserve(&mut self.present, 1)?;
            self.present.push(table);
            if self.dictionary.gaps {
                self.hold_changed_text(table)?;
            }
        }
        Ok(())
    }
}
