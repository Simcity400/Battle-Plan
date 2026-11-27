use std::{
    any::Any,
    cell::{Cell, UnsafeCell},
    collections::{BTreeSet, VecDeque},
    marker::PhantomData,
    panic::{AssertUnwindSafe, catch_unwind, resume_unwind},
    ptr,
    sync::{
        Arc, Condvar, Mutex, MutexGuard, OnceLock,
        atomic::{AtomicBool, AtomicU8, AtomicU64, AtomicUsize, Ordering},
    },
    thread::JoinHandle,
};

pub mod cpu;
pub mod dag;

pub use job_priority::usable_processors;

static NEXT_POOL_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_ROOT_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_BATCH_ID: AtomicU64 = AtomicU64::new(1);
static SHARED: OnceLock<Pool> = OnceLock::new();

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ActivePool {
    pool_id: u64,
    root_id: u64,
    batch_id: Option<u64>,
}

struct RootCancelled;

pub(crate) fn is_root_cancelled_panic(panic: &(dyn Any + Send)) -> bool {
    panic.is::<RootCancelled>()
}

thread_local! {
    static ACTIVE_POOL: Cell<Option<ActivePool>> = const { Cell::new(None) };
    static ALIVE: std::cell::RefCell<Option<Arc<AtomicBool>>> = const { std::cell::RefCell::new(None) };
}

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}

struct AliveGuard(Option<Arc<AtomicBool>>, PhantomData<std::rc::Rc<()>>);

impl AliveGuard {
    fn set(alive: Option<Arc<AtomicBool>>) -> Self {
        Self(ALIVE.with(|current| current.replace(alive)), PhantomData)
    }
}

impl Drop for AliveGuard {
    fn drop(&mut self) {
        ALIVE.with(|current| current.replace(self.0.take()));
    }
}

fn current_alive() -> Option<Arc<AtomicBool>> {
    ALIVE.with(|current| current.borrow().clone())
}

pub fn with_alive<T>(alive: Arc<AtomicBool>, work: impl FnOnce() -> T) -> Option<T> {
    let _guard = AliveGuard::set(Some(alive));
    match catch_unwind(AssertUnwindSafe(work)) {
        Ok(value) => Some(value),
        Err(panic) if is_root_cancelled_panic(&*panic) => None,
        Err(panic) => resume_unwind(panic),
    }
}

pub(crate) fn is_root_cancelled(alive: Option<&Arc<AtomicBool>>) -> bool {
    alive.is_some_and(|flag| !flag.load(Ordering::Acquire))
}

struct StrictGuard(bool, PhantomData<std::rc::Rc<()>>);

impl StrictGuard {
    fn set(value: bool) -> Self {
        Self(crate::content::set_strict(value), PhantomData)
    }
}

impl Drop for StrictGuard {
    fn drop(&mut self) {
        crate::content::set_strict(self.0);
    }
}

pub(crate) struct CarriedContext {
    caller: job_priority::Caller,
    accounting: job_priority::accounting::Context,
    diagnostics: crate::content::diagnostics::Context,
    generation: crate::content::generation::Captured,
    cpu: cpu::Captured,
    memory: engine_allocation::labels::Label,
    strict: bool,
    alive: Option<Arc<AtomicBool>>,
}

impl CarriedContext {
    pub(crate) fn capture() -> Self {
        Self {
            caller: job_priority::Caller::current(),
            accounting: job_priority::accounting::Context::capture(),
            diagnostics: crate::content::diagnostics::Context::capture(),
            generation: crate::content::generation::Captured::capture(),
            cpu: cpu::Captured::capture(),
            memory: engine_allocation::labels::current(),
            strict: crate::content::is_strict(),
            alive: current_alive(),
        }
    }

    pub(crate) fn enter_with<R>(
        &self,
        category: &'static str,
        enter: impl FnOnce(),
        work: impl FnOnce() -> R,
    ) -> (R, BTreeSet<String>) {
        let _alive = AliveGuard::set(self.alive.clone());
        let _generation = self.generation.resume();
        let _cpu = self.cpu.enter();
        let _memory = engine_allocation::labels::resume(self.memory);
        let _strict = StrictGuard::set(self.strict);
        let _caller = job_priority::enter_scoped(enter);
        let _accounting = self.accounting.enter(category);
        self.diagnostics
            .enter(|| crate::content::observe_overlay_refusals_isolated(work))
    }

    pub(crate) fn enter<R>(
        &self,
        category: &'static str,
        work: impl FnOnce() -> R,
    ) -> (R, BTreeSet<String>) {
        let caller = self.caller.clone();
        self.enter_with(category, move || caller.enter(), work)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BatchMeta {
    pub(crate) root_id: u64,
    pub(crate) batch_id: u64,
    pub(crate) parent_batch_id: Option<u64>,
}

pub(crate) fn is_strictly_nested_child_of_same_root(
    candidate: BatchMeta,
    waiting: BatchMeta,
    max_depth: usize,
    mut parent_of: impl FnMut(u64) -> Option<u64>,
) -> bool {
    if candidate.root_id != waiting.root_id || candidate.batch_id == waiting.batch_id {
        return false;
    }
    let mut current = candidate.parent_batch_id;
    for _ in 0..=max_depth {
        let Some(parent_id) = current else {
            return false;
        };
        if parent_id == waiting.batch_id {
            return true;
        }
        current = parent_of(parent_id);
    }
    false
}

pub(crate) struct ActiveBatchGuard {
    previous: Option<ActivePool>,
    _thread: PhantomData<std::rc::Rc<()>>,
}

impl Drop for ActiveBatchGuard {
    fn drop(&mut self) {
        ACTIVE_POOL.set(self.previous);
    }
}

pub(crate) trait WorkBatch: Send + Sync {
    fn has_claimable(&self) -> bool;
    fn run_turn(&self);
}

struct BatchEntry {
    meta: BatchMeta,
    alive: Option<Arc<AtomicBool>>,
    batch: Arc<dyn WorkBatch>,
    closed: AtomicBool,
    in_flight: AtomicUsize,
}

impl BatchEntry {
    fn is_cancelled(&self) -> bool {
        is_root_cancelled(self.alive.as_ref())
    }
}

struct PoolState {
    active_callers: usize,
    active_helpers: usize,
    spawned_helpers: usize,
    batches: Vec<Arc<BatchEntry>>,
    parent_links: Vec<(u64, u64)>,
    closed: bool,
}

struct PoolInner {
    pool_id: u64,
    budget: usize,
    state: Mutex<PoolState>,
    changed: Condvar,
}

impl PoolInner {
    fn claim_top_locked(
        &self,
        state: &PoolState,
        waiting: Option<BatchMeta>,
    ) -> Option<Arc<BatchEntry>> {
        let parent_of = |batch_id: u64| {
            state
                .batches
                .iter()
                .find(|entry| entry.meta.batch_id == batch_id)
                .and_then(|entry| entry.meta.parent_batch_id)
                .or_else(|| {
                    state
                        .parent_links
                        .iter()
                        .rev()
                        .find_map(|&(child, parent)| (child == batch_id).then_some(parent))
                })
        };
        let max_depth = state.batches.len() + state.parent_links.len();
        for entry in state.batches.iter().rev() {
            if entry.closed.load(Ordering::Acquire) || entry.is_cancelled() {
                continue;
            }
            if let Some(waiting) = waiting
                && !is_strictly_nested_child_of_same_root(
                    entry.meta,
                    waiting,
                    max_depth,
                    parent_of,
                )
            {
                continue;
            }
            if !entry.batch.has_claimable() {
                continue;
            }
            entry.in_flight.fetch_add(1, Ordering::AcqRel);
            return Some(Arc::clone(entry));
        }
        None
    }
}

struct ParentLinkGuard<'a> {
    pool: &'a Pool,
    child: u64,
}

impl Drop for ParentLinkGuard<'_> {
    fn drop(&mut self) {
        let mut state = lock(&self.pool.inner.state);
        if let Some(at) = state
            .parent_links
            .iter()
            .rposition(|&(child, _)| child == self.child)
        {
            state.parent_links.remove(at);
        }
    }
}

pub(crate) struct Pool {
    inner: Arc<PoolInner>,
    helpers: Mutex<Vec<JoinHandle<()>>>,
}

impl Pool {
    pub(crate) fn with_budget(budget: usize) -> Self {
        Self {
            inner: Arc::new(PoolInner {
                pool_id: NEXT_POOL_ID.fetch_add(1, Ordering::Relaxed),
                budget: budget.max(1),
                state: Mutex::new(PoolState {
                    active_callers: 0,
                    active_helpers: 0,
                    spawned_helpers: 0,
                    batches: Vec::new(),
                    parent_links: Vec::new(),
                    closed: false,
                }),
                changed: Condvar::new(),
            }),
            helpers: Mutex::new(Vec::new()),
        }
    }

