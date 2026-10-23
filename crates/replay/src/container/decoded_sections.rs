//! Retain live reader windows and up to one measured block of obsolete columns.
use super::{BlockIndex, Replay};
use crate::{
    frame::{TABLE_COUNT, TableUpdate, recorded::Updates},
    rows::{TableId, compatibility::HISTORY},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::{Arc, Condvar, Mutex, MutexGuard},
};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Key {
    format: u16,
    branch: u64,
    first: i32,
    last: i32,
    offset: u64,
    length: u64,
    raw_length: u64,
    table: usize,
    block: usize,
}

impl Key {
    pub(super) fn new(replay: &Replay, index: &BlockIndex, table: TableId) -> Self {
        Self {
            format: replay.format.version(),
            branch: index.branch,
            first: index.first_hour,
            last: index.last_hour,
            offset: index.offset,
            length: index.length,
            raw_length: index.raw_length,
            table: table.index(),
            block: replay.block_position(index).unwrap_or(0),
        }
    }

    /// Lower bound for all keys of a table and block in the retained index.
    const FLOOR: Self = Self {
        format: 0,
        branch: 0,
        first: i32::MIN,
        last: i32::MIN,
        offset: 0,
        length: 0,
        raw_length: 0,
        table: 0,
        block: 0,
    };
}

