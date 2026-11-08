//! Values shared while anyone holds them: a value made for a key is found again by everyone
//! asking for that key until its last holder lets go, and nothing is kept after. A thread
//! [`holding`] what it is handed keeps each value until it lets go of the [`Held`] set.

use std::{
    any::Any,
    cell::RefCell,
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

impl<K: Ord, V: Send + Sync + 'static> Default for Shared<K, V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: Ord, V: Send + Sync + 'static> Shared<K, V> {
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
        let value = self.lock().held.get(key)?.upgrade()?;
        hand(&value);
        Some(value)
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
            drop(inner);
            hand(&value);
            return Ok(value);
        }
        inner.held.insert(key, Arc::downgrade(&made));
        // Sweeping when the map has doubled keeps it within twice what is held, at a constant
        // cost an insert.
        if inner.held.len() > 2 * inner.swept.max(8) {
            inner.held.retain(|_, value| value.strong_count() > 0);
            inner.swept = inner.held.len();
        }
        drop(inner);
        hand(&made);
        Ok(made)
    }
}

type Value = Arc<dyn Any + Send + Sync>;

thread_local! {
    /// What each [`holding`] scope on this thread was handed, innermost last.
    static HOLDING: RefCell<Vec<Vec<Value>>> = const { RefCell::new(Vec::new()) };
}

/// `value` to the innermost [`holding`] scope on this thread, if any.
fn hand<V: Send + Sync + 'static>(value: &Arc<V>) {
    HOLDING.with(|holding| {
        if let Some(scope) = holding.borrow_mut().last_mut() {
            scope.push(Arc::clone(value) as Value);
        }
    });
}

/// Every value any [`Shared`] hands this thread from now until [`Holding::finish`], so that whoever
/// keeps the [`Held`] set finds them again. Scopes nest: a value goes to the innermost.
pub fn holding() -> Holding {
    HOLDING.with(|holding| holding.borrow_mut().push(Vec::new()));
    Holding { open: true }
}

#[must_use]
pub struct Holding {
    open: bool,
}

impl Holding {
    /// What this thread was handed since [`holding`], each value once.
    pub fn finish(mut self) -> Held {
        self.open = false;
        let mut values = HOLDING
            .with(|holding| holding.borrow_mut().pop())
            .unwrap_or_default();
        values.sort_unstable_by_key(|value| Arc::as_ptr(value).cast::<()>() as usize);
        values.dedup_by(|a, b| Arc::ptr_eq(a, b));
        Held(values)
    }
}

impl Drop for Holding {
    fn drop(&mut self) {
        if self.open {
            HOLDING.with(|holding| holding.borrow_mut().pop());
        }
    }
}

/// Values kept found while this is held ([`holding`]).
#[derive(Clone, Default)]
pub struct Held(Vec<Value>);

impl Held {
    /// The values it keeps.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}
