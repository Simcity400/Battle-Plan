use crate::country_input_changes::{self, Marks};
use collector_primitives::asymmetric::{FlightGate, FlightGuard};
use std::{
    collections::HashMap,
    hash::{BuildHasherDefault, Hasher},
    sync::{
        RwLock, RwLockReadGuard,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
};

const BLOOM_WORDS: usize = 256;
const BLOOM_MASK: usize = BLOOM_WORDS * 64 - 1;
static FLIGHT: FlightGate = FlightGate::new(0);
static COUNT: AtomicUsize = AtomicUsize::new(0);
static BLOOM: [AtomicU64; BLOOM_WORDS] = [const { AtomicU64::new(0) }; BLOOM_WORDS];

#[inline(always)]
fn bloom_bits(store: usize) -> (usize, u64, usize, u64) {
    let mut hash = (store as u64).wrapping_mul(0x517c_c1b7_2722_0a95);
    hash ^= hash >> 32;
    let first = (hash as usize) & BLOOM_MASK;
    let second = ((hash >> 14) as usize) & BLOOM_MASK;
    (first >> 6, 1_u64 << (first & 63), second >> 6, 1_u64 << (second & 63))
}

#[inline(always)]
fn bloom_contains(store: usize) -> bool {
    let (w0, m0, w1, m1) = bloom_bits(store);
    (BLOOM[w0].load(Ordering::Relaxed) & m0 != 0) && (BLOOM[w1].load(Ordering::Relaxed) & m1 != 0)
}

fn bloom_insert(store: usize) {
    let (w0, m0, w1, m1) = bloom_bits(store);
    BLOOM[w0].fetch_or(m0, Ordering::SeqCst);
    BLOOM[w1].fetch_or(m1, Ordering::SeqCst);
}

#[derive(Default)]
struct FastHasher(u64);

impl Hasher for FastHasher {
    #[inline]
    fn finish(&self) -> u64 {
        self.0
    }

    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        let mut state = self.0;
        for &byte in bytes {
            state = (state ^ u64::from(byte)).wrapping_mul(0x517c_c1b7_2722_0a95);
        }
        self.0 = state;
    }

    #[inline]
    fn write_usize(&mut self, value: usize) {
        self.0 = (self.0 ^ (value as u64)).wrapping_mul(0x517c_c1b7_2722_0a95);
        self.0 ^= self.0 >> 32;
    }
}

type FastMap<K, V> = HashMap<K, V, BuildHasherDefault<FastHasher>>;

struct Entry {
    owner: u32,
    id: u32,
    born_quiet: AtomicBool,
    revision: AtomicU64,
    active: AtomicUsize,
    pending_revision: AtomicU64,
    pending_active: AtomicUsize,
}
static OWNERS: RwLock<Option<FastMap<usize, Entry>>> = RwLock::new(None);
static EPOCH: AtomicU64 = AtomicU64::new(0);
static GLOBAL_CHANGE: AtomicU64 = AtomicU64::new(0);
static GLOBAL_ACTIVE: AtomicUsize = AtomicUsize::new(0);
static ENABLED: AtomicBool = AtomicBool::new(false);

pub fn global_quiet() -> Option<(u64, u64)> {
    if !ENABLED.load(Ordering::Acquire)
        || !PROJECT_HOOKS.load(Ordering::Acquire)
        || GLOBAL_ACTIVE.load(Ordering::SeqCst) != 0
    {
        return None;
    }
    FLIGHT.quiet_epoch()?;
    let change = GLOBAL_CHANGE.load(Ordering::SeqCst);
    let epoch = EPOCH.load(Ordering::SeqCst);
    if GLOBAL_ACTIVE.load(Ordering::SeqCst) != 0
        || GLOBAL_CHANGE.load(Ordering::SeqCst) != change
        || FLIGHT.quiet_epoch().is_none()
    {
        return None;
    }
    Some((epoch, change))
}

pub fn enabled(value: bool) {
    ENABLED.store(value, Ordering::SeqCst);
    invalidate();
}

fn finalize_born_quiet(store: usize, owner: u32, id: u32) -> Option<(u64, u64)> {
    FLIGHT.quiet_epoch()?;
    let entries = OWNERS.try_read().ok()?;
    let entry = entries.as_ref()?.get(&store)?;
    if entry.owner != owner || entry.id != id {
        return None;
    }
    entry.born_quiet.store(true, Ordering::SeqCst);
    let stamp = (
        EPOCH.load(Ordering::SeqCst),
        entry.revision.load(Ordering::SeqCst),
    );
    (entry.active.load(Ordering::SeqCst) == 0).then_some(stamp)
}