struct Column {
    updates: Updates,
    bytes: usize,
    table_read: bool,
}
#[derive(Default)]
struct Entry {
    value: Option<Arc<Column>>,
    reading: bool,
    priority: i32,
    used: u64,
    /// Obsolete columns have their own LRU order and one-block allowance.
    obsolete: bool,
}
/// A registered reader: the blocks it still reads and the tables it reads.
struct Frontier {
    next: usize,
    ahead: usize,
    /// Some means this reader needs only its current block.
    last: Option<usize>,
    /// None reads every table; declarations include decoded dependencies.
    tables: Option<Box<[bool; TABLE_COUNT]>>,
    /// Reader identity for acceptance accounting.
    name: &'static str,
}
impl Frontier {
    fn range(&self) -> (usize, usize) {
        let last = self.last.unwrap_or_else(|| self.ahead.max(self.next));
        (self.next, last.saturating_add(1))
    }
    fn reads(&self, table: usize) -> bool {
        self.tables.as_ref().is_none_or(|tables| tables[table])
    }
}
/// One locked accounting snapshot, not unique process heap usage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SectionCacheCensus {
    /// The retained charge and its budget.
    pub bytes: usize,
    pub budget: usize,
    /// Least next block and furthest scheduled block; None without readers.
    pub frontier: Option<(usize, usize)>,
    pub readers: usize,
    /// Table registry positions; largest is the measured per-table charge.
    pub largest: [usize; TABLE_COUNT],
    pub needed: [usize; TABLE_COUNT],
    pub obsolete: [usize; TABLE_COUNT],
    /// Retained charge before, within and after an inclusive walk window.
    pub behind: [usize; TABLE_COUNT],
    pub window: [usize; TABLE_COUNT],
    pub beyond: [usize; TABLE_COUNT],
    /// One reader at the least next block for each table; readers can overlap.
    pub retaining: [Option<&'static str>; TABLE_COUNT],
}
impl SectionCacheCensus {
    const EMPTY: Self = Self {
        bytes: 0,
        budget: 0,
        frontier: None,
        readers: 0,
        largest: [0; TABLE_COUNT],
        needed: [0; TABLE_COUNT],
        obsolete: [0; TABLE_COUNT],
        behind: [0; TABLE_COUNT],
        window: [0; TABLE_COUNT],
        beyond: [0; TABLE_COUNT],
        retaining: [None; TABLE_COUNT],
    };
}
/// Account retained values only; in-flight empty entries have no column charge.
fn classify<'e>(
    entries: impl IntoIterator<Item = (&'e Key, &'e Entry)>,
    window: Option<(usize, usize)>,
    census: &mut SectionCacheCensus,
) {
    for (key, entry) in entries {
        let Some(value) = &entry.value else {
            continue;
        };
        if entry.obsolete {
            census.obsolete[key.table] += value.bytes;
        } else {
            census.needed[key.table] += value.bytes;
        }
        match window {
            Some((from, _)) if key.block < from => census.behind[key.table] += value.bytes,
            Some((_, to)) if key.block <= to => census.window[key.table] += value.bytes,
            Some(_) => census.beyond[key.table] += value.bytes,
            None => {}
        }
    }
}
/// Grouped reads also decode Campaign, transitive alias sources and catalogued layout gaps.
fn widen(version: u16, tables: &[TableId]) -> Box<[bool; TABLE_COUNT]> {
    let mut read = Box::new([false; TABLE_COUNT]);
    let mut pending: Vec<TableId> = tables.to_vec();
    pending.push(TableId::Campaign);
    while let Some(table) = pending.pop() {
        if std::mem::replace(&mut read[table.index()], true) {
            continue;
        }
        if super::aliases::private::stored(table, version) {
            pending.extend_from_slice(super::aliases::private::dependencies(table));
        }
    }
    for table in TableId::ALL {
        let change = &HISTORY[table.index()];
        if change.present(version) && change.table_gap(table, version).is_some() {
            read[table.index()] = true;
        }
    }
    read
}
/// A completed cache miss; one admission can encompass several wire decodes.
#[cfg(any(test, feature = "test-support"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SectionDecode {
    pub block: usize,
    pub table: usize,
    pub hours: usize,
    pub keyframe_only: bool,
    pub returned: bool,
}
/// A wire-decode attempt, including retries and independent group verification.
#[cfg(any(test, feature = "test-support"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WireDecode {
    pub block: usize,
    pub table: usize,
    pub hours: usize,
    pub route: WireRoute,
}
/// How a [`WireDecode`] reads its column.
#[cfg(any(test, feature = "test-support"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WireRoute {
    /// The keyframe entry alone, for a keyframe read.
    Keyframe,
    /// Every hour read, on the direct route.
    Direct,
    /// Every hour read, on the original route.
    Original,
}
#[derive(Default)]
struct State {
    entries: BTreeMap<Key, Entry>,
    /// Needed and obsolete columns have separate LRU orders.
    lru: BTreeSet<(u64, Key)>,
    obsolete: BTreeSet<(u64, Key)>,
    obsolete_bytes: usize,
    obsolete_tables: BTreeMap<usize, usize>,
    /// Each retained column by table and block.
    retained: BTreeSet<(usize, usize, Key)>,
    largest: BTreeMap<usize, usize>,
    /// The sum of `largest`: one block's set of columns.
    allowance: usize,
    consumers: BTreeMap<u64, Frontier>,
    next_consumer: u64,
    enabled: bool,
    bytes: usize,
    clock: u64,
    #[cfg(any(test, feature = "test-support"))]
    decodes: Option<Vec<SectionDecode>>,
    #[cfg(test)]
    inspections: usize,
    #[cfg(test)]
    waiters: usize,
}
/// A reader's blocks (half-open) and tables, before or after it changes.
type Span = ((usize, usize), Option<Box<[bool; TABLE_COUNT]>>);
impl State {
    fn frontier(&self) -> Option<(usize, usize)> {
        let first = self.consumers.values().map(|reader| reader.next).min()?;
        let last = self
            .consumers
            .values()
            .map(|reader| reader.ahead)
            .max()
            .unwrap_or(first);
        Some((first, last))
    }
    fn span(&self) -> usize {
        self.frontier().map_or(1, |(first, last)| {
            last.saturating_sub(first).saturating_add(1)
        })
    }
    fn budget(&self) -> usize {
        if !self.enabled {
            return 0;
        }
        self.allowance.saturating_mul(self.span())
    }
    /// What columns no reader needs may keep: one block's set of columns.
    fn obsolete_allowance(&self) -> usize {
        if self.enabled { self.allowance } else { 0 }
    }
    fn needed(&self, table: usize, block: usize) -> bool {
        self.consumers.values().any(|reader| {
            let (from, to) = reader.range();
            reader.reads(table) && from <= block && block < to
        })
    }
    /// Reclassify only the tables and blocks changed by a reader transition.
    fn moved(&mut self, before: Option<Span>, after: Option<Span>) {
        let empty = || ((0, 0), None);
        let (before, after) = (before.unwrap_or_else(empty), after.unwrap_or_else(empty));
        let reads = |span: &Span, table: usize| span.1.as_ref().is_none_or(|tables| tables[table]);
        for table in 0..TABLE_COUNT {
            let old = if reads(&before, table) {
                before.0
            } else {
                (0, 0)
            };
            let new = if reads(&after, table) {
                after.0
            } else {
                (0, 0)
            };
            if old == new {
                continue;
            }
            for (from, to) in difference(old, new) {
                if from >= to {
                    continue;
                }
                if old.0 <= from && to <= old.1 {
                    self.shrink(table, from, to);
                } else {
                    self.grow(table, from, to);
                }
            }
        }
    }
    /// Only columns losing their last reader become obsolete.
    fn shrink(&mut self, table: usize, from: usize, to: usize) {
        let keys: Vec<Key> = self
            .retained
            .range((table, from, Key::FLOOR)..(table, to, Key::FLOOR))
            .map(|&(_, _, key)| key)
            .collect();
        for key in keys {
            #[cfg(test)]
            {
                self.inspections += 1;
            }
            if !self.needed(table, key.block) {
                self.mark(key, true);
            }
        }
    }
    /// Retained obsolete columns can become needed again.
    fn grow(&mut self, table: usize, from: usize, to: usize) {
        if self
            .obsolete_tables
            .get(&table)
            .is_none_or(|&count| count == 0)
        {
            return;
        }
        let keys: Vec<Key> = self
            .retained
            .range((table, from, Key::FLOOR)..(table, to, Key::FLOOR))
            .map(|&(_, _, key)| key)
            .collect();
        for key in keys {
            #[cfg(test)]
            {
                self.inspections += 1;
            }
            if self.needed(table, key.block) {
                self.mark(key, false);
            }
        }
    }
    /// Move a retained column into the needed or the obsolete order.
    fn mark(&mut self, key: Key, obsolete: bool) {
        let entry = self.entries.get_mut(&key).expect("a retained column");
        if entry.obsolete == obsolete {
            return;
        }
        let bytes = entry.value.as_ref().map_or(0, |value| value.bytes);
        let used = entry.used;
        entry.obsolete = obsolete;
        let (from, to) = if obsolete {
            (&mut self.lru, &mut self.obsolete)
        } else {
            (&mut self.obsolete, &mut self.lru)
        };
        from.remove(&(used, key));
        to.insert((used, key));
        let count = self.obsolete_tables.entry(key.table).or_default();
        if obsolete {
            self.obsolete_bytes += bytes;
            *count += 1;
        } else {
            self.obsolete_bytes -= bytes;
            *count -= 1;
        }
    }
    /// A retained column was used again: its place in its order moves.
    fn touch(&mut self, key: Key, used: u64) {
        let entry = self.entries.get_mut(&key).expect("a retained column");
        let order = if entry.obsolete {
            &mut self.obsolete
        } else {
            &mut self.lru
        };
        order.remove(&(entry.used, key));
        order.insert((used, key));
        entry.used = used;
    }
    /// Retain `value` as `key`'s column, in the order its need decides.
    fn retain(&mut self, key: Key, value: Arc<Column>) {
        let bytes = value.bytes;
        let obsolete = !self.needed(key.table, key.block);
        let entry = self.entries.get_mut(&key).expect("an entry being read");
        let used = entry.used;
        entry.value = Some(value);
        entry.obsolete = obsolete;
        self.bytes += bytes;
        self.retained.insert((key.table, key.block, key));
        if obsolete {
            self.obsolete.insert((used, key));
            self.obsolete_bytes += bytes;
            *self.obsolete_tables.entry(key.table).or_default() += 1;
        } else {
            self.lru.insert((used, key));
        }
    }
    /// An in-flight read keeps its entry after its retained value is evicted.
    fn evict(&mut self, key: Key) {
        let entry = self.entries.get_mut(&key).expect("a retained column");
        let Some(value) = entry.value.take() else {
            return;
        };
        let (used, obsolete) = (entry.used, entry.obsolete);
        entry.obsolete = false;
        if !entry.reading {
            self.entries.remove(&key);
        }
        self.bytes -= value.bytes;
        self.retained.remove(&(key.table, key.block, key));
        if obsolete {
            self.obsolete.remove(&(used, key));
            self.obsolete_bytes -= value.bytes;
            *self.obsolete_tables.entry(key.table).or_default() -= 1;
        } else {
            self.lru.remove(&(used, key));
        }
    }
}
/// The parts of half-open `a` and `b` in one and not the other.
fn difference(a: (usize, usize), b: (usize, usize)) -> [(usize, usize); 2] {
    if a.1 <= b.0 || b.1 <= a.0 || a.0 >= a.1 || b.0 >= b.1 {
        return [a, b];
    }
    [(a.0.min(b.0), a.0.max(b.0)), (a.1.min(b.1), a.1.max(b.1))]
}
const ENTRY_BYTES: usize = size_of::<Column>()
    + 2 * size_of::<usize>()
    + 2 * size_of::<(Key, Entry)>()
    + 2 * size_of::<(u64, Key)>()
    + 2 * size_of::<(usize, usize, Key)>();
