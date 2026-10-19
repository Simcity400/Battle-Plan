//! Compact block observations and branch answers shared by queries. A source
//! revision owns its cache; append does not inherit it. A content binding change
//! discards that product's observations. Request keys name every input affecting
//! the observation, excluding the prepared prefix that only clips its use.
//!
//! Each product's observations and branch replies share a budget of its largest
//! observed entry (including owned allocation capacities) times the live block
//! count, evicting least recently used entries across request shapes. Branch
//! replies retain at most the current publication per request. Concurrent asks
//! for one key share one extraction. Values held by callers and in-flight
//! extractions are transient, outside retained bytes.

use crate::{Source, metadata::check_cancel};
use replay::container::BlockIndex;
use serde::Serialize;
use std::{
    any::{Any, TypeId},
    collections::BTreeMap,
    io,
    sync::{Arc, Mutex, atomic::AtomicBool},
};

type Value = Arc<dyn Any + Send + Sync>;
type Slot = Arc<Mutex<Option<Value>>>;

/// Capacity charge for a transient reply kept as ordinary serde JSON. Object
/// nodes are conservatively charged twice their key/value inline storage.
pub fn json_bytes(value: &serde_json::Value) -> usize {
    use serde_json::Value as Json;
    std::mem::size_of::<Json>()
        + match value {
            Json::String(text) => text.capacity(),
            Json::Array(values) => {
                values.capacity() * std::mem::size_of::<Json>()
                    + values.iter().map(json_bytes).sum::<usize>()
            }
            Json::Object(values) => values
                .iter()
                .map(|(key, value)| {
                    2 * std::mem::size_of::<(String, Json)>() + key.capacity() + json_bytes(value)
                })
                .sum(),
            _ => 0,
        }
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Key {
    product: &'static str,
    generation: u64,
    branch: u64,
    block: Option<(i32, i32)>,
    kind: TypeId,
    params: Vec<u8>,
}
struct Entry {
    slot: Slot,
    publication: Option<Vec<u8>>,
    bytes: usize,
    used: u64,
}
#[derive(Default)]
struct Product {
    binding: Option<store::SourceKey>,
    generation: u64,
    largest: usize,
    bytes: usize,
    blocks: usize,
    #[cfg(any(test, feature = "test-support"))]
    computations: usize,
}
#[derive(Default)]
struct Cache {
    binding: Option<store::SourceKey>,
    products: BTreeMap<&'static str, Product>,
    entries: BTreeMap<Key, Entry>,
    clock: u64,
}

/// A dashboard request binds every raw and content-dependent observation to
/// its current recording/content inputs before using any product scope.
pub fn bind(source: &Source, binding: &store::SourceKey) {
    let shared = source.state::<Cache>();
    let mut cache = shared.lock().unwrap_or_else(|error| error.into_inner());
    if cache.binding.as_ref() == Some(binding) {
        return;
    }
    cache.binding = Some(binding.clone());
    cache.entries.clear();
    for product in cache.products.values_mut() {
        product.generation += 1;
        product.bytes = 0;
        product.largest = 0;
        product.blocks = 0;
    }
}

pub struct Scope {
    cache: Arc<Mutex<Cache>>,
    product: &'static str,
    generation: u64,
    blocks: usize,
}

/// Bind once per query. Raw observations use None; content-dependent ones use
/// the prepared store's authenticated recording/content key.
pub fn scope(source: &Source, product: &'static str, binding: Option<&store::SourceKey>) -> Scope {
    let shared = source.state::<Cache>();
    let mut cache = shared.lock().unwrap_or_else(|error| error.into_inner());
    let state = cache.products.entry(product).or_default();
    if state.binding.as_ref() != binding {
        state.binding = binding.cloned();
        state.generation += 1;
        state.largest = 0;
        state.bytes = 0;
        state.blocks = 0;
        cache.entries.retain(|key, _| key.product != product);
    }
    let generation = cache.products[product].generation;
    drop(cache);
    Scope {
        cache: shared,
        product,
        generation,
        blocks: source.replay_ref().index.len(),
    }
}

impl Scope {
    /// An exact query needs only the blocks in its live window. Its largest
    /// requested working set sets the bound; sampled campaigns use all blocks.
    pub fn working_set(mut self, blocks: usize) -> Self {
        self.blocks = self.blocks.min(blocks.max(1));
        self
    }
    /// Extract an immutable observation once. `bytes` includes its inline size
    /// and every allocation it retains, at capacity. Look-backs may cache a
    /// block's fold/anchor facts or a proved window by putting its end hour and
    /// relevant selection in `params`; no decoder or predecessor is assumed.
    pub fn get<T: Send + Sync + 'static>(
        &self,
        block: &BlockIndex,
        params: &impl Serialize,
        cancelled: &AtomicBool,
        bytes: impl FnOnce(&T) -> usize,
        extract: impl FnOnce() -> io::Result<T>,
    ) -> io::Result<Arc<T>> {
        self.get_inner(
            block.branch,
            Some((block.first_hour, block.last_hour)),
            params,
            None,
            cancelled,
            bytes,
            |_| extract(),
            true,
        )
    }

    /// An answer whose extractor enforces its own recording-read boundary.
    /// A miss may read prepared pages on the prepared lane; recording reads
    /// must go through progressive helpers or `get`, which enforce that lane's
    /// decode refusal. Publication identities in `params` invalidate replies
    /// whose prepared inputs moved.
    pub fn get_query<T: Send + Sync + 'static>(
        &self,
        block: &BlockIndex,
        params: &impl Serialize,
        cancelled: &AtomicBool,
        bytes: impl FnOnce(&T) -> usize,
        extract: impl FnOnce() -> io::Result<T>,
    ) -> io::Result<Arc<T>> {
        self.get_inner(
            block.branch,
            Some((block.first_hour, block.last_hour)),
            params,
            None,
            cancelled,
            bytes,
            |_| extract(),
            false,
        )
    }

    /// The current branch-wide answer for one request. A new publication
    /// replaces the previous answer for the same parameters, including an
    /// in-flight extraction: its callers can finish, but cannot retain it.
    /// Replies share the product's bounded LRU with block observations, so
    /// distinct requests can evict each other. The extractor enforces the
    /// recording-read boundary as `get_query` does.
    #[allow(clippy::too_many_arguments)]
    pub fn get_entry<T: Send + Sync + 'static>(
        &self,
        branch: u64,
        params: &impl Serialize,
        publication: &impl Serialize,
        cancelled: &AtomicBool,
        bytes: impl FnOnce(&T) -> usize,
        extract: impl FnOnce() -> io::Result<T>,
    ) -> io::Result<Arc<T>> {
        self.get_inner(
            branch,
            None,
            params,
            Some(serde_json::to_vec(publication)?),
            cancelled,
            bytes,
            |_| extract(),
            false,
        )
    }

    /// Replace a publication while offering its completed predecessor to an
    /// incremental reader. A busy predecessor is skipped without waiting; the
    /// extractor must validate every part it reuses against the new publication.
    #[allow(clippy::too_many_arguments)]
    pub fn continue_entry<T: Send + Sync + 'static>(
        &self,
        branch: u64,
        params: &impl Serialize,
        publication: &impl Serialize,
        cancelled: &AtomicBool,
        bytes: impl FnOnce(&T) -> usize,
        extract: impl FnOnce(Option<Arc<T>>) -> io::Result<T>,
    ) -> io::Result<Arc<T>> {
        self.get_inner(
            branch,
            None,
            params,
            Some(serde_json::to_vec(publication)?),
            cancelled,
            bytes,
            extract,
            false,
        )
    }

    /// Find an already extracted block observation without waiting or starting
    /// work. Callers can reuse a whole fold before requesting a narrower one.
    pub fn get_cached<T: Send + Sync + 'static>(
        &self,
        block: &BlockIndex,
        params: &impl Serialize,
        cancelled: &AtomicBool,
    ) -> io::Result<Option<Arc<T>>> {
        check_cancel(cancelled)?;
        let key = Key {
            product: self.product,
            generation: self.generation,
            branch: block.branch,
            block: Some((block.first_hour, block.last_hour)),
            kind: TypeId::of::<T>(),
            params: serde_json::to_vec(params)?,
        };
        let slot = {
            let mut cache = self.cache.lock().unwrap_or_else(|error| error.into_inner());
            cache.clock += 1;
            let clock = cache.clock;
            let Some(entry) = cache.entries.get_mut(&key) else {
                return Ok(None);
            };
            entry.used = clock;
            Arc::clone(&entry.slot)
        };
        let held = match slot.try_lock() {
            Ok(held) => held,
            Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return Ok(None),
        };
        held.as_ref()
            .map(|value| {
                Arc::clone(value)
                    .downcast::<T>()
                    .map_err(|_| crate::invalid("Cached observation type changed"))
            })
            .transpose()
    }

    #[allow(clippy::too_many_arguments)]
    fn get_inner<T: Send + Sync + 'static>(
        &self,
        branch: u64,
        block: Option<(i32, i32)>,
        params: &impl Serialize,
        publication: Option<Vec<u8>>,
        cancelled: &AtomicBool,
        bytes: impl FnOnce(&T) -> usize,
        extract: impl FnOnce(Option<Arc<T>>) -> io::Result<T>,
        recording: bool,
    ) -> io::Result<Arc<T>> {
        check_cancel(cancelled)?;
        let key = Key {
            product: self.product,
            generation: self.generation,
            branch,
            block,
            kind: TypeId::of::<T>(),
            params: serde_json::to_vec(params)?,
        };
        let mut previous = None;
        let slot = {
            let mut cache = self.cache.lock().unwrap_or_else(|error| error.into_inner());
            cache.clock += 1;
            let clock = cache.clock;
            if cache
                .entries
                .get(&key)
                .is_some_and(|entry| entry.publication != publication)
            {
                let old = cache.entries.remove(&key).unwrap();
                // No lock is held across extraction. A superseded in-flight
                // reply still belongs to its original callers.
                if let Ok(held) = old.slot.try_lock() {
                    previous = held
                        .as_ref()
                        .and_then(|value| Arc::clone(value).downcast::<T>().ok());
                }
                let product = cache.products.get_mut(self.product).unwrap();
                product.bytes -= old.bytes;
            }
            if recording && !cache.entries.contains_key(&key) {
                super::progressive::decodes_allowed()?;
            }
            let entry = cache.entries.entry(key.clone()).or_insert_with(|| Entry {
                slot: Arc::new(Mutex::new(None)),
                publication: publication.clone(),
                bytes: 0,
                used: clock,
            });
            entry.used = clock;
            Arc::clone(&entry.slot)
        };
        let mut held = match slot.try_lock() {
            Ok(held) => held,
            Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => {
                super::progressive::decodes_allowed()?;
                slot.lock().unwrap_or_else(|error| error.into_inner())
            }
        };
        if let Err(error) = check_cancel(cancelled) {
            if held.is_none() {
                self.remove_pending(&key, &slot);
            }
            return Err(error);
        }
        if let Some(value) = &*held {
            return Arc::clone(value)
                .downcast::<T>()
                .map_err(|_| crate::invalid("Cached observation type changed"));
        }
        if let Err(error) = if recording {
            super::progressive::decodes_allowed()
        } else {
            Ok(())
        } {
            self.remove_pending(&key, &slot);
            return Err(error);
        }
        let value = match extract(previous).and_then(|value| {
            check_cancel(cancelled)?;
            Ok(value)
        }) {
            Ok(value) => Arc::new(value),
            Err(error) => {
                self.remove_pending(&key, &slot);
                return Err(error);
            }
        };
        // Two keys can be live during lookup/publication; B-tree nodes and the
        // slot/Arc allocation are charged conservatively at their inline sizes.
        let size = bytes(&value).saturating_add(
            2 * (std::mem::size_of::<Key>()
                + key.params.capacity()
                + publication.as_ref().map_or(0, Vec::capacity)
                + std::mem::size_of::<Entry>()
                + std::mem::size_of::<Mutex<Option<Value>>>()
                + 2 * std::mem::size_of::<usize>()),
        );
        *held = Some(value.clone());
        let mut cache = self.cache.lock().unwrap_or_else(|error| error.into_inner());
        if cache.products[&self.product].generation == self.generation
            && cache
                .entries
                .get(&key)
                .is_some_and(|entry| Arc::ptr_eq(&entry.slot, &slot))
        {
            let product = cache.products.get_mut(self.product).unwrap();
            product.largest = product.largest.max(size);
            product.blocks = product.blocks.max(self.blocks);
            product.bytes = product.bytes.saturating_add(size);
            #[cfg(any(test, feature = "test-support"))]
            {
                product.computations += 1;
            }
            let budget = product.largest.saturating_mul(product.blocks);
            if let Some(entry) = cache.entries.get_mut(&key) {
                entry.bytes = size;
            }
            while cache.products[self.product].bytes > budget {
                let oldest = cache
                    .entries
                    .iter()
                    .filter(|(key, entry)| key.product == self.product && entry.bytes > 0)
                    .min_by_key(|(_, entry)| entry.used)
                    .map(|(key, _)| key.clone());
                let Some(oldest) = oldest else {
                    break;
                };
                let entry = cache.entries.remove(&oldest).unwrap();
                cache.products.get_mut(self.product).unwrap().bytes -= entry.bytes;
            }
        } else if cache
            .entries
            .get(&key)
            .is_some_and(|entry| Arc::ptr_eq(&entry.slot, &slot))
        {
            cache.entries.remove(&key);
        }
        Ok(value)
    }

    fn remove_pending(&self, key: &Key, slot: &Slot) {
        let mut cache = self.cache.lock().unwrap_or_else(|error| error.into_inner());
        if cache
            .entries
            .get(key)
            .is_some_and(|entry| Arc::ptr_eq(&entry.slot, slot))
        {
            cache.entries.remove(key);
        }
    }
}