    #[cfg(test)]
    pub(crate) fn spawned_helpers(&self) -> usize {
        lock(&self.inner.state).spawned_helpers
    }

    #[cfg(test)]
    pub(crate) fn queued_batches(&self) -> usize {
        lock(&self.inner.state).batches.len()
    }

    pub(crate) fn notify_all(&self) {
        let _state = lock(&self.inner.state);
        self.inner.changed.notify_all();
    }

    pub(crate) fn enter_root(&self) -> RootGuard<'_> {
        let previous = ACTIVE_POOL.get();
        if let Some(active) = previous
            && active.pool_id == self.inner.pool_id
        {
            return RootGuard {
                pool: self,
                root_id: active.root_id,
                entered_top_level: false,
                previous,
            };
        }
        let root_id = NEXT_ROOT_ID.fetch_add(1, Ordering::Relaxed);
        ACTIVE_POOL.set(Some(ActivePool {
            pool_id: self.inner.pool_id,
            root_id,
            batch_id: None,
        }));
        lock(&self.inner.state).active_callers += 1;
        RootGuard {
            pool: self,
            root_id,
            entered_top_level: true,
            previous,
        }
    }

    pub(crate) fn enter_batch(&self, meta: BatchMeta) -> ActiveBatchGuard {
        let previous = ACTIVE_POOL.replace(Some(ActivePool {
            pool_id: self.inner.pool_id,
            root_id: meta.root_id,
            batch_id: Some(meta.batch_id),
        }));
        ActiveBatchGuard {
            previous,
            _thread: PhantomData,
        }
    }

    fn link_parent(&self, child: u64, parent: u64) -> ParentLinkGuard<'_> {
        lock(&self.inner.state).parent_links.push((child, parent));
        ParentLinkGuard { pool: self, child }
    }

    pub(crate) fn register_batch(
        &self,
        root_id: u64,
        alive: Option<Arc<AtomicBool>>,
        batch: Arc<dyn WorkBatch>,
        want_helpers: usize,
    ) -> BatchRegistration<'_> {
        let parent_batch_id = ACTIVE_POOL.get().and_then(|active| {
            (active.pool_id == self.inner.pool_id && active.root_id == root_id)
                .then_some(active.batch_id)
                .flatten()
        });
        let entry = Arc::new(BatchEntry {
            meta: BatchMeta {
                root_id,
                batch_id: NEXT_BATCH_ID.fetch_add(1, Ordering::Relaxed),
                parent_batch_id,
            },
            alive,
            batch,
            closed: AtomicBool::new(false),
            in_flight: AtomicUsize::new(0),
        });
        let spawn_range = {
            let mut state = lock(&self.inner.state);
            state.batches.push(Arc::clone(&entry));
            self.notify_and_plan_locked(&mut state, want_helpers)
        };
        self.spawn_helpers(spawn_range);
        BatchRegistration { pool: self, entry }
    }

    pub(crate) fn wake_helpers(&self, want_helpers: usize) {
        let spawn_range = {
            let mut state = lock(&self.inner.state);
            self.notify_and_plan_locked(&mut state, want_helpers)
        };
        self.spawn_helpers(spawn_range);
    }

    fn notify_and_plan_locked(
        &self,
        state: &mut PoolState,
        want_helpers: usize,
    ) -> Option<(usize, usize)> {
        self.inner.changed.notify_all();
        if self.inner.budget <= 1 || want_helpers == 0 || state.closed {
            return None;
        }
        let max_helpers = self.inner.budget - 1;
        let available = self
            .inner
            .budget
            .saturating_sub(state.active_callers + state.active_helpers);
        let needed = (state.active_helpers + want_helpers.min(available)).min(max_helpers);
        if needed <= state.spawned_helpers {
            return None;
        }
        let from = state.spawned_helpers;
        state.spawned_helpers = needed;
        Some((from, needed))
    }

    fn spawn_helpers(&self, range: Option<(usize, usize)>) {
        let Some((from, to)) = range else {
            return;
        };
        let mut helpers = lock(&self.helpers);
        let mut failed = 0usize;
        for index in from..to {
            let inner = Arc::clone(&self.inner);
            match std::thread::Builder::new()
                .name(format!("content worker {index}"))
                .spawn(move || helper_loop(inner))
            {
                Ok(handle) => helpers.push(handle),
                Err(_) => {
                    failed = to - index;
                    break;
                }
            }
        }
        if failed > 0 {
            let mut state = lock(&self.inner.state);
            state.spawned_helpers = state.spawned_helpers.saturating_sub(failed);
        }
    }

    fn run_claimed(&self, entry: &Arc<BatchEntry>) {
        let _active = self.enter_batch(entry.meta);
        let _ = catch_unwind(AssertUnwindSafe(|| entry.batch.run_turn()));
        drop(_active);
        let _state = lock(&self.inner.state);
        entry.in_flight.fetch_sub(1, Ordering::AcqRel);
        self.inner.changed.notify_all();
    }

    pub(crate) fn wait_until(&self, waiting: BatchMeta, mut done: impl FnMut() -> bool) {
        loop {
            let child = {
                let mut state = lock(&self.inner.state);
                loop {
                    if done() {
                        return;
                    }
                    if let Some(child) = self.inner.claim_top_locked(&state, Some(waiting)) {
                        break child;
                    }
                    state = self
                        .inner
                        .changed
                        .wait(state)
                        .unwrap_or_else(|error| error.into_inner());
                }
            };
            self.run_claimed(&child);
        }
    }

    pub(crate) fn map<T: Sync, R: Send>(&self, items: &[T], work: impl Fn(&T) -> R + Sync) -> Vec<R> {
        let caller = job_priority::Caller::current();
        self.map_entering(items, move || caller.clone().enter(), work)
    }

    pub(crate) fn map_entering<T: Sync, R: Send>(
        &self,
        items: &[T],
        enter: impl Fn() + Sync,
        work: impl Fn(&T) -> R + Sync,
    ) -> Vec<R> {
        if items.is_empty() {
            return Vec::new();
        }
        let alive = current_alive();
        if items.len() <= 1 || self.inner.budget <= 1 {
            let accounting = job_priority::accounting::Context::capture();
            let _caller_priority = job_priority::enter_scoped(&enter);
            let _accounting = accounting.enter("content parallel");
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                if is_root_cancelled(alive.as_ref()) {
                    resume_unwind(Box::new(RootCancelled));
                }
                out.push(work(item));
            }
            return out;
        }
        let root = self.enter_root();
        let slots = Slots::<R>::new(items.len());
        let step = |index: usize| slots.write(index, work(&items[index]));
        let context = CarriedContext::capture();
        let batch = Arc::new(MapBatch::new(items.len(), &enter, &step, context));
        let registration = self.register_batch(
            root.root_id,
            alive.clone(),
            Arc::clone(&batch) as Arc<dyn WorkBatch>,
            items.len().saturating_sub(1),
        );
        let entry = Arc::clone(&registration.entry);
        let caller_result = catch_unwind(AssertUnwindSafe(|| {
            let _active = self.enter_batch(entry.meta);
            let _caller_priority = job_priority::enter_scoped(&enter);
            let _accounting = batch.context.accounting.enter("content parallel");
            batch.drain_on_caller();
        }));
        if let Err(panic) = caller_result {
            if is_root_cancelled_panic(&*panic) {
                batch.mark_cancelled();
            } else {
                batch.record_panic(panic);
            }
        }
        self.wait_until(entry.meta, || {
            batch.done() && entry.in_flight.load(Ordering::Acquire) == 0
        });
        drop(registration);
        if let Some(panic) = lock(&batch.panic).take() {
            resume_unwind(panic);
        }
        if batch.cancelled.load(Ordering::Acquire) || is_root_cancelled(alive.as_ref()) {
            resume_unwind(Box::new(RootCancelled));
        }
        crate::content::credit_overlay_observers(&std::mem::take(&mut *lock(&batch.overlay_refusals)));
        slots.into_vec()
    }

    pub(crate) fn scope<'env, R>(
        &self,
        work: impl for<'scope> FnOnce(&'scope Scope<'scope, 'env>) -> R,
    ) -> R {
        let root = self.enter_root();
        let alive = current_alive();
        let batch = Arc::new(ScopeBatch::new(self, root.root_id));
        let scope = Scope {
            pool: self,
            batch: Arc::clone(&batch),
            _scope: PhantomData,
            _env: PhantomData,
        };
        let registration = self.register_batch(
            root.root_id,
            alive.clone(),
            Arc::clone(&batch) as Arc<dyn WorkBatch>,
            0,
        );
        let meta = registration.meta();
        lock(&batch.state).meta = meta;
        let result = catch_unwind(AssertUnwindSafe(|| {
            let _active = self.enter_batch(meta);
            work(&scope)
        }));
        let mut cancelling = result.is_err() || is_root_cancelled(alive.as_ref());
        let mut skipped_by_cancel = false;
        let mut drop_panic: Option<Box<dyn Any + Send>> = None;
        loop {
            if cancelling {
                registration.entry.closed.store(true, Ordering::Release);
                let (cancelled, panic) = batch.cancel_pending();
                if cancelled > 0 {
                    skipped_by_cancel = true;
                }
                if drop_panic.is_none() {
                    drop_panic = panic;
                }
            } else {
                batch.drain_pending_on_caller();
                if is_root_cancelled(alive.as_ref()) {
                    cancelling = true;
                    continue;
                }
            }
            if batch.unfinished() == 0 && registration.in_flight() == 0 {
                break;
            }
            self.wait_until(meta, || {
                batch.has_claimable()
                    || (batch.unfinished() == 0 && registration.in_flight() == 0)
            });
        }
        drop(registration);
        if result.is_ok() {
            batch.credit_remaining_refusals();
        }
        let (child_panic, child_cancelled) = batch.take_unobserved_panic();
        match result {
            Ok(value) => {
                if let Some(panic) = child_panic {
                    resume_unwind(panic);
                }
                if let Some(panic) = drop_panic {
                    resume_unwind(panic);
                }
                if (skipped_by_cancel || child_cancelled) && is_root_cancelled(alive.as_ref()) {
                    resume_unwind(Box::new(RootCancelled));
                }
                value
            }
            Err(panic) if is_root_cancelled_panic(&*panic) => {
                if let Some(child_panic) = child_panic {
                    resume_unwind(child_panic);
                }
                if let Some(drop_panic) = drop_panic {
                    resume_unwind(drop_panic);
                }
                resume_unwind(panic);
            }
            Err(panic) => resume_unwind(panic),
        }
    }
}