#[derive(Default)]
struct Inner {
    state: Mutex<State>,
    changed: Condvar,
    #[cfg(test)]
    waiting: Condvar,
    /// The wire decodes begun since the log started, while it is kept.
    #[cfg(any(test, feature = "test-support"))]
    wire: Mutex<Option<Vec<WireDecode>>>,
}
#[derive(Clone, Default)]
pub(super) struct Cache(Arc<Inner>);
impl Cache {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.0
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }
    pub(super) fn enable(&self, enabled: bool) {
        let mut state = self.lock();
        state.enabled = enabled;
        Self::prune(&mut state);
    }
    /// Read measured column sizes without copying them.
    pub(super) fn with_largest<T>(&self, read: impl FnOnce(&BTreeMap<usize, usize>) -> T) -> T {
        read(&self.lock().largest)
    }
    /// [`Replay::decoded_section_census`].
    fn census(&self, window: Option<(usize, usize)>) -> SectionCacheCensus {
        let state = self.lock();
        let mut census = SectionCacheCensus::EMPTY;
        census.bytes = state.bytes;
        census.budget = state.budget();
        census.frontier = state.frontier();
        census.readers = state.consumers.len();
        for (&table, &size) in &state.largest {
            census.largest[table] = size;
        }
        let mut least = [usize::MAX; TABLE_COUNT];
        for reader in state.consumers.values() {
            let tables = least.iter_mut().zip(&mut census.retaining).enumerate();
            for (table, (least, retaining)) in tables {
                if reader.reads(table) && reader.next < *least {
                    *least = reader.next;
                    *retaining = Some(reader.name);
                }
            }
        }
        classify(&state.entries, window, &mut census);
        census
    }
    fn prune(state: &mut State) {
        // Columns no reader needs keep one block's set of columns at most, least recently used first.
        let allowance = state.obsolete_allowance();
        while state.obsolete_bytes > allowance {
            let Some((_, key)) = state.obsolete.pop_first() else {
                break;
            };
            #[cfg(test)]
            {
                state.inspections += 1;
            }
            state.evict(key);
        }
        let budget = state.budget();
        if state.bytes <= budget {
            return;
        }
        // Over budget: every column no reader needs goes first.
        while state.bytes > budget {
            let Some((_, key)) = state.obsolete.pop_first() else {
                break;
            };
            #[cfg(test)]
            {
                state.inspections += 1;
            }
            state.evict(key);
        }
        if state.bytes <= budget {
            return;
        }
        if let Some((first, last)) = state.frontier() {
            // Preserve the existing needed-column eviction order when over budget.
            let entries = &mut state.entries;
            let retained = &mut state.retained;
            let bytes = &mut state.bytes;
            #[cfg(test)]
            let inspections = &mut state.inspections;
            state.lru.retain(|(_, key)| {
                #[cfg(test)]
                {
                    *inspections += 1;
                }
                if *bytes <= budget || (first <= key.block && key.block <= last) {
                    return true;
                }
                let entry = entries.get_mut(key).unwrap();
                *bytes -= entry.value.take().unwrap().bytes;
                retained.remove(&(key.table, key.block, *key));
                if !entry.reading {
                    entries.remove(key);
                }
                false
            });
        }
        while state.bytes > budget {
            let Some((_, key)) = state.lru.pop_first() else {
                break;
            };
            #[cfg(test)]
            {
                state.inspections += 1;
            }
            state.evict(key);
        }
    }
    /// Register `reader`.
    fn register(&self, reader: Frontier) -> SectionConsumer {
        let mut state = self.lock();
        let id = state.next_consumer;
        state.next_consumer += 1;
        let after = (reader.range(), reader.tables.clone());
        state.consumers.insert(id, reader);
        state.moved(None, Some(after));
        Self::prune(&mut state);
        SectionConsumer {
            cache: self.clone(),
            id,
        }
    }
    fn consumer(&self, block: usize) -> SectionConsumer {
        self.register(Frontier {
            next: block,
            ahead: block,
            last: None,
            tables: None,
            name: "reader",
        })
    }
    pub(super) fn read(
        &self,
        key: Key,
        hours: usize,
        keyframe_only: bool,
        decode: impl FnOnce() -> io::Result<Vec<TableUpdate>>,
    ) -> io::Result<Updates> {
        self.read_updates(key, hours, keyframe_only, || Updates::new(decode()?))
    }
    /// Cache a column already decoded as shared updates.
    pub(super) fn read_updates(
        &self,
        key: Key,
        hours: usize,
        keyframe_only: bool,
        decode: impl FnOnce() -> io::Result<Updates>,
    ) -> io::Result<Updates> {
        let mut state = self.lock();
        let owns_slot = loop {
            if !state.enabled {
                drop(state);
                return decode();
            }
            state.clock = state.clock.saturating_add(1);
            let used = state.clock;
            if state
                .entries
                .get(&key)
                .is_some_and(|entry| entry.value.is_some())
            {
                state.touch(key, used);
            }
            let entry = state.entries.entry(key).or_default();
            entry.used = used;
            if let Some(value) = &entry.value
                && value.updates.len() >= hours
                && (keyframe_only || value.table_read)
            {
                return Ok(value.updates.prefix(hours));
            }
            let priority = thread_priority();
            if !entry.reading {
                entry.reading = true;
                entry.priority = priority;
                break true;
            }
            if priority > entry.priority {
                break false;
            }
            #[cfg(test)]
            {
                state.waiters += 1;
                self.0.waiting.notify_all();
            }
            state = self
                .0
                .changed
                .wait(state)
                .unwrap_or_else(|error| error.into_inner());
            #[cfg(test)]
            {
                state.waiters -= 1;
            }
        };
        drop(state);
        #[allow(unused_mut)]
        let mut reading = Reading {
            cache: self,
            key,
            owns_slot,
            #[cfg(any(test, feature = "test-support"))]
            attempt: Some((hours, keyframe_only)),
        };
        let updates = decode()?;
        let bytes = ENTRY_BYTES.saturating_add(updates.bytes());
        let value = Arc::new(Column {
            updates: updates.clone(),
            bytes,
            table_read: !keyframe_only,
        });
        let mut state = self.lock();
        #[cfg(any(test, feature = "test-support"))]
        if let (Some(decodes), Some((hours, keyframe_only))) =
            (&mut state.decodes, reading.attempt.take())
        {
            decodes.push(SectionDecode {
                block: key.block,
                table: key.table,
                hours,
                keyframe_only,
                returned: true,
            });
        }
        let covered = state.entries.get(&key).is_some_and(|entry| {
            entry.value.as_ref().is_some_and(|value| {
                value.updates.len() >= hours && (keyframe_only || value.table_read)
            })
        });
        if state.enabled && (owns_slot || !covered) {
            let largest = state.largest.entry(key.table).or_default();
            if bytes > *largest {
                let grown = bytes - *largest;
                *largest = bytes;
                state.allowance += grown;
            }
            let used = state
                .entries
                .get(&key)
                .map_or(state.clock, |entry| entry.used);
            if state.entries.contains_key(&key) {
                state.evict(key);
            }
            state.entries.entry(key).or_default().used = used;
            state.retain(key, value);
            Self::prune(&mut state);
        }
        drop(state);
        drop(reading);
        Ok(updates)
    }
    /// Return a ready column without waiting for another read or decoding it.
    pub(super) fn ready(&self, key: Key, hours: usize, keyframe_only: bool) -> Option<Updates> {
        let mut state = self.lock();
        if !state.enabled {
            return None;
        }
        let entry = state.entries.get(&key)?;
        let updates = entry
            .value
            .as_ref()
            .filter(|value| value.updates.len() >= hours && (keyframe_only || value.table_read))?
            .updates
            .prefix(hours);
        state.clock = state.clock.saturating_add(1);
        let used = state.clock;
        state.touch(key, used);
        Some(updates)
    }
    /// Start keeping a log of the decodes this cache runs, and one of the wire decodes the replay runs.
    #[cfg(any(test, feature = "test-support"))]
    fn log_decodes(&self) {
        self.lock().decodes = Some(Vec::new());
        *self.wire() = Some(Vec::new());
    }
    /// The decodes logged since the log started, which goes on.
    #[cfg(any(test, feature = "test-support"))]
    fn decodes(&self) -> Vec<SectionDecode> {
        self.lock().decodes.clone().unwrap_or_default()
    }
    /// The wire decodes logged since the log started, which goes on.
    #[cfg(any(test, feature = "test-support"))]
    fn wire_decodes(&self) -> Vec<WireDecode> {
        self.wire().clone().unwrap_or_default()
    }
    #[cfg(any(test, feature = "test-support"))]
    fn wire(&self) -> MutexGuard<'_, Option<Vec<WireDecode>>> {
        self.0
            .wire
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }
    /// Log wire work before decoding, so refusals and retries count separately.
    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn wire_decode(
        &self,
        position: Option<usize>,
        table: TableId,
        hours: usize,
        route: WireRoute,
    ) {
        if let Some(log) = &mut *self.wire() {
            log.push(WireDecode {
                block: position.unwrap_or(0),
                table: table.index(),
                hours,
                route,
            });
        }
    }
}
fn thread_priority() -> i32 {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Threading::{GetCurrentThread, GetThreadPriority};
        GetThreadPriority(GetCurrentThread())
    }
    #[cfg(not(windows))]
    {
        0
    }
}

