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

    pub fn available(&self) -> usize { self.budget.limit - self.budget.used() }

    pub fn resize(&mut self, bytes: usize) -> std::io::Result<()> {
        if bytes > self.bytes {
            let extra = bytes - self.bytes;
            self.budget.used.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |used|
                used.checked_add(extra).filter(|&total| total <= self.budget.limit)
            ).map_err(|_| std::io::Error::other(format!("Dated playback exceeds the shared {} byte payload limit", self.budget.limit)))?;
        } else if bytes < self.bytes {
            self.budget.used.fetch_sub(self.bytes - bytes, Ordering::Relaxed);
        }
        self.bytes = bytes;
        Ok(())
    }
}

impl Drop for Charge {
    fn drop(&mut self) { self.budget.used.fetch_sub(self.bytes, Ordering::Relaxed); }
}