impl Drop for Pool {
    fn drop(&mut self) {
        {
            let mut state = lock(&self.inner.state);
            state.closed = true;
            self.inner.changed.notify_all();
        }
        for handle in std::mem::take(&mut *lock(&self.helpers)) {
            let _ = handle.join();
        }
    }
}

fn helper_loop(inner: Arc<PoolInner>) {
    loop {
        let entry = {
            let mut state = lock(&inner.state);
            loop {
                if state.closed {
                    return;
                }
                if state.active_callers + state.active_helpers < inner.budget
                    && let Some(entry) = inner.claim_top_locked(&state, None)
                {
                    state.active_helpers += 1;
                    break entry;
                }
                state = inner
                    .changed
                    .wait(state)
                    .unwrap_or_else(|error| error.into_inner());
            }
        };
        let previous = ACTIVE_POOL.replace(Some(ActivePool {
            pool_id: inner.pool_id,
            root_id: entry.meta.root_id,
            batch_id: Some(entry.meta.batch_id),
        }));
        let _ = catch_unwind(AssertUnwindSafe(|| entry.batch.run_turn()));
        ACTIVE_POOL.set(previous);
        let mut state = lock(&inner.state);
        entry.in_flight.fetch_sub(1, Ordering::AcqRel);
        state.active_helpers -= 1;
        inner.changed.notify_all();
    }
}

pub(crate) struct RootGuard<'a> {
    pool: &'a Pool,
    pub(crate) root_id: u64,
    entered_top_level: bool,
    previous: Option<ActivePool>,
}

impl Drop for RootGuard<'_> {
    fn drop(&mut self) {
        ACTIVE_POOL.set(self.previous);
        if self.entered_top_level {
            lock(&self.pool.inner.state).active_callers -= 1;
            self.pool.inner.changed.notify_all();
        }
    }
}

pub(crate) struct BatchRegistration<'a> {
    pool: &'a Pool,
    entry: Arc<BatchEntry>,
}

impl BatchRegistration<'_> {
    pub(crate) fn meta(&self) -> BatchMeta {
        self.entry.meta
    }

    pub(crate) fn in_flight(&self) -> usize {
        self.entry.in_flight.load(Ordering::Acquire)
    }
}

impl Drop for BatchRegistration<'_> {
    fn drop(&mut self) {
        self.entry.closed.store(true, Ordering::Release);
        {
            let mut state = lock(&self.pool.inner.state);
            if let Some(at) = state
                .batches
                .iter()
                .position(|item| Arc::ptr_eq(item, &self.entry))
            {
                state.batches.remove(at);
            }
            self.pool.inner.changed.notify_all();
        }
        let entry = Arc::clone(&self.entry);
        self.pool
            .wait_until(entry.meta, || entry.in_flight.load(Ordering::Acquire) == 0);
    }
}

struct Slots<R>(Box<[UnsafeCell<Option<R>>]>);

// SAFETY: `MapBatch` hands out each index in `0..len` at most once through `AtomicUsize::fetch_add`,
// so at most one thread writes to any slot, and `into_vec` reads slots only after every helper has
// finished (`in_flight == 0`).
unsafe impl<R: Send> Sync for Slots<R> {}

impl<R> Slots<R> {
    fn new(len: usize) -> Self {
        let mut slots = Vec::with_capacity(len);
        slots.resize_with(len, || UnsafeCell::new(None));
        Self(slots.into_boxed_slice())
    }

    fn write(&self, index: usize, value: R) {
        // SAFETY: the caller claimed `index` exclusively via `AtomicUsize::fetch_add`.
        unsafe {
            *self.0[index].get() = Some(value);
        }
    }

    fn into_vec(self) -> Vec<R> {
        self.0
            .into_vec()
            .into_iter()
            .map(|slot| slot.into_inner().expect("every item is worked"))
            .collect()
    }
}

