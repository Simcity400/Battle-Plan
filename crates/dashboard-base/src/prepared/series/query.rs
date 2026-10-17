//! Reuse settled numeric buckets across moving publications. Membership and
//! sampled tails are applied afterwards, with their current provenance.
use super::*;
use std::{cell::RefCell, rc::Rc};

thread_local! {
    static SCOPES: RefCell<Vec<Rc<RefCell<State>>>> = const { RefCell::new(Vec::new()) };
    #[cfg(test)]
    pub(super) static BUCKET_FOLDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Numeric folds held with one whole query reply and charged to its cache entry.
/// Only folds used by that request survive its next publication.
#[derive(Clone, Default)]
pub struct SummaryCache(BTreeMap<Vec<u8>, Arc<Fold>>);

impl SummaryCache {
    pub fn bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self
                .0
                .iter()
                .map(|(key, fold)| {
                    2 * std::mem::size_of::<(Vec<u8>, Arc<Fold>)>()
                        + key.capacity()
                        + 2 * std::mem::size_of::<usize>()
                        + fold.bytes()
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
                .0
                .get(&params)
                .or_else(|| cache.previous.0.get(&params))
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
        cache.borrow_mut().current.0.insert(
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
