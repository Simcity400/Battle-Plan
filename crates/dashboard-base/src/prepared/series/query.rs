//! Reuse immutable page reads and settled numeric buckets across publications.
//! Membership and sampled tails keep their current provenance.
use super::*;
use std::{cell::RefCell, rc::Rc};

type ValueKey = (std::any::TypeId, Vec<[u64; 3]>, Vec<u8>);
type TextKey = (TextKind, Vec<[u64; 3]>);

thread_local! {
    static SCOPES: RefCell<Vec<Rc<RefCell<State>>>> = const { RefCell::new(Vec::new()) };
    #[cfg(test)]
    pub(super) static BUCKET_FOLDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static TEXT_READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
/// Uncached compressed catalogue/reply reads on this thread.
#[cfg(any(test, feature = "test-support"))]
pub fn text_reads() -> usize {
    TEXT_READS.get()
}

/// Page reads and numeric folds held with one reply and charged to its LRU entry.
/// Only entries used by that request survive its next publication.
#[derive(Clone, Default)]
pub struct SummaryCache {
    folds: BTreeMap<Vec<u8>, Arc<Fold>>,
    blobs: BTreeMap<([u64; 3], u64), Arc<Cached>>,
    summaries: BTreeMap<([u64; 3], u64), Arc<Summaries>>,
    tables: BTreeMap<[u64; 3], Arc<Option<Table>>>,
    // Each text binds its kind and ordered page identities to the retained bytes.
    texts: BTreeMap<TextKey, Arc<Vec<u8>>>,
    events: BTreeMap<([u64; 3], u64, u64), Arc<Value>>,
    values: BTreeMap<ValueKey, Arc<Retained>>,
}

struct Retained {
    value: Arc<dyn std::any::Any + Send + Sync>,
    bytes: usize,
}

impl SummaryCache {
    pub fn bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self
                .folds
                .iter()
                .map(|(key, fold)| {
                    2 * std::mem::size_of::<(Vec<u8>, Arc<Fold>)>()
                        + key.capacity()
                        + 2 * std::mem::size_of::<usize>()
                        + fold.bytes()
                })
                .sum::<usize>()
            + self
                .values
                .iter()
                .map(|((_, ids, params), value)| {
                    2 * std::mem::size_of::<(ValueKey, Arc<Retained>)>()
                        + ids.capacity() * std::mem::size_of::<[u64; 3]>()
                        + params.capacity()
                        + std::mem::size_of::<Retained>()
                        + 4 * std::mem::size_of::<usize>()
                        + value.bytes
                })
                .sum::<usize>()
            + self
                .tables
                .values()
                .map(|table| {
                    2 * std::mem::size_of::<([u64; 3], Arc<Option<Table>>)>()
                        + 2 * std::mem::size_of::<usize>()
                        + std::mem::size_of::<Option<Table>>()
                        + table
                            .as_ref()
                            .as_ref()
                            .map_or(0, |table| table.0.capacity())
                })
                .sum::<usize>()
            + self
                .blobs
                .values()
                .map(|blob| {
                    2 * std::mem::size_of::<(([u64; 3], u64), Arc<Cached>)>()
                        + 2 * std::mem::size_of::<usize>()
                        + blob.bytes()
                })
                .sum::<usize>()
            + self
                .summaries
                .values()
                .map(|summaries| {
                    2 * std::mem::size_of::<(([u64; 3], u64), Arc<Summaries>)>()
                        + 2 * std::mem::size_of::<usize>()
                        + summaries_bytes(summaries)
                })
                .sum::<usize>()
            + self
                .texts
                .iter()
                .map(|((_, ids), text)| {
                    2 * std::mem::size_of::<((TextKind, Vec<[u64; 3]>), Arc<Vec<u8>>)>()
                        + ids.capacity() * std::mem::size_of::<[u64; 3]>()
                        + 2 * std::mem::size_of::<usize>()
                        + std::mem::size_of::<Vec<u8>>()
                        + text.capacity()
                })
                .sum::<usize>()
            + self
                .events
                .values()
                .map(|value| {
                    2 * std::mem::size_of::<(([u64; 3], u64, u64), Arc<Value>)>()
                        + 2 * std::mem::size_of::<usize>()
                        + crate::prepared::query::observations::json_bytes(value)
                })
                .sum::<usize>()
    }
}

struct State {
    previous: SummaryCache,
    current: SummaryCache,
}

/// Reuse the completed predecessor of this request while reading one manifest.
/// The caller publishes the resulting folds with the reply, under its ordinary
/// source, publication, presentation and verification binding.
pub struct CacheScope(Rc<RefCell<State>>);