struct MapBatch {
    enter: *const (dyn Fn() + Sync + 'static),
    step: *const (dyn Fn(usize) + Sync + 'static),
    len: usize,
    next: AtomicUsize,
    completed: AtomicUsize,
    failed: AtomicBool,
    cancelled: AtomicBool,
    context: CarriedContext,
    panic: Mutex<Option<Box<dyn Any + Send>>>,
    overlay_refusals: Mutex<BTreeSet<String>>,
}

// SAFETY: `enter` and `step` are `Sync` and point to closures on `Pool::map_entering`'s
// stack frame, which outlives every helper because `BatchRegistration::drop` closes the batch and
// waits until `in_flight == 0` before returning.
unsafe impl Send for MapBatch {}
// SAFETY: same lifetime and synchronization invariant as `Send` above.
unsafe impl Sync for MapBatch {}

impl MapBatch {
    fn new(
        len: usize,
        enter: &(dyn Fn() + Sync),
        step: &(dyn Fn(usize) + Sync),
        context: CarriedContext,
    ) -> Self {
        // SAFETY: lifetimes are erased only while registered in `Pool`; `BatchRegistration::drop`
        // waits until `in_flight == 0` before the borrowed closures expire.
        let enter: *const (dyn Fn() + Sync + 'static) = unsafe {
            std::mem::transmute::<*const (dyn Fn() + Sync + '_), *const (dyn Fn() + Sync + 'static)>(
                ptr::from_ref(enter),
            )
        };
        // SAFETY: same as `enter`.
        let step: *const (dyn Fn(usize) + Sync + 'static) = unsafe {
            std::mem::transmute::<
                *const (dyn Fn(usize) + Sync + '_),
                *const (dyn Fn(usize) + Sync + 'static),
            >(ptr::from_ref(step))
        };
        Self {
            enter,
            step,
            len,
            next: AtomicUsize::new(0),
            completed: AtomicUsize::new(0),
            failed: AtomicBool::new(false),
            cancelled: AtomicBool::new(false),
            context,
            panic: Mutex::new(None),
            overlay_refusals: Mutex::new(BTreeSet::new()),
        }
    }

    fn done(&self) -> bool {
        self.failed.load(Ordering::Acquire)
            || self.cancelled.load(Ordering::Acquire)
            || self.completed.load(Ordering::Acquire) >= self.len
    }

    fn mark_cancelled(&self) {
        self.cancelled.store(true, Ordering::Release);
        self.next.store(self.len, Ordering::Relaxed);
    }

    fn record_panic(&self, panic: Box<dyn Any + Send>) {
        self.failed.store(true, Ordering::Release);
        self.next.store(self.len, Ordering::Relaxed);
        lock(&self.panic).get_or_insert(panic);
    }

    fn drain_on_caller(&self) {
        // SAFETY: called synchronously on `map_entering`'s stack frame while `step` is live.
        let step = unsafe { &*self.step };
        loop {
            if self.failed.load(Ordering::Acquire) || self.cancelled.load(Ordering::Acquire) {
                return;
            }
            if is_root_cancelled(self.context.alive.as_ref()) {
                self.mark_cancelled();
                return;
            }
            let index = self.next.fetch_add(1, Ordering::Relaxed);
            if index >= self.len {
                return;
            }
            step(index);
            self.completed.fetch_add(1, Ordering::Release);
        }
    }
}

impl WorkBatch for MapBatch {
    fn has_claimable(&self) -> bool {
        !self.failed.load(Ordering::Acquire)
            && !self.cancelled.load(Ordering::Acquire)
            && self.next.load(Ordering::Relaxed) < self.len
    }

    fn run_turn(&self) {
        if self.failed.load(Ordering::Acquire) || self.cancelled.load(Ordering::Acquire) {
            return;
        }
        if is_root_cancelled(self.context.alive.as_ref()) {
            self.mark_cancelled();
            return;
        }
        let first = self.next.fetch_add(1, Ordering::Relaxed);
        if first >= self.len {
            return;
        }
        // SAFETY: `BatchEntry::in_flight > 0` holds the caller in `BatchRegistration::drop`, so
        // `enter` and `step` remain valid for this entire call.
        let (enter, step) = unsafe { (&*self.enter, &*self.step) };
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            self.context.enter_with("content parallel", enter, || {
                let mut index = first;
                loop {
                    step(index);
                    self.completed.fetch_add(1, Ordering::Release);
                    if self.failed.load(Ordering::Acquire) || self.cancelled.load(Ordering::Acquire)
                    {
                        return;
                    }
                    if is_root_cancelled(self.context.alive.as_ref()) {
                        self.mark_cancelled();
                        return;
                    }
                    index = self.next.fetch_add(1, Ordering::Relaxed);
                    if index >= self.len {
                        return;
                    }
                }
            })
        }));
        match outcome {
            Ok(((), refusals)) => {
                if !refusals.is_empty() {
                    lock(&self.overlay_refusals).extend(refusals);
                }
            }
            Err(panic) if is_root_cancelled_panic(&*panic) => self.mark_cancelled(),
            Err(panic) => self.record_panic(panic),
        }
    }
}

const TASK_PENDING: u8 = 0;
const TASK_RUNNING: u8 = 1;
const TASK_DONE: u8 = 2;
const TASK_CANCELLED: u8 = 3;

type ErasedTask = Box<dyn FnOnce() + Send + 'static>;

fn drop_erased_tasks(tasks: Vec<ErasedTask>) -> Option<Box<dyn Any + Send>> {
    let mut first_panic = None;
    for task in tasks {
        if let Err(panic) = catch_unwind(AssertUnwindSafe(|| drop(task)))
            && first_panic.is_none()
        {
            first_panic = Some(panic);
        }
    }
    first_panic
}

struct ScopeTask {
    batch_id: u64,
    state: AtomicU8,
    context: CarriedContext,
    work: Mutex<Option<ErasedTask>>,
    panic: Arc<Mutex<Option<Box<dyn Any + Send>>>>,
    refusals: Mutex<BTreeSet<String>>,
}

impl ScopeTask {
    fn try_claim(&self) -> bool {
        self.state
            .compare_exchange(
                TASK_PENDING,
                TASK_RUNNING,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    }

    fn cancel_if_pending(&self) -> Option<Option<ErasedTask>> {
        self.state
            .compare_exchange(
                TASK_PENDING,
                TASK_CANCELLED,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .ok()
            .map(|_| lock(&self.work).take())
    }

    fn is_terminal(&self) -> bool {
        matches!(
            self.state.load(Ordering::Acquire),
            TASK_DONE | TASK_CANCELLED
        )
    }

    fn run_claimed(&self) {
        if let Some(work) = lock(&self.work).take()
            && let Ok(((), refusals)) = catch_unwind(AssertUnwindSafe(|| {
                self.context.enter("content helper", work)
            }))
            && !refusals.is_empty()
        {
            lock(&self.refusals).extend(refusals);
        }
    }
}

struct TaskFinishGuard<'a> {
    batch: &'a ScopeBatch,
    task: &'a ScopeTask,
}

impl Drop for TaskFinishGuard<'_> {
    fn drop(&mut self) {
        self.task.state.store(TASK_DONE, Ordering::Release);
        self.batch.unfinished.fetch_sub(1, Ordering::AcqRel);
        self.batch.pool().notify_all();
    }
}

struct ScopeState {
    meta: BatchMeta,
    tasks: Vec<Arc<ScopeTask>>,
    queue: VecDeque<Arc<ScopeTask>>,
    pending: usize,
}

struct ScopeBatch {
    pool: *const Pool,
    state: Mutex<ScopeState>,
    has_pending: AtomicBool,
    unfinished: AtomicUsize,
}

// SAFETY: `ScopeBatch` is owned by `Pool::scope` and unregistered before `Pool::scope` returns;
// `pool` outlives `Pool::scope` and `Pool` is `Sync`.
unsafe impl Send for ScopeBatch {}
// SAFETY: same as `Send`.
unsafe impl Sync for ScopeBatch {}

impl ScopeBatch {
    fn new(pool: &Pool, root_id: u64) -> Self {
        Self {
            pool: ptr::from_ref(pool),
            state: Mutex::new(ScopeState {
                meta: BatchMeta {
                    root_id,
                    batch_id: 0,
                    parent_batch_id: None,
                },
                tasks: Vec::new(),
                queue: VecDeque::new(),
                pending: 0,
            }),
            has_pending: AtomicBool::new(false),
            unfinished: AtomicUsize::new(0),
        }
    }

    fn pool(&self) -> &Pool {
        // SAFETY: `ScopeBatch` lives inside `Pool::scope` while `&Pool` is borrowed.
        unsafe { &*self.pool }
    }

    fn unfinished(&self) -> usize {
        self.unfinished.load(Ordering::Acquire)
    }

    fn push_task(&self, task: Arc<ScopeTask>) {
        let pending = {
            let mut state = lock(&self.state);
            self.unfinished.fetch_add(1, Ordering::AcqRel);
            state.tasks.push(Arc::clone(&task));
            state.queue.push_back(task);
            state.pending += 1;
            self.has_pending.store(true, Ordering::Release);
            state.pending
        };
        self.pool().wake_helpers(pending);
    }

    fn claim_specific(&self, task: &ScopeTask) -> bool {
        let mut state = lock(&self.state);
        if task.try_claim() {
            state.pending -= 1;
            self.has_pending.store(state.pending > 0, Ordering::Release);
            true
        } else {
            false
        }
    }

    fn cancel_specific(&self, task: &ScopeTask) -> Option<Box<dyn Any + Send>> {
        let dropped = {
            let mut state = lock(&self.state);
            if let Some(work) = task.cancel_if_pending() {
                state.pending -= 1;
                self.unfinished.fetch_sub(1, Ordering::AcqRel);
                self.has_pending.store(state.pending > 0, Ordering::Release);
                Some(work)
            } else {
                None
            }
        };
        let work = dropped?;
        self.pool().notify_all();
        work.and_then(|work| catch_unwind(AssertUnwindSafe(|| drop(work))).err())
    }

    fn claim_next(&self) -> Option<Arc<ScopeTask>> {
        let mut state = lock(&self.state);
        while let Some(task) = state.queue.pop_front() {
            if task.try_claim() {
                state.pending -= 1;
                self.has_pending.store(state.pending > 0, Ordering::Release);
                return Some(task);
            }
        }
        self.has_pending.store(false, Ordering::Release);
        None
    }

    fn cancel_pending(&self) -> (usize, Option<Box<dyn Any + Send>>) {
        let (cancelled, dropped) = {
            let mut state = lock(&self.state);
            let mut cancelled = 0usize;
            let mut dropped = Vec::new();
            for task in &state.tasks {
                if let Some(work) = task.cancel_if_pending() {
                    cancelled += 1;
                    if let Some(work) = work {
                        dropped.push(work);
                    }
                }
            }
            state.pending -= cancelled;
            self.unfinished.fetch_sub(cancelled, Ordering::AcqRel);
            state.queue.clear();
            self.has_pending.store(false, Ordering::Release);
            (cancelled, dropped)
        };
        if cancelled > 0 {
            self.pool().notify_all();
        }
        let drop_panic = drop_erased_tasks(dropped);
        (cancelled, drop_panic)
    }

