//! Values shared while anyone holds them: a value made for a key is found again by everyone
//! asking for that key until its last holder lets go, and nothing is kept after.

use std::{
    collections::BTreeMap,
    io,
    sync::{Arc, Mutex, MutexGuard, Weak},
};

pub struct Shared<K, V> {
    inner: Mutex<Inner<K, V>>,
}

struct Inner<K, V> {
    held: BTreeMap<K, Weak<V>>,
    /// Entries the last sweep left: the next sweeps once there are twice as many.
    swept: usize,
}

impl<K: Ord, V> Default for Shared<K, V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: Ord, V> Shared<K, V> {
    pub const fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                held: BTreeMap::new(),
                swept: 0,
            }),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner<K, V>> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The value of `key` someone holds now.
    pub fn get(&self, key: &K) -> Option<Arc<V>> {
        self.lock().held.get(key)?.upgrade()
    }

    /// The value of `key` someone holds now, else `make`'s, shared from now on while held.
    /// Two first askers may both make it; the first to finish is the one shared.
    pub fn get_or_make(
        &self,
        key: K,
        make: impl FnOnce() -> io::Result<V>,
    ) -> io::Result<Arc<V>> {
        if let Some(value) = self.get(&key) {
            return Ok(value);
        }
        let made = Arc::new(make()?);
        let mut inner = self.lock();
        if let Some(value) = inner.held.get(&key).and_then(Weak::upgrade) {
            return Ok(value);
        }
        inner.held.insert(key, Arc::downgrade(&made));
        // Sweeping when the map has doubled keeps it within twice what is held, at a constant
        // cost an insert.
        if inner.held.len() > 2 * inner.swept.max(8) {
            inner.held.retain(|_, value| value.strong_count() > 0);
            inner.swept = inner.held.len();
        }
        Ok(made)
    }
}
