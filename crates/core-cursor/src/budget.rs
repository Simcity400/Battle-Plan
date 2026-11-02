use std::io;
use std::mem::size_of;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Weak};

pub const PLAYBACK_BYTES: usize = 400_000_000;

#[derive(Clone, Copy, Debug)]
#[repr(usize)]
pub enum Owner {
    State,
    Forward,
    Reverse,
    Delta,
    Metadata,
    Transient,
}

struct Ledger {
    limit: usize,
    used: AtomicUsize,
    peak: AtomicUsize,
    owners: [AtomicUsize; 6],
    owner_peaks: [AtomicUsize; 6],
    parent: Option<Memory>,
    _ledger_memory: Option<Reservation>,
}

#[derive(Clone)]
pub struct Memory(Arc<Ledger>, bool, Option<Arc<Mutex<Credit>>>);
struct Credit {
    reservation: Reservation,
    _metadata: Reservation,
}

impl Memory {
    pub fn new(limit: usize) -> io::Result<Self> {
        if limit == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "playback allowance must be positive",
            ));
        }
        Ok(Self::root_with(limit))
    }

    fn root() -> Self {
        Self::root_with(PLAYBACK_BYTES)
    }

    fn root_with(limit: usize) -> Self {
        Self(
            Arc::new(Ledger {
                limit,
                used: AtomicUsize::new(0),
                peak: AtomicUsize::new(0),
                owners: std::array::from_fn(|_| AtomicUsize::new(0)),
                owner_peaks: std::array::from_fn(|_| AtomicUsize::new(0)),
                parent: None,
                _ledger_memory: None,
            }),
            true,
            None,
        )
    }

    pub fn child(parent: &Self, limit: usize) -> io::Result<Self> {
        if limit == 0 || limit > parent.limit() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid working-set allowance",
            ));
        }
        let metadata = parent.reserve(
            Owner::Metadata,
            size_of::<Ledger>() + 2 * size_of::<usize>(),
        )?;
        Ok(Self(
            Arc::new(Ledger {
                limit,
                used: AtomicUsize::new(0),
                peak: AtomicUsize::new(0),
                owners: std::array::from_fn(|_| AtomicUsize::new(0)),
                owner_peaks: std::array::from_fn(|_| AtomicUsize::new(0)),
                parent: Some(parent.clone()),
                _ledger_memory: Some(metadata),
            }),
            true,
            None,
        ))
    }

    pub fn global() -> Self {
        static MEMORY: std::sync::OnceLock<Memory> = std::sync::OnceLock::new();
        MEMORY.get_or_init(Self::root).clone()
    }

    pub fn limit(&self) -> usize {
        self.0.limit
    }
    pub fn used(&self) -> usize {
        self.0.used.load(Ordering::Relaxed)
    }
    pub fn peak(&self) -> usize {
        self.0.peak.load(Ordering::Relaxed)
    }
    pub fn owner_peak(&self, owner: Owner) -> usize {
        self.0.owner_peaks[owner as usize].load(Ordering::Relaxed)
    }
    pub fn owner_used(&self, owner: Owner) -> usize {
        self.0.owners[owner as usize].load(Ordering::Relaxed)
    }

    pub fn reset_peaks(&self) {
        self.0.peak.store(self.used(), Ordering::Relaxed);
        for owner in 0..self.0.owners.len() {
            self.0.owner_peaks[owner].store(
                self.0.owners[owner].load(Ordering::Relaxed),
                Ordering::Relaxed,
            );
        }
    }

    pub fn prepaid(reservation: Reservation) -> io::Result<Self> {
        let memory = reservation.memory.clone();
        let metadata = memory.reserve(
            Owner::Transient,
            size_of::<Mutex<Credit>>() + 2 * size_of::<usize>(),
        )?;
        Ok(Self(
            memory.0,
            memory.1,
            Some(Arc::new(Mutex::new(Credit {
                reservation,
                _metadata: metadata,
            }))),
        ))
    }

    pub fn reserve(&self, owner: Owner, bytes: usize) -> io::Result<Reservation> {
        if let Some(credit) = self
            .2
            .as_ref()
            .filter(|_| !matches!(owner, Owner::Transient))
        {
            let mut bank = credit.lock().unwrap();
            let credit = &mut bank.reservation;
            if bytes <= credit.bytes {
                credit.bytes -= bytes;
                credit.memory.reclassify(credit.owner, owner, bytes);
                return Ok(Reservation {
                    memory: Self(self.0.clone(), false, None),
                    owner,
                    bytes,
                });
            }
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "page exceeded directory resident_bytes",
            ));
        }
        if self.acquire(owner, bytes).is_err() {
            if !self.1 || !reclaim(self, owner, bytes) {
                return Err(io::Error::new(
                    io::ErrorKind::OutOfMemory,
                    "playback working set exhausted",
                ));
            }
        }
        Ok(Reservation {
            memory: Self(self.0.clone(), self.1, None),
            owner,
            bytes,
        })
    }

    pub(crate) fn remaining_credit(&self) -> usize {
        self.2
            .as_ref()
            .map_or(0, |credit| credit.lock().unwrap().reservation.bytes())
    }

    pub(crate) fn soft(&self) -> Self {
        Self(self.0.clone(), false, self.2.clone())
    }
    pub(crate) fn is_within(&self, ancestor: &Memory) -> bool {
        Arc::ptr_eq(&self.0, &ancestor.0)
            || self
                .0
                .parent
                .as_ref()
                .is_some_and(|parent| parent.is_within(ancestor))
    }

    pub(crate) fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    fn acquire(&self, owner: Owner, bytes: usize) -> Result<(), Memory> {
        if let Some(parent) = &self.0.parent {
            parent.acquire(owner, bytes)?;
        }
        if self
            .0
            .used
            .fetch_update(Ordering::AcqRel, Ordering::Relaxed, |used| {
                used.checked_add(bytes).filter(|&next| next <= self.0.limit)
            })
            .is_err()
        {
            if let Some(parent) = &self.0.parent {
                parent.release(owner, bytes);
            }
            return Err(self.clone());
        }
        let used = self.0.owners[owner as usize].fetch_add(bytes, Ordering::Relaxed) + bytes;
        self.0.owner_peaks[owner as usize].fetch_max(used, Ordering::Relaxed);
        self.0.peak.fetch_max(self.used(), Ordering::Relaxed);
        Ok(())
    }

    fn reclassify(&self, from: Owner, to: Owner, bytes: usize) {
        self.0.owners[from as usize].fetch_sub(bytes, Ordering::Relaxed);
        let used = self.0.owners[to as usize].fetch_add(bytes, Ordering::Relaxed) + bytes;
        self.0.owner_peaks[to as usize].fetch_max(used, Ordering::Relaxed);
        if let Some(parent) = &self.0.parent {
            parent.reclassify(from, to, bytes);
        }
    }

    fn release(&self, owner: Owner, bytes: usize) {
        self.0.owners[owner as usize].fetch_sub(bytes, Ordering::Relaxed);
        self.0.used.fetch_sub(bytes, Ordering::AcqRel);
        if let Some(parent) = &self.0.parent {
            parent.release(owner, bytes);
        }
    }
}

