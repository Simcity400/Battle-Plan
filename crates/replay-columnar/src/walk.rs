//! A recording's current state while it is walked: every schema table, what each changed
//! in the last hour applied, and the string dictionary. Where an hour's bytes live is the
//! framing's business: an hour applies from its manifest and its sections' bytes.

use crate::{
    invalid,
    leaf::{Reader, Writer},
    manifest::{Manifest, Status},
    restart::{self, Acquisition, Last, Restored},
    section::{self, Changes, Scratch},
    state::{Column, Kind, Table},
};
use std::{
    collections::HashMap,
    fmt, io,
    sync::{Arc, Mutex, PoisonError},
};

/// Runs `job` for each index below `count`, on any threads, every one before it returns.
pub type Spread<'s> = dyn Fn(usize, &(dyn Fn(usize) + Sync)) + 's;

/// Runs each job on the calling thread, in turn.
fn serial(count: usize, job: &(dyn Fn(usize) + Sync)) {
    (0..count).for_each(job);
}

/// `job` of each index below `count`, run on `spread`, each in its place.
fn spread_each<T: Send>(
    spread: &Spread<'_>,
    count: usize,
    job: impl Fn(usize) -> T + Sync,
) -> io::Result<Vec<T>> {
    let cells = engine_allocation::collect_vec((0..count).map(|_| Mutex::new(None)))?;
    spread(count, &|at| {
        *cells[at].lock().unwrap_or_else(PoisonError::into_inner) = Some(job(at));
    });
    cells
        .into_iter()
        .map(|cell| {
            cell.into_inner()
                .unwrap_or_else(PoisonError::into_inner)
                .ok_or_else(|| invalid("A part of a spread never ran"))
        })
        .collect()
}

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
    /// A copy of its strings.
    fn copied(&self) -> io::Result<Self> {
        Ok(Self {
            first: self.first,
            bytes: engine_allocation::copy_vec(&self.bytes)?,
            ends: engine_allocation::copy_vec(&self.ends)?,
        })
    }

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
    fn copied(&self) -> io::Result<Self> {
        let mut ranges = HashMap::with_hasher(self.ranges.hasher().clone());
        engine_allocation::reserve_map(&mut ranges, self.ranges.len())?;
        ranges.extend(self.ranges.iter().map(|(&id, &range)| (id, range)));
        Ok(Self {
            bytes: engine_allocation::copy_vec(&self.bytes)?,
            ranges,
        })
    }

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
/// once it holds twice what it kept at its last trim it drops what no live cell names
/// ([`State::trim_dictionary`]) and reads a string back from its chunk when a cell names it
/// again.
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
    /// pinned, which reads what it dropped from `chunks`.
    pub fn run(chunks: Arc<dyn Chunks>, len: usize) -> Self {
        Self {
            log: Packed {
                first: len,
                ..Packed::default()
            },
            gaps: len != 0,
            chunks: Some(chunks),
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

    /// A copy holding the same strings at the same ids, its chunks and pins shared.
    pub fn share(&self) -> io::Result<Self> {
        let mut pins = engine_allocation::vec(self.pins.len())?;
        pins.extend(self.pins.iter().map(|pin| Pin {
            strings: Arc::clone(&pin.strings),
            held: pin.held,
        }));
        Ok(Self {
            log: self.log.copied()?,
            pins,
            sparse: self.sparse.copied()?,
            gaps: self.gaps,
            chunks: self.chunks.clone(),
            trimmed: self.trimmed,
        })
    }

    /// The bytes it holds of its own: its log and single strings, not the pins it shares.
    pub fn owned_bytes(&self) -> usize {
        self.log.held() + self.sparse.held() + self.pins.capacity() * size_of::<Pin>()
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

    /// Whether a run's dictionary holds more than twice what it held after its last trim.
    fn over(&self) -> bool {
        self.chunks.is_some() && self.held_bytes() / 2 > self.trimmed
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

/// What a state's tables' storage takes, counted: each table's, their sum, and the most that
/// sum reached since it was last taken ([`State::take_charge_peak`]).
struct Charged {
    tables: Vec<usize>,
    total: usize,
    peak: usize,
}

impl Charged {
    /// Replaces both charges before taking the peak, so a page moving between them never
    /// counts twice.
    fn set_both(&mut self, first: (usize, usize), second: (usize, usize)) {
        for (table, bytes) in [first, second] {
            self.total = self.total - self.tables[table] + bytes;
            self.tables[table] = bytes;
        }
        self.peak = self.peak.max(self.total);
    }
}

/// What one table's changes of an hour apply to.
/// The rows a truncated table's hour read: those its `changes` read that are still live in
/// `table`, and those it inserted.
fn view_read(view: &mut Vec<u32>, changes: &Changes, table: &Table) -> io::Result<()> {
    let live = table.live();
    view.clear();
    engine_allocation::reserve(view, changes.read.len() + changes.inserted.len())?;
    // A read row the hour deleted is gone; a slot it deleted and filled again holds the
    // inserted row, once.
    view.extend(changes.read.iter().copied().filter(|&slot| live.get(slot)));
    view.extend_from_slice(&changes.inserted);
    view.sort_unstable();
    view.dedup();
    Ok(())
}

/// Calls `hold` with every string `changes`, the last hour's to `held`, name in its text
/// columns `text`.
fn changed_text(
    held: &Table,
    changes: &Changes,
    text: &[usize],
    mut hold: impl FnMut(u32) -> io::Result<()>,
) -> io::Result<()> {
    let written = changes
        .masks
        .iter()
        .chain(&changes.fill_masks)
        .fold(0_u64, |bits, &mask| bits | mask);
    for &column in text {
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
                hold(id)?;
            }
        }
    }
    Ok(())
}

/// Clears the hour before's `changes`.
fn clear_hour(changes: &mut Changes) {
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

/// What a span's hours hold for every table ([`State::apply_span`]): by hour, then table.
struct SpanHours<'m> {
    manifests: &'m [Manifest<'m>],
    tables: usize,
    /// Each table's section's place among its hour's, or none.
    sections: Vec<u32>,
    sampled: Vec<Option<i32>>,
}

const NO_SECTION: u32 = u32::MAX;

/// One table's hours of a span, applied apart from the others' ([`State::apply_span`]): its
/// hours one after another, each as [`State::apply`] applies it.
pub struct SpanPart<'s> {
    table: usize,
    hours: &'s SpanHours<'s>,
    kinds: &'s [Kind],
    recorded: usize,
    keeps: bool,
    text: &'s [usize],
    gaps: bool,
    rows: &'s mut Table,
    changes: &'s mut Changes,
    last: &'s mut Option<Last>,
    view: &'s mut Vec<u32>,
    /// The strings its hours' changes name, held once the span is applied.
    named: Vec<u32>,
}

