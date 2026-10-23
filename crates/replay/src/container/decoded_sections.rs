//! Revision-local section reuse, sized by measured columns and live reader frontiers.
use super::{BlockIndex, Replay};
use crate::{
    frame::{TableUpdate, recorded::Updates},
    rows::TableId,
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
    used: u64,
}
struct Frontier {
    next: usize,
    ahead: usize,
}
#[derive(Default)]
struct State {
    entries: BTreeMap<Key, Entry>,
    lru: BTreeSet<(u64, Key)>,
    largest: BTreeMap<usize, usize>,
    consumers: BTreeMap<u64, Frontier>,
    next_consumer: u64,
    enabled: bool,
    bytes: usize,
    clock: u64,
    #[cfg(test)]
    inspections: usize,
}
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
        self.largest
            .values()
            .fold(0usize, |sum, size| sum.saturating_add(*size))
            .saturating_mul(self.span())
    }
}
const ENTRY_BYTES: usize = size_of::<Column>()
    + 2 * size_of::<usize>()
    + 2 * size_of::<(Key, Entry)>()
    + 2 * size_of::<(u64, Key)>();
#[derive(Default)]
struct Inner {
    state: Mutex<State>,
    changed: Condvar,
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
    fn prune(state: &mut State) {
        let budget = state.budget();
        if state.bytes <= budget {
            return;
        }
        if let Some((first, last)) = state.frontier() {
            // A finished query can leave recently used blocks outside the
            // remaining readers' span. Retire those before any older column
            // a live reader still needs, in one LRU-ordered sweep. Retain
            // visits each entry once, including after the budget is met.
            let entries = &mut state.entries;
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
            let entry = state.entries.get_mut(&key).unwrap();
            state.bytes -= entry.value.take().unwrap().bytes;
            if !entry.reading {
                state.entries.remove(&key);
            }
        }
    }
    fn consumer(&self, block: usize) -> SectionConsumer {
        let mut state = self.lock();
        let id = state.next_consumer;
        state.next_consumer += 1;
        state.consumers.insert(
            id,
            Frontier {
                next: block,
                ahead: block,
            },
        );
        SectionConsumer {
            cache: self.clone(),
            id,
        }
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
        loop {
            if !state.enabled {
                drop(state);
                return decode();
            }
            state.clock = state.clock.saturating_add(1);
            let used = state.clock;
            if let Some(previous) = state
                .entries
                .get(&key)
                .filter(|entry| entry.value.is_some())
                .map(|entry| entry.used)
            {
                state.lru.remove(&(previous, key));
                state.lru.insert((used, key));
            }
            let entry = state.entries.entry(key).or_default();
            entry.used = used;
            if let Some(value) = &entry.value
                && value.updates.len() >= hours
                && (keyframe_only || value.table_read)
            {
                return Ok(value.updates.prefix(hours));
            }
            if !entry.reading {
                entry.reading = true;
                break;
            }
            state = self
                .0
                .changed
                .wait(state)
                .unwrap_or_else(|error| error.into_inner());
        }
        drop(state);
        let reading = Reading { cache: self, key };
        let updates = decode()?;
        let bytes = ENTRY_BYTES.saturating_add(updates.bytes());
        let value = Arc::new(Column {
            updates: updates.clone(),
            bytes,
            table_read: !keyframe_only,
        });
        let mut state = self.lock();
        if state.enabled {
            let largest = state.largest.entry(key.table).or_default();
            *largest = (*largest).max(bytes);
            let entry = state.entries.get_mut(&key).unwrap();
            let old = entry.value.replace(value);
            let used = entry.used;
            state.lru.insert((used, key));
            state.bytes = state
                .bytes
                .saturating_sub(old.map_or(0, |old| old.bytes))
                .saturating_add(bytes);
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
        let previous = entry.used;
        state.clock = state.clock.saturating_add(1);
        let used = state.clock;
        state.lru.remove(&(previous, key));
        state.lru.insert((used, key));
        state.entries.get_mut(&key).unwrap().used = used;
        Some(updates)
    }
}
struct Reading<'a> {
    cache: &'a Cache,
    key: Key,
}
impl Drop for Reading<'_> {
    fn drop(&mut self) {
        let mut state = self.cache.lock();
        let entry = state.entries.get_mut(&self.key).unwrap();
        entry.reading = false;
        if entry.value.is_none() {
            state.entries.remove(&self.key);
        }
        drop(state);
        self.cache.0.changed.notify_all();
    }
}
/// A consumer's next block until its read or preparation finishes. Reader
/// tasks extend the far frontier; advancing or dropping consumers shrinks it.
pub struct SectionConsumer {
    cache: Cache,
    id: u64,
}
impl SectionConsumer {
    pub fn at(&self, block: usize) {
        let mut state = self.cache.lock();
        let frontier = state.consumers.get_mut(&self.id).unwrap();
        frontier.next = block;
        frontier.ahead = frontier.ahead.max(block);
        Cache::prune(&mut state);
    }
    /// Extend this reader's frontier to a block it schedules ahead. The
    /// extension lasts until it is passed or this consumer finishes.
    pub fn reading(&self, block: usize) {
        let mut state = self.cache.lock();
        let frontier = state.consumers.get_mut(&self.id).unwrap();
        frontier.ahead = frontier.ahead.max(block);
    }
}
impl Drop for SectionConsumer {
    fn drop(&mut self) {
        let mut state = self.cache.lock();
        state.consumers.remove(&self.id);
        Cache::prune(&mut state);
    }
}
impl Replay {
    /// Enable revision-local section reuse. Each table's largest measured
    /// column is retained for the live span between reader frontiers. When
    /// no multi-block consumer remains, retention falls to one block's set
    /// of columns. LRU handles future query reads whose blocks are unknown.
    pub fn cache_decoded_sections(&self, enabled: bool) {
        self.decoded_sections.enable(enabled);
    }
    /// Register a consumer at its next block; advance it at block boundaries.
    pub fn section_consumer(&self, next_block: usize) -> SectionConsumer {
        self.decoded_sections.consumer(next_block)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rows::{CountryRow, TableRows};
    fn key(block: usize) -> Key {
        Key {
            format: 1,
            branch: 1,
            first: block as i32,
            last: block as i32,
            offset: block as u64,
            length: 1,
            raw_length: 1,
            table: 0,
            block,
        }
    }
    fn rows(countries: usize) -> Vec<TableUpdate> {
        vec![TableUpdate::Snapshot(TableRows::Countries(
            (0..countries)
                .map(|id| CountryRow {
                    id: id as u32,
                    ..Default::default()
                })
                .collect(),
        ))]
    }
    #[test]
    fn ready_columns_share_prefixes_and_keep_the_cache_lru_consistent() {
        let cache = Cache::default();
        let mut column = rows(2);
        column.push(TableUpdate::Unchanged { baseline: 0 });
        let column = Updates::new(column).unwrap();
        assert!(cache.ready(key(0), 1, true).is_none());
        cache.enable(true);
        cache
            .read_updates(key(0), 1, true, || Ok(column.prefix(1)))
            .unwrap();
        assert!(cache.ready(key(0), 1, false).is_none());
        assert!(cache.ready(key(0), 2, true).is_none());
        let keyframe = cache.ready(key(0), 1, true).unwrap();
        assert!(Arc::ptr_eq(keyframe.get(0), column.get(0)));
        cache
            .read_updates(key(0), 2, false, || Ok(column.clone()))
            .unwrap();
        let previous = cache.lock().clock;
        let prefix = cache.ready(key(0), 1, false).unwrap();
        assert_eq!(prefix.len(), 1);
        assert!(Arc::ptr_eq(prefix.get(0), column.get(0)));
        let state = cache.lock();
        assert_eq!(state.clock, previous + 1);
        assert_eq!(state.lru.len(), 1);
        assert!(state.lru.contains(&(state.clock, key(0))));
        assert_eq!(state.entries[&key(0)].used, state.clock);
        drop(state);
        let cached = cache
            .read_updates(key(0), 2, false, || panic!("a ready column must be reused"))
            .unwrap();
        assert!(Arc::ptr_eq(cached.get(1), column.get(1)));
        cache.enable(false);
        assert!(cache.ready(key(0), 1, false).is_none());
    }

    #[test]
    fn measured_country_sizes_and_live_block_span_set_the_budget() {
        let mut previous = 0;
        for countries in [1, 1_500, 5_000] {
            let cache = Cache::default();
            cache.enable(true);
            let behind = cache.consumer(0);
            let ahead = cache.consumer(4);
            for block in 0..5 {
                cache
                    .read(key(block), 1, false, || Ok(rows(countries)))
                    .unwrap();
            }
            let largest = cache.lock().largest[&0];
            assert!(largest > previous, "country growth must grow the bound");
            previous = largest;
            assert_eq!(cache.lock().budget(), largest * 5);
            assert_eq!(cache.lock().bytes, largest * 5);
            behind.at(3);
            assert_eq!(cache.lock().budget(), largest * 2);
            assert_eq!(
                cache.lock().entries.len(),
                2,
                "retire LRU entries as the live span shrinks"
            );
            drop(ahead);
            behind.at(5);
            assert_eq!(cache.lock().entries.len(), 1);
            cache.enable(false);
            assert!(cache.lock().entries.is_empty());
        }
    }
    #[test]
    fn finished_query_releases_its_read_ahead_span_while_preparation_stays_behind() {
        let cache = Cache::default();
        cache.enable(true);
        let preparation = cache.consumer(0);
        let query = cache.consumer(2);
        query.reading(7);
        for block in 0..8 {
            cache
                .read(key(block), 1, false, || Ok(rows(1_500)))
                .unwrap();
        }
        let largest = cache.lock().largest[&0];
        assert_eq!(cache.lock().budget(), largest * 8);
        assert_eq!(cache.lock().entries.len(), 8);
        drop(query);
        assert_eq!(cache.lock().budget(), largest);
        assert_eq!(cache.lock().entries.len(), 1);
        preparation.reading(2);
        assert_eq!(cache.lock().budget(), largest * 3);
        preparation.at(2);
        assert_eq!(cache.lock().budget(), largest);
    }
    #[test]
    fn finished_query_cannot_evict_columns_still_in_the_preparation_span() {
        for countries in [128, 1_500, 5_000] {
            let cache = Cache::default();
            cache.enable(true);
            let preparation = cache.consumer(0);
            preparation.reading(1);
            let query = cache.consumer(4);
            let mut decoded = 0;
            for block in [0, 1, 4] {
                cache
                    .read(key(block), 1, false, || {
                        decoded += 1;
                        Ok(rows(countries))
                    })
                    .unwrap();
            }
            drop(query);
            for block in [0, 1] {
                cache
                    .read(key(block), 1, false, || {
                        decoded += 1;
                        Ok(rows(countries))
                    })
                    .unwrap();
            }
            assert_eq!(
                decoded, 3,
                "live columns survive a finished query at {countries} countries"
            );
            let state = cache.lock();
            assert_eq!(state.entries.len(), 2);
            assert_eq!(state.bytes, state.budget());
        }
    }
    #[test]
    fn finishing_a_63_block_query_inspects_each_column_once_before_lru_fallback() {
        let tables = TableId::ALL.len();
        let cache = Cache::default();
        cache.enable(true);
        let preparation = cache.consumer(0);
        preparation.reading(30);
        let query = cache.consumer(31);
        query.reading(62);
        for block in 0..63 {
            for table in 0..tables {
                let key = Key {
                    table,
                    ..key(block)
                };
                cache
                    .read(key, 1, false, || {
                        Ok(vec![TableUpdate::Unavailable {
                            reason: "synthetic column".into(),
                        }])
                    })
                    .unwrap();
            }
        }
        let before = {
            let state = cache.lock();
            assert_eq!(state.entries.len(), 63 * tables);
            state.inspections
        };
        drop(query);
        {
            let state = cache.lock();
            assert_eq!(
                state.inspections - before,
                63 * tables,
                "one sweep, without rescanning the 31 retained blocks for each victim"
            );
            assert_eq!(state.entries.len(), 31 * tables);
            assert!(state.entries.keys().all(|key| key.block <= 30));
            assert_eq!(state.bytes, state.budget());
        }
        // With every remaining column in the live span, the sweep finds no
        // outside victim and LRU retires them without another search.
        let before = cache.lock().inspections;
        cache.enable(false);
        let state = cache.lock();
        assert_eq!(state.inspections - before, 2 * 31 * tables);
        assert!(state.entries.is_empty());
        assert!(state.lru.is_empty());
        assert_eq!(state.bytes, 0);
    }
    #[test]
    fn hits_share_cells_and_keyframe_admission_never_supplies_a_table_read() {
        let cache = Cache::default();
        cache.enable(true);
        let first = cache.read(key(0), 1, true, || Ok(rows(2))).unwrap();
        let second = cache
            .read(key(0), 1, true, || panic!("keyframe decoded twice"))
            .unwrap();
        assert!(Arc::ptr_eq(first.get(0), second.get(0)));
        let mut called = false;
        cache
            .read(key(0), 1, false, || {
                called = true;
                Ok(rows(2))
            })
            .unwrap();
        assert!(
            called,
            "a table read must admit its later section even in a one-frame block"
        );
    }
    #[test]
    fn failed_and_panicked_extensions_preserve_the_earlier_prefix_and_release_the_slot() {
        let cache = Cache::default();
        cache.enable(true);
        let first = cache.read(key(0), 1, false, || Ok(rows(2))).unwrap();
        assert!(
            cache
                .read(key(0), 2, false, || Err(io::Error::other(
                    "synthetic refusal"
                )))
                .is_err()
        );
        assert!(
            std::panic::catch_unwind(|| {
                let _ = cache.read(key(0), 2, false, || panic!("synthetic panic"));
            })
            .is_err()
        );
        let second = cache
            .read(key(0), 1, false, || panic!("prefix lost"))
            .unwrap();
        assert!(Arc::ptr_eq(first.get(0), second.get(0)));
        assert!(!cache.lock().entries[&key(0)].reading);
    }
}