pub struct Reservation {
    memory: Memory,
    owner: Owner,
    bytes: usize,
}

impl Reservation {
    pub fn bytes(&self) -> usize {
        self.bytes
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        self.memory.release(self.owner, self.bytes);
    }
}

pub struct Buffer<T> {
    values: Vec<T>,
    _memory: Reservation,
}

impl<T> Buffer<T> {
    pub fn new(memory: &Memory, owner: Owner, capacity: usize) -> io::Result<Self> {
        let bytes = size_of::<T>()
            .checked_mul(capacity)
            .ok_or_else(|| io::Error::new(io::ErrorKind::OutOfMemory, "buffer size overflow"))?;
        let reservation = memory.reserve(owner, bytes)?;
        let mut values = Vec::new();
        values
            .try_reserve_exact(capacity)
            .map_err(io::Error::other)?;
        Ok(Self {
            values,
            _memory: reservation,
        })
    }

    pub fn push(&mut self, value: T) -> bool {
        if self.values.len() == self.values.capacity() {
            return false;
        }
        self.values.push(value);
        true
    }

    pub fn clear(&mut self) {
        self.values.clear();
    }
    pub fn truncate(&mut self, len: usize) {
        self.values.truncate(len);
    }
    pub fn capacity(&self) -> usize {
        self.values.capacity()
    }
    pub(crate) fn swap_remove(&mut self, index: usize) -> T {
        self.values.swap_remove(index)
    }
    pub fn pop(&mut self) -> Option<T> {
        self.values.pop()
    }
    pub fn retain(&mut self, mut keep: impl FnMut(&T) -> bool) {
        self.values.retain(|value| keep(value));
    }
    pub fn insert(&mut self, index: usize, value: T) -> bool {
        if self.values.len() == self.values.capacity() {
            return false;
        }
        self.values.insert(index, value);
        true
    }
    pub fn remove(&mut self, index: usize) -> T {
        self.values.remove(index)
    }
    pub fn grow(&mut self, memory: &Memory, owner: Owner, capacity: usize) -> io::Result<()> {
        if capacity <= self.capacity() {
            return Ok(());
        }
        let mut next = Self::new(memory, owner, capacity)?;
        next.values.append(&mut self.values);
        *self = next;
        Ok(())
    }
    pub fn compact(&mut self, memory: &Memory, owner: Owner) -> io::Result<()> {
        if self.values.len() == self.values.capacity() {
            return Ok(());
        }
        let mut next = Self::new(memory, owner, self.values.len())?;
        next.values.append(&mut self.values);
        *self = next;
        Ok(())
    }
    pub fn bytes(&self) -> usize {
        self._memory.bytes()
    }
    pub fn as_slice(&self) -> &[T] {
        &self.values
    }
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        &mut self.values
    }
}

