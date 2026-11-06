//! Compact block observations and branch answers shared by queries. A source
//! revision owns its cache; append does not inherit it. A content binding change
//! discards that product's observations. Request keys name every input affecting
//! the observation, excluding the prepared prefix that only clips its use.
//!
//! A product keeps the latest branch answer of each kind on a branch, a
//! continuation's lineage its latest, and the latest observation asked of each
//! block; an earlier one lives only while someone holds it, so nothing grows
//! with the requests made. Concurrent asks for one key share one extraction.

use crate::{Source, metadata::check_cancel};
use replay::container::BlockIndex;
use serde::Serialize;
use std::{
    any::{Any, TypeId},
    collections::BTreeMap,
    io,
    sync::{Arc, Mutex, Weak, atomic::AtomicBool},
};

type Value = Arc<dyn Any + Send + Sync>;

/// The publication of store sections, from the manifest alone without opening their pages: every
/// range's identity, which a rewritten, appended or emptied range changes.
pub fn published(
    store: &store::Store,
    key: &store::SourceKey,
    sections: &[&str],
) -> io::Result<[u8; 32]> {
    let mut hash = blake3::Hasher::new();
    if let Some(session) = store.session(key)? {
        for section in sections {
            let count = session.range_count(section).unwrap_or(0);
            hash.update(&count.to_le_bytes());
            for range in 0..count {
                let id = session
                    .range_id(section, range)?
                    .map_or([u64::MAX; 3], |id| id.cache_key());
                for part in id {
                    hash.update(&part.to_le_bytes());
                }
            }
        }
    }
    Ok(*hash.finalize().as_bytes())
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

/// A completed value: the latest of its block, its lineage or its branch answers is kept; one a
/// later one replaced lives only while someone holds it.
enum Held {
    Kept(Value),
    Shared(Weak<dyn Any + Send + Sync>),
}

impl Held {
    fn value(&self) -> Option<Value> {
        match self {
            Self::Kept(value) => Some(Arc::clone(value)),
            Self::Shared(value) => value.upgrade(),
        }
    }
}

type Slot = Arc<Mutex<Option<Held>>>;

struct Entry {
    slot: Slot,
    publication: Option<Vec<u8>>,
    lineage: Option<Vec<u8>>,
}

#[derive(Default)]
struct Product {
    binding: Option<store::SourceKey>,
    generation: u64,
    #[cfg(any(test, feature = "test-support"))]
    computations: usize,
}

#[derive(Default)]
struct Cache {
    binding: Option<store::SourceKey>,
    products: BTreeMap<&'static str, Product>,
    entries: BTreeMap<Key, Entry>,
    /// Entries the last sweep left: the next sweeps once there are twice as many.
    swept: usize,
}

impl Cache {
    /// Drops entries nobody holds, read or waits on, once the map has doubled since the last
    /// sweep: it stays within twice what is held, at a constant cost a completed value.
    fn sweep(&mut self) {
        if self.entries.len() <= 2 * self.swept.max(8) {
            return;
        }
        self.entries.retain(|_, entry| {
            Arc::strong_count(&entry.slot) > 1
                || match entry.slot.try_lock() {
                    Ok(held) => held.as_ref().is_some_and(|held| held.value().is_some()),
                    Err(_) => true,
                }
        });
        self.swept = self.entries.len();
    }
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
    cache.swept = 0;
    for product in cache.products.values_mut() {
        product.generation += 1;
    }
}

pub struct Scope {
    cache: Arc<Mutex<Cache>>,
    product: &'static str,
    generation: u64,
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
        cache.entries.retain(|key, _| key.product != product);
    }
    let generation = cache.products[product].generation;
    drop(cache);
    Scope {
        cache: shared,
        product,
        generation,
    }
}