impl SpanPart<'_> {
    pub fn table(&self) -> usize {
        self.table
    }

    /// How many hours the span holds.
    pub fn hours(&self) -> usize {
        self.hours.manifests.len()
    }

    /// The table's status at the span's `hour`th hour.
    pub fn status(&self, hour: usize) -> Status {
        self.hours.manifests[hour].statuses[self.table]
    }

    /// The table's section of the span's `hour`th hour, if any.
    pub fn section(&self, hour: usize) -> Option<&crate::manifest::Section> {
        let at = self.hours.sections[hour * self.hours.tables + self.table];
        (at != NO_SECTION).then(|| &self.hours.manifests[hour].sections[at as usize])
    }

    /// The table's rows and changes as the hour applied last left them.
    pub fn applied(&self) -> (&Table, &Changes) {
        (self.rows, self.changes)
    }

    /// Applies the span's `hour`th hour, the hour after the one applied last, its section's
    /// changes by `changes` as [`Part::apply`] applies them.
    pub fn apply_hour(
        &mut self,
        hour: usize,
        changes: impl FnOnce(
            &crate::manifest::Section,
            &[Kind],
            bool,
            &mut Table,
            &mut Changes,
        ) -> io::Result<()>,
    ) -> io::Result<()> {
        let status = self.status(hour);
        let truncated = matches!(status, Status::Truncated { .. });
        clear_hour(self.changes);
        if !truncated {
            self.view.clear();
        }
        let hours = self.hours;
        let at = hours.sections[hour * hours.tables + self.table];
        let sectioned = at != NO_SECTION;
        if sectioned {
            let entry = &hours.manifests[hour].sections[at as usize];
            if self.keeps && matches!(self.last, Some(Last::Held(_))) {
                Last::before(self.last, self.rows, status)?;
            }
            changes(
                entry,
                &self.kinds[..self.recorded],
                truncated,
                self.rows,
                self.changes,
            )?;
            // The columns past the recording's hold no inserted row's cell.
            for column in self.recorded..self.kinds.len() {
                self.rows.unrecord(column, &self.changes.inserted)?;
            }
            if truncated {
                view_read(self.view, self.changes, self.rows)?;
            }
            if self.gaps {
                let named = &mut self.named;
                changed_text(self.rows, self.changes, self.text, |id| {
                    engine_allocation::reserve(named, 1)?;
                    named.push(id);
                    Ok(())
                })?;
            }
        }
        if self.keeps {
            let sampled = hours.sampled[hour * hours.tables + self.table];
            Last::after(self.last, status, sectioned, sampled);
        }
        Ok(())
    }
}