    fn run_task(&self, task: &ScopeTask) {
        let _finish = TaskFinishGuard { batch: self, task };
        let scope_meta = lock(&self.state).meta;
        let task_meta = BatchMeta {
            root_id: scope_meta.root_id,
            batch_id: task.batch_id,
            parent_batch_id: Some(scope_meta.batch_id),
        };
        let _link = self.pool().link_parent(task.batch_id, scope_meta.batch_id);
        let _active = self.pool().enter_batch(task_meta);
        task.run_claimed();
    }

    fn drain_pending_on_caller(&self) {
        while !is_root_cancelled(current_alive().as_ref())
            && let Some(task) = self.claim_next()
        {
            self.run_task(&task);
        }
    }

    fn credit_remaining_refusals(&self) {
        let tasks = lock(&self.state).tasks.clone();
        for task in tasks {
            let refusals = std::mem::take(&mut *lock(&task.refusals));
            if !refusals.is_empty() {
                crate::content::credit_overlay_observers(&refusals);
            }
        }
    }

    fn take_unobserved_panic(&self) -> (Option<Box<dyn Any + Send>>, bool) {
        let tasks = std::mem::take(&mut lock(&self.state).tasks);
        let mut first_panic = None;
        let mut any_cancelled = false;
        for task in tasks {
            if let Some(panic) = lock(&task.panic).take() {
                if is_root_cancelled_panic(&*panic) {
                    any_cancelled = true;
                } else if first_panic.is_none() {
                    first_panic = Some(panic);
                }
            }
        }
        (first_panic, any_cancelled)
    }
}

impl WorkBatch for ScopeBatch {
    fn has_claimable(&self) -> bool {
        self.has_pending.load(Ordering::Acquire)
    }

    fn run_turn(&self) {
        let Some(task) = self.claim_next() else {
            return;
        };
        self.run_task(&task);
    }
}

pub struct Scope<'scope, 'env: 'scope> {
    pool: &'scope Pool,
    batch: Arc<ScopeBatch>,
    _scope: PhantomData<&'scope mut &'scope ()>,
    _env: PhantomData<&'env mut &'env ()>,
}

impl<'scope, 'env> Scope<'scope, 'env> {
    pub fn spawn<T: Send + 'scope>(
        &'scope self,
        work: impl FnOnce() -> T + Send + 'scope,
    ) -> ScopedHandle<'scope, T> {
        let value: Arc<Mutex<Option<T>>> = Arc::new(Mutex::new(None));
        let panic_slot: Arc<Mutex<Option<Box<dyn Any + Send>>>> = Arc::new(Mutex::new(None));
        let output = Arc::clone(&value);
        let output_panic = Arc::clone(&panic_slot);
        let boxed: Box<dyn FnOnce() + Send + 'scope> = Box::new(move || {
            match catch_unwind(AssertUnwindSafe(work)) {
                Ok(result) => *lock(&output) = Some(result),
                Err(panic) => *lock(&output_panic) = Some(panic),
            }
        });
        // SAFETY: `Pool::scope` keeps `'scope` invariant, keeps `scope` alive until every running
        // task in `ScopeBatch` finishes, and cancels/drops pending closures before returning, so
        // `'scope` outlives every execution.
        let erased: ErasedTask = unsafe {
            std::mem::transmute::<Box<dyn FnOnce() + Send + 'scope>, ErasedTask>(boxed)
        };
        let task = Arc::new(ScopeTask {
            batch_id: NEXT_BATCH_ID.fetch_add(1, Ordering::Relaxed),
            state: AtomicU8::new(TASK_PENDING),
            context: CarriedContext::capture(),
            work: Mutex::new(Some(erased)),
            panic: panic_slot,
            refusals: Mutex::new(BTreeSet::new()),
        });
        self.batch.push_task(Arc::clone(&task));
        ScopedHandle {
            pool: self.pool,
            batch: Arc::clone(&self.batch),
            task,
            value,
            _scope: PhantomData,
        }
    }
}

pub struct ScopedHandle<'scope, T> {
    pool: &'scope Pool,
    batch: Arc<ScopeBatch>,
    task: Arc<ScopeTask>,
    value: Arc<Mutex<Option<T>>>,
    _scope: PhantomData<&'scope mut T>,
}

impl<T> ScopedHandle<'_, T> {
    pub fn join(self) -> std::thread::Result<T> {
        let scope_meta = lock(&self.batch.state).meta;
        if is_root_cancelled(current_alive().as_ref()) {
            if let Some(drop_panic) = self.batch.cancel_specific(&self.task) {
                resume_unwind(drop_panic);
            }
            resume_unwind(Box::new(RootCancelled));
        }
        if self.batch.claim_specific(&self.task) {
            self.batch.run_task(&self.task);
        } else {
            let task_meta = BatchMeta {
                root_id: scope_meta.root_id,
                batch_id: self.task.batch_id,
                parent_batch_id: Some(scope_meta.batch_id),
            };
            self.pool.wait_until(task_meta, || self.task.is_terminal());
        }
        crate::content::credit_overlay_observers(&std::mem::take(&mut *lock(&self.task.refusals)));
        if let Some(panic) = lock(&self.task.panic).take() {
            return Err(panic);
        }
        if let Some(value) = lock(&self.value).take() {
            return Ok(value);
        }
        if is_root_cancelled(current_alive().as_ref()) {
            return Err(Box::new(RootCancelled));
        }
        Err(Box::new("scoped task cancelled"))
    }
}

pub(crate) fn shared_pool() -> &'static Pool {
    SHARED.get_or_init(|| Pool::with_budget(usable_processors()))
}

pub fn map<T: Sync, R: Send>(items: &[T], work: impl Fn(&T) -> R + Sync) -> Vec<R> {
    shared_pool().map(items, work)
}

pub fn map_entering<T: Sync, R: Send>(
    items: &[T],
    enter: impl Fn() + Sync,
    work: impl Fn(&T) -> R + Sync,
) -> Vec<R> {
    shared_pool().map_entering(items, enter, work)
}

pub fn scope<'env, R>(work: impl for<'scope> FnOnce(&'scope Scope<'scope, 'env>) -> R) -> R {
    shared_pool().scope(work)
}

pub fn spawn<'scope, 'env, T: Send + 'scope>(
    scope: &'scope Scope<'scope, 'env>,
    work: impl FnOnce() -> T + Send + 'scope,
) -> ScopedHandle<'scope, T> {
    scope.spawn(work)
}