/// Retained bytes, the per-product bound, and completed extractions.
#[cfg(any(test, feature = "test-support"))]
pub fn usage(source: &Source, product: &'static str) -> (usize, usize, usize) {
    let shared = source.state::<Cache>();
    let cache = shared.lock().unwrap_or_else(|error| error.into_inner());
    cache.products.get(product).map_or((0, 0, 0), |state| {
        (
            state.bytes,
            state.largest.saturating_mul(state.blocks),
            state.computations,
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::{file_source, replay, source as frame};
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn continuation_receives_only_the_completed_predecessor_of_its_request() {
        let (source, root) = file_source(replay(vec![frame(1, 0, None)]));
        let flag = AtomicBool::new(false);
        let cache = scope(&source, "test-continuation", None);
        for publication in 0..4u32 {
            let answer = cache
                .continue_entry(
                    1,
                    &"selection",
                    &publication,
                    &flag,
                    |_: &u32| 4,
                    |old: Option<Arc<u32>>| {
                        assert_eq!(old.as_deref().copied(), publication.checked_sub(1));
                        Ok(publication)
                    },
                )
                .unwrap();
            assert_eq!(*answer, publication);
            assert_eq!(
                *cache
                    .continue_entry(
                        1,
                        &"selection",
                        &publication,
                        &flag,
                        |_: &u32| 4,
                        |_| panic!("unchanged publication recomputed")
                    )
                    .unwrap(),
                publication
            );
        }
        let (bytes, bound, computations) = usage(&source, "test-continuation");
        assert!(bytes <= bound);
        assert_eq!(computations, 4);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn branch_entries_are_distinct_from_blocks_and_share_the_source_binding() {
        let (source, root) = file_source(replay(vec![frame(1, 0, None)]));
        let block = &source.replay_ref().index[0];
        let flag = AtomicBool::new(false);
        let cache = scope(&source, "test-entries", None);
        assert_eq!(
            *cache
                .get_query(block, &(), &flag, |_: &u32| 4, || Ok(11_u32))
                .unwrap(),
            11
        );
        assert_eq!(
            *cache
                .get_entry(block.branch, &(), &(), &flag, |_: &u32| 4, || Ok(22_u32))
                .unwrap(),
            22
        );
        assert!(
            cache
                .get_cached::<u32>(block, &(), &flag)
                .unwrap()
                .is_none(),
            "branch replies and block observations compete for the same budget"
        );
        let (answer, refused) = super::super::progressive::without_block_decodes(|| {
            cache.get_entry(
                block.branch,
                &(),
                &(),
                &flag,
                |_: &u32| 4,
                || panic!("held answer"),
            )
        });
        assert!(!refused);
        assert_eq!(*answer.unwrap(), 22);
        assert_eq!(usage(&source, "test-entries").2, 2);
        assert_eq!(
            *cache
                .get_entry(2, &(), &(), &flag, |_: &u32| 4, || Ok(33_u32))
                .unwrap(),
            33
        );
        let (bytes, bound, computed) = usage(&source, "test-entries");
        assert!(bytes > 0 && bytes <= bound);
        assert_eq!(computed, 3, "branches and blocks have separate keys");
        let binding = store::SourceKey::capture(source.path(), &[]).unwrap();
        bind(&source, &binding);
        let cache = scope(&source, "test-entries", None);
        assert_eq!(
            *cache
                .get_entry(2, &(), &(), &flag, |_: &u32| 4, || Ok(44_u32))
                .unwrap(),
            44
        );
        assert_eq!(usage(&source, "test-entries").2, computed + 1);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn branch_requests_replace_superseded_publications_within_the_shared_bound() {
        let (source, root) = file_source(replay(vec![frame(1, 0, None)]));
        let mut cache = scope(&source, "test-replies", None);
        // Even when the budget could retain all 27 publications, replacement
        // keeps only the current reply for each request.
        cache.blocks = 663;
        let flag = AtomicBool::new(false);
        let mut previous = Vec::new();
        for publication in 0..27 {
            for request in ["timeline", "moment"] {
                let answer = cache
                    .get_entry(
                        1,
                        &request,
                        &publication,
                        &flag,
                        |value: &Vec<u8>| value.capacity(),
                        || Ok(vec![42; (publication + 1) * 1024]),
                    )
                    .unwrap();
                previous.push(Arc::downgrade(&answer));
            }
            assert!(
                previous[..previous.len() - 2]
                    .iter()
                    .all(|value| value.upgrade().is_none()),
                "superseded replies must be released"
            );
            let (bytes, bound, computed) = usage(&source, "test-replies");
            assert!(bytes > 0 && bytes <= bound);
            assert!(bytes < 2 * ((publication + 1) * 1024 + 1024));
            assert_eq!(computed, 2 * (publication + 1));
            assert_eq!(cache.cache.lock().unwrap().entries.len(), 2);
        }
        let before = usage(&source, "test-replies").0;
        for request in ["timeline", "moment"] {
            cache
                .get_entry(
                    1,
                    &request,
                    &28,
                    &flag,
                    |value: &Vec<u8>| value.capacity(),
                    || Ok(vec![1]),
                )
                .unwrap();
        }
        assert!(usage(&source, "test-replies").0 < before);
        let (answer, refused) = super::super::progressive::without_block_decodes(|| {
            cache.get_entry(
                1,
                &"timeline",
                &28,
                &flag,
                |_: &Vec<u8>| 0,
                || panic!("current reply"),
            )
        });
        assert!(!refused);
        assert_eq!(answer.unwrap().as_slice(), &[1]);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn distinct_branch_requests_share_the_bounded_lru_and_hits_refresh_recency() {
        let (source, root) = file_source(replay(vec![frame(1, 0, None)]));
        let mut cache = scope(&source, "test-reply-lru", None);
        cache.blocks = 3;
        let flag = AtomicBool::new(false);
        let read = |hour: u32| {
            cache
                .get_entry(1, &hour, &(), &flag, |_: &u32| 4, || Ok(hour))
                .unwrap()
        };
        let hot = Arc::downgrade(&read(0));
        let mut previous = Vec::new();
        for hour in 1..128 {
            let hit = cache
                .get_entry(
                    1,
                    &0_u32,
                    &(),
                    &flag,
                    |_: &u32| 4,
                    || panic!("recent reply"),
                )
                .unwrap();
            assert!(Arc::ptr_eq(&hit, &hot.upgrade().unwrap()));
            previous.push(Arc::downgrade(&read(hour)));
            let (bytes, bound, computed) = usage(&source, "test-reply-lru");
            assert!(
                bytes > 0 && bytes <= bound,
                "distinct requests exceed the product bound"
            );
            assert_eq!(computed, hour as usize + 1);
            assert!(cache.cache.lock().unwrap().entries.len() <= 3);
            if previous.len() > 2 {
                assert!(
                    previous[..previous.len() - 2]
                        .iter()
                        .all(|reply| reply.upgrade().is_none()),
                    "older requests must be evicted before the refreshed reply"
                );
            }
        }
        assert_eq!(*read(1), 1, "an evicted request is recomputed exactly");
        assert_eq!(usage(&source, "test-reply-lru").2, 129);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn superseded_in_flight_reply_cannot_publish_over_or_remove_its_replacement() {
        let (source, root) = file_source(replay(vec![frame(1, 0, None)]));
        let cache = scope(&source, "test-replacement", None);
        let flag = AtomicBool::new(false);
        for fail in [false, true] {
            let (started, wait_started) = std::sync::mpsc::channel();
            let (finish, wait_finish) = std::sync::mpsc::channel();
            std::thread::scope(|threads| {
                let cache = &cache;
                let flag = &flag;
                let old = threads.spawn(move || {
                    cache.get_entry(
                        1,
                        &fail,
                        &0,
                        flag,
                        |_: &u32| 4,
                        || {
                            started.send(()).unwrap();
                            wait_finish.recv().unwrap();
                            if fail {
                                Err(io::Error::other("old failure"))
                            } else {
                                Ok(1_u32)
                            }
                        },
                    )
                });
                wait_started.recv().unwrap();
                let current = cache
                    .get_entry(1, &fail, &1, flag, |_: &u32| 4, || Ok(2_u32))
                    .unwrap();
                finish.send(()).unwrap();
                assert_eq!(old.join().unwrap().is_err(), fail);
                let again = cache
                    .get_entry(
                        1,
                        &fail,
                        &1,
                        flag,
                        |_: &u32| 4,
                        || panic!("replacement retained"),
                    )
                    .unwrap();
                assert!(Arc::ptr_eq(&current, &again));
            });
        }
        let (bytes, bound, computed) = usage(&source, "test-replacement");
        assert_eq!(bytes, bound);
        assert_eq!(computed, 2);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn prepared_page_facts_can_be_extracted_and_reused_without_recording_decodes() {
        let (source, root) = file_source(replay(vec![frame(1, 0, None)]));
        let cache = scope(&source, "test-prepared", None);
        let block = &source.replay_ref().index[0];
        let flag = AtomicBool::new(false);
        for _ in 0..2 {
            let (answer, refused) = super::super::progressive::without_block_decodes(|| {
                cache.get_query(block, &(), &flag, |_: &u32| 4, || Ok(9_u32))
            });
            assert!(!refused);
            assert_eq!(*answer.unwrap(), 9);
        }
        assert_eq!(usage(&source, "test-prepared").2, 1);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn request_keys_share_one_extraction_and_live_capacity_bounds_retention() {
        let (source, root) = file_source(replay(vec![frame(1, 0, None), frame(1, 1, None)]));
        let block = &source.replay_ref().index[0];
        let scope = scope(&source, "test", None);
        let reads = AtomicUsize::new(0);
        let flag = AtomicBool::new(false);
        std::thread::scope(|threads| {
            let mut jobs = Vec::new();
            for _ in 0..8 {
                jobs.push(threads.spawn(|| {
                    scope
                        .get(
                            block,
                            &0,
                            &flag,
                            |value: &Vec<u8>| value.capacity(),
                            || {
                                reads.fetch_add(1, Ordering::Relaxed);
                                Ok(vec![42; 5000])
                            },
                        )
                        .unwrap()
                }));
            }
            for job in jobs {
                assert_eq!(job.join().unwrap().as_slice(), vec![42; 5000]);
            }
        });
        assert_eq!(reads.load(Ordering::Relaxed), 1);
        // Different parameters share the same one-block budget, not one budget
        // each. An evicted observation is recomputed when it is requested again.
        for parameter in 1..20 {
            scope
                .get(
                    block,
                    &parameter,
                    &flag,
                    |value: &Vec<u8>| value.capacity(),
                    || Ok(vec![0; 5000]),
                )
                .unwrap();
            let (bytes, bound, _) = usage(&source, "test");
            assert!(bytes > 0 && bytes <= bound);
        }
        assert_eq!(usage(&source, "test").2, 20);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn content_binding_and_recording_growth_invalidate_raw_observations() {
        let recording = replay(vec![frame(1, 0, None), frame(1, 1, None)]);
        let (source, root) = file_source(recording);
        let content = root.join("synthetic-content.txt");
        std::fs::write(&content, "one").unwrap();
        let first =
            store::SourceKey::capture(source.path(), std::slice::from_ref(&content)).unwrap();
        bind(&source, &first);
        let flag = AtomicBool::new(false);
        let read = || {
            scope(&source, "test", None)
                .get(
                    &source.replay_ref().index[0],
                    &(),
                    &flag,
                    |_: &u32| std::mem::size_of::<u32>(),
                    || Ok(7),
                )
                .unwrap()
        };
        read();
        read();
        assert_eq!(usage(&source, "test").2, 1);
        std::fs::write(&content, "changed content").unwrap();
        bind(&source, &first.recapture().unwrap());
        read();
        assert_eq!(usage(&source, "test").2, 2);
        let recording = source.replay_ref();
        let grown = recording
            .append(&[crate::test_fixtures::continuation_records(
                recording,
                1,
                vec![frame(1, 2, None)],
            )])
            .unwrap();
        std::fs::write(source.path(), grown.bytes()).unwrap();
        let next = Source::from_replay(
            crate::SourceKey {
                source_id: source.key().source_id.clone(),
                revision: crate::protocol::replay_revision(source.path()).unwrap(),
            },
            source.path().to_owned(),
            Arc::new(grown),
            &flag,
        )
        .unwrap();
        next.inherit_append_states(&source);
        assert_eq!(usage(&next, "test"), (0, 0, 0));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn cached_answers_need_no_decodes_and_failed_or_cancelled_extractions_are_not_kept() {
        let (source, root) = file_source(replay(vec![frame(1, 0, None)]));
        let scope = scope(&source, "test", None);
        let block = &source.replay_ref().index[0];
        let flag = AtomicBool::new(false);
        scope.get(block, &0, &flag, |_: &u32| 4, || Ok(9)).unwrap();
        let (answer, refused) = super::super::progressive::without_block_decodes(|| {
            scope.get(block, &0, &flag, |_: &u32| 4, || panic!("cached"))
        });
        assert!(!refused);
        assert_eq!(*answer.unwrap(), 9);
        let (answer, refused) = super::super::progressive::without_block_decodes(|| {
            scope.get(block, &1, &flag, |_: &u32| 4, || Ok(1))
        });
        assert!(refused);
        assert_eq!(answer.unwrap_err().kind(), io::ErrorKind::WouldBlock);
        for _ in 0..2 {
            assert!(
                scope
                    .get::<u32>(
                        block,
                        &2,
                        &flag,
                        |_| 4,
                        || Err(io::Error::other("unreadable"))
                    )
                    .is_err()
            );
        }
        assert_eq!(usage(&source, "test").2, 1);
        flag.store(true, Ordering::Relaxed);
        assert_eq!(
            scope
                .get(block, &0, &flag, |_: &u32| 4, || Ok(0))
                .unwrap_err()
                .kind(),
            io::ErrorKind::Interrupted
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