struct Reading<'a> {
    cache: &'a Cache,
    key: Key,
    owns_slot: bool,
    /// The hours and mode of a decode not yet logged as returned.
    #[cfg(any(test, feature = "test-support"))]
    attempt: Option<(usize, bool)>,
}
impl Drop for Reading<'_> {
    fn drop(&mut self) {
        let mut state = self.cache.lock();
        // A decode that returned no column (an error or a panic).
        #[cfg(any(test, feature = "test-support"))]
        if let (Some(decodes), Some((hours, keyframe_only))) =
            (&mut state.decodes, self.attempt.take())
        {
            decodes.push(SectionDecode {
                block: self.key.block,
                table: self.key.table,
                hours,
                keyframe_only,
                returned: false,
            });
        }
        if self.owns_slot {
            let entry = state.entries.get_mut(&self.key).unwrap();
            entry.reading = false;
            if entry.value.is_none() {
                state.entries.remove(&self.key);
            }
        }
        drop(state);
        self.cache.0.changed.notify_all();
    }
}
/// A live reader whose window and declaration end when it is dropped.
pub struct SectionConsumer {
    cache: Cache,
    id: u64,
}
impl SectionConsumer {
    /// Change declarations atomically with respect to retention and accounting.
    fn change(&self, change: impl FnOnce(&mut Frontier)) {
        let mut state = self.cache.lock();
        let reader = &state.consumers[&self.id];
        let before = (reader.range(), reader.tables.clone());
        change(state.consumers.get_mut(&self.id).unwrap());
        let reader = &state.consumers[&self.id];
        let after = (reader.range(), reader.tables.clone());
        state.moved(Some(before), Some(after));
        Cache::prune(&mut state);
    }
    /// Backward moves retain outstanding claims, but never completed read-ahead.
    pub fn at(&self, block: usize) {
        self.change(|reader| {
            reader.ahead = if reader.ahead > reader.next {
                reader.ahead.max(block)
            } else {
                block
            };
            reader.next = block;
            if reader.last.is_some() {
                reader.last = Some(block);
            }
        });
    }
    /// The window reaches through the furthest outstanding scheduled block.
    pub fn reading(&self, block: usize) {
        self.change(|reader| reader.ahead = reader.ahead.max(block));
    }
    /// Replace the declaration without an interval where neither set is registered.
    pub fn declare(&self, replay: &Replay, tables: &[TableId]) {
        let tables = widen(replay.format.version(), tables);
        self.change(|reader| reader.tables = Some(tables));
    }
}
impl Drop for SectionConsumer {
    fn drop(&mut self) {
        let mut state = self.cache.lock();
        let reader = &state.consumers[&self.id];
        let before = (reader.range(), reader.tables.clone());
        state.consumers.remove(&self.id);
        state.moved(Some(before), None);
        Cache::prune(&mut state);
    }
}
impl Replay {
    /// Retain live windows plus one measured block of obsolete columns within the span budget.
    pub fn cache_decoded_sections(&self, enabled: bool) {
        self.decoded_sections.enable(enabled);
    }
    /// Retained column charge only, excluding caller-held frames and decode scratch.
    pub fn decoded_section_cache_bytes(&self) -> usize {
        self.decoded_sections.lock().bytes
    }
    /// Sample the cache without allocating or pruning; window endpoints are inclusive.
    pub fn decoded_section_census(&self, window: Option<(usize, usize)>) -> SectionCacheCensus {
        self.decoded_sections.census(window)
    }
    /// Register every table at the next block and extend its window with `reading`.
    pub fn section_consumer(&self, next_block: usize) -> SectionConsumer {
        self.decoded_sections.consumer(next_block)
    }
    /// Declare named reads, widened to Campaign, alias sources and layout-gap tables.
    pub fn section_reader(
        &self,
        next_block: usize,
        tables: &[TableId],
        name: &'static str,
    ) -> SectionConsumer {
        self.decoded_sections.register(Frontier {
            next: next_block,
            ahead: next_block,
            last: None,
            tables: Some(widen(self.format.version(), tables)),
            name,
        })
    }
    /// Register only the current block; None declares every table.
    pub fn section_block_reader(
        &self,
        block: usize,
        tables: Option<&[TableId]>,
        name: &'static str,
    ) -> SectionConsumer {
        self.decoded_sections.register(Frontier {
            next: block,
            ahead: block,
            last: Some(block),
            tables: tables.map(|tables| widen(self.format.version(), tables)),
            name,
        })
    }
    /// Start admission and wire-decode logs shared across this replay's threads.
    #[cfg(any(test, feature = "test-support"))]
    pub fn log_section_decodes(&self) {
        self.decoded_sections.log_decodes();
    }
    /// The cache's admissions logged since [`Self::log_section_decodes`], in the order they finished.
    #[cfg(any(test, feature = "test-support"))]
    pub fn section_decodes(&self) -> Vec<SectionDecode> {
        self.decoded_sections.decodes()
    }
    /// Wire attempts since logging began, in start order, including failed attempts.
    #[cfg(any(test, feature = "test-support"))]
    pub fn wire_decodes(&self) -> Vec<WireDecode> {
        self.decoded_sections.wire_decodes()
    }
}

#[cfg(test)]
mod tests;
