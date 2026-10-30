use std::{
    io,
    mem::size_of,
    sync::atomic::{AtomicUsize, Ordering},
};

pub struct Budget {
    limit: usize,
    used: AtomicUsize,
}

impl Budget {
    pub const fn new(limit: usize) -> Self {
        Self {
            limit,
            used: AtomicUsize::new(0),
        }
    }

    pub fn used(&self) -> usize {
        self.used.load(Ordering::Relaxed)
    }

    pub fn admit(&self, bytes: usize) -> io::Result<Lease<'_>> {
        self.reserve(bytes)?;
        Ok(Lease {
            budget: self,
            bytes,
        })
    }
    pub(crate) fn reserve(&self, bytes: usize) -> io::Result<()> {
        self.used
            .fetch_update(Ordering::AcqRel, Ordering::Relaxed, |used| {
                used.checked_add(bytes).filter(|sum| *sum <= self.limit)
            })
            .map(|_| ())
            .map_err(|_| {
                io::Error::new(io::ErrorKind::WouldBlock, "series byte admission exhausted")
            })
    }
    pub(crate) fn release(&self, bytes: usize) {
        self.used.fetch_sub(bytes, Ordering::AcqRel);
    }
}

pub struct Lease<'a> {
    budget: &'a Budget,
    bytes: usize,
}
impl Lease<'_> {
    /// Admits `bytes` more under this lease.
    pub(crate) fn grow(&mut self, bytes: usize) -> io::Result<()> {
        self.budget.reserve(bytes)?;
        self.bytes += bytes;
        Ok(())
    }
    fn shrink(&mut self, bytes: usize) {
        self.bytes -= bytes;
        self.budget.release(bytes);
    }
}
impl Drop for Lease<'_> {
    fn drop(&mut self) {
        self.budget.release(self.bytes);
    }
}

pub struct Slab<'a, T> {
    values: Vec<T>,
    lease: Lease<'a>,
}

impl<'a, T> Slab<'a, T> {
    pub fn new(budget: &'a Budget, capacity: usize) -> io::Result<Self> {
        let bytes = capacity
            .checked_mul(size_of::<T>())
            .ok_or_else(engine_allocation::oom)?;
        let lease = budget.admit(bytes)?;
        Ok(Self {
            values: engine_allocation::vec(capacity)?,
            lease,
        })
    }
    /// Admits room for `more` further values, at least doubling a full slab.
    pub fn reserve(&mut self, more: usize) -> io::Result<()> {
        let need = engine_allocation::add(self.values.len(), more)?;
        if need > self.values.capacity() {
            self.grow(need.max(self.values.capacity().max(4).saturating_mul(2)))?;
        }
        Ok(())
    }
    /// Appends into room [`Slab::reserve`] admitted.
    pub fn put(&mut self, value: T) {
        debug_assert!(self.values.len() < self.values.capacity());
        self.values.push(value);
    }
    pub fn push(&mut self, value: T) -> io::Result<()> {
        self.reserve(1)?;
        self.put(value);
        Ok(())
    }
    pub fn grow(&mut self, capacity: usize) -> io::Result<()> {
        let held = self.values.capacity();
        if capacity <= held {
            return Ok(());
        }
        let extra = (capacity - held)
            .checked_mul(size_of::<T>())
            .ok_or_else(engine_allocation::oom)?;
        self.lease.grow(extra)?;
        let more = capacity - self.values.len();
        if let Err(error) = engine_allocation::reserve_exact(&mut self.values, more) {
            self.lease.shrink(extra);
            return Err(error);
        }
        let added = self.values.capacity() - held;
        if added * size_of::<T>() > extra {
            self.lease.grow(added * size_of::<T>() - extra)?;
        }
        Ok(())
    }
    pub fn clear(&mut self) {
        self.values.clear();
    }
    pub fn truncate(&mut self, len: usize) {
        self.values.truncate(len);
    }
    pub fn pop(&mut self) -> Option<T> {
        self.values.pop()
    }
}

impl<T: Copy> Slab<'_, T> {
    /// Appends `values`, admitting at least a doubled capacity when full.
    pub fn extend(&mut self, values: &[T]) -> io::Result<()> {
        self.reserve(values.len())?;
        self.values.extend_from_slice(values);
        Ok(())
    }
}

impl<T> std::ops::Deref for Slab<'_, T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        &self.values
    }
}
impl<T> std::ops::DerefMut for Slab<'_, T> {
    fn deref_mut(&mut self) -> &mut [T] {
        &mut self.values
    }
}
