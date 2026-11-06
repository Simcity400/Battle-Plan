use std::{
    io,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

#[derive(Clone, Debug)]
pub struct MemoryBudget(Arc<Budget>);
#[derive(Debug)]
struct Budget {
    limit: usize,
    used: AtomicUsize,
    peak: AtomicUsize,
    release: Mutex<()>,
    available: Condvar,
}
#[derive(Debug)]
pub struct Credit {
    budget: MemoryBudget,
    bytes: usize,
}
impl MemoryBudget {
    pub fn preparation() -> Self {
        static SHARED: std::sync::OnceLock<MemoryBudget> = std::sync::OnceLock::new();
        SHARED
            .get_or_init(|| Self::new(6 * 1024 * 1024 * 1024))
            .clone()
    }
    pub fn new(bytes: usize) -> Self {
        Self(Arc::new(Budget {
            limit: bytes,
            used: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            release: Mutex::new(()),
            available: Condvar::new(),
        }))
    }
    pub fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
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
    pub fn admit(&self, bytes: usize) -> io::Result<Credit> {
        let previous = self
            .0
            .used
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |used| {
                used.checked_add(bytes).filter(|next| *next <= self.0.limit)
            })
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "Private fact memory credit unavailable",
                )
            })?;
        self.0.peak.fetch_max(previous + bytes, Ordering::Relaxed);
        Ok(Credit {
            budget: self.clone(),
            bytes,
        })
    }
    pub fn admit_wait(&self, bytes: usize, cancelled: impl Fn() -> bool) -> io::Result<Credit> {
        loop {
            if cancelled() {
                return Err(io::Error::from(io::ErrorKind::Interrupted));
            }
            if bytes > self.limit() {
                return Err(io::Error::new(
                    io::ErrorKind::OutOfMemory,
                    "memory request exceeds its whole budget",
                ));
            }
            let guard = self
                .0
                .release
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            match self.admit(bytes) {
                Ok(credit) => return Ok(credit),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                Err(error) => return Err(error),
            }
            drop(
                self.0
                    .available
                    .wait_timeout(guard, std::time::Duration::from_millis(100))
                    .unwrap_or_else(|error| error.into_inner()),
            );
        }
    }
    fn release(&self, bytes: usize) {
        if bytes == 0 {
            return;
        }
        let _guard = self
            .0
            .release
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        self.0.used.fetch_sub(bytes, Ordering::Relaxed);
        self.0.available.notify_all();
    }
}
impl Credit {
    pub fn bytes(&self) -> usize {
        self.bytes
    }
    pub fn belongs_to(&self, budget: &MemoryBudget) -> bool {
        self.budget.same(budget)
    }
    pub fn split_off(&mut self, bytes: usize) -> Credit {
        assert!(bytes <= self.bytes);
        self.bytes -= bytes;
        Credit {
            budget: self.budget.clone(),
            bytes,
        }
    }
    /// Takes `other`'s bytes into this credit: [`Self::split_off`] undone.
    pub fn absorb(&mut self, mut other: Credit) {
        assert!(self.budget.same(&other.budget));
        self.bytes += std::mem::take(&mut other.bytes);
    }
    pub fn resize(&mut self, bytes: usize) -> io::Result<()> {
        if bytes > self.bytes {
            let mut extra = self.budget.admit(bytes - self.bytes)?;
            extra.bytes = 0;
        } else {
            self.budget.release(self.bytes - bytes);
        }
        self.bytes = bytes;
        Ok(())
    }
}
impl Drop for Credit {
    fn drop(&mut self) {
        self.budget.release(self.bytes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preparation_is_one_shared_six_gib_owner() {
        let preparation = MemoryBudget::preparation();
        assert_eq!(preparation.limit(), 6 * 1024 * 1024 * 1024);
        assert!(preparation.same(&MemoryBudget::preparation()));
    }
    #[test]
    fn split_resize_and_failure_preserve_the_same_owner() {
        let budget = MemoryBudget::new(100);
        let mut first = budget.admit(70).unwrap();
        let mut second = first.split_off(20);
        assert_eq!(budget.used(), 70);
        assert_eq!(first.bytes(), 50);
        assert_eq!(second.bytes(), 20);
        assert!(budget.same(&second.budget));
        assert!(second.belongs_to(&budget));
        assert!(!second.belongs_to(&MemoryBudget::new(100)));
        assert_eq!(
            second.resize(80).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        assert_eq!(second.bytes(), 20);
        assert_eq!(budget.used(), 70);
        first.resize(40).unwrap();
        second.resize(60).unwrap();
        assert_eq!(budget.used(), 100);
        assert_eq!(budget.peak(), 100);
        let third = second.split_off(25);
        first.absorb(third);
        assert_eq!((first.bytes(), second.bytes()), (65, 35));
        assert_eq!(budget.used(), 100);
        drop(first);
        drop(second);
        assert_eq!(budget.used(), 0);
    }
    #[test]
    fn waiting_resumes_on_release_and_cancellation_is_not_a_data_refusal() {
        let budget = MemoryBudget::new(100);
        let held = budget.admit(100).unwrap();
        let (send, recv) = std::sync::mpsc::channel();
        std::thread::scope(|scope| {
            let waiter = scope.spawn(|| {
                budget
                    .admit_wait(100, || {
                        let _ = send.send(());
                        false
                    })
                    .unwrap()
            });
            recv.recv().unwrap();
            drop(held);
            let admitted = waiter.join().unwrap();
            assert_eq!(admitted.bytes(), 100);
            drop(admitted);
        });
        assert_eq!(budget.used(), 0);
        assert_eq!(
            budget.admit_wait(1, || true).unwrap_err().kind(),
            io::ErrorKind::Interrupted
        );
        assert_eq!(
            budget.admit_wait(101, || true).unwrap_err().kind(),
            io::ErrorKind::Interrupted
        );
    }
    #[test]
    fn a_wait_beyond_the_whole_budget_is_refused_not_pending() {
        let budget = MemoryBudget::new(4);
        assert_eq!(
            budget.admit_wait(5, || false).unwrap_err().kind(),
            io::ErrorKind::OutOfMemory
        );
        assert_eq!(budget.used(), 0);
    }
}
