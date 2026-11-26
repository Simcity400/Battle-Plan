use crate::country_input_changes::{self, Marks};
use std::{
    collections::HashMap,
    hash::{BuildHasherDefault, Hasher},
    sync::{
        RwLock, RwLockReadGuard,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
};

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
    revision: AtomicU64,
    active: AtomicUsize,
    pending_revision: AtomicU64,
    pending_active: AtomicUsize,
}
static OWNERS: RwLock<Option<FastMap<usize, Entry>>> = RwLock::new(None);
static EPOCH: AtomicU64 = AtomicU64::new(0);
static ENABLED: AtomicBool = AtomicBool::new(false);

pub fn enabled(value: bool) {
    ENABLED.store(value, Ordering::SeqCst);
    invalidate();
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
        return (entry.active.load(Ordering::SeqCst) == 0).then(|| {
            (
                EPOCH.load(Ordering::SeqCst),
                entry.revision.load(Ordering::SeqCst),
            )
        });
    }
    let mut entries = OWNERS.try_write().ok()?;
    let entries = entries.get_or_insert_with(FastMap::default);
    if !entries.contains_key(&store) {
        engine_allocation::reserve_map(entries, 1).ok()?;
        entries.insert(
            store,
            Entry {
                owner,
                id,
                revision: AtomicU64::new(0),
                active: AtomicUsize::new(0),
                pending_revision: AtomicU64::new(0),
                pending_active: AtomicUsize::new(0),
            },
        );
    }
    let entry = entries.get_mut(&store)?;
    if entry.owner != owner || entry.id != id {
        entry.owner = owner;
        entry.id = id;
        entry.revision.fetch_add(1, Ordering::SeqCst);
    }
    if entry.active.load(Ordering::SeqCst) != 0 {
        return None;
    }
    Some((
        EPOCH.load(Ordering::SeqCst),
        entry.revision.load(Ordering::SeqCst),
    ))
}

pub struct Write {
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
            entry.active.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

pub fn writing(store: usize) -> Write {
    match OWNERS.try_read() {
        Ok(entries) => {
            let marks =
                if let Some(entry) = entries.as_ref().and_then(|entries| entries.get(&store)) {
                    entry.active.fetch_add(1, Ordering::SeqCst);
                    entry.revision.fetch_add(1, Ordering::SeqCst);
                    country_input_changes::project_variables_written(Some(entry.owner))
                } else {
                    Marks::none()
                };
            Write {
                entries: Some(entries),
                store,
                _marks: marks,
            }
        }
        Err(_) => {
            EPOCH.fetch_add(1, Ordering::SeqCst);
            Write {
                entries: None,
                store,
                _marks: country_input_changes::project_variables_written(None),
            }
        }
    }
}

pub fn invalidate() {
    EPOCH.fetch_add(1, Ordering::SeqCst);
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
        entries.retain(|_, entry| keep(entry.owner, entry.id));
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
            let entry = entry.filter(|entry| entry.owner == owner && entry.id == id)?;
            (entry.active.load(Ordering::SeqCst) == 0)
                .then(|| (epoch, entry.revision.load(Ordering::SeqCst)))
        };
        let pending_stamp = if !project_hooks {
            None
        } else {
            entry.and_then(|entry| {
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
}
pub fn pending_stamp(store: usize) -> Option<(u64, u64)> {
    if !PROJECT_HOOKS.load(Ordering::SeqCst) {
        return None;
    }
    let entries = OWNERS.try_read().ok()?;
    let entry = entries.as_ref()?.get(&store)?;
    let stamp = (
        EPOCH.load(Ordering::SeqCst),
        entry.pending_revision.load(Ordering::SeqCst),
    );
    (entry.pending_active.load(Ordering::SeqCst) == 0).then_some(stamp)
}
pub struct ProjectWrite {
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
            entry.pending_active.fetch_sub(1, Ordering::SeqCst);
        }
    }
}
pub fn writing_project(project: usize) -> ProjectWrite {
    let store = project
        .checked_add(engine_projects::nproject_cproject::VARIABLES)
        .unwrap_or(0);
    match OWNERS.try_read() {
        Ok(entries) => {
            if let Some(entry) = entries.as_ref().and_then(|entries| entries.get(&store)) {
                entry.pending_active.fetch_add(1, Ordering::SeqCst);
                entry.pending_revision.fetch_add(1, Ordering::SeqCst);
            }
            ProjectWrite {
                entries: Some(entries),
                store,
            }
        }
        Err(_) => {
            EPOCH.fetch_add(1, Ordering::SeqCst);
            ProjectWrite {
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