pub fn cache_scope(previous: Option<&SummaryCache>) -> CacheScope {
    let state = Rc::new(RefCell::new(State {
        previous: previous.cloned().unwrap_or_default(),
        current: SummaryCache::default(),
    }));
    SCOPES.with(|scopes| scopes.borrow_mut().push(Rc::clone(&state)));
    CacheScope(state)
}

impl CacheScope {
    pub fn finish(self) -> SummaryCache {
        let mut state = self.0.borrow_mut();
        std::mem::take(&mut state.current)
    }
}

impl Drop for CacheScope {
    fn drop(&mut self) {
        SCOPES.with(|scopes| scopes.borrow_mut().pop());
    }
}

/// These entries live with the query's ordinary bounded LRU reply. Retain
/// only pages used by the current ask, and compare immutable identities rather
/// than batch ordinals: a rewritten prefix must never reuse its former bytes.
fn current() -> Option<Rc<RefCell<State>>> {
    SCOPES.with(|scopes| scopes.borrow().last().cloned())
}

/// A projected immutable page value retained with this request. The caller
/// names every supplying page and projection input and charges owned capacity.
pub fn reuse_value<T: Send + Sync + 'static>(
    pages: Vec<[u64; 3]>,
    params: &impl serde::Serialize,
    bytes: impl FnOnce(&T) -> usize,
    read: impl FnOnce() -> io::Result<T>,
) -> io::Result<Arc<T>> {
    let Some(scope) = current() else {
        return read().map(Arc::new);
    };
    let id = (
        std::any::TypeId::of::<T>(),
        pages,
        serde_json::to_vec(params)?,
    );
    let retained = reuse(
        Some(scope),
        id,
        |cache| &mut cache.values,
        || {
            let value = read()?;
            Ok(Retained {
                bytes: bytes(&value),
                value: Arc::new(value),
            })
        },
    )?;
    Arc::clone(&retained.value)
        .downcast()
        .map_err(|_| invalid("Prepared query value type changed"))
}
fn reuse<K: Ord, T>(
    scope: Option<Rc<RefCell<State>>>,
    id: K,
    map: fn(&mut SummaryCache) -> &mut BTreeMap<K, Arc<T>>,
    read: impl FnOnce() -> io::Result<T>,
) -> io::Result<Arc<T>> {
    let held = scope.as_ref().and_then(|scope| {
        let mut state = scope.borrow_mut();
        let State { current, previous } = &mut *state;
        map(current)
            .get(&id)
            .cloned()
            .or_else(|| map(previous).get(&id).cloned())
    });
    let value = match held {
        Some(value) => value,
        None => Arc::new(read()?),
    };
    if let Some(scope) = scope {
        map(&mut scope.borrow_mut().current).insert(id, Arc::clone(&value));
    }
    Ok(value)
}

pub(super) fn held_table(id: [u64; 3]) -> Option<Arc<Option<Table>>> {
    let scope = current()?;
    let mut state = scope.borrow_mut();
    let table = state
        .current
        .tables
        .get(&id)
        .or_else(|| state.previous.tables.get(&id))
        .cloned()?;
    state.current.tables.insert(id, Arc::clone(&table));
    Some(table)
}

pub(super) fn keep_table(id: [u64; 3], table: &Arc<Option<Table>>) {
    if let Some(scope) = current() {
        scope
            .borrow_mut()
            .current
            .tables
            .insert(id, Arc::clone(table));
    }
}

pub(super) fn blob(
    id: ([u64; 3], u64),
    read: impl FnOnce() -> io::Result<Cached>,
) -> io::Result<Arc<Cached>> {
    reuse(current(), id, |cache| &mut cache.blobs, read)
}

/// The blob this request's scope already holds, kept for its next publication too.
pub(super) fn held_blob(id: ([u64; 3], u64)) -> Option<Arc<Cached>> {
    let scope = current()?;
    let mut state = scope.borrow_mut();
    let blob = state
        .current
        .blobs
        .get(&id)
        .or_else(|| state.previous.blobs.get(&id))
        .cloned()?;
    state.current.blobs.insert(id, Arc::clone(&blob));
    Some(blob)
}

/// A blob decoded outside [`blob`], kept with this request's scope as `blob` keeps one.
pub(super) fn keep_blob(id: ([u64; 3], u64), blob: &Arc<Cached>) {
    if let Some(scope) = current() {
        scope
            .borrow_mut()
            .current
            .blobs
            .insert(id, Arc::clone(blob));
    }
}
pub(super) fn summaries(
    id: ([u64; 3], u64),
    read: impl FnOnce() -> io::Result<Summaries>,
) -> io::Result<Arc<Summaries>> {
    reuse(current(), id, |cache| &mut cache.summaries, read)
}
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum TextKind {
    Catalogue,
    Reply,
}