/// One table's changes of an hour, applied apart from the others' ([`State::apply_parts`]).
pub struct Part<'s> {
    index: usize,
    entry: &'s crate::manifest::Section,
    kinds: &'s [Kind],
    recorded: usize,
    truncated: bool,
    keeps: bool,
    status: Status,
    rows: &'s mut Table,
    changes: &'s mut Changes,
    last: &'s mut Option<Last>,
    view: &'s mut Vec<u32>,
}

impl Part<'_> {
    /// Its entry's place among the manifest's.
    pub fn index(&self) -> usize {
        self.index
    }

    pub fn table(&self) -> usize {
        self.entry.table
    }

    /// Applies its table's changes by `changes`, given its entry, the kinds of the columns the
    /// recording holds, whether the hour read the table in part, the table and what it changed;
    /// and what follows them, as [`State::apply`] does.
    pub fn apply(
        &mut self,
        changes: impl FnOnce(
            &crate::manifest::Section,
            &[Kind],
            bool,
            &mut Table,
            &mut Changes,
        ) -> io::Result<()>,
    ) -> io::Result<()> {
        if self.keeps && matches!(self.last, Some(Last::Held(_))) {
            Last::before(self.last, self.rows, self.status)?;
        }
        changes(
            self.entry,
            &self.kinds[..self.recorded],
            self.truncated,
            self.rows,
            self.changes,
        )?;
        // The columns past the recording's hold no inserted row's cell.
        for column in self.recorded..self.kinds.len() {
            self.rows.unrecord(column, &self.changes.inserted)?;
        }
        if self.truncated {
            view_read(self.view, self.changes, self.rows)?;
        }
        Ok(())
    }
}

struct Apply<'a> {
    kinds: &'a [Kind],
    truncated: bool,
    table: &'a mut Table,
    scratch: &'a mut Scratch,
    changes: &'a mut Changes,
}

/// Every schema table while a branch is walked.
pub struct State {
    pub(crate) kinds: Vec<Vec<Kind>>,
    /// Each table's columns the recording holds: the first of its schema's.
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
    /// Tables whose last complete read it keeps ([`State::keep_last`]), and each one's.
    keeps: Vec<bool>,
    last: Vec<Option<Last>>,
    /// Tables a reader of the walk is told it cannot read, each with why.
    withheld: Vec<Option<String>>,
    pub(crate) dictionary: Dictionary,
    /// The ids a dictionary trim keeps.
    named: Vec<u32>,
    scratch: Scratch,
    charged: Charged,
    /// Whether each table is counted again as an hour applies it ([`Self::charge_hours`]).
    charges: bool,
}

impl State {
    /// `schema` holds each table's column kinds, in the recording's table order.
    pub fn new(schema: Vec<Vec<Kind>>) -> io::Result<Self> {
        let columns = engine_allocation::collect_vec(schema.iter().map(Vec::len))?;
        Self::recorded(schema, columns)
    }

