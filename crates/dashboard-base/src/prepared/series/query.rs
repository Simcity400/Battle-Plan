//! Reuse immutable page reads and settled numeric buckets across publications.
//! Membership and sampled tails keep their current provenance.
use super::*;
use std::{cell::RefCell, rc::Rc};

thread_local! {
    static SCOPES: RefCell<Vec<Rc<RefCell<State>>>> = const { RefCell::new(Vec::new()) };
    #[cfg(test)]
    pub(super) static BUCKET_FOLDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Page reads and numeric folds held with one reply and charged to its LRU entry.
/// Only entries used by that request survive its next publication.
#[derive(Clone, Default)]
pub struct SummaryCache {
    folds: BTreeMap<Vec<u8>, Arc<Fold>>,
    blobs: BTreeMap<([u64; 3], u64), Arc<Cached>>,
    texts: BTreeMap<(TextKind, Vec<[u64; 3]>), Arc<Vec<u8>>>,
    events: BTreeMap<([u64; 3], u64, u64), Arc<Value>>,
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
                .blobs
                .iter()
                .map(|(_, blob)| {
                    2 * std::mem::size_of::<(([u64; 3], u64), Arc<Cached>)>()
                        + 2 * std::mem::size_of::<usize>()
                        + blob.bytes()
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
                .iter()
                .map(|(_, value)| {
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

pub(super) fn blob(
    id: ([u64; 3], u64),
    read: impl FnOnce() -> io::Result<Cached>,
) -> io::Result<Arc<Cached>> {
    reuse(current(), id, |cache| &mut cache.blobs, read)
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
    reuse(current(), (kind, ids), |cache| &mut cache.texts, read)
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