pub(super) fn text(
    kind: TextKind,
    ids: Vec<[u64; 3]>,
    read: impl FnOnce() -> io::Result<Vec<u8>>,
) -> io::Result<Arc<Vec<u8>>> {
    reuse(
        current(),
        (kind, ids),
        |cache| &mut cache.texts,
        || {
            #[cfg(any(test, feature = "test-support"))]
            TEXT_READS.set(TEXT_READS.get() + 1);
            read()
        },
    )
}
pub(super) fn event(
    id: ([u64; 3], u64, u64),
    read: impl FnOnce() -> io::Result<Value>,
) -> io::Result<Value> {
    let Some(scope) = current() else {
        return read();
    };
    let value = reuse(Some(scope), id, |cache| &mut cache.events, read)?;
    Ok((*value).clone())
}

struct Fold {
    pages: Arc<Vec<[u64; 3]>>,
    ranges: Vec<Option<(i32, i32)>>,
    buckets: Vec<Option<ChartBucket>>,
}

impl Fold {
    fn bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + 2 * std::mem::size_of::<usize>()
            + self.pages.capacity() * std::mem::size_of::<[u64; 3]>()
            + self.ranges.capacity() * std::mem::size_of::<Option<(i32, i32)>>()
            + self.buckets.capacity() * std::mem::size_of::<Option<ChartBucket>>()
    }
}

impl Reader<'_> {
    pub(super) fn folded_buckets(
        &mut self,
        key: u64,
        bounds: (i32, i32),
        view: (f64, f64, u32),
        ranges: &[Option<(i32, i32)>],
        decimals: u8,
    ) -> io::Result<Vec<Option<ChartBucket>>> {
        let cache = SCOPES.with(|scopes| scopes.borrow().last().cloned());
        let Some(cache) = cache else {
            return ranges
                .iter()
                .map(|range| match range {
                    Some((first, last)) => self.range_bucket(key, *first, *last, decimals),
                    None => Ok(None),
                })
                .collect();
        };
        if ranges.iter().all(Option::is_none) {
            return Ok(vec![None; ranges.len()]);
        }
        // A narrow viewport must not open unrelated prepared pages just to
        // discover their identities. Prefix reuse covers a request visiting
        // every accepted batch; other windows retain the ordinary page reads.
        if let (Some(first), Some(last)) = (
            ranges.iter().flatten().map(|range| range.0).min(),
            ranges.iter().flatten().map(|range| range.1).max(),
        ) {
            let batches = self.batches(first, last)?;
            if batches.start != 0 || batches.end != self.count {
                return ranges
                    .iter()
                    .map(|range| match range {
                        Some((first, last)) => self.range_bucket(key, *first, *last, decimals),
                        None => Ok(None),
                    })
                    .collect();
            }
        }
        if self.publication.is_none() {
            let mut pages = Vec::new();
            for batch in 0..self.count {
                pages.push(self.file(batch)?.id().cache_key());
            }
            self.publication = Some(Arc::new(pages));
        }
        let pages = Arc::clone(self.publication.as_ref().unwrap());
        // The moving last bound controls the grid, not the identity of a
        // request. Compare each actual interval before reusing its bucket.
        let params = serde_json::to_vec(&(&self.section, key, bounds.0, view, decimals))?;
        let previous = {
            let cache = cache.borrow();
            cache
                .current
                .folds
                .get(&params)
                .or_else(|| cache.previous.folds.get(&params))
                .cloned()
        };
        let common = previous.as_ref().map_or(0, |previous| {
            previous
                .pages
                .iter()
                .zip(pages.iter())
                .take_while(|(a, b)| a == b)
                .count()
        });
        let settled = common
            .checked_sub(1)
            .map(|last| self.hours(last as u64).map(|hours| hours.1))
            .transpose()?;
        let mut buckets = Vec::with_capacity(ranges.len());
        for (index, range) in ranges.iter().enumerate() {
            check_cancel(self.cancelled)?;
            let Some((first, last)) = *range else {
                buckets.push(None);
                continue;
            };
            let old = previous.as_ref().filter(|previous| {
                settled.is_some_and(|settled| last <= settled)
                    && previous.ranges.get(index) == Some(range)
            });
            buckets.push(match old {
                Some(previous) => previous.buckets[index],
                None => self.range_bucket(key, first, last, decimals)?,
            });
        }
        cache.borrow_mut().current.folds.insert(
            params,
            Arc::new(Fold {
                pages,
                ranges: ranges.to_vec(),
                buckets: buckets.clone(),
            }),
        );
        Ok(buckets)
    }
}

#[cfg(test)]
mod tests;