pub fn register(store: usize, owner: u32, id: u32) -> Option<(u64, u64)> {
    if !ENABLED.load(Ordering::SeqCst) {
        return None;
    }
    if let Ok(entries) = OWNERS.try_read()
        && let Some(entry) = entries.as_ref().and_then(|entries| entries.get(&store))
        && entry.owner == owner
        && entry.id == id
    {
        if entry.born_quiet.load(Ordering::SeqCst) {
            let stamp = (
                EPOCH.load(Ordering::SeqCst),
                entry.revision.load(Ordering::SeqCst),
            );
            return (entry.active.load(Ordering::SeqCst) == 0).then_some(stamp);
        }
        drop(entries);
        return finalize_born_quiet(store, owner, id);
    }
    {
        let mut entries = OWNERS.try_write().ok()?;
        let entries = entries.get_or_insert_with(FastMap::default);
        if !entries.contains_key(&store) {
            engine_allocation::reserve_map(entries, 1).ok()?;
            entries.insert(
                store,
                Entry {
                    owner,
                    id,
                    born_quiet: AtomicBool::new(false),
                    revision: AtomicU64::new(0),
                    active: AtomicUsize::new(0),
                    pending_revision: AtomicU64::new(0),
                    pending_active: AtomicUsize::new(0),
                },
            );
            bloom_insert(store);
            COUNT.store(entries.len(), Ordering::SeqCst);
        }
        let entry = entries.get_mut(&store)?;
        if entry.owner != owner || entry.id != id {
            entry.owner = owner;
            entry.id = id;
            entry.revision.fetch_add(1, Ordering::SeqCst);
            entry.pending_revision.fetch_add(1, Ordering::SeqCst);
            GLOBAL_CHANGE.fetch_add(1, Ordering::SeqCst);
        }
        if entry.born_quiet.load(Ordering::SeqCst) {
            let stamp = (
                EPOCH.load(Ordering::SeqCst),
                entry.revision.load(Ordering::SeqCst),
            );
            return (entry.active.load(Ordering::SeqCst) == 0).then_some(stamp);
        }
    }
    finalize_born_quiet(store, owner, id)
}

