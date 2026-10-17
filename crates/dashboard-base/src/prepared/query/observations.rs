//! Compact block observations shared by sampled and look-back queries. A source
//! revision owns its cache; append does not inherit it. A content binding change
//! discards that product's observations. Request keys name every input affecting
//! the observation, excluding the prepared prefix that only clips its use.
//!
//! Each product keeps at most one pass over the recording's live block count,
//! sized by its largest observed entry (including owned allocation capacities).
//! Different request shapes share that budget and evict least recently used
//! observations. Concurrent asks for one key share one extraction. Values held
//! by callers and in-flight extractions are transient, outside retained bytes.

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
    first: i32,
    last: i32,
    kind: TypeId,
    params: Vec<u8>,
}
struct Entry {
    slot: Slot,
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
        check_cancel(cancelled)?;
        let key = Key {
            product: self.product,
            generation: self.generation,
            branch: block.branch,
            first: block.first_hour,
            last: block.last_hour,
            kind: TypeId::of::<T>(),
            params: serde_json::to_vec(params)?,
        };
        let slot = {
            let mut cache = self.cache.lock().unwrap_or_else(|error| error.into_inner());
            cache.clock += 1;
            let clock = cache.clock;
            if !cache.entries.contains_key(&key) {
                super::progressive::decodes_allowed()?;
            }
            let entry = cache.entries.entry(key.clone()).or_insert_with(|| Entry {
                slot: Arc::new(Mutex::new(None)),
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
                self.cache
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .entries
                    .remove(&key);
            }
            return Err(error);
        }
        if let Some(value) = &*held {
            return Arc::clone(value)
                .downcast::<T>()
                .map_err(|_| crate::invalid("Cached observation type changed"));
        }
        if let Err(error) = super::progressive::decodes_allowed() {
            self.cache
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .entries
                .remove(&key);
            return Err(error);
        }
        let value = match extract().and_then(|value| {
            check_cancel(cancelled)?;
            Ok(value)
        }) {
            Ok(value) => Arc::new(value),
            Err(error) => {
                self.cache
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .entries
                    .remove(&key);
                return Err(error);
            }
        };
        // Two keys can be live during lookup/publication; B-tree nodes and the
        // slot/Arc allocation are charged conservatively at their inline sizes.
        let size = bytes(&value).saturating_add(
            2 * (std::mem::size_of::<Key>()
                + key.params.capacity()
                + std::mem::size_of::<Entry>()
                + std::mem::size_of::<Mutex<Option<Value>>>()
                + 2 * std::mem::size_of::<usize>()),
        );
        let mut cache = self.cache.lock().unwrap_or_else(|error| error.into_inner());
        if cache.products[&self.product].generation == self.generation {
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
            *held = Some(value.clone());
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
        } else {
            cache.entries.remove(&key);
        }
        Ok(value)
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
        let first = store::SourceKey::capture(source.path(), &[content.clone()]).unwrap();
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
