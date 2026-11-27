use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};

#[derive(Debug)]
pub struct ByteBudget {
    limit: usize,
    used: AtomicUsize,
}

impl ByteBudget {
    pub fn new(limit: usize) -> Arc<Self> {
        Arc::new(Self { limit, used: AtomicUsize::new(0) })
    }

    pub fn used(&self) -> usize { self.used.load(Ordering::Relaxed) }

    pub fn limit(&self) -> usize { self.limit }

    pub fn full(&self) -> bool { self.used() >= self.limit }

    pub fn charge(self: &Arc<Self>) -> Charge {
        Charge { budget: self.clone(), bytes: 0 }
    }
}

#[derive(Debug)]
pub struct Charge {
    budget: Arc<ByteBudget>,
    bytes: usize,
}

impl Charge {
    pub(crate) fn absorb(&mut self, mut reserved: Self) {
        debug_assert!(Arc::ptr_eq(&self.budget, &reserved.budget));
        self.bytes += std::mem::take(&mut reserved.bytes);
    }

    pub fn full(&self) -> bool { self.budget.full() }

    pub fn resize(&mut self, bytes: usize) {
        if bytes > self.bytes {
            self.budget.used.fetch_add(bytes - self.bytes, Ordering::Relaxed);
        } else {
            self.budget.used.fetch_sub(self.bytes - bytes, Ordering::Relaxed);
        }
        self.bytes = bytes;
    }
}

impl Drop for Charge {
    fn drop(&mut self) { self.budget.used.fetch_sub(self.bytes, Ordering::Relaxed); }
}
