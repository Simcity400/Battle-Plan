use std::{
    collections::BTreeMap,
    io,
    sync::{
        Arc, Condvar, Mutex, Weak,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
};

#[derive(Clone, Copy, Debug)]
pub struct ReleaseToken {
    generation: u64,
    scope: u64,
    local: u64,
    cleanup: u64,
}
struct LocalReleases {
    owner: usize,
    id: u64,
    count: u64,
}
thread_local! {
    static LOCAL_RELEASES: std::cell::RefCell<Vec<LocalReleases>> = const { std::cell::RefCell::new(Vec::new()) };
}
pub struct ReleaseScope<'a> {
    budget: &'a MemoryBudget,
    id: u64,
    _thread: std::marker::PhantomData<std::rc::Rc<()>>,
}
impl Drop for ReleaseScope<'_> {
    fn drop(&mut self) {
        LOCAL_RELEASES.with(|scopes| {
            let scope = scopes.borrow_mut().pop().expect("active release scope");
            assert_eq!(
                scope.id, self.id,
                "release scopes must finish in stack order"
            );
            assert_eq!(scope.owner, Arc::as_ptr(&self.budget.0) as usize);
        });
    }
}

#[derive(Clone, Debug)]
pub struct MemoryBudget(Arc<Budget>);
#[derive(Debug)]
struct Budget {
    limit: usize,
    used: AtomicUsize,
    peak: AtomicUsize,
    release: Mutex<()>,
    available: Condvar,
    generation: AtomicU64,
    listeners: Mutex<BTreeMap<usize, ListenerEntry>>,
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
            generation: AtomicU64::new(0),
            listeners: Mutex::new(BTreeMap::new()),
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
    pub fn admit_item(&self, bytes: usize) -> io::Result<Credit> {
        if bytes > self.limit() {
            return Err(io::Error::new(
                io::ErrorKind::OutOfMemory,
                "display item exceeds its memory owner; paging is required",
            ));
        }
        self.admit(bytes)
    }
    pub fn reclaim_failure(&self, error: io::Error, evict: impl FnOnce()) -> io::Error {
        if error.kind() != io::ErrorKind::WouldBlock {
            return error;
        }
        let before = self.used();
        evict();
        if self.used() < before {
            error
        } else {
            io::Error::new(
                io::ErrorKind::OutOfMemory,
                "display working set remains pinned after eviction; common paging is required",
            )
        }
    }
    pub fn admit_wait(&self, bytes: usize, cancelled: impl Fn() -> bool) -> io::Result<Credit> {
        loop {
            if cancelled() {
                return Err(io::Error::from(io::ErrorKind::Interrupted));
            }
            let guard = self
                .0
                .release
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            match self.admit_item(bytes) {
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
    /// Track releases by this synchronous call separately from independent releasers.
    pub fn track_releases(&self) -> ReleaseScope<'_> {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        LOCAL_RELEASES.with(|scopes| {
            scopes.borrow_mut().push(LocalReleases {
                owner: Arc::as_ptr(&self.0) as usize,
                id,
                count: 0,
            })
        });
        ReleaseScope {
            budget: self,
            id,
            _thread: std::marker::PhantomData,
        }
    }
    pub fn release_token(&self) -> ReleaseToken {
        let (scope, local) = self.local_releases().unwrap_or((0, 0));
        ReleaseToken {
            generation: self.generation(),
            scope,
            local,
            cleanup: 0,
        }
    }
    fn local_releases(&self) -> Option<(u64, u64)> {
        LOCAL_RELEASES.with(|scopes| {
            scopes
                .borrow()
                .iter()
                .find(|scope| scope.owner == Arc::as_ptr(&self.0) as usize)
                .map(|scope| (scope.id, scope.count))
        })
    }
    /// Freeze only this attempt's cleanup; an intervening external release stays observable.
    pub fn after_cleanup(&self, mut token: ReleaseToken) -> ReleaseToken {
        token.cleanup = self
            .local_releases()
            .filter(|(scope, _)| *scope == token.scope)
            .map_or(0, |(_, local)| local.wrapping_sub(token.local));
        token
    }
    pub fn released_since(&self, token: ReleaseToken) -> bool {
        self.generation().wrapping_sub(token.generation) > token.cleanup
    }
    pub fn generation(&self) -> u64 {
        self.0.generation.load(Ordering::Acquire)
    }
    /// Register before admission; the callback may only post a nonblocking wake.
    pub fn listen<F: Fn() + Send + Sync + 'static>(&self, wake: F) -> io::Result<ReleaseListener> {
        let bytes = size_of::<ReleaseSignal>()
            + size_of::<F>()
            + 2 * size_of::<usize>()
            + 12 * (size_of::<ListenerEntry>() + 2 * size_of::<usize>())
            + 128;
        let credit = self.admit(bytes)?;
        let signal = Arc::new(ReleaseSignal {
            pending: AtomicBool::new(false),
            wake: Box::new(wake),
        });
        let key = Arc::as_ptr(&signal) as usize;
        self.0
            .listeners
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(
                key,
                ListenerEntry {
                    signal: Arc::downgrade(&signal),
                    _credit: credit,
                },
            );
        Ok(ReleaseListener {
            budget: self.clone(),
            signal,
        })
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
        self.0.generation.fetch_add(1, Ordering::AcqRel);
        let _ = LOCAL_RELEASES.try_with(|scopes| {
            if let Some(scope) = scopes
                .borrow_mut()
                .iter_mut()
                .find(|scope| scope.owner == Arc::as_ptr(&self.0) as usize)
            {
                scope.count = scope.count.wrapping_add(1);
            }
        });
        self.0.available.notify_all();
        drop(_guard);
        let listeners = self
            .0
            .listeners
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        for entry in listeners.values() {
            let Some(signal) = entry.signal.upgrade() else {
                continue;
            };
            if !signal.pending.swap(true, Ordering::AcqRel) {
                if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (signal.wake)()))
                    .is_err()
                {
                    signal.pending.store(false, Ordering::Release);
                }
            }
        }
    }
}
#[derive(Debug)]
struct ListenerEntry {
    signal: Weak<ReleaseSignal>,
    _credit: Credit,
}
struct ReleaseSignal {
    pending: AtomicBool,
    wake: Box<dyn Fn() + Send + Sync>,
}
pub struct ReleaseListener {
    budget: MemoryBudget,
    signal: Arc<ReleaseSignal>,
}
impl ReleaseListener {
    pub fn acknowledge(&self) -> u64 {
        self.signal.pending.store(false, Ordering::Release);
        self.budget.generation()
    }
    pub fn generation(&self) -> u64 {
        self.budget.generation()
    }
}
impl Drop for ReleaseListener {
    fn drop(&mut self) {
        let entry = self
            .budget
            .0
            .listeners
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&(Arc::as_ptr(&self.signal) as usize));
        // Drop node credit outside the registry lock: release also wakes listeners.
        drop(entry);
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

// PLAN-V1 products (0.28 GB), CPU bank (0.32 GB) and staging (0.12 GB) share one owner.
pub fn display_products() -> &'static MemoryBudget {
    static BUDGET: std::sync::OnceLock<MemoryBudget> = std::sync::OnceLock::new();
    BUDGET.get_or_init(|| MemoryBudget::new(720_000_000))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preparation_keeps_its_shared_six_gib_owner_separate_from_display() {
        let preparation = MemoryBudget::preparation();
        assert_eq!(preparation.limit(), 6 * 1024 * 1024 * 1024);
        assert!(preparation.same(&MemoryBudget::preparation()));
        assert!(!preparation.same(display_products()));
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
    fn release_listeners_coalesce_and_rearm_without_losing_the_generation() {
        let budget = MemoryBudget::new(4096);
        let wakes = Arc::new(AtomicUsize::new(0));
        let seen = wakes.clone();
        let listener = budget
            .listen(move || {
                seen.fetch_add(1, Ordering::Relaxed);
            })
            .unwrap();
        let first = listener.acknowledge();
        drop(budget.admit(1).unwrap());
        drop(budget.admit(1).unwrap());
        assert_eq!(wakes.load(Ordering::Relaxed), 1);
        assert_ne!(first, listener.generation());
        let second = listener.acknowledge();
        drop(budget.admit(1).unwrap());
        assert_ne!(second, listener.generation());
        assert_eq!(wakes.load(Ordering::Relaxed), 2);
        drop(listener);
        drop(budget.admit(1).unwrap());
        assert_eq!(wakes.load(Ordering::Relaxed), 2);
        assert!(budget.0.listeners.lock().unwrap().is_empty());
        assert_eq!(budget.used(), 0);
    }
    #[test]
    fn admission_tokens_exclude_local_cleanup_but_keep_intervening_external_releases() {
        let budget = MemoryBudget::new(4);
        let mut held = budget.admit(4).unwrap();
        let cleanup = held.split_off(1);
        let scope = budget.track_releases();
        let token = budget.release_token();
        assert_eq!(
            budget.admit_item(1).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        drop(cleanup);
        let frozen = budget.after_cleanup(token);
        assert!(!budget.released_since(frozen));
        drop(scope);
        std::thread::spawn(move || drop(held)).join().unwrap();
        assert!(budget.released_since(frozen));
        assert_eq!(budget.used(), 0);

        let mut held = budget.admit(4).unwrap();
        let cleanup = held.split_off(1);
        let scope = budget.track_releases();
        let token = budget.release_token();
        assert_eq!(
            budget.admit_item(1).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        std::thread::spawn(move || drop(held)).join().unwrap();
        drop(cleanup);
        let frozen = budget.after_cleanup(token);
        assert!(budget.released_since(frozen));
        assert_eq!(budget.used(), 0);
        drop(scope);
    }

    #[test]
    fn every_display_holder_gets_the_same_process_owner() {
        assert!(display_products().same(display_products()));
    }
}

#[cfg(test)]
mod listener_admission_tests {
    use super::*;
    #[test]
    fn a_listener_does_not_allocate_its_callback_or_registry_node_without_credit() {
        let budget = MemoryBudget::new(0);
        assert!(
            matches!(budget.listen(|| {}), Err(error) if error.kind() == io::ErrorKind::WouldBlock)
        );
        assert!(budget.0.listeners.lock().unwrap().is_empty());
        assert_eq!(budget.used(), 0);
    }
}

#[cfg(test)]
mod item_admission_tests {
    use super::*;
    #[test]
    fn oversized_items_are_not_endless_pending_and_pressure_can_resume() {
        let budget = MemoryBudget::new(4);
        assert_eq!(
            budget.admit_item(5).unwrap_err().kind(),
            io::ErrorKind::OutOfMemory
        );
        let held = budget.admit_item(4).unwrap();
        assert_eq!(
            budget.admit_item(1).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        drop(held);
        assert!(budget.admit_item(4).is_ok());
    }
}