impl Scope {
    /// Extract an immutable observation of `block`, kept until another of the
    /// block replaces it, then shared while anyone holds it. Look-backs
    /// may share a block's fold/anchor facts or a proved window by putting its
    /// end hour and relevant selection in `params`; no decoder or predecessor
    /// is assumed.
    pub fn get<T: Send + Sync + 'static>(
        &self,
        block: &BlockIndex,
        params: &impl Serialize,
        cancelled: &AtomicBool,
        extract: impl FnOnce() -> io::Result<T>,
    ) -> io::Result<Arc<T>> {
        self.get_inner(
            block.branch,
            Some((block.first_hour, block.last_hour)),
            params,
            None,
            cancelled,
            |_| extract(),
            true,
            None,
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
        extract: impl FnOnce() -> io::Result<T>,
    ) -> io::Result<Arc<T>> {
        self.get_inner(
            block.branch,
            Some((block.first_hour, block.last_hour)),
            params,
            None,
            cancelled,
            |_| extract(),
            false,
            None,
        )
    }

    /// The current branch-wide answer for one request, kept until a new
    /// publication or another request's answer of its kind replaces it, then
    /// shared while anyone holds it. A superseded in-flight extraction's
    /// callers can finish, but cannot keep it. The extractor enforces the
    /// recording-read boundary as `get_query` does.
    pub fn get_entry<T: Send + Sync + 'static>(
        &self,
        branch: u64,
        params: &impl Serialize,
        publication: &impl Serialize,
        cancelled: &AtomicBool,
        extract: impl FnOnce() -> io::Result<T>,
    ) -> io::Result<Arc<T>> {
        self.get_inner(
            branch,
            None,
            params,
            Some(serde_json::to_vec(publication)?),
            cancelled,
            |_| extract(),
            false,
            None,
        )
    }

    /// Replace a publication while offering its completed predecessor to an
    /// incremental reader; a request without one is offered the latest
    /// completed answer of the same `lineage` on the branch. Only that latest
    /// answer is kept; the others live while someone holds them. A busy
    /// predecessor is skipped without waiting; the extractor must validate
    /// every part it reuses against the new publication and request.
    #[allow(clippy::too_many_arguments)]
    pub fn continue_entry<T: Send + Sync + 'static>(
        &self,
        branch: u64,
        params: &impl Serialize,
        publication: &impl Serialize,
        lineage: &impl Serialize,
        cancelled: &AtomicBool,
        extract: impl FnOnce(Option<Arc<T>>) -> io::Result<T>,
    ) -> io::Result<Arc<T>> {
        self.get_inner(
            branch,
            None,
            params,
            Some(serde_json::to_vec(publication)?),
            cancelled,
            extract,
            false,
            Some(serde_json::to_vec(lineage)?),
        )
    }

    /// Find a block observation kept or held now without waiting or starting
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
            let cache = self.cache.lock().unwrap_or_else(|error| error.into_inner());
            let Some(entry) = cache.entries.get(&key) else {
                return Ok(None);
            };
            Arc::clone(&entry.slot)
        };
        let held = match slot.try_lock() {
            Ok(held) => held,
            Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return Ok(None),
        };
        held.as_ref()
            .and_then(Held::value)
            .map(|value| {
                value
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
        extract: impl FnOnce(Option<Arc<T>>) -> io::Result<T>,
        recording: bool,
        lineage: Option<Vec<u8>>,
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
            let gone = cache.entries.get(&key).is_some_and(|entry| {
                entry.publication != publication
                    || entry.slot.try_lock().is_ok_and(|held| {
                        held.as_ref().is_some_and(|held| held.value().is_none())
                    })
            });
            if gone {
                let old = cache.entries.remove(&key).unwrap();
                // No lock is held across extraction. A superseded in-flight
                // reply still belongs to its original callers.
                if let Ok(held) = old.slot.try_lock() {
                    previous = held
                        .as_ref()
                        .and_then(Held::value)
                        .and_then(|value| value.downcast::<T>().ok());
                }
            }
            if lineage.is_some() && previous.is_none() && !cache.entries.contains_key(&key) {
                // The latest completed answer of the lineage is the one it keeps.
                previous = cache
                    .entries
                    .iter()
                    .filter(|(other, entry)| {
                        other.product == key.product
                            && other.generation == key.generation
                            && other.branch == key.branch
                            && other.block.is_none()
                            && other.kind == key.kind
                            && entry.lineage == lineage
                    })
                    .find_map(|(_, entry)| {
                        let held = entry.slot.try_lock().ok()?;
                        match held.as_ref()? {
                            Held::Kept(value) => Arc::clone(value).downcast::<T>().ok(),
                            Held::Shared(_) => None,
                        }
                    });
            }
            if recording && !cache.entries.contains_key(&key) {
                super::progressive::decodes_allowed()?;
            }
            let entry = cache.entries.entry(key.clone()).or_insert_with(|| Entry {
                slot: Arc::new(Mutex::new(None)),
                publication: publication.clone(),
                lineage: lineage.clone(),
            });
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
        if let Some(value) = held.as_ref().and_then(Held::value) {
            return value
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
        let shared: Value = value.clone();
        *held = Some(Held::Kept(shared));
        drop(held);
        let mut cache = self.cache.lock().unwrap_or_else(|error| error.into_inner());
        if cache.products[&self.product].generation == self.generation
            && cache
                .entries
                .get(&key)
                .is_some_and(|entry| Arc::ptr_eq(&entry.slot, &slot))
        {
            #[cfg(any(test, feature = "test-support"))]
            {
                cache.products.get_mut(self.product).unwrap().computations += 1;
            }
            // Earlier values of the block, the lineage or the branch's answers of this kind
            // live on only while someone holds them.
            for (other, entry) in &cache.entries {
                if other != &key
                    && other.product == key.product
                    && other.branch == key.branch
                    && other.block == key.block
                    && other.kind == key.kind
                    && entry.lineage == lineage
                    && let Ok(mut held) = entry.slot.try_lock()
                    && let Some(Held::Kept(value)) = held.as_ref()
                {
                    let weak = Arc::downgrade(value);
                    *held = Some(Held::Shared(weak));
                }
            }
            cache.sweep();
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

/// Completed extractions of `product` so far.
#[cfg(any(test, feature = "test-support"))]
pub fn computations(source: &Source, product: &'static str) -> usize {
    let shared = source.state::<Cache>();
    let cache = shared.lock().unwrap_or_else(|error| error.into_inner());
    cache
        .products
        .get(product)
        .map_or(0, |state| state.computations)
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
                    &"lineage",
                    &flag,
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
                        &"lineage",
                        &flag,
                        |_: Option<Arc<u32>>| panic!("unchanged publication recomputed")
                    )
                    .unwrap(),
                publication
            );
        }
        assert_eq!(computations(&source, "test-continuation"), 4);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_new_request_continues_from_its_lineages_latest_answer_on_the_branch() {
        let (source, root) = file_source(replay(vec![frame(1, 0, None)]));
        let flag = AtomicBool::new(false);
        let block = &source.replay_ref().index[0];
        let cache = scope(&source, "test-recent", None);
        let other = scope(&source, "test-recent-other", None);
        let ask = |cache: &Scope, branch: u64, hour: u32, lineage: &str, expected: Option<u32>| {
            *cache
                .continue_entry(
                    branch,
                    &hour,
                    &0,
                    &lineage,
                    &flag,
                    |old: Option<Arc<u32>>| {
                        assert_eq!(old.as_deref().copied(), expected, "hour {hour}");
                        Ok(hour)
                    },
                )
                .unwrap()
        };
        let _block = cache.get(block, &(), &flag, || Ok(99_u32)).unwrap();
        other.get_entry(1, &(), &(), &flag, || Ok(77_u32)).unwrap();
        assert_eq!(ask(&cache, 1, 10, "a", None), 10, "blocks and other products are not offered");
        assert_eq!(ask(&cache, 1, 11, "a", Some(10)), 11);
        assert_eq!(ask(&cache, 1, 20, "b", None), 20, "another lineage starts afresh");
        assert_eq!(ask(&cache, 1, 12, "a", Some(11)), 12);
        assert_eq!(ask(&cache, 2, 12, "a", None), 12, "another branch starts afresh");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_lineage_keeps_only_its_latest_answer() {
        let (source, root) = file_source(replay(vec![frame(1, 0, None)]));
        let flag = AtomicBool::new(false);
        let cache = scope(&source, "test-lineage", None);
        let mut answers = Vec::new();
        for hour in 0..64_u32 {
            let answer = cache
                .continue_entry(1, &hour, &0, &"lineage", &flag, |_| Ok(hour))
                .unwrap();
            answers.push(Arc::downgrade(&answer));
        }
        assert!(answers[..63].iter().all(|answer| answer.upgrade().is_none()));
        assert_eq!(answers[63].upgrade().as_deref(), Some(&63));
        assert!(cache.cache.lock().unwrap().entries.len() <= 2 * 8 + 1);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn branch_entries_are_distinct_from_blocks_and_share_the_source_binding() {
        let (source, root) = file_source(replay(vec![frame(1, 0, None)]));
        let block = &source.replay_ref().index[0];
        let flag = AtomicBool::new(false);
        let cache = scope(&source, "test-entries", None);
        assert_eq!(*cache.get_query(block, &(), &flag, || Ok(11_u32)).unwrap(), 11);
        assert_eq!(
            *cache
                .get_entry(block.branch, &(), &(), &flag, || Ok(22_u32))
                .unwrap(),
            22
        );
        assert_eq!(
            cache
                .get_cached::<u32>(block, &(), &flag)
                .unwrap()
                .as_deref(),
            Some(&11),
            "the block's latest observation is kept beside the branch answer"
        );
        let (answer, refused) = super::super::progressive::without_block_decodes(|| {
            cache.get_entry::<u32>(block.branch, &(), &(), &flag, || panic!("held answer"))
        });
        assert!(!refused);
        assert_eq!(*answer.unwrap(), 22);
        assert_eq!(computations(&source, "test-entries"), 2);
        assert_eq!(*cache.get_entry(2, &(), &(), &flag, || Ok(33_u32)).unwrap(), 33);
        let computed = computations(&source, "test-entries");
        assert_eq!(computed, 3, "branches and blocks have separate keys");
        let binding = store::SourceKey::capture(source.path(), &[]).unwrap();
        bind(&source, &binding);
        let cache = scope(&source, "test-entries", None);
        assert_eq!(*cache.get_entry(2, &(), &(), &flag, || Ok(44_u32)).unwrap(), 44);
        assert_eq!(computations(&source, "test-entries"), computed + 1);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn branch_requests_replace_superseded_publications() {
        let (source, root) = file_source(replay(vec![frame(1, 0, None)]));
        let cache = scope(&source, "test-replies", None);
        let flag = AtomicBool::new(false);
        let mut previous = Vec::new();
        for publication in 0..27 {
            for request in ["timeline", "moment"] {
                let answer = cache
                    .get_entry(1, &request, &publication, &flag, || {
                        Ok(vec![42; (publication + 1) * 1024])
                    })
                    .unwrap();
                previous.push(Arc::downgrade(&answer));
            }
            assert!(
                previous[..previous.len() - 2]
                    .iter()
                    .all(|value| value.upgrade().is_none()),
                "superseded replies must be released"
            );
            assert_eq!(computations(&source, "test-replies"), 2 * (publication + 1));
            assert_eq!(cache.cache.lock().unwrap().entries.len(), 2);
        }
        let held = cache.get_entry(1, &"timeline", &28, &flag, || Ok(vec![1])).unwrap();
        cache.get_entry(1, &"moment", &28, &flag, || Ok(vec![2])).unwrap();
        let (answer, refused) = super::super::progressive::without_block_decodes(|| {
            cache.get_entry::<Vec<i32>>(1, &"moment", &28, &flag, || panic!("current reply"))
        });
        assert!(!refused);
        assert_eq!(answer.unwrap().as_slice(), &[2]);
        let again = cache
            .get_entry(1, &"timeline", &28, &flag, || panic!("held reply"))
            .unwrap();
        assert!(Arc::ptr_eq(&held, &again), "an earlier request's answer is shared while held");
        drop((held, again));
        cache
            .get_entry::<Vec<i32>>(1, &"moment", &28, &flag, || panic!("current reply"))
            .unwrap();
        let mut asked = Vec::new();
        for request in 0..200_u32 {
            let answer = cache.get_entry(1, &request, &28, &flag, || Ok(vec![0])).unwrap();
            asked.push(Arc::downgrade(&answer));
        }
        assert!(asked[..199].iter().all(|answer| answer.upgrade().is_none()));
        assert!(asked[199].upgrade().is_some(), "the latest request's answer is kept");
        assert!(cache.cache.lock().unwrap().entries.len() <= 2 * 8 + 1);
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
                    cache.get_entry(1, &fail, &0, flag, || {
                        started.send(()).unwrap();
                        wait_finish.recv().unwrap();
                        if fail {
                            Err(io::Error::other("old failure"))
                        } else {
                            Ok(1_u32)
                        }
                    })
                });
                wait_started.recv().unwrap();
                let current = cache.get_entry(1, &fail, &1, flag, || Ok(2_u32)).unwrap();
                finish.send(()).unwrap();
                assert_eq!(old.join().unwrap().is_err(), fail);
                let again = cache
                    .get_entry(1, &fail, &1, flag, || panic!("replacement retained"))
                    .unwrap();
                assert!(Arc::ptr_eq(&current, &again));
            });
        }
        assert_eq!(computations(&source, "test-replacement"), 2);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn prepared_page_facts_are_reused_without_recording_decodes() {
        let (source, root) = file_source(replay(vec![frame(1, 0, None)]));
        let cache = scope(&source, "test-prepared", None);
        let block = &source.replay_ref().index[0];
        let flag = AtomicBool::new(false);
        for _ in 0..2 {
            let (answer, refused) = super::super::progressive::without_block_decodes(|| {
                cache.get_query(block, &(), &flag, || Ok(9_u32))
            });
            assert!(!refused);
            assert_eq!(*answer.unwrap(), 9);
        }
        assert_eq!(computations(&source, "test-prepared"), 1);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn concurrent_asks_share_one_extraction_and_a_block_keeps_only_its_latest() {
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
                        .get(block, &0, &flag, || {
                            reads.fetch_add(1, Ordering::Relaxed);
                            Ok(vec![42; 5000])
                        })
                        .unwrap()
                }));
            }
            for job in jobs {
                assert_eq!(job.join().unwrap().as_slice(), vec![42; 5000]);
            }
        });
        assert_eq!(reads.load(Ordering::Relaxed), 1);
        let mut asked = Vec::new();
        for parameter in 1..200 {
            let value = scope.get(block, &parameter, &flag, || Ok(vec![0; 5000])).unwrap();
            asked.push(Arc::downgrade(&value));
        }
        assert_eq!(computations(&source, "test"), 200);
        assert!(asked[..198].iter().all(|value| value.upgrade().is_none()));
        assert!(asked[198].upgrade().is_some(), "the block's latest is kept");
        assert!(scope.cache.lock().unwrap().entries.len() <= 2 * 8 + 1);
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
                .get(&source.replay_ref().index[0], &(), &flag, || Ok(7_u32))
                .unwrap()
        };
        read();
        read();
        assert_eq!(computations(&source, "test"), 1);
        std::fs::write(&content, "changed content").unwrap();
        bind(&source, &first.recapture().unwrap());
        read();
        assert_eq!(computations(&source, "test"), 2);
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
        assert_eq!(computations(&next, "test"), 0);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn kept_answers_need_no_decodes_and_failed_or_cancelled_extractions_are_not_kept() {
        let (source, root) = file_source(replay(vec![frame(1, 0, None)]));
        let scope = scope(&source, "test", None);
        let block = &source.replay_ref().index[0];
        let flag = AtomicBool::new(false);
        scope.get(block, &0, &flag, || Ok(9_u32)).unwrap();
        let (answer, refused) = super::super::progressive::without_block_decodes(|| {
            scope.get::<u32>(block, &0, &flag, || panic!("kept"))
        });
        assert!(!refused);
        assert_eq!(*answer.unwrap(), 9);
        let (answer, refused) = super::super::progressive::without_block_decodes(|| {
            scope.get(block, &1, &flag, || Ok(1_u32))
        });
        assert!(refused);
        assert_eq!(answer.unwrap_err().kind(), io::ErrorKind::WouldBlock);
        for _ in 0..2 {
            assert!(
                scope
                    .get::<u32>(block, &2, &flag, || Err(io::Error::other("unreadable")))
                    .is_err()
            );
        }
        assert_eq!(computations(&source, "test"), 1);
        flag.store(true, Ordering::Relaxed);
        assert_eq!(
            scope
                .get(block, &0, &flag, || Ok(0_u32))
                .unwrap_err()
                .kind(),
            io::ErrorKind::Interrupted
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