pub(crate) trait Reclaim: Send + Sync {
    fn reclaim_one(&self, budget: &Memory) -> bool;
}

static WINDOWS: Mutex<Option<Buffer<Weak<dyn Reclaim>>>> = Mutex::new(None);

pub(crate) struct Registration(Arc<dyn Reclaim>);

pub(crate) fn register(window: Arc<dyn Reclaim>) -> io::Result<Registration> {
    loop {
        let mut registry = WINDOWS
            .lock()
            .map_err(|_| io::Error::other("playback admission poisoned"))?;
        if let Some(entries) = registry.as_mut() {
            for index in (0..entries.len()).rev() {
                if entries[index].strong_count() == 0 {
                    entries.swap_remove(index);
                }
            }
            if entries.len() < entries.capacity() {
                entries.push(Arc::downgrade(&window));
                return Ok(Registration(window));
            }
        }
        let old_capacity = registry.as_ref().map_or(0, |entries| entries.capacity());
        let capacity = old_capacity.checked_mul(2).unwrap_or(usize::MAX).max(1);
        drop(registry);
        let mut grown = Buffer::new(&Memory::global(), Owner::Metadata, capacity)?;
        let mut registry = WINDOWS
            .lock()
            .map_err(|_| io::Error::other("playback admission poisoned"))?;
        if registry.as_ref().map_or(0, |entries| entries.capacity()) != old_capacity {
            continue;
        }
        if let Some(entries) = registry.as_ref() {
            for entry in entries.iter() {
                grown.push(entry.clone());
            }
        }
        grown.push(Arc::downgrade(&window));
        *registry = Some(grown);
        return Ok(Registration(window));
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        if let Ok(mut registry) = WINDOWS.lock() {
            if let Some(entries) = registry.as_mut() {
                let window = Arc::downgrade(&self.0);
                for index in (0..entries.len()).rev() {
                    if Weak::ptr_eq(&entries[index], &window) {
                        entries.swap_remove(index);
                        break;
                    }
                }
                if entries.is_empty() {
                    *registry = None;
                }
            }
        }
    }
}

fn reclaim(memory: &Memory, owner: Owner, bytes: usize) -> bool {
    let Ok(registry) = WINDOWS.try_lock() else {
        return false;
    };
    let Some(entries) = registry.as_ref() else {
        return false;
    };
    let mut denied = match memory.acquire(owner, bytes) {
        Ok(()) => return true,
        Err(next) => next,
    };
    for entry in entries.iter() {
        if let Some(window) = entry.upgrade() {
            while window.reclaim_one(&denied) {
                match memory.acquire(owner, bytes) {
                    Ok(()) => return true,
                    Err(next) => denied = next,
                }
            }
        }
    }
    false
}

impl<T> std::ops::Deref for Buffer<T> {
    type Target = [T];
    fn deref(&self) -> &Self::Target {
        &self.values
    }
}

impl<T> std::ops::DerefMut for Buffer<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.values
    }
}