    /// The tables of `schema` read from a recording holding only the first `columns[table]` of
    /// each table's columns: the others hold every row's cell unrecorded.
    pub fn recorded(schema: Vec<Vec<Kind>>, columns: Vec<usize>) -> io::Result<Self> {
        if columns.len() != schema.len()
            || columns.iter().zip(&schema).any(|(&held, kinds)| held > kinds.len())
        {
            return Err(invalid("A recording's columns pass the schema's"));
        }
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
            columns,
            text,
            tables,
            changes: engine_allocation::collect_vec((0..count).map(|_| Changes::default()))?,
            statuses: engine_allocation::filled(count, Status::NotRecorded)?,
            sampled: engine_allocation::filled(count, None)?,
            views: engine_allocation::collect_vec((0..count).map(|_| Vec::new()))?,
            skipped: engine_allocation::filled(count, false)?,
            keeps: engine_allocation::filled(count, false)?,
            last: engine_allocation::collect_vec((0..count).map(|_| None))?,
            withheld: engine_allocation::collect_vec((0..count).map(|_| None))?,
            kinds: schema,
            present: Vec::new(),
            dictionary: Dictionary::default(),
            named: Vec::new(),
            scratch: Scratch::default(),
            // A table's last complete read held apart is counted at its index past the tables.
            charged: Charged {
                tables: engine_allocation::filled(2 * count, 0)?,
                total: 0,
                peak: 0,
            },
            charges: true,
        })
    }

    /// What its tables' storage takes that no other state shares, as last counted: each table
    /// is counted again as an hour applies it, or all at once ([`Self::recharge`]).
    pub fn charged_bytes(&self) -> usize {
        self.charged.total
    }

    /// Counts what each table holds as an hour applies it ([`Self::charged_bytes`]), or stops,
    /// for a walk no reader asks: counted all again once started.
    pub fn charge_hours(&mut self, charges: bool) -> io::Result<()> {
        let started = charges && !self.charges;
        self.charges = charges;
        if started {
            self.recharge()?;
        }
        Ok(())
    }

    /// Counts again what every table holds alone: after a share, both hold none of the pages.
    pub fn recharge(&mut self) -> io::Result<()> {
        self.recharge_spread(&serial)
    }

    /// [`Self::recharge`], its tables counted on `spread`.
    pub fn recharge_spread(&mut self, spread: &Spread<'_>) -> io::Result<()> {
        if !self.charges {
            return Ok(());
        }
        let count = self.tables.len();
        let this = &*self;
        let charges = spread_each(spread, count, |table| this.charges_of(table))?;
        for (table, charges) in charges.into_iter().enumerate() {
            self.set_charges(table, charges);
        }
        self.charged.peak = self.charged.total;
        Ok(())
    }

    /// The most its tables' storage took at once since this was last taken, or it was made.
    pub fn take_charge_peak(&mut self) -> usize {
        std::mem::replace(&mut self.charged.peak, self.charged.total)
    }

    /// Counts none of what its tables take now in its peak, for tables about to be replaced.
    pub fn forget_charge_peak(&mut self) {
        self.charged.peak = 0;
    }

    /// What `table`'s last complete read held apart holds alone and the pages only it and the
    /// table hold, which neither's own count holds; none when it holds none.
    fn apart_bytes(&self, table: usize) -> usize {
        match &self.last[table] {
            Some(Last::Apart(apart, _)) => {
                apart.owned_bytes() + apart.shared_bytes(&self.tables[table])
            }
            _ => 0,
        }
    }

    /// What `table` and its last complete read held apart hold alone, each page once.
    fn charges_of(&self, table: usize) -> [usize; 2] {
        [self.tables[table].owned_bytes(), self.apart_bytes(table)]
    }

    fn set_charges(&mut self, table: usize, [live, apart]: [usize; 2]) {
        let at = self.tables.len() + table;
        self.charged.set_both((table, live), (at, apart));
    }

    /// Counts what `table` and its last complete read held apart hold, each page once.
    fn charge_both(&mut self, table: usize) -> io::Result<()> {
        if self.charges {
            let charges = self.charges_of(table);
            self.set_charges(table, charges);
        }
        Ok(())
    }

    /// Each table's column count the recording holds, as a manifest is read with.
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

    /// Keeps the last complete read of the tables `keeps` marks, as their writers keep it for
    /// restarts, from the next restore or hour on.
    pub fn keep_last(&mut self, keeps: &[bool]) -> io::Result<()> {
        if keeps.len() != self.keeps.len() {
            return Err(invalid("A selection disagrees with the schema's tables"));
        }
        self.keeps.copy_from_slice(keeps);
        let count = self.tables.len();
        for at in 0..count {
            if !keeps[at] && self.last[at].take().is_some() {
                self.charge_both(at)?;
            }
        }
        Ok(())
    }

    /// `table`'s last complete read, when it keeps one: its rows (the table's own while no hour
    /// since changed them) and the hour of the read when known. A walk started at a restart
    /// knows it without the hours before.
    pub fn last_complete(&self, table: usize) -> Option<(&Table, Option<i32>)> {
        if self.skips(table) || !self.keeps.get(table).copied().unwrap_or(false) {
            return None;
        }
        match self.last.get(table)?.as_ref()? {
            Last::Held(read) => Some((&self.tables[table], *read)),
            Last::Apart(apart, read) => Some((&**apart, *read)),
        }
    }

    /// Tells readers of the walk they cannot read `table`, and why; none clears it. What the
    /// walk applies of the table is not changed: a walk withholding it also skips it.
    pub fn withhold(&mut self, table: usize, reason: Option<&str>) -> io::Result<()> {
        let held = self
            .withheld
            .get_mut(table)
            .ok_or_else(|| invalid("A withheld table is past the schema"))?;
        *held = reason.map(engine_allocation::string).transpose()?;
        Ok(())
    }

    /// Why readers of the walk cannot read `table`; none when they can.
    pub fn withheld(&self, table: usize) -> Option<&str> {
        self.withheld.get(table)?.as_deref()
    }

    /// `table`'s last complete read as a restart carries it, when it keeps one.
    pub fn last(&self, table: usize) -> Option<&Last> {
        if self.skips(table) || !self.keeps.get(table).copied().unwrap_or(false) {
            return None;
        }
        self.last.get(table)?.as_ref()
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

    /// A copy whose tables share their pages with these, copied before either writes one: the
    /// walk as it stands after its last hour, that hour's changes aside.
    pub fn share(&self) -> io::Result<Self> {
        self.share_spread(&serial)
    }

    /// [`Self::share`], its tables shared and counted on `spread`.
    pub fn share_spread(&self, spread: &Spread<'_>) -> io::Result<Self> {
        let count = self.tables.len();
        let parts = spread_each(spread, count, |at| -> io::Result<(Table, Option<Last>)> {
            Ok((
                self.tables[at].share()?,
                self.last[at].as_ref().map(Last::share).transpose()?,
            ))
        })?;
        let mut kinds = engine_allocation::vec(count)?;
        let mut text = engine_allocation::vec(count)?;
        let mut tables = engine_allocation::vec(count)?;
        let mut views = engine_allocation::vec(count)?;
        let mut last = engine_allocation::vec(count)?;
        let mut withheld = engine_allocation::vec(count)?;
        for (at, part) in parts.into_iter().enumerate() {
            let (table, kept) = part?;
            kinds.push(engine_allocation::copy_vec(&self.kinds[at])?);
            text.push(engine_allocation::copy_vec(&self.text[at])?);
            tables.push(table);
            views.push(engine_allocation::copy_vec(&self.views[at])?);
            last.push(kept);
            withheld.push(self.withheld[at].as_deref().map(engine_allocation::string).transpose()?);
        }
        let mut shared = Self {
            kinds,
            columns: engine_allocation::copy_vec(&self.columns)?,
            text,
            tables,
            changes: engine_allocation::collect_vec((0..count).map(|_| Changes::default()))?,
            statuses: engine_allocation::copy_vec(&self.statuses)?,
            sampled: engine_allocation::copy_vec(&self.sampled)?,
            views,
            present: Vec::new(),
            skipped: engine_allocation::copy_vec(&self.skipped)?,
            keeps: engine_allocation::copy_vec(&self.keeps)?,
            last,
            withheld,
            dictionary: self.dictionary.share()?,
            named: Vec::new(),
            scratch: Scratch::default(),
            charged: Charged {
                tables: engine_allocation::copy_vec(&self.charged.tables)?,
                total: self.charged.total,
                peak: self.charged.total,
            },
            charges: self.charges,
        };
        shared.recharge_spread(spread)?;
        Ok(shared)
    }

    /// The bytes it holds apart from `other`, a state shared from it or it from: its tables'
    /// pages not at the same place in `other`, its views and its dictionary's own strings;
    /// counted table by table on `spread`.
    pub fn unshared_bytes(&self, other: &Self, spread: &Spread<'_>) -> io::Result<usize> {
        let apart = |mine: &Table, theirs: Option<&Table>| {
            mine.held_bytes() - theirs.map_or(0, |theirs| mine.common_bytes(theirs))
        };
        let count = self.tables.len().min(other.tables.len());
        let tables = spread_each(spread, count, |at| {
            apart(&self.tables[at], Some(&other.tables[at]))
                + match (&self.last[at], &other.last[at]) {
                    (Some(Last::Apart(mine, _)), Some(Last::Apart(theirs, _))) => {
                        apart(mine, Some(theirs))
                    }
                    (Some(Last::Apart(mine, _)), _) => apart(mine, None),
                    _ => 0,
                }
        })?;
        Ok(tables.into_iter().sum::<usize>()
            + self
                .views
                .iter()
                .map(|view| view.capacity() * size_of::<u32>())
                .sum::<usize>()
            + self.dictionary.owned_bytes())
    }

    /// The bytes its tables and dictionary hold, chunks shared with other walkers included.
    pub fn held_bytes(&self) -> usize {
        self.tables.iter().map(Table::held_bytes).sum::<usize>()
            + self.last.iter().flatten().map(Last::held_bytes).sum::<usize>()
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

    /// Appends `strings` to the dictionary, as the hours that first used them would.
    pub fn append_strings(&mut self, strings: &[Box<[u8]>]) -> io::Result<()> {
        strings.iter().try_for_each(|text| self.dictionary.append(text))
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

    /// Once a run's dictionary has doubled since its last trim, drops the strings no recorded
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
            last,
            named,
            dictionary,
            ..
        } = self;
        named.clear();
        for (at, table) in tables.iter().enumerate() {
            if skipped[at] {
                continue;
            }
            let mut name = |id: u32| -> io::Result<()> {
                engine_allocation::reserve(named, 1)?;
                named.push(id);
                Ok(())
            };
            each_text_id(table, &text[at], statuses[at], &mut name)?;
            if let Some(Last::Apart(apart, _)) = &last[at] {
                each_text_id(apart, &text[at], Status::Complete, &mut name)?;
            }
        }
        named.sort_unstable();
        named.dedup();
        dictionary.trim(named)
    }

    /// The dictionary ids `table`'s recorded text cells at live slots name, and its
    /// status's reason.
    /// A last complete read held apart names its own.
    pub fn text_ids(
        &self,
        table: usize,
        mut visit: impl FnMut(u32) -> io::Result<()>,
    ) -> io::Result<()> {
        let held = self
            .tables
            .get(table)
            .ok_or_else(|| invalid("Table past the schema"))?;
        each_text_id(held, &self.text[table], self.statuses[table], &mut visit)?;
        if let Some(Last::Apart(apart, _)) = &self.last[table] {
            each_text_id(apart, &self.text[table], Status::Complete, &mut visit)?;
        }
        Ok(())
    }

    /// Holds every string the last hour's changes to `table` name.
    fn hold_changed_text(&mut self, table: usize) -> io::Result<()> {
        let Self {
            tables,
            changes,
            text,
            dictionary,
            ..
        } = self;
        changed_text(&tables[table], &changes[table], &text[table], |id| {
            dictionary.hold(id)
        })
    }

    /// Replaces `table` with a restart's.
    pub fn restore(&mut self, table: usize, bytes: &[u8]) -> io::Result<Acquisition> {
        if table >= self.kinds.len() {
            return Err(invalid("Restart names a table past the schema"));
        }
        // The table it replaces goes first.
        self.tables[table] = Table::new(&self.kinds[table])?;
        self.last[table] = None;
        self.charge_both(table)?;
        let restored = self.decode(table, bytes)?;
        Ok(self.take(table, restored))
    }

    /// `table` as a restart's `bytes` hold it, for [`Self::restore_restored`]: the columns past
    /// the recording's hold every row's cell unrecorded.
    pub fn decode(&mut self, table: usize, bytes: &[u8]) -> io::Result<Restored> {
        let mut scratch = std::mem::take(&mut self.scratch);
        let restored = self.decode_with(table, bytes, &mut scratch);
        self.scratch = scratch;
        restored
    }

    /// [`Self::decode`] with `scratch` of the caller's, so tables decode apart.
    pub fn decode_with(
        &self,
        table: usize,
        bytes: &[u8],
        scratch: &mut Scratch,
    ) -> io::Result<Restored> {
        let kinds = self
            .kinds
            .get(table)
            .ok_or_else(|| invalid("Restart names a table past the schema"))?;
        let (held, appended) = kinds.split_at(self.columns[table]);
        let mut restored = restart::read(bytes, held, scratch)?;
        restored.table.widen(appended)?;
        if let Some(Last::Apart(apart, _)) = &mut restored.last {
            apart.widen(appended)?;
        }
        Ok(restored)
    }

    /// Replaces `table` with `restored`, a restart's: one decoded once for every walk starting
    /// there shares its pages with them, each copying only those its hours write.
    pub fn restore_restored(
        &mut self,
        table: usize,
        restored: Restored,
    ) -> io::Result<Acquisition> {
        if table >= self.kinds.len() {
            return Err(invalid("Restart names a table past the schema"));
        }
        if !restored.table.kinds().eq(self.kinds[table].iter().copied()) {
            return Err(invalid("Restart table's columns differ from its schema"));
        }
        Ok(self.take(table, restored))
    }

    fn take(&mut self, table: usize, restored: Restored) -> Acquisition {
        self.tables[table] = restored.table;
        self.statuses[table] = restored.status;
        self.sampled[table] = restored.acquisition.last_read;
        self.views[table] = restored.view;
        self.changes[table] = Changes::default();
        self.last[table] = if self.keeps[table] {
            restored.last
        } else {
            None
        };
        // Counting what the tables hold never fails.
        let _ = self.charge_both(table);
        restored.acquisition
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
        self.apply_with(manifest, |index, entry, apply| {
            let bytes = section(index)?;
            if bytes.len() as u64 != entry.bytes {
                return Err(invalid("Section length differs from its manifest's"));
            }
            section::apply(bytes, apply.kinds, apply.truncated, apply.table, apply.scratch, apply.changes)
        })
    }

    /// Applies one hour of a block: its record, and each present table it does not skip from
    /// that table's block decoder, which `decoders` holds by table.
    pub fn apply_block(
        &mut self,
        manifest: &Manifest<'_>,
        decoders: &mut [Option<crate::block::TableDecoder<'_>>],
        mut lost: Option<&mut Vec<(usize, io::Error)>>,
    ) -> io::Result<()> {
        self.apply_with(manifest, |_, entry, apply| {
            let decoder = decoders
                .get_mut(entry.table)
                .and_then(Option::as_mut)
                .ok_or_else(|| invalid("An hour names a table its block does not hold"))?;
            let applied = decoder.hour(apply.table, apply.changes, entry, apply.truncated);
            match (applied, lost.as_deref_mut()) {
                // Damage of the table's own block loses it; any other failure is the walk's.
                (Err(error), Some(lost)) if error.kind() == io::ErrorKind::InvalidData => {
                    decoders[entry.table] = None;
                    engine_allocation::reserve(lost, 1)?;
                    lost.push((entry.table, error));
                    Ok(())
                }
                (applied, _) => applied,
            }
        })
    }

    /// [`Self::apply`], each present section of a table it does not skip applied as a
    /// [`Part`] by `run`, which applies every part once, on any threads.
    pub fn apply_parts(
        &mut self,
        manifest: &Manifest<'_>,
        run: impl FnOnce(&mut [Part<'_>]) -> io::Result<()>,
    ) -> io::Result<()> {
        self.begin_hour(manifest)?;
        {
            let Self {
                kinds,
                columns,
                tables,
                changes,
                statuses,
                views,
                skipped,
                keeps,
                last,
                ..
            } = self;
            let mut parts = engine_allocation::vec(manifest.sections.len())?;
            let mut sections = manifest.sections.iter().enumerate().peekable();
            let held = tables.iter_mut().zip(changes.iter_mut());
            let held = held.zip(last.iter_mut()).zip(views.iter_mut());
            for (table, (((rows, changes), last), view)) in held.enumerate() {
                let Some((index, entry)) = sections.next_if(|(_, entry)| entry.table == table)
                else {
                    continue;
                };
                if skipped[table] {
                    continue;
                }
                parts.push(Part {
                    index,
                    entry,
                    kinds: &kinds[table],
                    recorded: columns[table],
                    truncated: matches!(statuses[table], Status::Truncated { .. }),
                    keeps: keeps[table],
                    status: statuses[table],
                    rows,
                    changes,
                    last,
                    view,
                });
            }
            if sections.next().is_some() {
                return Err(invalid("A manifest's sections are not in table order"));
            }
            run(&mut parts)?;
        }
        for entry in &manifest.sections {
            if !self.skipped[entry.table] {
                self.applied(entry.table)?;
            }
        }
        self.end_hour()
    }

    /// Applies `manifests`' hours, one after another, each table's hours by `run` as one
    /// [`SpanPart`] apart from the others': as [`Self::apply_parts`] applies each hour, with the
    /// table-wide part of every hour (its appends, statuses and samples) taken first. Each
    /// manifest's sections are in table order.
    pub fn apply_span(
        &mut self,
        manifests: &[Manifest<'_>],
        run: impl FnOnce(&mut [SpanPart<'_>]) -> io::Result<()>,
    ) -> io::Result<()> {
        let count = self.tables.len();
        let hours = manifests.len();
        if hours == 0 {
            return Ok(());
        }
        let mut sections = engine_allocation::filled(hours * count, NO_SECTION)?;
        let mut sampled = engine_allocation::vec(hours * count)?;
        let mut held = engine_allocation::copy_vec(&self.sampled)?;
        for (hour, manifest) in manifests.iter().enumerate() {
            if manifest.statuses.len() != count || manifest.read.len() != count.div_ceil(64) {
                return Err(invalid("Manifest disagrees with the schema's tables"));
            }
            let row = &mut sections[hour * count..(hour + 1) * count];
            let mut previous = None;
            for (at, entry) in manifest.sections.iter().enumerate() {
                if entry.table >= count {
                    return Err(invalid("A section names a table past the schema"));
                }
                if previous.is_some_and(|previous| previous >= entry.table) {
                    return Err(invalid("A manifest's sections are not in table order"));
                }
                previous = Some(entry.table);
                row[entry.table] = at as u32;
            }
            for table in 0..count {
                let truncated = matches!(manifest.statuses[table], Status::Truncated { .. });
                if truncated && row[table] == NO_SECTION {
                    return Err(invalid("A partly read table names no rows it read"));
                }
            }
            for append in &manifest.appends {
                self.dictionary.append(append)?;
            }
            if self.dictionary.gaps {
                for (table, status) in manifest.statuses.iter().enumerate() {
                    if let Status::Unavailable { reason } = *status
                        && !self.skipped[table]
                    {
                        self.dictionary.hold(reason)?;
                    }
                }
            }
            for (table, sample) in held.iter_mut().enumerate() {
                if manifest.read[table / 64] >> (table % 64) & 1 != 0 {
                    *sample = Some(manifest.hour);
                }
            }
            for &(table, hour) in &manifest.samples {
                *held
                    .get_mut(table)
                    .ok_or_else(|| invalid("A sample names a table past the schema"))? = hour;
            }
            sampled.extend_from_slice(&held);
        }
        let span = SpanHours {
            manifests,
            tables: count,
            sections,
            sampled,
        };
        let gaps = self.dictionary.gaps;
        let mut named = Vec::new();
        {
            let Self {
                kinds,
                columns,
                text,
                tables,
                changes,
                views,
                skipped,
                keeps,
                last,
                ..
            } = self;
            let mut parts = engine_allocation::vec(count)?;
            let held = tables.iter_mut().zip(changes.iter_mut());
            let held = held.zip(last.iter_mut()).zip(views.iter_mut());
            for (table, (((rows, changes), last), view)) in held.enumerate() {
                if skipped[table] {
                    continue;
                }
                parts.push(SpanPart {
                    table,
                    hours: &span,
                    kinds: &kinds[table],
                    recorded: columns[table],
                    keeps: keeps[table],
                    text: &text[table],
                    gaps,
                    rows,
                    changes,
                    last,
                    view,
                    named: Vec::new(),
                });
            }
            run(&mut parts)?;
            for part in &mut parts {
                if gaps {
                    engine_allocation::reserve(&mut named, part.named.len())?;
                    named.append(&mut part.named);
                }
            }
        }
        for id in named {
            self.dictionary.hold(id)?;
        }
        // A skipped table's rows read are kept only while it is partly read.
        for (table, view) in self.views.iter_mut().enumerate() {
            if self.skipped[table]
                && !matches!(manifests[hours - 1].statuses[table], Status::Truncated { .. })
            {
                view.clear();
            }
        }
        let last = &manifests[hours - 1];
        self.statuses.copy_from_slice(&last.statuses);
        self.sampled.copy_from_slice(&span.sampled[(hours - 1) * count..]);
        self.present.clear();
        engine_allocation::reserve(&mut self.present, last.sections.len())?;
        self.present.extend(
            last.sections
                .iter()
                .map(|entry| entry.table)
                .filter(|&table| !self.skipped[table]),
        );
        if self.charges {
            self.recharge()?;
        }
        Ok(())
    }

    /// [`Self::apply`], each present table's changes applied by `section` with the manifest's
    /// index of its entry, the entry, and the table it changes.
    fn apply_with(
        &mut self,
        manifest: &Manifest<'_>,
        mut section: impl FnMut(usize, &crate::manifest::Section, Apply<'_>) -> io::Result<()>,
    ) -> io::Result<()> {
        self.begin_hour(manifest)?;
        for (index, entry) in manifest.sections.iter().enumerate() {
            let table = entry.table;
            if self.skipped[table] {
                continue;
            }
            let truncated = matches!(self.statuses[table], Status::Truncated { .. });
            let recorded = self.columns[table];
            if self.keeps[table] && matches!(self.last[table], Some(Last::Held(_))) {
                Last::before(
                    &mut self.last[table],
                    &self.tables[table],
                    self.statuses[table],
                )?;
                if matches!(self.last[table], Some(Last::Apart(..))) {
                    self.charge_both(table)?;
                }
            }
            section(
                index,
                entry,
                Apply {
                    kinds: &self.kinds[table][..recorded],
                    truncated,
                    table: &mut self.tables[table],
                    scratch: &mut self.scratch,
                    changes: &mut self.changes[table],
                },
            )?;
            // The columns past the recording's hold no inserted row's cell.
            for column in recorded..self.kinds[table].len() {
                self.tables[table].unrecord(column, &self.changes[table].inserted)?;
            }
            if truncated {
                view_read(
                    &mut self.views[table],
                    &self.changes[table],
                    &self.tables[table],
                )?;
            }
            self.applied(table)?;
        }
        self.end_hour()
    }

    /// Counts what `table` holds once the hour applied its section, and keeps what its changes
    /// name.
    fn applied(&mut self, table: usize) -> io::Result<()> {
        self.charge_both(table)?;
        engine_allocation::reserve(&mut self.present, 1)?;
        self.present.push(table);
        if self.dictionary.gaps {
            self.hold_changed_text(table)?;
        }
        Ok(())
    }

    /// An hour's start: last hour's changes cleared, its appends, statuses and samples taken.
    fn begin_hour(&mut self, manifest: &Manifest<'_>) -> io::Result<()> {
        if manifest.statuses.len() != self.tables.len() {
            return Err(invalid("Manifest disagrees with the schema's tables"));
        }
        for &table in &self.present {
            clear_hour(&mut self.changes[table]);
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
        Ok(())
    }

    /// An hour's end: each kept last complete read follows what the hour read.
    fn end_hour(&mut self) -> io::Result<()> {
        let count = self.tables.len();
        for table in 0..count {
            if !self.keeps[table] || self.skipped[table] {
                continue;
            }
            let apart = matches!(self.last[table], Some(Last::Apart(..)));
            let sectioned = self.present.contains(&table);
            Last::after(
                &mut self.last[table],
                self.statuses[table],
                sectioned,
                self.sampled[table],
            );
            if apart && !matches!(self.last[table], Some(Last::Apart(..))) {
                self.charge_both(table)?;
            }
        }
        Ok(())
    }
}