pub fn join<T>(task: ScopedHandle<'_, T>) -> T {
    task.join()
        .unwrap_or_else(|panic| resume_unwind(panic))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::HashSet,
        sync::Barrier,
        thread::ThreadId,
    };

    #[test]
    fn results_keep_their_items_places() {
        let items: Vec<usize> = (0..1000).collect();
        assert_eq!(
            super::map(&items, |item| item * 2),
            items.iter().map(|item| item * 2).collect::<Vec<_>>()
        );
        assert!(super::map(&[] as &[u8], |_| 0).is_empty());
        assert_eq!(super::map(&[7u8], |item| item + 1), [8]);
    }

    #[test]
    fn only_strictly_nested_child_batches_of_same_root_are_stolen_while_waiting() {
        let dag_root = BatchMeta {
            root_id: 1,
            batch_id: 1,
            parent_batch_id: None,
        };
        let waiting = BatchMeta {
            root_id: 1,
            batch_id: 10,
            parent_batch_id: Some(1),
        };
        let direct_child = BatchMeta {
            root_id: 1,
            batch_id: 11,
            parent_batch_id: Some(10),
        };
        let grandchild = BatchMeta {
            root_id: 1,
            batch_id: 12,
            parent_batch_id: Some(11),
        };
        let same_root_sibling = BatchMeta {
            root_id: 1,
            batch_id: 13,
            parent_batch_id: Some(1),
        };
        let other_root = BatchMeta {
            root_id: 2,
            batch_id: 14,
            parent_batch_id: None,
        };
        let batches = [
            dag_root,
            waiting,
            direct_child,
            grandchild,
            same_root_sibling,
            other_root,
        ];
        let parent_of = |id: u64| {
            batches
                .iter()
                .find(|meta| meta.batch_id == id)
                .and_then(|meta| meta.parent_batch_id)
        };
        assert!(is_strictly_nested_child_of_same_root(
            direct_child,
            waiting,
            batches.len(),
            parent_of,
        ));
        assert!(is_strictly_nested_child_of_same_root(
            grandchild,
            waiting,
            batches.len(),
            parent_of,
        ));
        assert!(!is_strictly_nested_child_of_same_root(
            waiting,
            waiting,
            batches.len(),
            parent_of,
        ));
        assert!(!is_strictly_nested_child_of_same_root(
            dag_root,
            waiting,
            batches.len(),
            parent_of,
        ));
        assert!(!is_strictly_nested_child_of_same_root(
            same_root_sibling,
            waiting,
            batches.len(),
            parent_of,
        ));
        assert!(!is_strictly_nested_child_of_same_root(
            other_root,
            waiting,
            batches.len(),
            parent_of,
        ));

        let pool = Arc::new(Pool::with_budget(2));
        let both_active = Arc::new(Barrier::new(2));
        let root_a_started = Arc::new(Barrier::new(2));
        let release_root_a_helper = Arc::new(Barrier::new(2));
        let root_a_caller_id = Mutex::new(None::<ThreadId>);
        let root_b_threads = Arc::new(Mutex::new(HashSet::<ThreadId>::new()));

        std::thread::scope(|threads| {
            let pool_a = Arc::clone(&pool);
            let active = Arc::clone(&both_active);
            let started = Arc::clone(&root_a_started);
            let release = Arc::clone(&release_root_a_helper);
            let caller_id_slot = &root_a_caller_id;
            let a = threads.spawn(move || {
                let caller = std::thread::current().id();
                *caller_id_slot.lock().unwrap() = Some(caller);
                let out = pool_a.map(&[0usize, 1], |&index| {
                    active.wait();
                    if std::thread::current().id() != caller {
                        started.wait();
                        release.wait();
                    }
                    index * 10
                });
                assert_eq!(out, [0, 10]);
            });

            root_a_started.wait();
            let pool_b = Arc::clone(&pool);
            let b_threads = Arc::clone(&root_b_threads);
            let b = threads.spawn(move || {
                let out = pool_b.map(&[1usize, 2, 3, 4], |&index| {
                    b_threads
                        .lock()
                        .unwrap()
                        .insert(std::thread::current().id());
                    index + 1
                });
                assert_eq!(out, [2, 3, 4, 5]);
            });
            b.join().unwrap();
            release_root_a_helper.wait();
            a.join().unwrap();
        });

        let root_a_caller = root_a_caller_id.into_inner().unwrap().unwrap();
        assert!(
            !root_b_threads.lock().unwrap().contains(&root_a_caller),
            "root A waiting on its own batch must not steal root B's batch"
        );

        let sibling_pool = Pool::with_budget(3);
        let jobs_ready = Barrier::new(2);
        let map_a_registered = Barrier::new(2);
        let map_a_helper_in = Barrier::new(2);
        let map_b_done = Barrier::new(2);
        let job_a_lock = Mutex::new(());
        let job_a_thread = Mutex::new(None::<ThreadId>);
        let map_b_threads = Mutex::new(HashSet::<ThreadId>::new());
        let mut dag = super::dag::Dag::default();
        let job_a = dag.add(&[], || {
            jobs_ready.wait();
            let _held = job_a_lock.lock().unwrap();
            let owner = std::thread::current().id();
            *job_a_thread.lock().unwrap() = Some(owner);
            let out = sibling_pool.map(&[0usize, 1], |&index| {
                if std::thread::current().id() == owner {
                    map_a_helper_in.wait();
                    map_a_registered.wait();
                } else {
                    map_a_helper_in.wait();
                    map_b_done.wait();
                }
                index + 10
            });
            Ok(out)
        });
        let job_b = dag.add(&[], || {
            jobs_ready.wait();
            map_a_registered.wait();
            let out = sibling_pool.map(&[1usize, 2, 3, 4], |&index| {
                let current = std::thread::current().id();
                map_b_threads.lock().unwrap().insert(current);
                if Some(current) == *job_a_thread.lock().unwrap() {
                    let _would_deadlock = job_a_lock.lock().unwrap();
                }
                index * 2
            });
            map_b_done.wait();
            Ok(out)
        });
        dag.run_in(&sibling_pool);
        assert_eq!(job_a.take().unwrap(), [10, 11]);
        assert_eq!(job_b.take().unwrap(), [2, 4, 6, 8]);
        let job_a_id = job_a_thread.into_inner().unwrap().unwrap();
        assert!(
            !map_b_threads.into_inner().unwrap().contains(&job_a_id),
            "sibling DAG job A waiting on its map must not steal sibling job B's higher-ID map"
        );

        let scope_pool = Pool::with_budget(2);
        let nested_map_open = Barrier::new(2);
        let nested_stolen = Barrier::new(2);
        let caller_in_item_0 = AtomicBool::new(false);
        let reentered_outer = AtomicBool::new(false);
        let scope_caller = std::thread::current().id();
        scope_pool.scope(|scope| {
            let open_ref = &nested_map_open;
            let stolen_ref = &nested_stolen;
            let pool_ref = &scope_pool;
            let running = scope.spawn(move || {
                pool_ref.map(&[10usize, 20], |&inner| {
                    if inner == 10 {
                        open_ref.wait();
                    }
                    stolen_ref.wait();
                    inner + 1
                })
            });
            nested_map_open.wait();
            let running_slot = Mutex::new(Some(running));
            let out = scope_pool.map(&[0usize, 1, 2], |&index| {
                if index == 0 {
                    caller_in_item_0.store(true, Ordering::Release);
                    let handle = running_slot.lock().unwrap().take().unwrap();
                    let inner = handle.join().unwrap();
                    caller_in_item_0.store(false, Ordering::Release);
                    inner.into_iter().sum::<usize>()
                } else {
                    if std::thread::current().id() == scope_caller
                        && caller_in_item_0.load(Ordering::Acquire)
                    {
                        reentered_outer.store(true, Ordering::Release);
                    }
                    index * 10
                }
            });
            assert_eq!(out, [32, 10, 20]);
        });
        assert!(
            !reentered_outer.load(Ordering::Acquire),
            "joining a running scoped task must steal its nested map without reentering the outer map"
        );
    }

    #[test]
    fn nested_maps_inside_dag_jobs_stay_within_worker_budget() {
        for budget in [1usize, 2, 4] {
            let pool = Pool::with_budget(budget);
            let active = AtomicUsize::new(0);
            let peak = AtomicUsize::new(0);
            let threads = Mutex::new(HashSet::<ThreadId>::new());
            let items = [0usize, 1, 2, 3, 4, 5, 6, 7];
            for _round in 0..2 {
                let mut dag = super::dag::Dag::default();
                let tasks: Vec<_> = (0..8usize)
                    .map(|job| {
                        let pool = &pool;
                        let active = &active;
                        let peak = &peak;
                        let threads = &threads;
                        let items = &items;
                        dag.add(&[], move || {
                            lock(threads).insert(std::thread::current().id());
                            let out = pool.map(items, |&item| {
                                let now = active.fetch_add(1, Ordering::AcqRel) + 1;
                                peak.fetch_max(now, Ordering::Relaxed);
                                lock(threads).insert(std::thread::current().id());
                                let value = job * 100 + item;
                                active.fetch_sub(1, Ordering::AcqRel);
                                value
                            });
                            Ok(out)
                        })
                    })
                    .collect();
                dag.run_in(&pool);
                for (job, task) in tasks.into_iter().enumerate() {
                    let expected: Vec<usize> = items.iter().map(|&item| job * 100 + item).collect();
                    assert_eq!(task.take().unwrap(), expected);
                }
            }
            assert!(peak.load(Ordering::Acquire) <= budget);
            assert!(threads.into_inner().unwrap().len() <= budget);
            assert!(pool.spawned_helpers() <= budget.saturating_sub(1));
            assert_eq!(pool.queued_batches(), 0);
        }
    }

    #[test]
    fn budget_of_one_runs_nested_maps_dag_and_scope_inline_with_no_helpers() {
        let pool = Pool::with_budget(1);
        let caller = std::thread::current().id();
        let mut dag = super::dag::Dag::default();
        let first = dag.add(&[], || {
            assert_eq!(std::thread::current().id(), caller);
            let nested = pool.scope(|scope| {
                let spawned = scope.spawn(|| {
                    assert_eq!(std::thread::current().id(), caller);
                    pool.map(&[1usize, 2, 3], |&outer| {
                        assert_eq!(std::thread::current().id(), caller);
                        pool.map(&[10usize, 20], |&inner| outer + inner)
                    })
                });
                spawned.join().unwrap()
            });
            Ok(nested)
        });
        let second_input = first.clone();
        let second = dag.add(&[first.id()], move || {
            assert_eq!(std::thread::current().id(), caller);
            let rows = second_input.read()?;
            Ok(rows.iter().flatten().sum::<usize>())
        });
        dag.run_in(&pool);
        assert_eq!(first.take().unwrap(), [[11, 21], [12, 22], [13, 23]]);
        assert_eq!(second.take().unwrap(), 102);
        assert_eq!(pool.spawned_helpers(), 0);
        assert_eq!(pool.queued_batches(), 0);
    }

    #[test]
    fn thread_local_state_reaches_workers_and_restores_afterward_including_on_panic() {
        static MARKER: u8 = 7;
        let pool = Pool::with_budget(2);
        let caller = std::thread::current().id();
        let _generation = crate::content::Generation::enter();
        let expected_generation = Arc::new(crate::content::generation::Captured::capture());
        assert!(expected_generation.is_some());
        let items = [0usize, 1];
        let (((), observed_refusals), observed_diagnostics) =
            crate::content::diagnostics::observe(|| {
                crate::content::observe_overlay_refusals(|| {
                    crate::content::strict(|| {
                        job_priority::JobPriority::run(Arc::new(AtomicBool::new(true)), || {
                            job_priority::with_context(Some(&MARKER), || {
                                let barrier = Barrier::new(2);
                                let map_ok = pool.map(&items, |&index| {
                                    barrier.wait();
                                    assert!(crate::content::is_strict());
                                    assert!(job_priority::JobPriority::current().background());
                                    assert!(job_priority::context().is_some());
                                    let current = crate::content::generation::Captured::capture();
                                    assert!(current.is_same(&expected_generation));
                                    crate::content::diagnostics::Diagnostics::note(format!(
                                        "map-diag-{index}"
                                    ));
                                    crate::content::credit_overlay_refusals(&BTreeSet::from([
                                        format!("map-refusal-{index}"),
                                    ]));
                                    index * 3
                                });
                                assert_eq!(map_ok, [0, 3]);

                                let dag_barrier = Barrier::new(2);
                                let mut dag = super::dag::Dag::default();
                                let tasks: Vec<_> = items
                                    .iter()
                                    .map(|&index| {
                                        let expected = Arc::clone(&expected_generation);
                                        let dag_barrier = &dag_barrier;
                                        dag.add(&[], move || {
                                            dag_barrier.wait();
                                            assert!(crate::content::is_strict());
                                            assert!(
                                                job_priority::JobPriority::current().background()
                                            );
                                            assert!(job_priority::context().is_some());
                                            let current =
                                                crate::content::generation::Captured::capture();
                                            assert!(current.is_same(&expected));
                                            crate::content::diagnostics::Diagnostics::note(
                                                format!("dag-diag-{index}"),
                                            );
                                            crate::content::credit_overlay_refusals(
                                                &BTreeSet::from([format!("dag-refusal-{index}")]),
                                            );
                                            Ok(index * 5)
                                        })
                                    })
                                    .collect();
                                dag.run_in(&pool);
                                let dag_ok: Vec<usize> =
                                    tasks.into_iter().map(|task| task.take().unwrap()).collect();
                                assert_eq!(dag_ok, [0, 5]);

                                let scope_barrier = Barrier::new(2);
                                let spawned_value = pool.scope(|scope| {
                                    let handle = scope.spawn(|| {
                                        scope_barrier.wait();
                                        assert!(crate::content::is_strict());
                                        assert!(job_priority::JobPriority::current().background());
                                        assert!(job_priority::context().is_some());
                                        crate::content::credit_overlay_refusals(&BTreeSet::from([
                                            "scope-refusal".to_owned(),
                                        ]));
                                        42usize
                                    });
                                    let _unjoined = scope.spawn(|| {
                                        crate::content::credit_overlay_refusals(&BTreeSet::from([
                                            "scope-unjoined-refusal".to_owned(),
                                        ]));
                                    });
                                    scope_barrier.wait();
                                    super::join(handle)
                                });
                                assert_eq!(spawned_value, 42);
                            });
                        });
                    });
                })
            });
        for index in items {
            assert!(observed_refusals.contains(&format!("map-refusal-{index}")));
            assert!(observed_refusals.contains(&format!("dag-refusal-{index}")));
            assert!(
                observed_diagnostics
                    .shader_warnings()
                    .contains(&format!("map-diag-{index}"))
            );
            assert!(
                observed_diagnostics
                    .shader_warnings()
                    .contains(&format!("dag-diag-{index}"))
            );
        }
        assert!(observed_refusals.contains("scope-refusal"));
        assert!(observed_refusals.contains("scope-unjoined-refusal"));

        let panic_barrier = Barrier::new(2);
        let helper_panicked = catch_unwind(AssertUnwindSafe(|| {
            crate::content::strict(|| {
                job_priority::JobPriority::run(Arc::new(AtomicBool::new(true)), || {
                    job_priority::with_context(Some(&MARKER), || {
                        pool.map(&items, |_| {
                            panic_barrier.wait();
                            if std::thread::current().id() != caller {
                                panic!("helper unwind boom");
                            }
                        });
                    });
                });
            });
        }));
        assert!(helper_panicked.is_err());

        drop(_generation);
        let clean_barrier = Barrier::new(2);
        let helper_clean = pool.map(&items, |_| {
            clean_barrier.wait();
            (
                crate::content::is_strict(),
                job_priority::JobPriority::current().background(),
                job_priority::context().is_some(),
                crate::content::generation::Captured::capture().is_some(),
                crate::content::diagnostics::is_active(),
            )
        });
        assert_eq!(
            helper_clean,
            [
                (false, false, false, false, false),
                (false, false, false, false, false),
            ]
        );

        let caller_restore = pool.map(&items, |_| {
            job_priority::set_context(Some(&MARKER));
            job_priority::context().is_some()
        });
        assert_eq!(caller_restore, [true, true]);
        assert!(job_priority::context().is_none());

        let inline_pool = Pool::with_budget(1);
        let inline_panicked = catch_unwind(AssertUnwindSafe(|| {
            inline_pool.map(&[0usize], |_| {
                job_priority::set_context(Some(&MARKER));
                panic!("inline caller unwind");
            });
        }));
        assert!(inline_panicked.is_err());
        assert!(job_priority::context().is_none());

        let measurement = cpu::Measurement::start();
        for _ in 0..4 {
            let turn_barrier = Barrier::new(2);
            let _ = pool.map(&items, |&value| {
                turn_barrier.wait();
                value + 1
            });
        }
        let sample = measurement.finish();
        assert_eq!(sample.threads, 2);
    }

    #[test]
    fn worker_panics_in_map_dag_and_scope_surface_and_leave_pool_usable() {
        let pool = Pool::with_budget(2);
        let caller = std::thread::current().id();

        let barrier = Barrier::new(2);
        let map_panic = catch_unwind(AssertUnwindSafe(|| {
            pool.map(&[0usize, 1], |_| {
                barrier.wait();
                if std::thread::current().id() != caller {
                    panic!("map helper boom");
                }
            });
        }));
        assert!(map_panic.is_err());

        let dag_barrier = Barrier::new(2);
        let dag_panic = catch_unwind(AssertUnwindSafe(|| {
            let mut dag = super::dag::Dag::default();
            let _a = dag.add(&[], || -> std::io::Result<()> {
                dag_barrier.wait();
                if std::thread::current().id() != caller {
                    panic!("dag helper boom");
                }
                Ok(())
            });
            let _b = dag.add(&[], || -> std::io::Result<()> {
                dag_barrier.wait();
                if std::thread::current().id() != caller {
                    panic!("dag helper boom");
                }
                Ok(())
            });
            dag.run_in(&pool);
        }));
        assert!(dag_panic.is_err());

        let scope_barrier = Barrier::new(2);
        let scope_panic = catch_unwind(AssertUnwindSafe(|| {
            pool.scope(|scope| {
                let handle = scope.spawn(|| {
                    scope_barrier.wait();
                    panic!("scope helper boom");
                });
                scope_barrier.wait();
                super::join(handle);
            });
        }));
        assert!(scope_panic.is_err());

        let unjoined_panic = catch_unwind(AssertUnwindSafe(|| {
            pool.scope(|scope| {
                let _dropped = scope.spawn(|| {
                    panic!("unjoined scope boom");
                });
            });
        }));
        assert!(unjoined_panic.is_err());

        let child_started = Barrier::new(2);
        let sibling_join_errored = AtomicBool::new(false);
        let sibling_cancel_panic = catch_unwind(AssertUnwindSafe(|| {
            pool.scope(|scope| {
                let (send_sibling, recv_sibling) = std::sync::mpsc::channel();
                let child_started_ref = &child_started;
                let errored_ref = &sibling_join_errored;
                let _child = scope.spawn(move || {
                    child_started_ref.wait();
                    let sibling: ScopedHandle<'_, usize> = recv_sibling.recv().unwrap();
                    while sibling.task.state.load(Ordering::Acquire) != TASK_CANCELLED {
                        std::thread::yield_now();
                    }
                    errored_ref.store(sibling.join().is_err(), Ordering::Release);
                });
                child_started.wait();
                let sibling = scope.spawn(|| 99usize);
                send_sibling.send(sibling).unwrap();
                panic!("scope body boom");
            });
        }));
        assert!(sibling_cancel_panic.is_err());
        assert!(sibling_join_errored.load(Ordering::Acquire));

        let grandchild_ran = AtomicBool::new(false);
        let late_spawn_ready = Barrier::new(2);
        pool.scope(|scope| {
            let flag = &grandchild_ran;
            let ready = &late_spawn_ready;
            let _child = scope.spawn(move || {
                ready.wait();
                let _grandchild = scope.spawn(move || {
                    flag.store(true, Ordering::Release);
                });
            });
            late_spawn_ready.wait();
        });
        assert!(grandchild_ran.load(Ordering::Acquire));

        let drop_ran = AtomicBool::new(false);
        let child_finished = AtomicBool::new(false);
        let child_in_scope = Barrier::new(2);
        let drop_panic_result = catch_unwind(AssertUnwindSafe(|| {
            pool.scope(|scope| {
                let child_in = &child_in_scope;
                let drop_seen = &drop_ran;
                let done_flag = &child_finished;
                let _running = scope.spawn(move || {
                    child_in.wait();
                    while !drop_seen.load(Ordering::Acquire) {
                        std::thread::yield_now();
                    }
                    let _late = scope.spawn(|| 1usize);
                    done_flag.store(true, Ordering::Release);
                });
                child_in_scope.wait();
                let other_pending = scope.spawn(|| 2usize);
                struct DropJoinAndPanic<'a, 's> {
                    other: Option<ScopedHandle<'s, usize>>,
                    drop_ran: &'a AtomicBool,
                }
                impl Drop for DropJoinAndPanic<'_, '_> {
                    fn drop(&mut self) {
                        if let Some(handle) = self.other.take() {
                            let _ = handle.join();
                        }
                        self.drop_ran.store(true, Ordering::Release);
                        panic!("cancelled closure drop boom");
                    }
                }
                let bomb = DropJoinAndPanic {
                    other: Some(other_pending),
                    drop_ran: &drop_ran,
                };
                let _doomed = scope.spawn(move || {
                    let _keep = bomb;
                });
                panic!("trigger cancel drop");
            });
        }));
        assert!(drop_panic_result.is_err());
        assert!(drop_ran.load(Ordering::Acquire));
        assert!(child_finished.load(Ordering::Acquire));

        let still_concurrent = Barrier::new(2);
        let recovered = pool.map(&[10usize, 20], |&value| {
            still_concurrent.wait();
            value + 1
        });
        assert_eq!(recovered, [11, 21]);
        assert_eq!(pool.queued_batches(), 0);
    }

    #[test]
    fn cancelled_root_stops_taking_batches_clears_stack_and_next_load_runs_normally() {
        let pool = Pool::with_budget(2);
        let alive = Arc::new(AtomicBool::new(true));
        let started = AtomicUsize::new(0);
        let mut dag = super::dag::Dag::default();
        let first = dag.add(&[], || {
            started.fetch_add(1, Ordering::AcqRel);
            alive.store(false, Ordering::Release);
            Ok(1usize)
        });
        let second = dag.add(&[first.id()], || {
            started.fetch_add(1, Ordering::AcqRel);
            Ok(2usize)
        });
        let third = dag.add(&[second.id()], || {
            started.fetch_add(1, Ordering::AcqRel);
            Ok(3usize)
        });
        let ran = with_alive(Arc::clone(&alive), || dag.run_in(&pool));
        assert!(ran.is_some());

        assert_eq!(started.load(Ordering::Acquire), 1);
        assert_eq!(first.take().unwrap(), 1);
        assert_eq!(
            second.take().unwrap_err().kind(),
            std::io::ErrorKind::Interrupted
        );
        assert_eq!(
            third.take().unwrap_err().kind(),
            std::io::ErrorKind::Interrupted
        );
        assert_eq!(pool.queued_batches(), 0);

        let map_alive = Arc::new(AtomicBool::new(true));
        let map_worked = AtomicUsize::new(0);
        let many: Vec<usize> = (0..64).collect();
        let map_outcome = with_alive(Arc::clone(&map_alive), || {
            pool.map(&many, |&index| {
                map_worked.fetch_add(1, Ordering::AcqRel);
                map_alive.store(false, Ordering::Release);
                index
            })
        });
        assert!(map_outcome.is_none());
        assert!(map_worked.load(Ordering::Acquire) < many.len());
        assert_eq!(pool.queued_batches(), 0);

        let inline_pool = Pool::with_budget(1);
        let inline_alive = Arc::new(AtomicBool::new(true));
        let inline_worked = AtomicUsize::new(0);
        let inline_outcome = with_alive(Arc::clone(&inline_alive), || {
            inline_pool.map(&[0usize, 1, 2, 3], |&index| {
                inline_worked.fetch_add(1, Ordering::AcqRel);
                inline_alive.store(false, Ordering::Release);
                index
            })
        });
        assert!(inline_outcome.is_none());
        assert_eq!(inline_worked.load(Ordering::Acquire), 1);

        let scope_alive = Arc::new(AtomicBool::new(true));
        let unjoined_ran = AtomicBool::new(false);
        let scope_outcome = with_alive(Arc::clone(&scope_alive), || {
            inline_pool.scope(|scope| {
                let flag = &unjoined_ran;
                let _unjoined = scope.spawn(move || {
                    flag.store(true, Ordering::Release);
                });
                scope_alive.store(false, Ordering::Release);
                123usize
            })
        });
        assert!(scope_outcome.is_none());
        assert!(!unjoined_ran.load(Ordering::Acquire));
        assert_eq!(inline_pool.queued_batches(), 0);

        let drop_cancel_alive = Arc::new(AtomicBool::new(true));
        let drop_cancel_panic = catch_unwind(AssertUnwindSafe(|| {
            with_alive(Arc::clone(&drop_cancel_alive), || {
                inline_pool.scope(|scope| {
                    struct PanicOnDrop;
                    impl Drop for PanicOnDrop {
                        fn drop(&mut self) {
                            panic!("cancel specific drop boom");
                        }
                    }
                    let guard = PanicOnDrop;
                    let handle = scope.spawn(move || {
                        let _keep = guard;
                        1usize
                    });
                    drop_cancel_alive.store(false, Ordering::Release);
                    let _ = handle.join();
                });
            })
        }));
        assert!(drop_cancel_panic.is_err());
        assert_eq!(inline_pool.queued_batches(), 0);

        let child_panic_alive = Arc::new(AtomicBool::new(true));
        let child_panicked_before_cancel = Barrier::new(2);
        let child_vs_cancel = catch_unwind(AssertUnwindSafe(|| {
            with_alive(Arc::clone(&child_panic_alive), || {
                pool.scope(|scope| {
                    let sync = &child_panicked_before_cancel;
                    let _failing = scope.spawn(move || {
                        sync.wait();
                        panic!("genuine child panic during cancel");
                    });
                    child_panicked_before_cancel.wait();
                    let pending = scope.spawn(|| 2usize);
                    child_panic_alive.store(false, Ordering::Release);
                    let _ = pending.join();
                });
            })
        }));
        assert!(child_vs_cancel.is_err());
        assert_eq!(pool.queued_batches(), 0);

        let nested_alive = Arc::new(AtomicBool::new(true));
        let mut nested_dag = super::dag::Dag::default();
        let nested_node = nested_dag.add(&[], || {
            let out = pool.map(&many, |&index| {
                nested_alive.store(false, Ordering::Release);
                index
            });
            Ok(out)
        });
        let nested_ran = with_alive(Arc::clone(&nested_alive), || nested_dag.run_in(&pool));
        assert!(nested_ran.is_some());
        assert_eq!(
            nested_node.take().unwrap_err().kind(),
            std::io::ErrorKind::Interrupted
        );
        assert_eq!(pool.queued_batches(), 0);

        let next_barrier = Barrier::new(2);
        let next_out = pool.map(&[1usize, 2], |&value| {
            next_barrier.wait();
            value * 10
        });
        assert_eq!(next_out, [10, 20]);
        assert_eq!(pool.queued_batches(), 0);
    }
}