pub struct Write {
    _flight: FlightGuard<'static>,
    entries: Option<RwLockReadGuard<'static, Option<FastMap<usize, Entry>>>>,
    store: usize,
    _marks: Marks<'static>,
}
impl Drop for Write {
    fn drop(&mut self) {
        if let Some(entry) = self
            .entries
            .as_ref()
            .and_then(|entries| entries.as_ref())
            .and_then(|entries| entries.get(&self.store))
        {
            entry.revision.fetch_add(1, Ordering::SeqCst);
            GLOBAL_CHANGE.fetch_add(1, Ordering::SeqCst);
            entry.active.fetch_sub(1, Ordering::SeqCst);
            GLOBAL_ACTIVE.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

#[inline]
pub fn writing(store: usize) -> Write {
    let flight = FLIGHT.enter(false);
    if COUNT.load(Ordering::Relaxed) == 0 || !bloom_contains(store) {
        return Write {
            _flight: flight,
            entries: None,
            store,
            _marks: Marks::none(),
        };
    }
    writing_slow(flight, store)
}

#[cold]
#[inline(never)]
fn writing_slow(flight: FlightGuard<'static>, store: usize) -> Write {
    match OWNERS.try_read() {
        Ok(entries) => {
            if let Some(entry) = entries.as_ref().and_then(|entries| entries.get(&store)) {
                GLOBAL_ACTIVE.fetch_add(1, Ordering::SeqCst);
                entry.active.fetch_add(1, Ordering::SeqCst);
                entry.revision.fetch_add(1, Ordering::SeqCst);
                GLOBAL_CHANGE.fetch_add(1, Ordering::SeqCst);
                let marks = country_input_changes::project_variables_written(Some(entry.owner));
                Write {
                    _flight: flight,
                    entries: Some(entries),
                    store,
                    _marks: marks,
                }
            } else {
                drop(entries);
                Write {
                    _flight: flight,
                    entries: None,
                    store,
                    _marks: Marks::none(),
                }
            }
        }
        Err(_) => {
            EPOCH.fetch_add(1, Ordering::SeqCst);
            GLOBAL_CHANGE.fetch_add(1, Ordering::SeqCst);
            Write {
                _flight: flight,
                entries: None,
                store,
                _marks: country_input_changes::project_variables_written(None),
            }
        }
    }
}

pub fn invalidate() {
    EPOCH.fetch_add(1, Ordering::SeqCst);
    GLOBAL_CHANGE.fetch_add(1, Ordering::SeqCst);
    drop(country_input_changes::project_variables_written(None));
}

pub fn retain_live(mut keep: impl FnMut(u32, u32) -> bool) {
    if let Ok(entries) = OWNERS.try_read() {
        let Some(entries) = entries.as_ref() else {
            return;
        };
        if entries.values().all(|entry| keep(entry.owner, entry.id)) {
            return;
        }
    }
    if let Ok(mut entries) = OWNERS.try_write()
        && let Some(entries) = entries.as_mut()
    {
        let before = entries.len();
        entries.retain(|_, entry| keep(entry.owner, entry.id));
        if entries.len() != before {
            EPOCH.fetch_add(1, Ordering::SeqCst);
            GLOBAL_CHANGE.fetch_add(1, Ordering::SeqCst);
        }
        COUNT.store(entries.len(), Ordering::SeqCst);
    }
}

pub struct StampReader(RwLockReadGuard<'static, Option<FastMap<usize, Entry>>>);

pub fn stamp_reader() -> Option<StampReader> {
    OWNERS.try_read().ok().map(StampReader)
}

impl StampReader {
    #[inline]
    pub fn stamps(
        &self,
        store: usize,
        owner: u32,
        id: u32,
    ) -> Option<(Option<(u64, u64)>, Option<(u64, u64)>)> {
        let enabled = ENABLED.load(Ordering::Acquire);
        let project_hooks = PROJECT_HOOKS.load(Ordering::Acquire);
        let entry = self.0.as_ref().and_then(|entries| entries.get(&store));
        let epoch = if enabled || project_hooks {
            EPOCH.load(Ordering::SeqCst)
        } else {
            0
        };
        let stamp = if !enabled {
            None
        } else {
            let entry = entry.filter(|entry| {
                entry.owner == owner
                    && entry.id == id
                    && entry.born_quiet.load(Ordering::SeqCst)
            })?;
            let stamp = (epoch, entry.revision.load(Ordering::SeqCst));
            (entry.active.load(Ordering::SeqCst) == 0).then_some(stamp)
        };
        let pending_stamp = if !project_hooks {
            None
        } else {
            entry.and_then(|entry| {
                if !entry.born_quiet.load(Ordering::SeqCst) {
                    return None;
                }
                let stamp = (epoch, entry.pending_revision.load(Ordering::SeqCst));
                (entry.pending_active.load(Ordering::SeqCst) == 0).then_some(stamp)
            })
        };
        Some((stamp, pending_stamp))
    }
}

static PROJECT_HOOKS: AtomicBool = AtomicBool::new(false);
pub fn project_hooks_enabled(value: bool) {
    PROJECT_HOOKS.store(value, Ordering::SeqCst);
    EPOCH.fetch_add(1, Ordering::SeqCst);
    GLOBAL_CHANGE.fetch_add(1, Ordering::SeqCst);
}
pub fn pending_stamp(store: usize) -> Option<(u64, u64)> {
    if !PROJECT_HOOKS.load(Ordering::SeqCst) {
        return None;
    }
    let entries = OWNERS.try_read().ok()?;
    let entry = entries.as_ref()?.get(&store)?;
    if !entry.born_quiet.load(Ordering::SeqCst) {
        return None;
    }
    let stamp = (
        EPOCH.load(Ordering::SeqCst),
        entry.pending_revision.load(Ordering::SeqCst),
    );
    (entry.pending_active.load(Ordering::SeqCst) == 0).then_some(stamp)
}
pub struct ProjectWrite {
    _flight: FlightGuard<'static>,
    entries: Option<RwLockReadGuard<'static, Option<FastMap<usize, Entry>>>>,
    store: usize,
}
impl Drop for ProjectWrite {
    fn drop(&mut self) {
        if let Some(entry) = self
            .entries
            .as_ref()
            .and_then(|entries| entries.as_ref())
            .and_then(|entries| entries.get(&self.store))
        {
            entry.pending_revision.fetch_add(1, Ordering::SeqCst);
            GLOBAL_CHANGE.fetch_add(1, Ordering::SeqCst);
            entry.pending_active.fetch_sub(1, Ordering::SeqCst);
            GLOBAL_ACTIVE.fetch_sub(1, Ordering::SeqCst);
        }
    }
}
pub fn writing_project(project: usize) -> ProjectWrite {
    let flight = FLIGHT.enter(false);
    let store = project
        .checked_add(engine_projects::nproject_cproject::VARIABLES)
        .unwrap_or(0);
    if COUNT.load(Ordering::Relaxed) == 0 || !bloom_contains(store) {
        return ProjectWrite {
            _flight: flight,
            entries: None,
            store,
        };
    }
    match OWNERS.try_read() {
        Ok(entries) => {
            if let Some(entry) = entries.as_ref().and_then(|entries| entries.get(&store)) {
                GLOBAL_ACTIVE.fetch_add(1, Ordering::SeqCst);
                entry.pending_active.fetch_add(1, Ordering::SeqCst);
                entry.pending_revision.fetch_add(1, Ordering::SeqCst);
                GLOBAL_CHANGE.fetch_add(1, Ordering::SeqCst);
                ProjectWrite {
                    _flight: flight,
                    entries: Some(entries),
                    store,
                }
            } else {
                drop(entries);
                ProjectWrite {
                    _flight: flight,
                    entries: None,
                    store,
                }
            }
        }
        Err(_) => {
            EPOCH.fetch_add(1, Ordering::SeqCst);
            GLOBAL_CHANGE.fetch_add(1, Ordering::SeqCst);
            ProjectWrite {
                _flight: flight,
                entries: None,
                store,
            }
        }
    }
}

pub struct Reset;
pub fn resetting() -> Reset {
    invalidate();
    Reset
}
impl Drop for Reset {
    fn drop(&mut self) {
        invalidate();
    }
}
